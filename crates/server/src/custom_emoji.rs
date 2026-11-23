//! Server-owned, bounded emoji assets. Every read rechecks membership; no public blob URLs.
use crate::{
    db::*,
    error::{require, Error, Result},
    http, permissions,
    state::App,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
    routing::get,
};
use image::{AnimationDecoder, ImageDecoder};
use sqlx::{Connection, Row, SqliteConnection};
use sticktalk_proto::*;
pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/guilds/{id}/emoji", get(list).post(upload))
        .route("/api/v1/emoji/{id}", get(asset))
        .route("/api/v1/emoji/{id}/preview", get(preview))
}
pub(crate) async fn emojis(c: &mut SqliteConnection, guild: Id) -> Result<Vec<GuildEmoji>> {
    Ok(
        sqlx::query("SELECT id,name,animated FROM guild_emoji WHERE guild_id=? ORDER BY name,id")
            .bind(id(guild)?)
            .fetch_all(c)
            .await?
            .into_iter()
            .map(|r| GuildEmoji {
                id: Id(r.get::<i64, _>("id") as u64),
                guild_id: guild,
                name: r.get("name"),
                animated: r.get("animated"),
            })
            .collect(),
    )
}
async fn manage(c: &mut SqliteConnection, u: Id, g: Id) -> Result<()> {
    require(
        permissions::evaluate(c, u, g, None)
            .await?
            .contains(Permissions::MANAGE_EMOJI),
        ErrorCode::Forbidden,
    )
}
async fn list(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    guild(&mut c, u, Id(g)).await?;
    Ok(http::json(Snapshot {
        data: emojis(&mut c, Id(g)).await?,
        cursor: core.cursor(&key, u),
    }))
}
#[derive(serde::Deserialize)]
struct Name {
    name: String,
}
async fn upload(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Query<Name>,
    h: HeaderMap,
    body: Body,
) -> Result<http::ApiResponse> {
    require(valid_emoji_name(&q.name), ErrorCode::InvalidRequest)?;
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let (key, u) = {
        let mut c = app.0.pool.acquire().await?;
        let a = auth(&mut c, http::bearer(&h)?).await?;
        manage(&mut c, a.1, Id(g)).await?;
        a
    };
    {
        let mut core = app.0.core.lock().await;
        core.rate(format!("emoji-upload:{}", u.0), 10., 2.)?;
    }
    let bytes = to_bytes(body, MAX_EMOJI_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let media = tokio::task::spawn_blocking(move || {
        // Cancelling a request cannot cancel a running blocking decoder. Keep
        // its permit inside that work so disconnects cannot bypass the bound.
        let _slot = slot;
        normalize(&bytes)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Unavailable))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Auth and permissions can change while bytes are being decoded.
    let (_, current) = auth(&mut c, http::bearer(&h)?).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    manage(&mut c, u, Id(g)).await?;
    // AutoMod screens the name (security review F6).
    crate::moderation::coverage::name_check(&mut core, &mut c, u, Id(g), "emoji name", &q.name)
        .await?;
    let mut tx = c.begin().await?;
    // Still and animated emoji have their own slots (expression.md).
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_emoji WHERE guild_id=? AND animated=?")
            .bind(g as i64)
            .bind(media.animated)
            .fetch_one(&mut *tx)
            .await?;
    let max = if media.animated {
        MAX_GUILD_ANIMATED_EMOJI
    } else {
        MAX_GUILD_STATIC_EMOJI
    };
    if n >= max as i64 {
        let mut e = Error::new(ErrorCode::LimitExceeded);
        e.0.message = format!(
            "This server already has {max} {} emoji.",
            if media.animated { "animated" } else { "still" }
        );
        return Err(e);
    }
    unique_name(&mut tx, Id(g), None, &q.name).await?;
    let eid = core.id().await?;
    sqlx::query("INSERT INTO guild_emoji(id,guild_id,name,animated,content_type,data,preview,creator_id,created_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(id(eid)?)
        .bind(g as i64)
        .bind(&q.name)
        .bind(media.animated)
        .bind(media.mime)
        .bind(media.data)
        .bind(media.preview)
        .bind(id(u)?)
        .bind(crate::state::now() as i64)
        .execute(&mut *tx)
        .await?;
    crate::moderation::audit::emoji_created(&mut tx, u, Id(g), eid, &q.name).await?;
    core.persist_id(&mut tx).await?;
    let data = emojis(&mut tx, Id(g)).await?;
    let users = guild_users(&mut tx, Id(g)).await?;
    tx.commit().await?;
    core.emit(
        &users,
        Event::GuildEmojiUpdated {
            guild_id: Id(g),
            emoji: data.clone(),
        },
    )?;
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}
async fn unique_name(c: &mut SqliteConnection, g: Id, e: Option<Id>, name: &str) -> Result<()> {
    require(valid_emoji_name(name), ErrorCode::InvalidRequest)?;
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM guild_emoji WHERE guild_id=? AND name=? COLLATE NOCASE AND id<>?",
    )
    .bind(id(g)?)
    .bind(name)
    .bind(e.map_or(0, |e| e.0) as i64)
    .fetch_one(c)
    .await?;
    if n != 0 {
        let mut e = Error::new(ErrorCode::AlreadyExists);
        e.0.message = "An emoji with that name already exists.".into();
        e.0.field = Some("name".into());
        return Err(e);
    }
    Ok(())
}
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    e: Id,
    name: Option<String>,
    out: &mut Vec<(Vec<Id>, Event)>,
) -> Result<CommandResult> {
    manage(c, u, g).await?;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_emoji WHERE guild_id=? AND id=?")
            .bind(id(g)?)
            .bind(id(e)?)
            .fetch_one(&mut *c)
            .await?;
    require(exists == 1, ErrorCode::NotFound)?;
    if let Some(name) = name {
        unique_name(c, g, Some(e), &name).await?;
        sqlx::query("UPDATE guild_emoji SET name=? WHERE id=?")
            .bind(name)
            .bind(id(e)?)
            .execute(&mut *c)
            .await?;
    } else {
        sqlx::query("DELETE FROM guild_emoji WHERE id=?")
            .bind(id(e)?)
            .execute(&mut *c)
            .await?;
    }
    out.push((
        guild_users(c, g).await?,
        Event::GuildEmojiUpdated {
            guild_id: g,
            emoji: emojis(c, g).await?,
        },
    ));
    Ok(CommandResult::Ack)
}
async fn asset(State(app): State<App>, Path(e): Path<u64>, h: HeaderMap) -> Result<Response> {
    serve(app, Id(e), h, false).await
}
async fn preview(State(app): State<App>, Path(e): Path<u64>, h: HeaderMap) -> Result<Response> {
    serve(app, Id(e), h, true).await
}
async fn serve(app: App, e: Id, h: HeaderMap, preview: bool) -> Result<Response> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
    // The shared budget for pictures (security review F5). The global lock is
    // held for the budget only: the checks below just read, and a browser
    // that has the image asks with If-None-Match and gets a 304.
    app.0
        .core
        .lock()
        .await
        .rate(format!("file:{}", u.0), 600., 120.)?;
    let r = sqlx::query("SELECT * FROM guild_emoji WHERE id=?")
        .bind(id(e)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    // Members of its server, or readers of a conversation it was used in.
    let g = Id(r.get::<i64, _>("guild_id") as u64);
    require(
        crate::expression::may_see(&mut c, u, "emoji", e, g).await?
            || crate::community::featured_emoji(&mut c, g, e).await?,
        ErrorCode::NotFound,
    )?;
    // An emoji's image never changes (only its name does).
    let tag = crate::caching::etag("emoji", e.0, if preview { "still" } else { "image" });
    if crate::caching::fresh(&h, &tag) {
        return Ok(crate::caching::not_modified(
            crate::caching::Policy::Checked,
            &tag,
        ));
    }
    let bytes: Vec<u8> = r.get(if preview && r.get::<bool, _>("animated") {
        "preview"
    } else {
        "data"
    });
    let mime: String = if preview {
        "image/png".into()
    } else {
        r.get("content_type")
    };
    Ok(crate::caching::image(
        crate::caching::Policy::Checked,
        &tag,
        &mime,
        bytes,
    ))
}
struct Media {
    data: Vec<u8>,
    preview: Vec<u8>,
    mime: &'static str,
    animated: bool,
}
fn invalid() -> Error {
    Error::new(ErrorCode::UnsupportedMediaType)
}
fn normalize(bytes: &[u8]) -> Result<Media> {
    require(
        !bytes.is_empty() && bytes.len() <= MAX_EMOJI_BYTES,
        ErrorCode::PayloadTooLarge,
    )?;
    let format = image::guess_format(bytes).map_err(|_| invalid())?;
    require(
        matches!(
            format,
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
        ),
        ErrorCode::UnsupportedMediaType,
    )?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    let reader = std::io::Cursor::new(bytes);
    let frames = match format {
        image::ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            Some(d.into_frames())
        }
        image::ImageFormat::WebP => {
            let mut d = image::codecs::webp::WebPDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            if d.has_animation() {
                Some(d.into_frames())
            } else {
                None
            }
        }
        image::ImageFormat::Png => {
            let mut d = image::codecs::png::PngDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            if d.is_apng().map_err(|_| invalid())? {
                Some(d.apng().map_err(|_| invalid())?.into_frames())
            } else {
                None
            }
        }
        _ => None,
    };
    let mut normalized = Vec::new();
    if let Some(frames) = frames {
        let mut decoded = 0usize;
        for f in frames {
            require(normalized.len() < 120, ErrorCode::LimitExceeded)?;
            let f = f.map_err(|_| invalid())?;
            decoded = decoded.saturating_add(f.buffer().as_raw().len());
            require(decoded <= 32 * 1024 * 1024, ErrorCode::LimitExceeded)?;
            let delay = f.delay();
            let img = image::DynamicImage::ImageRgba8(f.into_buffer())
                .thumbnail(128, 128)
                .into_rgba8();
            normalized.push(image::Frame::from_parts(img, 0, 0, delay));
        }
    } else {
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        reader.limits(limits);
        normalized.push(image::Frame::new(
            reader
                .decode()
                .map_err(|_| invalid())?
                .thumbnail(128, 128)
                .into_rgba8(),
        ));
    }
    let first = normalized.first().ok_or_else(invalid)?;
    let mut preview = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(first.buffer().clone())
        .write_to(&mut preview, image::ImageFormat::Png)
        .map_err(|_| invalid())?;
    let preview = preview.into_inner();
    let animated = normalized.len() > 1;
    let (data, mime) = if animated {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)
                .map_err(|_| invalid())?;
            enc.encode_frames(normalized).map_err(|_| invalid())?;
        }
        (out, "image/gif")
    } else {
        (preview.clone(), "image/png")
    };
    require(data.len() <= MAX_EMOJI_BYTES, ErrorCode::PayloadTooLarge)?;
    Ok(Media {
        data,
        preview: if animated { preview } else { Vec::new() },
        mime,
        animated,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audit_tests::fixture,
        commands,
        state::{hash, now},
    };
    use axum::http::{Request, StatusCode};
    use base64::Engine;
    use tower::ServiceExt;
    fn png() -> Vec<u8> {
        let mut v = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(512, 256)
            .write_to(&mut v, image::ImageFormat::Png)
            .unwrap();
        v.into_inner()
    }
    async fn token(app: &App, u: u8) -> String {
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u; 32]);
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(hash(token.as_bytes()))
            .bind(i64::from(u))
            .bind(now() as i64)
            .bind((now() + 600000) as i64)
            .bind(now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        token
    }
    async fn request(app: &App, token: &str, path: &str, bytes: Option<Vec<u8>>) -> Response {
        app.0.core.lock().await.limits.clear(); // Isolate validation/ACLs from independent rate policy.
        let method = if bytes.is_some() { "POST" } else { "GET" };
        http::router(app.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(bytes.unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn cmd(app: &App, u: u64, n: u64, command: Command) -> CommandOutcome {
        let mut core = app.0.core.lock().await;
        core.limits.clear();
        commands::execute(
            app,
            &mut core,
            &[u as u8; 32],
            Id(u),
            CommandRequest { nonce: n, command },
        )
        .await
        .unwrap()
        .outcome
    }
    #[test]
    fn normalize_resizes_preserves_alpha_and_animation_and_rejects_bombs() {
        let v = normalize(&png()).unwrap();
        assert!(!v.animated);
        let img = image::load_from_memory(&v.data).unwrap();
        assert_eq!((img.width(), img.height()), (128, 64));
        assert_eq!(img.to_rgba8().get_pixel(0, 0).0[3], 0);
        let mut gif = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
            for c in [0, 255] {
                enc.encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(200, 200, image::Rgba([c, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(80, 1),
                ))
                .unwrap();
            }
        }
        let v = normalize(&gif).unwrap();
        assert!(v.animated);
        assert_eq!(
            image::codecs::gif::GifDecoder::new(std::io::Cursor::new(v.data))
                .unwrap()
                .into_frames()
                .count(),
            2
        );
        for bytes in [
            vec![0; MAX_EMOJI_BYTES + 1],
            b"<svg></svg>".to_vec(),
            b"GIF89a".to_vec(),
        ] {
            assert!(normalize(&bytes).is_err());
        }
    }
    #[tokio::test]
    async fn emoji_upload_names_permissions_assets_rename_delete_and_reactions() {
        let (_dir, app) = fixture().await;
        let owner = token(&app, 1).await;
        let member = token(&app, 2).await;
        let outsider = token(&app, 3).await;
        assert_eq!(
            request(
                &app,
                &member,
                "/api/v1/guilds/10/emoji?name=wave",
                Some(png())
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(&app, &owner, "/api/v1/guilds/10/emoji?name=x", Some(png()))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                &app,
                &owner,
                "/api/v1/guilds/10/emoji?name=wave",
                Some(vec![0; MAX_EMOJI_BYTES + 1])
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let r = request(
            &app,
            &owner,
            "/api/v1/guilds/10/emoji?name=wave",
            Some(png()),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let v: Snapshot<Vec<GuildEmoji>> =
            serde_json::from_slice(&to_bytes(r.into_body(), 65536).await.unwrap()).unwrap();
        let e = v.data[0].clone();
        assert_eq!(
            request(
                &app,
                &owner,
                "/api/v1/guilds/10/emoji?name=WAVE",
                Some(png())
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(&app, &member, &format!("/api/v1/emoji/{}", e.id.0), None)
                .await
                .status(),
            StatusCode::OK
        );
        assert!(
            !request(&app, &outsider, &format!("/api/v1/emoji/{}", e.id.0), None)
                .await
                .status()
                .is_success()
        );
        assert!(matches!(
            cmd(
                &app,
                2,
                1,
                Command::Launch(LaunchCommand::RenameEmoji {
                    guild_id: Id(10),
                    emoji_id: e.id,
                    name: "hello".into()
                })
            )
            .await,
            CommandOutcome::Err(_)
        ));
        assert!(matches!(
            cmd(
                &app,
                1,
                1,
                Command::Launch(LaunchCommand::RenameEmoji {
                    guild_id: Id(10),
                    emoji_id: e.id,
                    name: "hello".into()
                })
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        let m = cmd(
            &app,
            2,
            2,
            Command::SendMessage(MessageDraft {
                channel_id: Id(11),
                content: "hi".into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }),
        )
        .await;
        let CommandOutcome::Ok(CommandResult::Message(m)) = m else {
            panic!("{m:?}")
        };
        let token = format!("<:hello:{}>", e.id.0);
        assert!(matches!(
            cmd(
                &app,
                2,
                3,
                Command::AddReaction {
                    channel_id: Id(11),
                    message_id: m.id,
                    emoji: token.clone()
                }
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            cmd(
                &app,
                2,
                4,
                Command::AddReaction {
                    channel_id: Id(11),
                    message_id: m.id,
                    emoji: "<:fake:999>".into()
                }
            )
            .await,
            CommandOutcome::Err(_)
        ));
        assert!(matches!(
            cmd(
                &app,
                1,
                2,
                Command::Launch(LaunchCommand::DeleteEmoji {
                    guild_id: Id(10),
                    emoji_id: e.id
                })
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        assert_eq!(
            request(&app, &member, &format!("/api/v1/emoji/{}", e.id.0), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert!(matches!(
            cmd(
                &app,
                2,
                5,
                Command::RemoveReaction {
                    channel_id: Id(11),
                    message_id: m.id,
                    emoji: token
                }
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        // A role can grant emoji management without granting Manage Server.
        sqlx::query("UPDATE roles SET permissions=permissions|? WHERE guild_id=10 AND id=10")
            .bind(Permissions::MANAGE_EMOJI.bits() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        assert_eq!(
            request(
                &app,
                &member,
                "/api/v1/guilds/10/emoji?name=allowed",
                Some(png())
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
}
