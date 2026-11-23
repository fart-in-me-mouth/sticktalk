//! Custom camera backgrounds: up to ten per account, each a still image (PNG,
//! JPEG, WebP or GIF) or a short MP4/WebM loop of at most 10 MB. They are the
//! owner's alone: every read checks the account, and nobody else can list or
//! fetch them. The segmentation and compositing run in the owner's browser.
use super::*;
use axum::routing::get;

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route(
            "/api/v1/users/@me/camera-backgrounds",
            get(list).post(upload),
        )
        .route(
            "/api/v1/users/@me/camera-backgrounds/{id}",
            get(file).delete(remove),
        )
}

/// What the file is, from its own bytes; the client's claim is not asked.
pub(crate) fn sniff(bytes: &[u8]) -> Option<(BackgroundKind, &'static str)> {
    let image = BackgroundKind::Image;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some((image, "image/png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some((image, "image/jpeg"))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some((image, "image/webp"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some((image, "image/gif"))
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && [&b"isom"[..], b"iso2", b"mp41", b"mp42", b"avc1", b"M4V "].contains(&&bytes[8..12])
    {
        Some((BackgroundKind::Video, "video/mp4"))
    } else if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        Some((BackgroundKind::Video, "video/webm"))
    } else {
        None
    }
}

/// A still image must also be a real, sensibly sized picture.
fn check_image(bytes: &[u8]) -> Result<()> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    require(
        (16..=8192).contains(&w) && (16..=8192).contains(&h),
        ErrorCode::InvalidRequest,
    )
}

fn row(r: &sqlx::sqlite::SqliteRow) -> CameraBackground {
    CameraBackground {
        id: Id(r.get::<i64, _>("id") as u64),
        kind: if r.get::<String, _>("kind") == "video" {
            BackgroundKind::Video
        } else {
            BackgroundKind::Image
        },
        mime: r.get("mime"),
        bytes: r.get::<i64, _>("bytes") as u64,
        created_at: r.get::<i64, _>("created_at") as u64,
    }
}
async fn items(c: &mut SqliteConnection, u: Id) -> Result<Vec<CameraBackground>> {
    Ok(sqlx::query("SELECT id,kind,mime,length(data) AS bytes,created_at FROM camera_backgrounds WHERE user_id=? ORDER BY created_at,id")
        .bind(id(u)?)
        .fetch_all(c)
        .await?
        .iter()
        .map(row)
        .collect())
}
async fn list(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok(js(&items(&mut c, u).await?))
}
async fn upload(State(app): State<App>, h: HeaderMap, body: Body) -> Result<http::ApiResponse> {
    let token = token(&h)?;
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let u = {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = db::auth(&mut c, &token).await?;
        core.rate(format!("background-upload:{}", u.0), 10., 3.)?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM camera_backgrounds WHERE user_id=?")
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
        require(n < MAX_CAMERA_BACKGROUNDS as i64, ErrorCode::LimitExceeded)?;
        u
    };
    let bytes = to_bytes(body, MAX_CAMERA_BACKGROUND_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let (kind, mime) = sniff(&bytes).ok_or_else(|| {
        let mut e = Error::new(ErrorCode::UnsupportedMediaType);
        e.0.message = "Use a PNG, JPEG, WebP or GIF picture, or an MP4 or WebM video.".into();
        e
    })?;
    if kind == BackgroundKind::Image {
        let bytes = bytes.clone();
        // Header parsing only, off the async workers; the permit bounds it.
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            check_image(&bytes)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))??;
    } else {
        drop(slot);
    }
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, current) = db::auth(&mut c, &token).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM camera_backgrounds WHERE user_id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    require(n < MAX_CAMERA_BACKGROUNDS as i64, ErrorCode::LimitExceeded)?;
    let bid = core.id().await?;
    sqlx::query("INSERT INTO camera_backgrounds VALUES(?,?,?,?,?,?)")
        .bind(id(bid)?)
        .bind(id(u)?)
        .bind(if kind == BackgroundKind::Video {
            "video"
        } else {
            "image"
        })
        .bind(mime)
        .bind(&bytes[..])
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    core.persist_id(&mut c).await?;
    let all = items(&mut c, u).await?;
    let created = all
        .into_iter()
        .find(|b| b.id == bid)
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    Ok(js(&created))
}
async fn file(State(app): State<App>, Path(b): Path<u64>, h: HeaderMap) -> Result<Response> {
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    let r = sqlx::query("SELECT mime,data FROM camera_backgrounds WHERE id=? AND user_id=?")
        .bind(id(Id(b))?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let mime: String = r.get("mime");
    let bytes: Vec<u8> = r.get("data");
    Ok((
        StatusCode::OK,
        [
            ("content-type", mime),
            ("cache-control", "private, no-store".into()),
            ("x-content-type-options", "nosniff".into()),
            (
                "content-security-policy",
                "default-src 'none'; sandbox".into(),
            ),
        ],
        bytes,
    )
        .into_response())
}
async fn remove(State(app): State<App>, Path(b): Path<u64>, h: HeaderMap) -> Result<Response> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let done = sqlx::query("DELETE FROM camera_backgrounds WHERE id=? AND user_id=?")
        .bind(id(Id(b))?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    require(done.rows_affected() == 1, ErrorCode::NotFound)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
