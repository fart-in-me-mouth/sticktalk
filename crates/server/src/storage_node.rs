//! Private, HMAC-authenticated storage-node protocol. No chat or admin endpoints.
use crate::{
    blob::{self, BlobStore, LocalDisk},
    error::{require, Error, Result},
    state::now,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::Response,
    routing::get,
};
use ring::hmac;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use sticktalk_proto::ErrorCode;
use tokio::sync::{Mutex, Semaphore};

pub fn private_address(ip: IpAddr) -> bool {
    ip.is_loopback() || matches!(ip, IpAddr::V4(v) if u32::from(v) & 0xffc0_0000 == 0x6440_0000)
}
#[derive(clap::Args, Clone, Debug)]
pub struct StorageCommand {
    #[arg(long)]
    pub listen: SocketAddr,
    #[arg(long)]
    pub data: PathBuf,
    #[arg(long, default_value_t = 10)]
    pub min_free_gb: u64,
}
#[derive(Clone)]
pub struct StorageNode(Arc<Node>);
struct Node {
    disk: LocalDisk,
    backup_disk: LocalDisk,
    pool: SqlitePool,
    secret: Vec<u8>,
    io: Mutex<()>,
    slots: Arc<Semaphore>,
    min_free: u64,
    stop: tokio_util::sync::CancellationToken,
}
// All security-relevant fields, including Range and body digest, are signed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign(
    secret: &[u8],
    method: &str,
    path: &str,
    time: &str,
    nonce: &str,
    offset: u64,
    length: u64,
    sha: &str,
    range: &str,
) -> String {
    let text = format!(
        "sticktalk-blob-v1\n{method}\n{path}\n{time}\n{nonce}\n{offset}\n{length}\n{sha}\n{range}"
    );
    blob::hex(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret), text.as_bytes()).as_ref())
}
fn header<'a>(h: &'a HeaderMap, name: &str) -> Result<&'a str> {
    require(
        h.get_all(name).iter().count() == 1,
        ErrorCode::Unauthenticated,
    )?;
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))
}
fn number(h: &HeaderMap, name: &str) -> Result<u64> {
    header(h, name)?
        .parse()
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))
}
impl StorageNode {
    fn disk(&self, key: &str) -> &LocalDisk {
        if key.starts_with("backup-")
            && (key.ends_with(".sqlite3") || key.ends_with(".sqlite3.age"))
        {
            &self.0.backup_disk
        } else {
            &self.0.disk
        }
    }
    pub async fn open(data: PathBuf, secret: &str, min_free_gb: u64) -> Result<Self> {
        require(
            (32..=256).contains(&secret.len()),
            ErrorCode::InvalidRequest,
        )?;
        tokio::fs::create_dir_all(data.join("files")).await?;
        tokio::fs::create_dir_all(data.join("backups")).await?;
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(data.join("storage.sqlite3"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full)
                    .busy_timeout(std::time::Duration::from_secs(5)),
            )
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS blobs(key TEXT PRIMARY KEY, size INTEGER NOT NULL, complete INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER, updated_at INTEGER NOT NULL DEFAULT 0)").execute(&pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS nonces(nonce TEXT PRIMARY KEY, expires INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS nonce_expiry ON nonces(expires)")
            .execute(&pool)
            .await?;
        Ok(Self(Arc::new(Node {
            disk: LocalDisk::new(data.join("files")),
            backup_disk: LocalDisk::new(data.join("backups")),
            pool,
            secret: secret.as_bytes().to_vec(),
            io: Mutex::new(()),
            slots: Arc::new(Semaphore::new(4)),
            stop: tokio_util::sync::CancellationToken::new(),
            min_free: min_free_gb.saturating_mul(1024 * 1024 * 1024),
        })))
    }
    pub fn shutdown(&self) {
        self.0.stop.cancel();
    }
    pub async fn serve(self, listener: tokio::net::TcpListener) -> std::io::Result<()> {
        let stop = self.0.stop.clone();
        axum::serve(listener, self.router())
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
    }
    pub fn router(self) -> axum::Router {
        axum::Router::new()
            .route("/v1/space", get(space))
            .route(
                "/v1/blob/{key}",
                get(download)
                    .head(stat)
                    .put(put)
                    .post(finalize)
                    .delete(delete),
            )
            .layer(axum::middleware::from_fn_with_state(
                self.clone(),
                authenticate,
            ))
            .with_state(self)
    }
}
async fn authenticate(
    State(node): State<StorageNode>,
    r: Request<Body>,
    next: Next,
) -> Result<Response> {
    let permit = node
        .0
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let h = r.headers();
    let time = header(h, "x-time")?;
    let nonce = header(h, "x-nonce")?;
    let timestamp = time
        .parse::<u64>()
        .map_err(|_| Error::new(ErrorCode::Unauthenticated))?;
    require(
        (now() / 1000).abs_diff(timestamp) <= 60
            && nonce.len() == 32
            && nonce.bytes().all(|b| b.is_ascii_hexdigit())
            && r.uri().query().is_none(),
        ErrorCode::Unauthenticated,
    )?;
    let expected = sign(
        &node.0.secret,
        r.method().as_str(),
        r.uri().path(),
        time,
        nonce,
        number(h, "upload-offset")?,
        number(h, "x-size")?,
        header(h, "x-sha256")?,
        header(h, "range")?,
    );
    use subtle::ConstantTimeEq;
    require(
        bool::from(
            expected
                .as_bytes()
                .ct_eq(header(h, "x-signature")?.as_bytes()),
        ),
        ErrorCode::Unauthenticated,
    )?;
    // Persist nonce BEFORE reading the body. Replay remains rejected after restart.
    // Bounded table; fail closed if an authenticated caller floods the window.
    let mut tx = node.0.pool.begin().await?;
    sqlx::query("DELETE FROM nonces WHERE expires<?")
        .bind((now() / 1000) as i64)
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nonces")
        .fetch_one(&mut *tx)
        .await?;
    require(count < 32768, ErrorCode::Unavailable)?;
    let inserted = sqlx::query("INSERT OR IGNORE INTO nonces VALUES(?,?)")
        .bind(nonce)
        .bind(timestamp.saturating_add(61) as i64)
        .execute(&mut *tx)
        .await?;
    require(inserted.rows_affected() == 1, ErrorCode::Unauthenticated)?;
    tx.commit().await?;
    let deadline = if r.method() == axum::http::Method::POST {
        3600
    } else {
        120
    };
    let response = tokio::select! {
        _ = node.0.stop.cancelled() => return Err(blob::offline()),
        response = tokio::time::timeout(std::time::Duration::from_secs(deadline), next.run(r)) => response.map_err(|_| Error::new(ErrorCode::Unavailable))?,
    };
    let (parts, body) = response.into_parts();
    use futures_util::StreamExt;
    Ok(Response::from_parts(
        parts,
        Body::from_stream(
            body.into_data_stream()
                .take_until(node.0.stop.clone().cancelled_owned())
                .map(move |b| {
                    let _keep = &permit;
                    b
                }),
        ),
    ))
}
async fn record(node: &StorageNode, key: &str) -> Result<blob::BlobStat> {
    require(blob::valid_key(key), ErrorCode::InvalidRequest)?;
    let r = sqlx::query("SELECT size,complete FROM blobs WHERE key=?")
        .bind(key)
        .fetch_optional(&node.0.pool)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok(blob::BlobStat {
        size: r.get::<i64, _>("size") as u64,
        complete: r.get("complete"),
    })
}
async fn space(State(node): State<StorageNode>) -> Result<axum::Json<u64>> {
    let _io = node.0.io.lock().await;
    let stale: Vec<String> =
        sqlx::query_scalar("SELECT key FROM blobs WHERE deleted_at<? OR (complete=0 AND key LIKE 'backup-%.sqlite3%' AND updated_at<?) LIMIT 32")
            .bind(now().saturating_sub(14 * 86400000) as i64)
        .bind(now().saturating_sub(86400000) as i64)
            .fetch_all(&node.0.pool)
            .await?;
    for key in stale {
        node.disk(&key).delete(&key).await?;
        sqlx::query("DELETE FROM blobs WHERE key=?")
            .bind(key)
            .execute(&node.0.pool)
            .await?;
    }
    Ok(axum::Json(
        node.0
            .disk
            .free_space()
            .await?
            .saturating_sub(node.0.min_free),
    ))
}
async fn stat(State(node): State<StorageNode>, Path(key): Path<String>) -> Result<Response> {
    let r = record(&node, &key).await?;
    if r.complete {
        // An authenticated stat acknowledges a still-needed immutable replica,
        // including objects referenced again after restoring a DB snapshot.
        sqlx::query("UPDATE blobs SET deleted_at=NULL WHERE key=? AND complete=1 AND deleted_at IS NOT NULL").bind(&key).execute(&node.0.pool).await?;
    }
    Ok(Response::builder()
        .header("upload-offset", r.size)
        .header("x-complete", if r.complete { "1" } else { "0" })
        .body(Body::empty())
        .unwrap())
}
async fn put(
    State(node): State<StorageNode>,
    Path(key): Path<String>,
    r: Request<Body>,
) -> Result<StatusCode> {
    require(blob::valid_key(&key), ErrorCode::InvalidRequest)?;
    let offset = number(r.headers(), "upload-offset")?;
    let length = number(r.headers(), "x-size")?;
    let sha = header(r.headers(), "x-sha256")?.to_owned();
    require(
        length <= crate::resumable::CHUNK
            && offset
                .checked_add(length)
                .is_some_and(|n| n <= i64::MAX as u64)
            && crate::resumable::digest_valid(&sha)
            && number(r.headers(), "content-length")? == length,
        ErrorCode::InvalidRequest,
    )?;
    let _io = node.0.io.lock().await;
    let old = record(&node, &key).await;
    match old {
        Ok(old) => require(!old.complete && offset <= old.size, ErrorCode::Conflict)?,
        Err(e) if e.0.code == ErrorCode::NotFound => {
            require(offset == 0, ErrorCode::Conflict)?;
            sqlx::query("INSERT INTO blobs(key,size,complete,updated_at) VALUES(?,0,0,?)")
                .bind(&key)
                .bind(now() as i64)
                .execute(&node.0.pool)
                .await?;
        }
        Err(e) => return Err(e),
    }
    require(
        node.0.disk.free_space().await? >= node.0.min_free.saturating_add(length),
        ErrorCode::Unavailable,
    )?;
    // Persist rollback first: failed/cancelled retries must never leave a durable
    // offset beyond the bytes remaining on disk.
    sqlx::query("UPDATE blobs SET size=?,updated_at=? WHERE key=?")
        .bind(offset as i64)
        .bind(now() as i64)
        .bind(&key)
        .execute(&node.0.pool)
        .await?;
    node.disk(&key)
        .put_chunk(&key, offset, length, &sha, r.into_body())
        .await?;
    sqlx::query("UPDATE blobs SET size=?,updated_at=? WHERE key=?")
        .bind((offset + length) as i64)
        .bind(now() as i64)
        .bind(key)
        .execute(&node.0.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn finalize(
    State(node): State<StorageNode>,
    Path(key): Path<String>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let _io = node.0.io.lock().await;
    let size = number(&h, "x-size")?;
    let sha = header(&h, "x-sha256")?;
    require(
        crate::resumable::digest_valid(sha) && record(&node, &key).await?.size == size,
        ErrorCode::Conflict,
    )?;
    node.disk(&key).finalize(&key, size, sha).await?;
    sqlx::query("UPDATE blobs SET complete=1 WHERE key=?")
        .bind(&key)
        .execute(&node.0.pool)
        .await?;
    // Exactly fourteen UTC-day windows; failed backups never prune history.
    if key.starts_with("backup-") && (key.ends_with(".sqlite3") || key.ends_with(".sqlite3.age")) {
        let cutoff = format!("backup-{}.sqlite3", now().saturating_sub(14 * 86400000));
        let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM blobs WHERE complete=1 AND key LIKE 'backup-%.sqlite3%' AND key<? LIMIT 32").bind(cutoff).fetch_all(&node.0.pool).await?;
        for key in keys {
            node.disk(&key).delete(&key).await?;
            sqlx::query("DELETE FROM blobs WHERE key=?")
                .bind(key)
                .execute(&node.0.pool)
                .await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn download(
    State(node): State<StorageNode>,
    Path(key): Path<String>,
    h: HeaderMap,
) -> Result<Response> {
    let r = record(&node, &key).await?;
    require(r.complete, ErrorCode::Conflict)?;
    let range = header(&h, "range")?;
    let (start, length) = if range.is_empty() {
        (0, r.size)
    } else {
        let (start, end) = match crate::files::byte_range(range, r.size) {
            Ok(value) => value,
            Err(true) => {
                return Ok(Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header("content-range", format!("bytes */{}", r.size))
                    .body(Body::empty())
                    .unwrap())
            }
            Err(false) => return Err(Error::new(ErrorCode::InvalidRequest)),
        };
        (start, end - start + 1)
    };
    let mut response = Response::builder()
        .status(if range.is_empty() {
            StatusCode::OK
        } else {
            StatusCode::PARTIAL_CONTENT
        })
        .header("content-length", length)
        .header("accept-ranges", "bytes");
    if !range.is_empty() {
        response = response.header(
            "content-range",
            format!(
                "bytes {start}-{}/{total}",
                start + length - 1,
                total = r.size
            ),
        );
    }
    Ok(response
        .body(node.disk(&key).get(&key, start, length).await?)
        .unwrap())
}
async fn delete(State(node): State<StorageNode>, Path(key): Path<String>) -> Result<StatusCode> {
    let _io = node.0.io.lock().await;
    require(blob::valid_key(&key), ErrorCode::InvalidRequest)?;
    let existing = match record(&node, &key).await {
        Ok(stat) => stat,
        Err(e) if e.0.code == ErrorCode::NotFound => return Ok(StatusCode::NO_CONTENT),
        Err(e) => return Err(e),
    };
    // Incomplete/cancelled prefixes are never referenced by a snapshot.
    if !existing.complete {
        node.disk(&key).delete(&key).await?;
        sqlx::query("DELETE FROM blobs WHERE key=?")
            .bind(key)
            .execute(&node.0.pool)
            .await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    // Retain deleted objects for 14 days so every retained daily DB snapshot can
    // still be restored. Chat authorization immediately removes public access.
    sqlx::query("UPDATE blobs SET deleted_at=COALESCE(deleted_at,?) WHERE key=?")
        .bind(now() as i64)
        .bind(key)
        .execute(&node.0.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn run(command: &StorageCommand, secret: &str) -> Result<()> {
    require(
        private_address(command.listen.ip()),
        ErrorCode::InvalidRequest,
    )?;
    let node = StorageNode::open(command.data.clone(), secret, command.min_free_gb).await?;
    let listener = tokio::net::TcpListener::bind(command.listen).await?;
    let stop = node.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        stop.shutdown();
    });
    node.serve(listener).await?;
    Ok(())
}
