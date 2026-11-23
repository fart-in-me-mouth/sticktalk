//! Server stickers and the built-in pack: upload (REST), rename, delete,
//! sending, and what a message carries.
use super::*;
use crate::{http, state::App};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use image::{AnimationDecoder, ImageDecoder};
use sqlx::Connection;

fn sticker_row(r: &sqlx::sqlite::SqliteRow) -> Sticker {
    Sticker {
        id: Id(r.get::<i64, _>("id") as u64),
        guild_id: Some(Id(r.get::<i64, _>("guild_id") as u64)),
        name: r.get("name"),
        emoji: r.get("emoji"),
        description: r.get("description"),
        format: StickerFormat::parse(&r.get::<String, _>("format")).unwrap_or(StickerFormat::Png),
        animated: r.get("animated"),
        creator_id: r
            .get::<Option<i64>, _>("creator_id")
            .map(|v| Id(v as u64)),
        created_at: r.get::<i64, _>("created_at") as u64,
    }
}

pub(crate) async fn list(c: &mut SqliteConnection, g: Id) -> Result<Vec<Sticker>> {
    Ok(sqlx::query(
        "SELECT id,guild_id,name,emoji,description,format,animated,creator_id,created_at FROM guild_stickers WHERE guild_id=? ORDER BY name,id",
    )
    .bind(id(g)?)
    .fetch_all(&mut *c)
    .await?
    .iter()
    .map(sticker_row)
    .collect())
}
async fn find(c: &mut SqliteConnection, s: Id) -> Result<Option<Sticker>> {
    if let Some(b) = builtin_sticker(s) {
        return Ok(Some(b));
    }
    Ok(sqlx::query(
        "SELECT id,guild_id,name,emoji,description,format,animated,creator_id,created_at FROM guild_stickers WHERE id=?",
    )
    .bind(id(s)?)
    .fetch_optional(&mut *c)
    .await?
    .as_ref()
    .map(sticker_row))
}

async fn unique_name(c: &mut SqliteConnection, g: Id, s: Option<Id>, name: &str) -> Result<()> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM guild_stickers WHERE guild_id=? AND name=? COLLATE NOCASE AND id<>?",
    )
    .bind(id(g)?)
    .bind(name)
    .bind(s.map_or(0, |s| s.0) as i64)
    .fetch_one(&mut *c)
    .await?;
    if n != 0 {
        let mut e = Error::new(ErrorCode::AlreadyExists);
        e.0.message = "A sticker with that name already exists.".into();
        e.0.field = Some("name".into());
        return Err(e);
    }
    Ok(())
}

/// Name, related emoji and description, checked as one.
pub(super) struct Edit {
    pub name: String,
    pub emoji: String,
    pub description: String,
}
impl Edit {
    fn validate(self) -> Result<Self> {
        let name = self.name.trim().to_owned();
        let description = self.description.trim().to_owned();
        let emoji = self.emoji.trim().to_owned();
        require(
            valid_sticker_name(&name)
                && valid_sticker_description(&description)
                && unicode_emoji(&emoji),
            ErrorCode::InvalidRequest,
        )?;
        Ok(Self {
            name,
            emoji,
            description,
        })
    }
}

async fn changed(c: &mut SqliteConnection, g: Id, out: &mut Deliveries) -> Result<()> {
    let stickers = list(c, g).await?;
    out.push((
        guild_users(c, g).await?,
        Event::Expression(ExpressionEvent::StickersUpdated {
            guild_id: g,
            stickers,
        }),
    ));
    Ok(())
}

async fn audit(
    c: &mut SqliteConnection,
    actor: Id,
    g: Id,
    action: AuditAction,
    sticker: Id,
    before: Option<&Sticker>,
    after: Option<&Sticker>,
) -> Result<()> {
    use crate::moderation::audit::{write, Entry};
    let name = after.or(before).map(|s| s.name.clone()).unwrap_or_default();
    let mut entry = Entry::new(g, action, AuditTarget::Sticker, Some(sticker)).named(name);
    let pick = |s: Option<&Sticker>, f: fn(&Sticker) -> &str| {
        s.map(|s| f(s).to_owned()).filter(|v| !v.is_empty())
    };
    for (field, get) in [
        ("name", (|s: &Sticker| s.name.as_str()) as fn(&Sticker) -> &str),
        ("emoji", |s: &Sticker| s.emoji.as_str()),
        ("description", |s: &Sticker| s.description.as_str()),
    ] {
        let (b, a) = (pick(before, get), pick(after, get));
        if b != a {
            entry = entry.change(field, b, a);
        }
    }
    write(c, actor, entry).await
}

pub(super) async fn update(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    s: Id,
    edit: Edit,
    out: &mut Deliveries,
) -> Result<ExpressionResult> {
    manage(c, u, g).await?;
    let before = find(c, s)
        .await?
        .filter(|v| v.guild_id == Some(g))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let edit = edit.validate()?;
    unique_name(c, g, Some(s), &edit.name).await?;
    sqlx::query("UPDATE guild_stickers SET name=?,emoji=?,description=? WHERE id=?")
        .bind(&edit.name)
        .bind(&edit.emoji)
        .bind(&edit.description)
        .bind(id(s)?)
        .execute(&mut *c)
        .await?;
    let after = find(c, s)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    if after != before {
        audit(c, u, g, AuditAction::StickerUpdate, s, Some(&before), Some(&after)).await?;
        changed(c, g, out).await?;
    }
    Ok(ExpressionResult::Sticker(after))
}

pub(super) async fn delete(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    s: Id,
    out: &mut Deliveries,
) -> Result<ExpressionResult> {
    manage(c, u, g).await?;
    let before = find(c, s)
        .await?
        .filter(|v| v.guild_id == Some(g))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    sqlx::query("DELETE FROM guild_stickers WHERE id=?")
        .bind(id(s)?)
        .execute(&mut *c)
        .await?;
    audit(c, u, g, AuditAction::StickerDelete, s, Some(&before), None).await?;
    changed(c, g, out).await?;
    Ok(ExpressionResult::Ack)
}

/// Favorites: built-in stickers, or stickers of servers the person is in.
pub(super) async fn validate_favorites(
    c: &mut SqliteConnection,
    u: Id,
    ids: &[Id],
) -> Result<()> {
    require(
        ids.len() <= MAX_STICKER_FAVORITES,
        ErrorCode::LimitExceeded,
    )?;
    for (i, s) in ids.iter().enumerate() {
        require(!ids[..i].contains(s), ErrorCode::InvalidRequest)?;
        let sticker = find(c, *s)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        if let Some(g) = sticker.guild_id {
            require(role(c, u, g).await.is_ok(), ErrorCode::InvalidRequest)?;
        }
    }
    Ok(())
}

pub(super) struct Send {
    pub channel_id: Id,
    pub sticker_ids: Vec<Id>,
    pub content: String,
    pub reply_to: Option<Id>,
    pub mention_ids: Vec<Id>,
}

/// One to three stickers, with optional text, through the ordinary send path
/// (permissions, slowmode, mentions, AutoMod and timeouts all apply).
pub(super) async fn send(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    s: Send,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    require(
        (1..=MAX_MESSAGE_STICKERS).contains(&s.sticker_ids.len()),
        ErrorCode::InvalidRequest,
    )?;
    let channel = text(c, u, s.channel_id, Permissions::SEND_MESSAGES).await?;
    let mut items = vec![];
    for (i, sid) in s.sticker_ids.iter().enumerate() {
        require(!s.sticker_ids[..i].contains(sid), ErrorCode::InvalidRequest)?;
        let sticker = find(c, *sid)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        if let Some(g) = sticker.guild_id {
            if !usable(c, u, &channel.summary, g, Permissions::USE_EXTERNAL_STICKERS).await? {
                let mut e = Error::new(ErrorCode::Forbidden);
                e.0.message = "You can't use that sticker here.".into();
                e.0.field = Some("sticker".into());
                return Err(e);
            }
        }
        items.push(sticker.item());
    }
    let text_given = !s.content.trim().is_empty();
    // The ordinary send needs some text. A stickers-only message is stored
    // with none: the placeholder is cleared below, in the same transaction.
    let draft = MessageDraft {
        channel_id: s.channel_id,
        content: if text_given {
            s.content
        } else {
            items
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        reply_to: s.reply_to,
        attachment_ids: vec![],
        mention_ids: if text_given { s.mention_ids } else { vec![] },
    };
    let CommandResult::Message(mut message) =
        crate::content::send(c, core, key, u, draft, out).await?
    else {
        return Err(Error::new(ErrorCode::Internal));
    };
    for (i, item) in items.iter().enumerate() {
        sqlx::query("INSERT INTO message_stickers(message_id,position,sticker_id,guild_id,name,description,format,animated) VALUES(?,?,?,?,?,?,?,?)")
            .bind(id(message.id)?)
            .bind(i as i64)
            .bind(id(item.id)?)
            .bind(opt(item.guild_id)?)
            .bind(&item.name)
            .bind(&item.description)
            .bind(item.format.name())
            .bind(item.animated)
            .execute(&mut *c)
            .await?;
        if item.guild_id.is_some() {
            grant(c, "sticker", item.id, s.channel_id).await?;
        }
    }
    if !text_given {
        sqlx::query("UPDATE messages SET content='' WHERE id=?")
            .bind(id(message.id)?)
            .execute(&mut *c)
            .await?;
        message.content.clear();
    }
    message.extra.stickers = items;
    for (_, e) in out.iter_mut() {
        if let Event::MessageCreated { message: m, .. } = e {
            if m.id == message.id {
                m.content = message.content.clone();
                m.extra.stickers = message.extra.stickers.clone();
            }
        }
    }
    Ok(CommandResult::Message(message))
}

/// The stickers a message carries, marked deleted once their server removes them.
pub(super) async fn load(c: &mut SqliteConnection, m: Id) -> Result<Vec<StickerItem>> {
    Ok(sqlx::query("SELECT ms.*,(ms.guild_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM guild_stickers s WHERE s.id=ms.sticker_id)) AS gone FROM message_stickers ms WHERE ms.message_id=? ORDER BY ms.position")
        .bind(id(m)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| StickerItem {
            id: Id(r.get::<i64, _>("sticker_id") as u64),
            name: r.get("name"),
            format: StickerFormat::parse(&r.get::<String, _>("format")).unwrap_or(StickerFormat::Png),
            animated: r.get("animated"),
            guild_id: r.get::<Option<i64>, _>("guild_id").map(|v| Id(v as u64)),
            description: r.get("description"),
            deleted: r.get::<i64, _>("gone") != 0,
        })
        .collect())
}

#[derive(serde::Deserialize)]
pub(super) struct UploadQuery {
    name: String,
    emoji: String,
    #[serde(default)]
    description: String,
}

/// `POST /api/v1/guilds/{id}/stickers?name=&emoji=&description=` with the image as the body.
pub(super) async fn upload(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Query<UploadQuery>,
    h: HeaderMap,
    body: Body,
) -> Result<http::ApiResponse> {
    let edit = Edit {
        name: q.name,
        emoji: q.emoji,
        description: q.description,
    }
    .validate()?;
    let g = Id(g);
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let u = {
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
        manage(&mut c, u, g).await?;
        u
    };
    app.0
        .core
        .lock()
        .await
        .rate(format!("sticker-upload:{}", u.0), 10., 2.)?;
    let bytes = to_bytes(body, MAX_STICKER_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let media = tokio::task::spawn_blocking(move || {
        // The permit stays with the decoder: a disconnect cannot cancel it.
        let _slot = slot;
        normalize(&bytes)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Unavailable))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Rights can change while the image is decoded.
    let (_, current) = auth(&mut c, http::bearer(&h)?).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    manage(&mut c, u, g).await?;
    let mut tx = c.begin().await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_stickers WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_one(&mut *tx)
        .await?;
    if n as usize >= MAX_GUILD_STICKERS {
        let mut e = Error::new(ErrorCode::LimitExceeded);
        e.0.message = format!("This server already has {MAX_GUILD_STICKERS} stickers.");
        return Err(e);
    }
    unique_name(&mut tx, g, None, &edit.name).await?;
    let sid = core.id().await?;
    sqlx::query("INSERT INTO guild_stickers(id,guild_id,name,emoji,description,format,animated,data,still,creator_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(id(sid)?)
        .bind(id(g)?)
        .bind(&edit.name)
        .bind(&edit.emoji)
        .bind(&edit.description)
        .bind(media.format.name())
        .bind(media.animated)
        .bind(media.data)
        .bind(media.still)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *tx)
        .await?;
    let created = find(&mut tx, sid)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    audit(&mut tx, u, g, AuditAction::StickerCreate, sid, None, Some(&created)).await?;
    core.persist_id(&mut tx).await?;
    let mut out = vec![];
    changed(&mut tx, g, &mut out).await?;
    tx.commit().await?;
    for (users, e) in out {
        core.emit(&users, e)?;
    }
    Ok(http::json(created))
}

/// `GET /api/v1/stickers/{id}` (and `/still`): members of its server, or
/// anyone who can read a conversation it was sent in.
pub(super) async fn asset(
    State(app): State<App>,
    Path(s): Path<u64>,
    h: HeaderMap,
) -> Result<Response> {
    serve(app, Id(s), h, false).await
}
pub(super) async fn still(
    State(app): State<App>,
    Path(s): Path<u64>,
    h: HeaderMap,
) -> Result<Response> {
    serve(app, Id(s), h, true).await
}
async fn serve(app: App, s: Id, h: HeaderMap, still: bool) -> Result<Response> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("file:{}", u.0), 600., 120.)?;
    let r = sqlx::query("SELECT guild_id,format,animated,data,still FROM guild_stickers WHERE id=?")
        .bind(id(s)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let g = Id(r.get::<i64, _>("guild_id") as u64);
    require(
        may_see(&mut c, u, "sticker", s, g).await?,
        ErrorCode::NotFound,
    )?;
    let format = StickerFormat::parse(&r.get::<String, _>("format")).unwrap_or(StickerFormat::Png);
    let (kind, bytes): (&str, Vec<u8>) = if still && r.get::<bool, _>("animated") {
        ("image/png", r.get("still"))
    } else {
        (format.content_type(), r.get("data"))
    };
    Ok(asset_response(kind, bytes))
}
/// Image bytes this server checked: cached privately, never sniffed or run.
pub(crate) fn asset_response(kind: &str, bytes: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", kind.to_owned()),
            ("cache-control", "private, max-age=86400".into()),
            ("x-content-type-options", "nosniff".into()),
            (
                "content-security-policy",
                "default-src 'none'; sandbox".into(),
            ),
        ],
        bytes,
    )
        .into_response()
}

pub(super) struct Media {
    pub data: Vec<u8>,
    pub still: Vec<u8>,
    pub format: StickerFormat,
    pub animated: bool,
}
fn invalid() -> Error {
    let mut e = Error::new(ErrorCode::UnsupportedMediaType);
    e.0.message = "Stickers must be PNG, APNG, GIF or WebP images.".into();
    e
}
fn png(img: &image::RgbaImage) -> Result<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|_| invalid())?;
    Ok(out.into_inner())
}

/// Decodes the whole image (every frame) before accepting it. A still image
/// is re-encoded as PNG at most 320 px; an animated one must already be at
/// most 320 px and is stored as sent, with a still first frame beside it.
pub(super) fn normalize(bytes: &[u8]) -> Result<Media> {
    require(
        !bytes.is_empty() && bytes.len() <= MAX_STICKER_BYTES,
        ErrorCode::PayloadTooLarge,
    )?;
    let format = image::guess_format(bytes).map_err(|_| invalid())?;
    require(
        matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Gif | image::ImageFormat::WebP
        ),
        ErrorCode::UnsupportedMediaType,
    )?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let reader = std::io::Cursor::new(bytes);
    let (frames, kind) = match format {
        image::ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            (Some(d.into_frames()), StickerFormat::Gif)
        }
        image::ImageFormat::WebP => {
            let mut d = image::codecs::webp::WebPDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            if d.has_animation() {
                (Some(d.into_frames()), StickerFormat::Webp)
            } else {
                (None, StickerFormat::Webp)
            }
        }
        _ => {
            let mut d = image::codecs::png::PngDecoder::new(reader).map_err(|_| invalid())?;
            d.set_limits(limits.clone()).map_err(|_| invalid())?;
            if d.is_apng().map_err(|_| invalid())? {
                (
                    Some(d.apng().map_err(|_| invalid())?.into_frames()),
                    StickerFormat::Apng,
                )
            } else {
                (None, StickerFormat::Png)
            }
        }
    };
    let mut first = None;
    let mut count = 0usize;
    if let Some(frames) = frames {
        let mut decoded = 0usize;
        for f in frames {
            count += 1;
            require(count <= 600, ErrorCode::LimitExceeded)?;
            let f = f.map_err(|_| invalid())?;
            let buffer = f.into_buffer();
            require(
                buffer.width() <= STICKER_STORED_PX && buffer.height() <= STICKER_STORED_PX,
                ErrorCode::InvalidRequest,
            )?;
            decoded = decoded.saturating_add(buffer.as_raw().len());
            require(decoded <= 256 * 1024 * 1024, ErrorCode::LimitExceeded)?;
            if first.is_none() {
                first = Some(buffer);
            }
        }
    }
    if count > 1 {
        let still = png(&first.ok_or_else(invalid)?)?;
        return Ok(Media {
            data: bytes.to_vec(),
            still,
            format: kind,
            animated: true,
        });
    }
    // A still image (or a one-frame animation): decode, fit, re-encode.
    let img = match first {
        Some(frame) => image::DynamicImage::ImageRgba8(frame),
        None => {
            let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
            reader.limits(limits);
            reader.decode().map_err(|_| invalid())?
        }
    };
    let img = img
        .thumbnail(STICKER_STORED_PX, STICKER_STORED_PX)
        .into_rgba8();
    let data = png(&img)?;
    require(data.len() <= MAX_STICKER_BYTES, ErrorCode::PayloadTooLarge)?;
    Ok(Media {
        data,
        still: Vec::new(),
        format: StickerFormat::Png,
        animated: false,
    })
}
