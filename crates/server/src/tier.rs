//! Durable write-back tier. The authoritative upload/session offsets stay in the
//! chat database; only hash-verified, fsynced Pi replicas can be evicted locally.
use crate::{
    blob::{self, BlobStore, LocalDisk, RemoteStore},
    error::Result,
    state::{now, App},
};
use axum::body::Body;
use sqlx::Row;
use tokio::sync::Mutex;

pub(crate) struct Tier {
    pub local: LocalDisk,
    pub remote: Option<RemoteStore>,
    pub io: Mutex<()>,
    pub wake: tokio::sync::Notify,
}
impl Tier {
    pub fn new(config: &crate::Config) -> Result<Self> {
        let options = &config.operations.storage;
        let remote = options
            .storage_url
            .as_ref()
            .map(|url| RemoteStore::new(url, options.storage_secret.as_deref().unwrap_or_default()))
            .transpose()?;
        Ok(Self {
            local: LocalDisk::new(config.data_dir.join("files")),
            remote,
            io: Mutex::new(()),
            wake: tokio::sync::Notify::new(),
        })
    }
    pub async fn get(&self, app: &App, key: &str, start: u64, length: u64) -> Result<Body> {
        match self.local.get(key, start, length).await {
            Ok(body) => {
                if self.remote.is_some() {
                    sqlx::query("UPDATE blob_replicas SET accessed_at=? WHERE key=?")
                        .bind(now() as i64)
                        .bind(key)
                        .execute(&app.0.pool)
                        .await?;
                }
                Ok(body)
            }
            Err(e) if e.0.code == sticktalk_proto::ErrorCode::NotFound => match &self.remote {
                Some(remote) => remote
                    .get(key, start, length)
                    .await
                    .map_err(|_| blob::offline()),
                None => Err(e),
            },
            Err(e) => Err(e),
        }
    }
}
/// Mirror a durable prefix, or a complete published object. On any remote error
/// leave the local copy and queue metadata untouched for a later retry.
pub(crate) async fn mirror(app: &App, key: &str, size: u64, complete: bool) -> Result<()> {
    let Some(remote) = &app.0.tier.remote else {
        return Ok(());
    };
    let _serial = app.0.tier.io.lock().await;
    mirror_locked(app, remote, key, size, complete).await
}
async fn mirror_locked(
    app: &App,
    remote: &RemoteStore,
    key: &str,
    size: u64,
    complete: bool,
) -> Result<()> {
    // Register before networking: cancellation cannot strand an untracked Pi blob.
    sqlx::query("INSERT OR IGNORE INTO blob_replicas VALUES(?,?,?,0)")
        .bind(key)
        .bind(size as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await?;
    let stat = match remote.stat(key).await {
        Ok(s) => Some(s),
        Err(e) if e.0.code == sticktalk_proto::ErrorCode::NotFound => None,
        Err(e) => return Err(e),
    };
    let mut offset = stat.as_ref().map_or(0, |s| s.size).min(size);
    let already_complete = stat.as_ref().is_some_and(|s| s.complete);
    if already_complete {
        crate::error::require(
            complete && stat.as_ref().unwrap().size == size,
            sticktalk_proto::ErrorCode::Conflict,
        )?;
    } else {
        while offset < size || (size == 0 && stat.is_none()) {
            let len = (size - offset).min(crate::resumable::CHUNK);
            let sha = blob::hash_region(&app.0.tier.local.path(key)?, offset, len).await?;
            let body = app.0.tier.local.get(key, offset, len).await?;
            remote.put_chunk(key, offset, len, &sha, body).await?;
            offset += len;
            if size == 0 {
                break;
            }
        }
    }
    if complete {
        // An immutable, previously verified object may outlive its local spool
        // or replica ledger (e.g. restoring a DB backup onto a new Oracle VM).
        if !already_complete {
            let sha = blob::hash_region(&app.0.tier.local.path(key)?, 0, size).await?;
            remote.finalize(key, size, &sha).await?;
        }
        sqlx::query("UPDATE blob_replicas SET byte_size=?,synced=1 WHERE key=?")
            .bind(size as i64)
            .bind(key)
            .execute(&app.0.pool)
            .await?;
    }
    Ok(())
}
/// One bounded batch per wake/retry, plus immediate publication attempts.
/// No idle polling and no network under the chat gate.
pub(crate) async fn maintain(app: &App) -> Result<bool> {
    let Some(remote) = &app.0.tier.remote else {
        return Ok(false);
    };
    let Ok(_serial) = app.0.tier.io.try_lock() else {
        return Ok(true);
    };
    // Never race a chunk fsync/offset update, finalization, or cancellation.
    let Ok(_uploads) = app.0.upload_io.try_lock() else {
        return Ok(true);
    };
    // Reclaim expired tombstones while real work is pending, including when
    // the node is too full to accept another chunk. Never poll an idle node.
    remote.free_space().await?;
    let stale: Vec<String> = sqlx::query_scalar("SELECT key FROM blob_replicas r WHERE NOT EXISTS(SELECT 1 FROM uploads WHERE storage_key=r.key OR thumbnail_key=r.key) AND NOT EXISTS(SELECT 1 FROM partial_uploads WHERE storage_key=r.key OR thumbnail_key=r.key) LIMIT 32").fetch_all(&app.0.pool).await?;
    let mut more = stale.len() == 32;
    for key in stale {
        remote.delete(&key).await?;
        sqlx::query("DELETE FROM blob_replicas WHERE key=?")
            .bind(key)
            .execute(&app.0.pool)
            .await?;
    }
    let rows = sqlx::query("SELECT key,size FROM (SELECT storage_key AS key,byte_size AS size FROM uploads UNION ALL SELECT thumbnail_key,thumbnail_size FROM uploads WHERE thumbnail_key IS NOT NULL) b WHERE NOT EXISTS(SELECT 1 FROM blob_replicas r WHERE r.key=b.key AND r.synced=1) LIMIT 8").fetch_all(&app.0.pool).await?;
    more |= rows.len() == 8;
    for row in rows {
        mirror_locked(
            app,
            remote,
            &row.get::<String, _>("key"),
            row.get::<i64, _>("size") as u64,
            true,
        )
        .await?;
    }
    evict_locked(app).await?;
    Ok(more)
}
async fn evict_locked(app: &App) -> Result<()> {
    let options = &app.0.config.operations.storage;
    // Scan LRU in bounded pages; do not hold an unbounded in-memory file index.
    let mut after = (i64::MIN, String::new());
    let mut total = 0u64;
    loop {
        let rows = sqlx::query("SELECT key,byte_size,accessed_at FROM blob_replicas r WHERE synced=1 AND EXISTS(SELECT 1 FROM uploads WHERE storage_key=r.key OR thumbnail_key=r.key) AND (accessed_at,key)>(?,?) ORDER BY accessed_at,key LIMIT 128").bind(after.0).bind(&after.1).fetch_all(&app.0.pool).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let key: String = row.get("key");
            after = (row.get("accessed_at"), key.clone());
            if app.0.tier.local.stat(&key).await.is_ok() {
                total = total.saturating_add(row.get::<i64, _>("byte_size") as u64);
            }
        }
    }
    after = (i64::MIN, String::new());
    loop {
        let rows = sqlx::query("SELECT key,byte_size,accessed_at FROM blob_replicas r WHERE synced=1 AND EXISTS(SELECT 1 FROM uploads WHERE storage_key=r.key OR thumbnail_key=r.key) AND (accessed_at,key)>(?,?) ORDER BY accessed_at,key LIMIT 128").bind(after.0).bind(&after.1).fetch_all(&app.0.pool).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let key: String = row.get("key");
            after = (row.get("accessed_at"), key.clone());
            let size = row.get::<i64, _>("byte_size") as u64;
            let pin: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM uploads WHERE storage_key=? AND purpose!='message')",
            )
            .bind(&key)
            .fetch_one(&app.0.pool)
            .await?;
            if ((!pin
                && !key.ends_with(".jpg")
                && size > options.cache_file_mb.saturating_mul(1024 * 1024))
                || total > options.cache_mb.saturating_mul(1024 * 1024))
                && app.0.tier.local.stat(&key).await.is_ok()
            {
                app.0.tier.local.delete(&key).await?;
                total = total.saturating_sub(size);
            }
        }
    }
    Ok(())
}
pub(crate) async fn published(app: &App, key: &str, thumb: Option<&str>) {
    if app.0.tier.remote.is_none() {
        return;
    }
    for key in std::iter::once(key).chain(thumb) {
        if let Ok(s) = app.0.tier.local.stat(key).await {
            if mirror(app, key, s.size, true).await.is_err() {
                tracing::warn!("storage replica pending; local copy retained");
                app.0.tier.wake.notify_one();
                break;
            }
        }
    }
    // Eviction is serialized with upload finalization by the caller.
    let _serial = app.0.tier.io.lock().await;
    let _ = evict_locked(app).await;
}

/// No idle timer: wake on mutations, and retry only while a queue is pending.
pub(crate) async fn background(app: App) {
    let mut delay = Some(std::time::Duration::ZERO);
    loop {
        tokio::select! {
            _ = app.0.shutdown.cancelled() => break,
            _ = app.0.tier.wake.notified() => {},
            _ = async { match delay { Some(d) => tokio::time::sleep(d).await, None => std::future::pending().await } } => {},
        }
        delay = match maintain(&app).await {
            Ok(false) => None,
            Ok(true) => Some(std::time::Duration::from_secs(1)),
            Err(_) => {
                tracing::warn!("storage sync deferred");
                Some(std::time::Duration::from_secs(60))
            }
        };
    }
}
