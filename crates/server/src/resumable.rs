//! Durable offsets are committed only after fsync. Extra bytes after a crash are
//! truncated on the next PUT. Storage keys never change during publication.
use crate::{
    db::*,
    error::{require, Error, Result},
    http::{bearer, url_id},
    state::{now, stream_id, App},
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Connection, Row};
use sticktalk_proto::*;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
pub const CHUNK: u64 = 16 * 1024 * 1024;
const DAY: u64 = crate::upload_accounting::IDLE_MS;
fn full() -> Error {
    let mut e = Error::new(ErrorCode::Unavailable);
    e.0.message = "Server storage is full.".into();
    e
}
pub(crate) async fn disk_guard(app: &App, additional: u64) -> Result<()> {
    let pending: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(byte_size-received+65536),0) FROM partial_uploads WHERE attachment_id IS NULL AND expires_at>?").bind(now() as i64).fetch_one(&app.0.pool).await?;
    let legacy: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(65536),0) FROM upload_reservations")
        .fetch_one(&app.0.pool)
        .await?;
    let free = fs2::available_space(&app.0.config.data_dir)?;
    let needed = (pending as u64)
        .saturating_add(legacy as u64)
        .saturating_add(additional)
        .saturating_add(
            app.0
                .config
                .operations
                .min_free_disk_gb
                .saturating_mul(1024 * 1024 * 1024),
        );
    if free < needed {
        return Err(full());
    }
    Ok(())
}
pub(crate) fn digest_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
async fn access(app: &App, headers: &HeaderMap, key: &str) -> Result<(Id, SqliteRow)> {
    require(
        key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        ErrorCode::NotFound,
    )?;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, bearer(headers)?).await?;
    let r = sqlx::query("SELECT * FROM partial_uploads WHERE id=? AND owner_id=? AND expires_at>?")
        .bind(key)
        .bind(id(u)?)
        .bind(now() as i64)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    if let Some(ch) = r.get::<Option<i64>, _>("channel_id") {
        text(&mut c, u, Id(ch as u64), Permissions::ATTACH_FILES).await?;
    }
    Ok((u, r))
}
async fn projection(app: &App, r: &SqliteRow) -> Result<ResumableUpload> {
    let completed = if let Some(a) = r.get::<Option<i64>, _>("attachment_id") {
        let row = sqlx::query("SELECT * FROM uploads WHERE id=?")
            .bind(a)
            .fetch_one(&app.0.pool)
            .await?;
        Some(UploadResponse {
            attachment: attachment_row(&row),
            expires_at: row.get::<Option<i64>, _>("expires_at").unwrap_or(0) as u64,
        })
    } else {
        None
    };
    Ok(ResumableUpload {
        upload_id: r.get("id"),
        offset: r.get::<i64, _>("received") as u64,
        byte_size: r.get::<i64, _>("byte_size") as u64,
        chunk_size: CHUNK as u32,
        expires_at: r.get::<i64, _>("expires_at") as u64,
        completed,
    })
}
pub async fn create(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    let headers = r.headers().clone();
    let (_, u) = auth(&mut *app.0.pool.acquire().await?, bearer(&headers)?).await?;
    let req: CreateUploadRequest = crate::http::parse(r).await?;
    require(
        req.byte_size <= i64::MAX as u64 - 65536
            && digest_valid(&req.sha256)
            && req.filename.len() <= 1024
            && req.content_type.len() <= 128
            && !req.content_type.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    require(
        (req.purpose == UploadPurpose::Message) == req.channel_id.is_some(),
        ErrorCode::InvalidRequest,
    )?;
    let max = app.0.config.max_upload_mb.saturating_mul(1024 * 1024);
    require(max == 0 || req.byte_size <= max, ErrorCode::PayloadTooLarge)?;
    gc(&app).await?;
    let _io = app.0.upload_io.lock().await;
    disk_guard(&app, req.byte_size + 65536).await?;
    let mut core = app.0.core.lock().await;
    // A message holds up to ten attachments; let two sets be added together.
    core.rate(
        format!("upload:{}", u.0),
        20.,
        2. * MAX_MESSAGE_ATTACHMENTS as f64,
    )?;
    let mut c = app.0.pool.acquire().await?;
    let (_, current) = auth(&mut c, bearer(&headers)?).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    if let Some(ch) = req.channel_id {
        text(&mut c, u, ch, Permissions::ATTACH_FILES).await?;
    }
    crate::upload_accounting::admit(&mut c, u).await?;
    crate::upload_accounting::quota(&app, &mut c, u, req.byte_size + 65536).await?;
    let key = stream_id();
    let storage = format!("{key}.part");
    let thumb = format!("{key}.jpg");
    sqlx::query("INSERT INTO partial_uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,sha256,storage_key,thumbnail_key,expires_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(&key).bind(id(u)?).bind(enum_str(req.purpose)).bind(opt(req.channel_id)?).bind(crate::files::filename(&req.filename)).bind(req.content_type).bind(req.byte_size as i64).bind(req.sha256).bind(&storage).bind(thumb).bind((now()+DAY) as i64).execute(&mut *c).await?;
    drop(c);
    drop(core);
    tokio::fs::File::create(app.0.config.data_dir.join("files").join(storage))
        .await?
        .sync_all()
        .await?;
    let (_, row) = access(&app, &headers, &key).await?;
    Ok((StatusCode::CREATED, Json(projection(&app, &row).await?)).into_response())
}
pub async fn status(
    State(app): State<App>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ResumableUpload>> {
    let (_, r) = access(&app, &headers, &key).await?;
    Ok(Json(projection(&app, &r).await?))
}
pub async fn chunk(
    State(app): State<App>,
    Path(key): Path<String>,
    r: Request<Body>,
) -> Result<Json<ResumableUpload>> {
    let _slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let headers = r.headers().clone();
    let offset = headers
        .get("upload-offset")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let len = headers
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    require(len > 0 && len <= CHUNK, ErrorCode::PayloadTooLarge)?;
    let _io = app.0.upload_io.lock().await;
    let (user, row) = access(&app, &headers, &key).await?;
    require(
        row.get::<Option<i64>, _>("attachment_id").is_none()
            && offset == row.get::<i64, _>("received") as u64,
        ErrorCode::Conflict,
    )?;
    require(
        offset
            .checked_add(len)
            .is_some_and(|end| end <= row.get::<i64, _>("byte_size") as u64),
        ErrorCode::PayloadTooLarge,
    )?;
    disk_guard(&app, 0).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(
            app.0
                .config
                .data_dir
                .join("files")
                .join(row.get::<String, _>("storage_key")),
        )
        .await?;
    file.set_len(offset).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut body = r.into_body().into_data_stream();
    let mut received = 0;
    while let Some(bytes) = body.next().await {
        let bytes = bytes.map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        crate::upload_accounting::charge(&app, user, bytes.len()).await?;
        sqlx::query("UPDATE partial_uploads SET expires_at=? WHERE id=?")
            .bind((now() + DAY) as i64)
            .bind(&key)
            .execute(&app.0.pool)
            .await?;
        received += bytes.len() as u64;
        require(received <= len, ErrorCode::PayloadTooLarge)?;
        file.write_all(&bytes).await?;
    }
    require(received == len, ErrorCode::InvalidRequest)?;
    file.sync_all().await?;
    let gate = app.0.core.lock().await;
    access(&app, &headers, &key).await?; // Reauthorize after network I/O.
    sqlx::query("UPDATE partial_uploads SET received=?,expires_at=? WHERE id=?")
        .bind((offset + len) as i64)
        .bind((now() + DAY) as i64)
        .bind(&key)
        .execute(&app.0.pool)
        .await?;
    let (_, row) = access(&app, &headers, &key).await?;
    let result = projection(&app, &row).await?;
    drop(gate);
    // The local fsynced prefix remains the outage journal until publication.
    let storage = row.get::<String, _>("storage_key");
    let _ = crate::tier::mirror(&app, &storage, offset + len, false).await;
    Ok(Json(result))
}
pub(crate) async fn file_hash(path: &std::path::Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 65536];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
pub async fn finish(
    State(app): State<App>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Result<Json<UploadResponse>> {
    let _io = app.0.upload_io.lock().await;
    let (u, row) = access(&app, &headers, &key).await?;
    if let Some(result) = projection(&app, &row).await?.completed {
        return Ok(Json(result));
    }
    let size = row.get::<i64, _>("byte_size");
    require(row.get::<i64, _>("received") == size, ErrorCode::Conflict)?;
    let storage = row.get::<String, _>("storage_key");
    let path = app.0.config.data_dir.join("files").join(&storage);
    require(
        file_hash(&path).await? == row.get::<String, _>("sha256"),
        ErrorCode::InvalidAttachment,
    )?;
    let claimed = row.get::<String, _>("content_type");
    let purpose: UploadPurpose = en(row.get("purpose"))?;
    let permit = app
        .0
        .images
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let media = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::profiles::media::prepare(&path, &claimed, purpose)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Internal))??;
    disk_guard(&app, 0).await?;
    let thumb = row.get::<String, _>("thumbnail_key");
    if !media.thumbnail_bytes.is_empty() {
        let mut f =
            tokio::fs::File::create(app.0.config.data_dir.join("files").join(&thumb)).await?;
        f.write_all(&media.thumbnail_bytes).await?;
        f.sync_all().await?;
    }
    let mut core = app.0.core.lock().await;
    access(&app, &headers, &key).await?;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let a = core.id().await?;
    let expires = now() + 3_600_000;
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,width,height,thumbnail_key,thumbnail_width,thumbnail_height,thumbnail_size,thumbnail_type,created_at,expires_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(id(a)?).bind(id(u)?).bind(enum_str(purpose)).bind(row.get::<Option<i64>,_>("channel_id")).bind(row.get::<String,_>("filename")).bind(media.mime).bind(size).bind(&storage).bind(media.width.map(i64::from)).bind(media.height.map(i64::from)).bind(media.thumb.map(|_|&thumb)).bind(media.thumb.map(|t|i64::from(t.0))).bind(media.thumb.map(|t|i64::from(t.1))).bind(media.thumb.map(|t|i64::from(t.2))).bind(media.thumb.map(|_|"image/jpeg")).bind(now() as i64).bind(expires as i64).execute(&mut *tx).await?;
    sqlx::query("UPDATE partial_uploads SET attachment_id=? WHERE id=?")
        .bind(id(a)?)
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query("SELECT * FROM uploads WHERE id=?")
        .bind(id(a)?)
        .fetch_one(&mut *tx)
        .await?;
    core.persist_id(&mut tx).await?;
    tx.commit().await?;
    drop(c);
    drop(core);
    crate::tier::published(&app, &storage, media.thumb.map(|_| thumb.as_str())).await;
    Ok(Json(UploadResponse {
        attachment: attachment_row(&row),
        expires_at: expires,
    }))
}
pub async fn cancel(
    State(app): State<App>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let _io = app.0.upload_io.lock().await;
    let (_, r) = access(&app, &headers, &key).await?;
    require(
        r.get::<Option<i64>, _>("attachment_id").is_none(),
        ErrorCode::Conflict,
    )?;
    sqlx::query("DELETE FROM partial_uploads WHERE id=?")
        .bind(key)
        .execute(&app.0.pool)
        .await?;
    for col in ["storage_key", "thumbnail_key"] {
        let _ = tokio::fs::remove_file(
            app.0
                .config
                .data_dir
                .join("files")
                .join(r.get::<String, _>(col)),
        )
        .await;
    }
    app.0.tier.wake.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
pub async fn usage(
    State(app): State<App>,
    Path(g): Path<String>,
    headers: HeaderMap,
) -> Result<Json<StorageUsage>> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, bearer(&headers)?).await?;
    let guild = url_id(&g)?;
    guild_perm(&mut c, u, guild, Permissions::OWNER).await?;
    let committed: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(u.byte_size+COALESCE(u.thumbnail_size,0)),0) FROM uploads u WHERE EXISTS(SELECT 1 FROM channels c WHERE c.id=u.channel_id AND c.guild_id=?1) OR EXISTS(SELECT 1 FROM guilds g WHERE g.id=?1 AND g.icon_id=u.id)",
    )
    .bind(id(guild)?)
    .fetch_one(&mut *c)
    .await?;
    let r=sqlx::query("SELECT COALESCE(SUM(p.received),0) AS partial,COALESCE(SUM(p.byte_size-p.received+65536),0) AS reserved FROM partial_uploads p JOIN channels c ON c.id=p.channel_id WHERE p.attachment_id IS NULL AND c.guild_id=?").bind(id(guild)?).fetch_one(&mut *c).await?;
    Ok(Json(StorageUsage {
        // Host capacity is deliberately not exposed to guild owners.
        total_bytes: 0, available_bytes: 0,
        committed_bytes: committed as u64,
        partial_bytes: r.get::<i64, _>("partial") as u64,
        reserved_bytes: (r.get::<i64, _>("reserved") + sqlx::query_scalar::<_,i64>("SELECT COALESCE(SUM(r.reserved_bytes+65536),0) FROM upload_reservations r JOIN channels c ON c.id=r.channel_id WHERE c.guild_id=? AND r.expires_at>?").bind(id(guild)?).bind(now() as i64).fetch_one(&mut *c).await?) as u64,
        min_free_bytes: 0,
    }))
}
pub(crate) async fn gc(app: &App) -> Result<()> {
    // Large final hashes must not delay session expiry/revocation housekeeping.
    let Ok(_io) = app.0.upload_io.try_lock() else {
        return Ok(());
    };
    let _gate = app.0.core.lock().await;
    let rows=sqlx::query("SELECT id,storage_key,thumbnail_key,attachment_id FROM partial_uploads WHERE expires_at<=? LIMIT 100").bind(now() as i64).fetch_all(&app.0.pool).await?;
    if !rows.is_empty() {
        app.0.tier.wake.notify_one();
    }
    for r in rows {
        sqlx::query("DELETE FROM partial_uploads WHERE id=?")
            .bind(r.get::<String, _>("id"))
            .execute(&app.0.pool)
            .await?;
        if r.get::<Option<i64>, _>("attachment_id").is_none() {
            for col in ["storage_key", "thumbnail_key"] {
                let _ = tokio::fs::remove_file(
                    app.0
                        .config
                        .data_dir
                        .join("files")
                        .join(r.get::<String, _>(col)),
                )
                .await;
            }
        }
    }
    Ok(())
}
