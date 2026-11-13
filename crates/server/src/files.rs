use crate::{
    db::*,
    error::{require, Error, Result},
    http::{bearer, url_id},
    state::{now, stream_id, App},
};
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use image::{GenericImageView, ImageDecoder};
use sqlx::{Connection, Row};
use std::path::PathBuf;
use sticktalk_proto::*;
use tokio::io::AsyncWriteExt;
struct UploadGuard {
    app: App,
    key: String,
    paths: Vec<PathBuf>,
    complete: bool,
}
impl Drop for UploadGuard {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        let app = self.app.clone();
        let key = self.key.clone();
        let paths = std::mem::take(&mut self.paths);
        tokio::spawn(async move {
            let _gate = app.0.core.lock().await;
            if let Ok(mut c) = app.0.pool.acquire().await {
                if let Ok(mut tx) = c.begin().await {
                    let _ = sqlx::query("DELETE FROM upload_reservations WHERE key=?")
                        .bind(&key)
                        .execute(&mut *tx)
                        .await;
                    let _ = tx.commit().await;
                }
            }
            // Cancellation can land after SQLite committed but before complete=true.
            // Never unlink files which the committed upload now references.
            let published =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploads WHERE storage_key=?")
                    .bind(format!("{key}.bin"))
                    .fetch_one(&app.0.pool)
                    .await;
            drop(_gate);
            if published.is_ok_and(|n| n == 0) {
                for p in paths {
                    let _ = tokio::fs::remove_file(p).await;
                }
            }
        });
    }
}
pub(crate) struct Media {
    pub(crate) mime: String,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) thumb: Option<(u16, u16, u32)>,
    pub(crate) thumbnail_bytes: Vec<u8>,
}
pub(crate) fn prepare(
    path: &std::path::Path,
    claimed: &str,
    image_required: bool,
) -> Result<Media> {
    let mut f = std::fs::File::open(path)?;
    let mut head = [0; 4096];
    use std::io::Read;
    let n = f.read(&mut head)?;
    let format = if n >= 8 && head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(image::ImageFormat::Png)
    } else if n >= 3 && head.starts_with(b"\xff\xd8\xff") {
        Some(image::ImageFormat::Jpeg)
    } else if n >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some(image::ImageFormat::WebP)
    } else {
        None
    };
    if let Some(format) = format {
        let mut reader = image::ImageReader::with_format(
            std::io::BufReader::new(std::fs::File::open(path)?),
            format,
        );
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
        let (w, h) = decoder.dimensions();
        require(
            w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 16_000_000,
            ErrorCode::UnsupportedMediaType,
        )?;
        let orientation = decoder
            .orientation()
            .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
        let mut img = image::DynamicImage::from_decoder(decoder)
            .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
        img.apply_orientation(orientation);
        let (w, h) = img.dimensions();
        let t = img.thumbnail(w.min(256), h.min(256)).to_rgb8();
        let mut bytes = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
            .encode_image(&t)
            .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
        require(bytes.len() <= 65536, ErrorCode::UnsupportedMediaType)?;
        Ok(Media {
            mime: format.to_mime_type().into(),
            width: Some(w),
            height: Some(h),
            thumb: Some((t.width() as u16, t.height() as u16, bytes.len() as u32)),
            thumbnail_bytes: bytes,
        })
    } else {
        require(
            !image_required
                && !claimed.starts_with("image/")
                && !String::from_utf8_lossy(&head[..n])
                    .to_ascii_lowercase()
                    .contains("<svg"),
            ErrorCode::UnsupportedMediaType,
        )?;
        Ok(Media {
            mime: "application/octet-stream".into(),
            width: None,
            height: None,
            thumb: None,
            thumbnail_bytes: vec![],
        })
    }
}
pub(crate) fn filename(s: &str) -> String {
    let s = s.rsplit(['/', '\\']).next().unwrap_or("file");
    let mut s: String = s
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\r' | '\n'))
        .collect();
    s = s.trim().trim_matches('.').to_string();
    while s.len() > 255 {
        s.pop();
    }
    if s.is_empty() {
        "file".into()
    } else {
        s
    }
}
pub async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    multipart: std::result::Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Response> {
    let _slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let token = bearer(&headers)?;
    let (key, u) = {
        let mut c = app.0.pool.acquire().await?;
        auth(&mut c, token).await?
    };
    let _disk = app.0.upload_io.lock().await;
    let max = app.0.config.max_upload_mb.saturating_mul(1024 * 1024);
    crate::resumable::disk_guard(&app, 65536).await?;
    let day = (now() / 86_400_000) as i64;
    let reservation = stream_id();
    {
        let mut core = app.0.core.lock().await;
        core.rate(format!("upload:{}", u.0), 10., 2.)?;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        crate::upload_accounting::admit(&mut tx, u).await?;
        crate::upload_accounting::quota(&app, &mut tx, u, 65536).await?;
        sqlx::query("INSERT INTO upload_reservations(key,user_id,utc_day,reserved_bytes,expires_at) VALUES(?,?,?,0,?)")
            .bind(&reservation).bind(id(u)?).bind(day).bind((now()+crate::upload_accounting::IDLE_MS) as i64).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    let dir = app.0.config.data_dir.join("files");
    let staging = dir.join(format!("{reservation}.tmp"));
    let final_key = format!("{reservation}.bin");
    let thumb_key = format!("{reservation}.jpg");
    let final_path = dir.join(&final_key);
    let thumb_path = dir.join(&thumb_key);
    let mut guard = UploadGuard {
        app: app.clone(),
        key: reservation.clone(),
        paths: vec![staging.clone(), final_path.clone(), thumb_path.clone()],
        complete: false,
    };
    let mut multipart = multipart.map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let field = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    require(field.name() == Some("purpose"), ErrorCode::InvalidRequest)?;
    let p = small_text(field).await?;
    require(p.len() < 32, ErrorCode::InvalidRequest)?;
    let purpose: UploadPurpose = en(p).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let mut field = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let ch = if purpose == UploadPurpose::Message {
        require(
            field.name() == Some("channel_id"),
            ErrorCode::InvalidRequest,
        )?;
        let s = small_text(field).await?;
        let ch = url_id(&s)?;
        field = multipart
            .next_field()
            .await
            .map_err(multipart_error)?
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        Some(ch)
    } else {
        None
    };
    require(field.name() == Some("file"), ErrorCode::InvalidRequest)?;
    let name = filename(field.file_name().unwrap_or("file"));
    let claimed = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    if let Some(ch) = ch {
        let mut c = app.0.pool.acquire().await?;
        text(&mut c, u, ch, Permissions::ATTACH_FILES).await?;
    }
    sqlx::query("UPDATE upload_reservations SET channel_id=? WHERE key=?")
        .bind(opt(ch)?)
        .bind(&reservation)
        .execute(&app.0.pool)
        .await?;
    let mut file = tokio::fs::File::create(&staging).await?;
    let mut size = 0u64;
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        size += chunk.len() as u64;
        crate::upload_accounting::charge(&app, u, chunk.len()).await?;
        require(max == 0 || size <= max, ErrorCode::PayloadTooLarge)?;
        crate::resumable::disk_guard(&app, chunk.len() as u64).await?;
        {
            let _gate = app.0.core.lock().await;
            let mut c = app.0.pool.acquire().await?;
            crate::upload_accounting::quota(&app, &mut c, u, chunk.len() as u64).await?;
            sqlx::query("UPDATE upload_reservations SET reserved_bytes=?,expires_at=? WHERE key=?")
                .bind(size as i64)
                .bind((now() + crate::upload_accounting::IDLE_MS) as i64)
                .bind(&reservation)
                .execute(&mut *c)
                .await?;
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    drop(field);
    require(
        multipart
            .next_field()
            .await
            .map_err(multipart_error)?
            .is_none(),
        ErrorCode::InvalidRequest,
    )?;
    let permit = app
        .0
        .images
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let path = staging.clone();
    let media = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepare(&path, &claimed, purpose != UploadPurpose::Message)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Internal))??;
    if !media.thumbnail_bytes.is_empty() {
        tokio::fs::write(&thumb_path, &media.thumbnail_bytes).await?;
    }
    tokio::fs::rename(&staging, &final_path).await?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let alive: Option<i64> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;
    require(
        alive.is_some_and(|t| t > now() as i64),
        ErrorCode::Unauthenticated,
    )?;
    if let Some(ch) = ch {
        text(&mut tx, u, ch, Permissions::ATTACH_FILES).await?;
    }
    let a = core.id().await?;
    let expires = now() + 3_600_000;
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,width,height,thumbnail_key,thumbnail_width,thumbnail_height,thumbnail_size,thumbnail_type,created_at,expires_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(id(a)?).bind(id(u)?).bind(enum_str(purpose)).bind(opt(ch)?).bind(name).bind(media.mime).bind(size as i64).bind(&final_key).bind(media.width.map(i64::from)).bind(media.height.map(i64::from)).bind(media.thumb.map(|_|&thumb_key)).bind(media.thumb.map(|t|i64::from(t.0))).bind(media.thumb.map(|t|i64::from(t.1))).bind(media.thumb.map(|t|i64::from(t.2))).bind(media.thumb.map(|_|"image/jpeg")).bind(now() as i64).bind(expires as i64).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM upload_reservations WHERE key=?")
        .bind(reservation)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query("SELECT * FROM uploads WHERE id=?")
        .bind(id(a)?)
        .fetch_one(&mut *tx)
        .await?;
    core.persist_id(&mut tx).await?;
    tx.commit().await?;
    guard.complete = true;
    drop(c);
    drop(core);
    crate::tier::published(&app, &final_key, media.thumb.map(|_| thumb_key.as_str())).await;
    Ok((
        StatusCode::CREATED,
        axum::Json(UploadResponse {
            attachment: attachment_row(&row),
            expires_at: expires,
        }),
    )
        .into_response())
}
async fn small_text(mut field: axum::extract::multipart::Field<'_>) -> Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        require(bytes.len() + chunk.len() <= 64, ErrorCode::InvalidRequest)?;
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| Error::new(ErrorCode::InvalidRequest))
}
fn multipart_error(e: axum::extract::multipart::MultipartError) -> Error {
    Error::new(if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ErrorCode::PayloadTooLarge
    } else {
        ErrorCode::InvalidRequest
    })
}
pub async fn original(
    State(app): State<App>,
    Path(s): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    download(app, url_id(&s)?, headers, false).await
}
pub async fn thumbnail(
    State(app): State<App>,
    Path(s): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    download(app, url_id(&s)?, headers, true).await
}
async fn download(app: App, a: Id, headers: HeaderMap, thumb: bool) -> Result<Response> {
    let token = bearer(&headers)?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, token).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let row = sqlx::query("SELECT * FROM uploads WHERE id=?")
        .bind(id(a)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    if let Some(expires) = row.get::<Option<i64>, _>("expires_at") {
        require(
            expires > now() as i64 && row.get::<i64, _>("owner_id") == id(u)?,
            ErrorCode::NotFound,
        )?;
    } else {
        let ch:Option<i64>=sqlx::query_scalar("SELECT m.channel_id FROM message_attachments ma JOIN messages m ON ma.message_id=m.id WHERE ma.attachment_id=? AND m.deleted_at IS NULL").bind(id(a)?).fetch_optional(&mut *c).await?;
        if let Some(ch) = ch {
            text(&mut c, u, Id(ch as u64), Permissions::READ_HISTORY).await?;
        } else {
            let target: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE avatar_id=?")
                .bind(id(a)?)
                .fetch_optional(&mut *c)
                .await?;
            if let Some(v) = target {
                profile_access(&mut c, u, Id(v as u64)).await?;
            } else {
                let g: Option<i64> = sqlx::query_scalar("SELECT id FROM guilds WHERE icon_id=?")
                    .bind(id(a)?)
                    .fetch_optional(&mut *c)
                    .await?;
                role(
                    &mut c,
                    u,
                    Id(g.ok_or_else(|| Error::new(ErrorCode::NotFound))? as u64),
                )
                .await?;
            }
        }
    }
    require(
        (!thumb || !headers.contains_key("range")) && headers.get_all("range").iter().count() <= 1,
        ErrorCode::InvalidRequest,
    )?;
    let (key, mime, size) = if thumb {
        (
            row.get::<Option<String>, _>("thumbnail_key")
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?,
            row.get::<String, _>("thumbnail_type"),
            row.get::<i64, _>("thumbnail_size") as u64,
        )
    } else {
        (
            row.get::<String, _>("storage_key"),
            row.get::<String, _>("content_type"),
            row.get::<i64, _>("byte_size") as u64,
        )
    };
    let etag = format!(
        "\"{}-{}\"",
        a.0,
        if thumb { "thumbnail" } else { "original" }
    );
    let mut builder = Response::builder()
        .header("etag", &etag)
        .header("cache-control", "private, max-age=86400, immutable")
        .header("x-content-type-options", "nosniff");
    if headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|v| v.trim() == etag || v.trim() == "*"))
    {
        return builder
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .map_err(|_| Error::new(ErrorCode::Internal));
    }
    let range = headers
        .get("range")
        .filter(|_| {
            headers
                .get("if-range")
                .is_none_or(|v| v.to_str().is_ok_and(|s| s == etag))
        })
        .map(|v| {
            v.to_str()
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))
        })
        .transpose()?;
    let (start, end) = if let Some(range) = range {
        match byte_range(range, size) {
            Ok(v) => v,
            Err(unsatisfiable) => {
                if unsatisfiable {
                    let mut response = Error::new(ErrorCode::InvalidRequest).into_response();
                    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                    response.headers_mut().insert(
                        "content-range",
                        format!("bytes */{size}")
                            .parse()
                            .map_err(|_| Error::new(ErrorCode::Internal))?,
                    );
                    return Ok(response);
                }
                return Err(Error::new(ErrorCode::InvalidRequest));
            }
        }
    } else {
        (0, size.saturating_sub(1))
    };
    if range.is_some() {
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header("content-range", format!("bytes {start}-{end}/{size}"));
    }
    if !thumb {
        builder = builder.header("accept-ranges", "bytes").header(
            "content-disposition",
            format!(
                "attachment; filename=\"{}\"",
                row.get::<String, _>("filename")
                    .chars()
                    .map(|c| if c.is_ascii() { c } else { '_' })
                    .collect::<String>()
            ),
        );
    }
    let length = if range.is_some() {
        end - start + 1
    } else {
        size
    };
    builder = builder
        .header("content-type", mime)
        .header("content-length", length);
    drop(c);
    drop(core);
    let body = app.0.tier.get(&app, &key, start, length).await?;
    builder
        .body(body)
        .map_err(|_| Error::new(ErrorCode::Internal))
}
pub(crate) fn byte_range(s: &str, size: u64) -> std::result::Result<(u64, u64), bool> {
    let s = s.strip_prefix("bytes=").ok_or(false)?;
    if s.contains(',') {
        return Err(false);
    }
    let (a, b) = s.split_once('-').ok_or(false)?;
    if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| false)?;
        if n == 0 || size == 0 {
            return Err(true);
        }
        Ok((size.saturating_sub(n), size - 1))
    } else {
        let start: u64 = a.parse().map_err(|_| false)?;
        let end = if b.is_empty() {
            size.saturating_sub(1)
        } else {
            b.parse::<u64>()
                .map_err(|_| false)?
                .min(size.saturating_sub(1))
        };
        if start >= size || start > end {
            Err(true)
        } else {
            Ok((start, end))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_bounds_mime_and_small_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image");
        image::RgbImage::new(2, 3)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let media = prepare(&path, "text/html", true).unwrap();
        assert_eq!(media.mime, "image/png");
        assert_eq!(media.thumb.map(|t| (t.0, t.1)), Some((2, 3)));
        std::fs::write(&path, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert_eq!(
            prepare(&path, "application/octet-stream", false)
                .err()
                .unwrap()
                .0
                .code,
            ErrorCode::UnsupportedMediaType
        );
        std::fs::write(&path, b"not an image").unwrap();
        assert!(prepare(&path, "image/png", false).is_err());
        // Valid PNG header/CRC with hostile dimensions, no decoded pixel allocation.
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(1, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let original = bytes.into_inner();
        for (w, h) in [(20000_u32, 1_u32), (4096, 4096)] {
            let mut bytes = original.clone();
            bytes[16..20].copy_from_slice(&w.to_be_bytes());
            bytes[20..24].copy_from_slice(&h.to_be_bytes());
            let mut crc = 0xffff_ffff_u32;
            for byte in &bytes[12..29] {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
                }
            }
            bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(
                prepare(&path, "image/png", true).err().unwrap().0.code,
                ErrorCode::UnsupportedMediaType
            );
        }
        assert_eq!(filename("../../evil\r\n.png"), "evil.png");
        assert_eq!(filename("C:\\private\\secret.txt"), "secret.txt");
        assert_eq!(filename("..."), "file");
    }
    #[tokio::test]
    async fn cancellation_after_upload_commit_preserves_published_files() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let key = "cancellation";
        let path = app.0.config.data_dir.join("files/cancellation.bin");
        tokio::fs::write(&path, b"saved").await.unwrap();
        sqlx::query("INSERT INTO uploads(id,owner_id,purpose,filename,content_type,byte_size,storage_key,created_at,expires_at) VALUES(30,1,'avatar','file','application/octet-stream',5,'cancellation.bin',0,?)")
            .bind((now()+3600000) as i64).execute(&app.0.pool).await.unwrap();
        // Model the exact commit-before-guard-completion cancellation window.
        drop(UploadGuard {
            app: app.clone(),
            key: key.into(),
            paths: vec![path.clone()],
            complete: false,
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(tokio::fs::try_exists(&path).await.unwrap());
        // Failed pre-commit work must still be removed and settled.
        let path2 = app.0.config.data_dir.join("files/aborted.tmp");
        tokio::fs::write(&path2, b"bad").await.unwrap();
        sqlx::query("INSERT INTO upload_daily_usage VALUES(1,0,3)")
            .execute(&app.0.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO upload_reservations(key,user_id,utc_day,reserved_bytes) VALUES('aborted',1,0,3)")
            .execute(&app.0.pool)
            .await
            .unwrap();
        drop(UploadGuard {
            app: app.clone(),
            key: "aborted".into(),
            paths: vec![path2.clone()],
            complete: false,
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while tokio::fs::try_exists(&path2).await.unwrap() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let charged: i64 = sqlx::query_scalar(
            "SELECT charged_bytes FROM upload_daily_usage WHERE user_id=1 AND utc_day=0",
        )
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
        assert_eq!(charged, 3);
    }
}
