//! The server stores only the public key. Signing happens on the release PC.
use crate::{
    error::{require, Error, Result},
    resumable::{digest_valid, disk_guard, file_hash},
    state::{hash, stream_id, App},
};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use ring::signature::{UnparsedPublicKey, ED25519};
use sqlx::Row;
use sticktalk_proto::*;
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
pub fn verify(public_key: &str, signed: &SignedUpdateManifest) -> Result<UpdateManifest> {
    require(
        signed.manifest.len() <= 8192 && signed.signature.len() <= 128,
        ErrorCode::InvalidRequest,
    )?;
    let public = STANDARD
        .decode(public_key)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let signature = STANDARD
        .decode(&signed.signature)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    UnparsedPublicKey::new(&ED25519, public)
        .verify(signed.manifest.as_bytes(), &signature)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let m: UpdateManifest = serde_json::from_str(&signed.manifest)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        m.version.len() <= 64
            && !m.version.is_empty()
            && m.version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            && m.target == "x86_64-pc-windows-msvc"
            && m.byte_size > 0
            && m.byte_size <= 1024 * 1024 * 1024
            && digest_valid(&m.sha256),
        ErrorCode::InvalidRequest,
    )?;
    Ok(m)
}
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let p = self.0.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(p).await;
        });
    }
}
fn key(app: &App) -> Result<&str> {
    app.0
        .config
        .operations
        .update_public_key
        .as_deref()
        .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))
}
pub async fn latest(State(app): State<App>) -> Result<Response> {
    let public = key(&app)?;
    let envelope: Option<String> =
        sqlx::query_scalar("SELECT envelope FROM releases ORDER BY rowid DESC LIMIT 1")
            .fetch_optional(&app.0.pool)
            .await?;
    let signed: SignedUpdateManifest =
        serde_json::from_str(&envelope.ok_or_else(|| Error::new(ErrorCode::NotFound))?)
            .map_err(|_| Error::new(ErrorCode::Internal))?;
    verify(public, &signed)?;
    Ok(([("cache-control", "no-store")], Json(signed)).into_response())
}
pub async fn publish(State(app): State<App>, r: Request<Body>) -> Result<StatusCode> {
    let public = key(&app)?;
    let expected = app
        .0
        .config
        .operations
        .release_token
        .as_deref()
        .filter(|s| s.len() >= 32)
        .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?;
    let supplied = crate::http::bearer(r.headers())?;
    require(
        bool::from(hash(expected.as_bytes()).ct_eq(&hash(supplied.as_bytes()))),
        ErrorCode::Unauthenticated,
    )?;
    let header = r
        .headers()
        .get("x-sticktalk-manifest")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() <= 16384)
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let signed: SignedUpdateManifest = serde_json::from_slice(
        &STANDARD
            .decode(header)
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?,
    )
    .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let m = verify(public, &signed)?;
    require(
        m.url
            == format!(
                "{}/api/update/artifact?sha256={}",
                app.0.config.public_url.trim_end_matches('/'),
                m.sha256
            )
            && m.url.starts_with("https://"),
        ErrorCode::InvalidRequest,
    )?;
    let _io = app.0.upload_io.lock().await;
    disk_guard(&app, m.byte_size).await?;
    let dir = app.0.config.data_dir.join("updates");
    tokio::fs::create_dir_all(&dir).await?;
    let temp = Temp(dir.join(format!("{}.tmp", stream_id())));
    let mut f = tokio::fs::File::create(&temp.0).await?;
    let mut bytes = r.into_body().into_data_stream();
    let mut size = 0u64;
    while let Some(chunk) = bytes.next().await {
        let chunk = chunk.map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        size += chunk.len() as u64;
        require(size <= m.byte_size, ErrorCode::PayloadTooLarge)?;
        f.write_all(&chunk).await?;
    }
    require(size == m.byte_size, ErrorCode::InvalidRequest)?;
    f.sync_all().await?;
    drop(f);
    require(
        file_hash(&temp.0).await? == m.sha256,
        ErrorCode::InvalidAttachment,
    )?;
    let final_path = dir.join(format!("{}.exe", m.sha256));
    if tokio::fs::try_exists(&final_path).await? {
        require(
            file_hash(&final_path).await? == m.sha256,
            ErrorCode::Internal,
        )?;
    } else {
        tokio::fs::rename(&temp.0, &final_path).await?;
    }
    // Same version cannot silently be overwritten (downgrade policy is client-owned).
    let old: Option<String> = sqlx::query_scalar("SELECT sha256 FROM releases WHERE version=?")
        .bind(&m.version)
        .fetch_optional(&app.0.pool)
        .await?;
    require(old.is_none_or(|h| h == m.sha256), ErrorCode::Conflict)?;
    sqlx::query(
        "INSERT OR IGNORE INTO releases(version,sha256,byte_size,envelope) VALUES(?,?,?,?)",
    )
    .bind(m.version)
    .bind(m.sha256)
    .bind(m.byte_size as i64)
    .bind(serde_json::to_string(&signed).map_err(|_| Error::new(ErrorCode::Internal))?)
    .execute(&app.0.pool)
    .await?;
    Ok(StatusCode::CREATED)
}
#[derive(serde::Deserialize)]
pub struct ArtifactQuery {
    sha256: String,
}
pub async fn artifact(State(app): State<App>, Query(q): Query<ArtifactQuery>) -> Result<Response> {
    key(&app)?;
    require(digest_valid(&q.sha256), ErrorCode::NotFound)?;
    let row = sqlx::query("SELECT byte_size FROM releases WHERE sha256=? LIMIT 1")
        .bind(&q.sha256)
        .fetch_optional(&app.0.pool)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let f = tokio::fs::File::open(
        app.0
            .config
            .data_dir
            .join("updates")
            .join(format!("{}.exe", q.sha256)),
    )
    .await?;
    Response::builder()
        .header("content-type", "application/octet-stream")
        .header(
            "content-disposition",
            "attachment; filename=\"StickTalk-Setup.exe\"",
        )
        .header("content-length", row.get::<i64, _>("byte_size"))
        .header("cache-control", "public, max-age=31536000, immutable")
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(f)))
        .map_err(|_| Error::new(ErrorCode::Internal))
}
