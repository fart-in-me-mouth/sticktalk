//! Online SQLite snapshots sent to the private storage node, never a live DB/WAL copy.
use crate::{
    blob::{BlobStore, LocalDisk, RemoteStore},
    error::{require, Error, Result},
    state::{now, Config},
};
use sqlx::{Connection, Row};
use sticktalk_proto::ErrorCode;

async fn replicate(remote: &RemoteStore, local: &LocalDisk, key: &str, size: u64) -> Result<()> {
    let stat = match remote.stat(key).await {
        Ok(s) => Some(s),
        Err(e) if e.0.code == ErrorCode::NotFound => None,
        Err(e) => return Err(e),
    };
    if stat.as_ref().is_some_and(|s| s.complete && s.size == size) {
        return Ok(());
    }
    let mut offset = stat.map_or(0, |s| s.size).min(size);
    loop {
        let length = (size - offset).min(crate::resumable::CHUNK);
        let sha = crate::blob::hash_region(&local.path(key)?, offset, length).await?;
        remote
            .put_chunk(
                key,
                offset,
                length,
                &sha,
                local.get(key, offset, length).await?,
            )
            .await?;
        offset += length;
        if offset == size {
            break;
        }
    }
    remote
        .finalize(
            key,
            size,
            &crate::blob::hash_region(&local.path(key)?, 0, size).await?,
        )
        .await
}
pub async fn run(config: &Config) -> Result<()> {
    let tier = crate::tier::Tier::new(config)?;
    let remote = tier
        .remote
        .as_ref()
        .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?;
    // Also drives bounded tombstone cleanup on the Pi.
    remote.free_space().await?;
    let dir = config.data_dir.join("backups");
    tokio::fs::create_dir_all(&dir).await?;
    let key = format!("backup-{}.sqlite3", now());
    let snapshot = dir.join(&key);
    let mut db = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(config.data_dir.join("sticktalk.sqlite3"))
            .busy_timeout(std::time::Duration::from_secs(30)),
    )
    .await?;
    sqlx::query("VACUUM INTO ?")
        .bind(snapshot.to_string_lossy().as_ref())
        .execute(&mut db)
        .await?;
    drop(db);
    let result = async {
        let mut backup = sqlx::SqliteConnection::connect_with(&sqlx::sqlite::SqliteConnectOptions::new().filename(&snapshot).read_only(true)).await?;
        let check: String = sqlx::query_scalar("PRAGMA quick_check").fetch_one(&mut backup).await?;
        require(check == "ok",ErrorCode::Unavailable)?;
        let mut after = String::new();
        loop {
            let rows = sqlx::query("SELECT key,size FROM (SELECT storage_key AS key,byte_size AS size FROM uploads UNION ALL SELECT thumbnail_key,thumbnail_size FROM uploads WHERE thumbnail_key IS NOT NULL) WHERE key>? ORDER BY key LIMIT 64").bind(&after).fetch_all(&mut backup).await?;
            if rows.is_empty() { break; }
            for row in rows {
                let key: String = row.get("key");
                replicate(remote,&tier.local,&key,row.get::<i64,_>("size") as u64).await?;
                after = key;
            }
        }
        replicate(remote,&LocalDisk::new(dir.clone()),&key,tokio::fs::metadata(&snapshot).await?.len()).await?;
        println!("Verified backup stored on Pi: {key}");
        Ok(())
    }.await;
    let _ = tokio::fs::remove_file(&snapshot).await;
    result
}
