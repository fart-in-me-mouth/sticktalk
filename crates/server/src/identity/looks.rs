//! Server images (banner, icon with animation and crop, invite background)
//! and role styles and icons.
//!
//! Server images are avatar uploads, so they get the profile validation (every
//! frame decoded, capped) and a still first-frame rendition; the server keeps
//! them in `guild_uploads`, which also roots them against upload collection.
//! Role icon pictures are small: decoded in full and stored as a 64x64 PNG
//! rendition beside the original, in the database.
use super::*;
use crate::{http, state::App};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use std::collections::HashMap;

fn slot_name(s: GuildImageSlot) -> &'static str {
    match s {
        GuildImageSlot::Icon => "icon",
        GuildImageSlot::Banner => "banner",
        GuildImageSlot::InviteBackground => "invite_background",
    }
}

/// An uploaded image `u` may use for server `g`: their own fresh avatar
/// upload, or one this server already uses.
async fn claim(c: &mut SqliteConnection, u: Id, g: Id, r: ImageRef) -> Result<ImageMeta> {
    require(r.crop.is_none_or(|c| c.valid()), ErrorCode::InvalidRequest)?;
    let row = sqlx::query("SELECT owner_id,content_type,storage_key,expires_at FROM uploads WHERE id=? AND purpose='avatar'")
        .bind(id(r.id)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidAttachment))?;
    let fresh = row.get::<i64, _>("owner_id") == id(u)?
        && row
            .get::<Option<i64>, _>("expires_at")
            .is_some_and(|t| t > now() as i64);
    let in_use: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM guild_uploads WHERE guild_id=? AND upload_id=?)",
    )
    .bind(id(g)?)
    .bind(id(r.id)?)
    .fetch_one(&mut *c)
    .await?;
    require(fresh || in_use, ErrorCode::InvalidAttachment)?;
    let mime: String = row.get("content_type");
    require(
        matches!(
            mime.as_str(),
            "image/png" | "image/apng" | "image/jpeg" | "image/webp" | "image/gif"
        ),
        ErrorCode::UnsupportedMediaType,
    )?;
    sqlx::query("UPDATE uploads SET expires_at=NULL WHERE id=?")
        .bind(id(r.id)?)
        .execute(&mut *c)
        .await?;
    let animated = crate::profiles::media::animated(&row.get::<String, _>("storage_key"))
        .unwrap_or(matches!(mime.as_str(), "image/gif" | "image/apng"));
    Ok(ImageMeta {
        id: r.id,
        animated,
        crop: r.crop.filter(|c| !c.is_identity()),
    })
}
async fn track(
    c: &mut SqliteConnection,
    g: Id,
    slot: GuildImageSlot,
    m: Option<&ImageMeta>,
) -> Result<()> {
    sqlx::query("DELETE FROM guild_uploads WHERE guild_id=? AND slot=?")
        .bind(id(g)?)
        .bind(slot_name(slot))
        .execute(&mut *c)
        .await?;
    if let Some(m) = m {
        sqlx::query("INSERT INTO guild_uploads(guild_id,slot,upload_id) VALUES(?,?,?)")
            .bind(id(g)?)
            .bind(slot_name(slot))
            .bind(id(m.id)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
pub(super) async fn invite_background(
    c: &mut SqliteConnection,
    g: Id,
) -> Result<Option<ImageMeta>> {
    let s: Option<Option<String>> =
        sqlx::query_scalar("SELECT invite_background FROM guild_looks WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(parse(s.flatten()))
}
/// The icon as previews draw it: its animation and crop when the look
/// describes the current icon.
pub(super) async fn icon(c: &mut SqliteConnection, g: Id) -> Result<Option<ImageMeta>> {
    let icon: Option<Option<i64>> = sqlx::query_scalar("SELECT icon_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    let Some(icon) = icon.flatten().map(|v| Id(v as u64)) else {
        return Ok(None);
    };
    let look = guild_look(c, g).await?;
    Ok(Some(look.icon.filter(|m| m.id == icon).unwrap_or(
        ImageMeta {
            id: icon,
            animated: false,
            crop: None,
        },
    )))
}
fn described(m: &Option<ImageMeta>) -> Option<String> {
    m.as_ref().map(|m| {
        format!(
            "{}{}",
            if m.animated {
                "animated picture"
            } else {
                "picture"
            },
            if m.crop.is_some() { ", cropped" } else { "" }
        )
    })
}

pub(super) async fn set_image(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    slot: GuildImageSlot,
    image: Option<ImageRef>,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let meta = match image {
        Some(r) => Some(claim(c, u, g, r).await?),
        None => None,
    };
    let mut look = guild_look(c, g).await?;
    let before = match slot {
        GuildImageSlot::Icon => icon(c, g).await?,
        GuildImageSlot::Banner => look.banner.clone(),
        GuildImageSlot::InviteBackground => invite_background(c, g).await?,
    };
    if before == meta {
        return Ok(CommandResult::Ack);
    }
    track(c, g, slot, meta.as_ref()).await?;
    match slot {
        GuildImageSlot::Icon => {
            sqlx::query("UPDATE guilds SET icon_id=? WHERE id=?")
                .bind(opt(meta.as_ref().map(|m| m.id))?)
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
            look.icon = meta.clone().filter(|m| m.animated || m.crop.is_some());
            save_guild_look(c, g, &look).await?;
        }
        GuildImageSlot::Banner => {
            look.banner = meta.clone();
            save_guild_look(c, g, &look).await?;
        }
        GuildImageSlot::InviteBackground => {
            sqlx::query("INSERT INTO guild_looks(guild_id,look,invite_background) VALUES(?,?,?) ON CONFLICT(guild_id) DO UPDATE SET invite_background=excluded.invite_background")
                .bind(id(g)?)
                .bind(json(&look)?)
                .bind(meta.as_ref().map(json).transpose()?)
                .execute(&mut *c)
                .await?;
        }
    }
    let name: String = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(g, AuditAction::GuildUpdate, AuditTarget::Guild, Some(g))
            .named(name)
            .change(slot_name(slot), described(&before), described(&meta)),
    )
    .await?;
    if slot != GuildImageSlot::InviteBackground {
        guild_changed(c, g, out).await?;
    }
    Ok(CommandResult::Ack)
}

async fn role_of(c: &mut SqliteConnection, u: Id, g: Id, r: RoleId) -> Result<Role> {
    let role = crate::permissions::roles(c, g)
        .await?
        .into_iter()
        .find(|v| v.id == r)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    crate::permissions::manage_role(c, g, u, &role).await?;
    // @everyone is everyone: it has no style or icon of its own.
    require(role.id != g, ErrorCode::InvalidRequest)?;
    Ok(role)
}
async fn save_role_look(c: &mut SqliteConnection, g: Id, r: RoleId, l: &RoleLook) -> Result<()> {
    if l.is_empty() {
        sqlx::query("DELETE FROM role_looks WHERE guild_id=? AND role_id=?")
            .bind(id(g)?)
            .bind(id(r)?)
            .execute(&mut *c)
            .await?;
    } else {
        sqlx::query("INSERT INTO role_looks(guild_id,role_id,look) VALUES(?,?,?) ON CONFLICT(guild_id,role_id) DO UPDATE SET look=excluded.look")
            .bind(id(g)?)
            .bind(id(r)?)
            .bind(json(l)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
fn style_text(s: &Option<RoleStyle>) -> Option<String> {
    s.map(|s| match s.paint {
        RolePaint::Holographic => "holographic".into(),
        RolePaint::Gradient => format!(
            "gradient #{:06x}{}",
            s.color2.unwrap_or(0),
            s.color3.map(|c| format!(" #{c:06x}")).unwrap_or_default()
        ),
    })
}
fn icon_text(i: &Option<RoleIcon>) -> Option<String> {
    i.as_ref().map(|i| {
        if i.rev.is_some() {
            "picture".into()
        } else if i.emoji_id.is_some() {
            format!(":{}:", i.emoji)
        } else {
            i.emoji.clone()
        }
    })
}
async fn role_audit(
    c: &mut SqliteConnection,
    u: Id,
    role: &Role,
    field: &str,
    before: Option<String>,
    after: Option<String>,
) -> Result<()> {
    audit::write(
        c,
        u,
        audit::Entry::new(
            role.guild_id,
            AuditAction::RoleUpdate,
            AuditTarget::Role,
            Some(role.id),
        )
        .named(role.name.clone())
        .change(field, before, after),
    )
    .await
}

pub(super) async fn set_role_style(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    r: RoleId,
    style: Option<RoleStyle>,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let role = role_of(c, u, g, r).await?;
    if let Some(s) = &style {
        let colors = [s.color2, s.color3];
        require(
            colors.iter().flatten().all(|c| *c <= 0xFF_FFFF),
            ErrorCode::InvalidRequest,
        )?;
        match s.paint {
            RolePaint::Gradient => {
                if role.color == 0 || s.color2.is_none() {
                    return Err(invalid(
                        "style",
                        "A gradient starts at the role color: choose a color and a second color.",
                    ));
                }
            }
            RolePaint::Holographic => {
                require(
                    s.color2.is_none() && s.color3.is_none(),
                    ErrorCode::InvalidRequest,
                )?;
            }
        }
        require(
            s.color2.is_some() || s.color3.is_none(),
            ErrorCode::InvalidRequest,
        )?;
    }
    let mut look = role.look.as_deref().cloned().unwrap_or_default();
    if look.style == style {
        return Ok(CommandResult::Ack);
    }
    let before = style_text(&look.style);
    look.style = style;
    save_role_look(c, g, r, &look).await?;
    role_audit(c, u, &role, "style", before, style_text(&style)).await?;
    roles_changed(c, g, out).await?;
    Ok(CommandResult::Ack)
}

/// One Unicode emoji: a short run of pictographic characters and joiners.
fn unicode_emoji(s: &str) -> bool {
    let n = s.chars().count();
    (1..=10).contains(&n)
        && s.len() <= 40
        && s.chars().any(|c| u32::from(c) >= 0x2000)
        && s.chars().all(|c| {
            !c.is_control()
                && !c.is_whitespace()
                && !c.is_ascii_alphabetic()
                && (u32::from(c) >= 0x2000 || c.is_ascii_digit() || c == '#' || c == '*')
        })
}

pub(super) async fn set_role_icon(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    r: RoleId,
    icon: Option<RoleIconRef>,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let role = role_of(c, u, g, r).await?;
    let new = match icon {
        Some(RoleIconRef::Emoji { emoji }) => {
            let emoji = emoji.trim().to_owned();
            require(unicode_emoji(&emoji), ErrorCode::InvalidRequest)?;
            Some(RoleIcon::unicode(emoji))
        }
        Some(RoleIconRef::Custom { emoji_id }) => {
            let name: Option<String> =
                sqlx::query_scalar("SELECT name FROM guild_emoji WHERE id=? AND guild_id=?")
                    .bind(id(emoji_id)?)
                    .bind(id(g)?)
                    .fetch_optional(&mut *c)
                    .await?;
            let name = name.ok_or_else(|| invalid("icon", "Choose an emoji of this server."))?;
            Some(RoleIcon::custom(emoji_id, name))
        }
        None => None,
    };
    let mut look = role.look.as_deref().cloned().unwrap_or_default();
    if look.icon == new {
        return Ok(CommandResult::Ack);
    }
    let before = icon_text(&look.icon);
    look.icon = new;
    save_role_look(c, g, r, &look).await?;
    sqlx::query("DELETE FROM role_icons WHERE guild_id=? AND role_id=?")
        .bind(id(g)?)
        .bind(id(r)?)
        .execute(&mut *c)
        .await?;
    role_audit(c, u, &role, "icon", before, icon_text(&look.icon)).await?;
    roles_changed(c, g, out).await?;
    Ok(CommandResult::Ack)
}

/// Decode a role icon in full (PNG or WebP, at most 1024x1024 and 256 KB)
/// and fit it into a 64x64 PNG.
pub(crate) fn role_icon_rendition(bytes: &[u8]) -> Result<(Vec<u8>, &'static str)> {
    use image::ImageFormat;
    require(
        !bytes.is_empty() && bytes.len() <= MAX_ROLE_ICON_BYTES,
        ErrorCode::PayloadTooLarge,
    )?;
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        ImageFormat::WebP
    } else {
        return Err(Error::new(ErrorCode::UnsupportedMediaType));
    };
    let mut r = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut l = image::Limits::default();
    l.max_image_width = Some(MAX_ROLE_ICON_SIDE);
    l.max_image_height = Some(MAX_ROLE_ICON_SIDE);
    l.max_alloc = Some(32 * 1024 * 1024);
    r.limits(l);
    let img = r
        .decode()
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    require(
        img.width() > 0 && img.height() > 0,
        ErrorCode::UnsupportedMediaType,
    )?;
    let side = ROLE_ICON_RENDITION;
    let fitted = img.resize(side, side, image::imageops::FilterType::Lanczos3);
    let mut canvas = image::RgbaImage::new(side, side);
    let (x, y) = ((side - fitted.width()) / 2, (side - fitted.height()) / 2);
    image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), x.into(), y.into());
    let mut png = std::io::Cursor::new(vec![]);
    canvas
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    Ok((
        png.into_inner(),
        if format == ImageFormat::Png {
            "image/png"
        } else {
            "image/webp"
        },
    ))
}

/// `POST /api/v1/guilds/{g}/roles/{r}/icon` with the picture as the body.
pub(super) async fn upload_icon(
    State(app): State<App>,
    Path((g, r)): Path<(String, String)>,
    h: HeaderMap,
    body: Body,
) -> Result<axum::Json<RoleLook>> {
    let (g, r) = (http::url_id(&g)?, http::url_id(&r)?);
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let u = {
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
        role_of(&mut c, u, g, r).await?;
        u
    };
    app.0
        .core
        .lock()
        .await
        .rate(format!("role-icon:{}", u.0), 10., 3.)?;
    let bytes = to_bytes(body, MAX_ROLE_ICON_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let original = bytes.to_vec();
    let (png, mime) = tokio::task::spawn_blocking(move || {
        // The decoder keeps its permit until it finishes, even if the
        // request goes away, so uploads stay bounded.
        let _slot = slot;
        role_icon_rendition(&bytes)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Unavailable))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Rights can change while the picture is decoded.
    let (_, current) = auth(&mut c, http::bearer(&h)?).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    let role = role_of(&mut tx, u, g, r).await?;
    let rev: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(rev),0)+1 FROM role_icons WHERE guild_id=? AND role_id=?",
    )
    .bind(id(g)?)
    .bind(id(r)?)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO role_icons(guild_id,role_id,rev,png,original,original_type) VALUES(?,?,?,?,?,?) ON CONFLICT(guild_id,role_id) DO UPDATE SET rev=excluded.rev,png=excluded.png,original=excluded.original,original_type=excluded.original_type")
        .bind(id(g)?)
        .bind(id(r)?)
        .bind(rev)
        .bind(png)
        .bind(original)
        .bind(mime)
        .execute(&mut *tx)
        .await?;
    let mut look = role.look.as_deref().cloned().unwrap_or_default();
    let before = icon_text(&look.icon);
    look.icon = Some(RoleIcon::image(rev as u32));
    save_role_look(&mut tx, g, r, &look).await?;
    role_audit(&mut tx, u, &role, "icon", before, icon_text(&look.icon)).await?;
    let users = guild_users(&mut tx, g).await?;
    let roles = crate::permissions::roles(&mut tx, g).await?;
    tx.commit().await?;
    core.emit(&users, Event::RolesUpdated { guild_id: g, roles })?;
    Ok(axum::Json(look))
}

/// `GET /api/v1/guilds/{g}/roles/{r}/icon`: the 64x64 rendition, for members.
pub(super) async fn icon_endpoint(
    State(app): State<App>,
    Path((g, r)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    h: HeaderMap,
) -> Result<Response> {
    let (g, r) = (http::url_id(&g)?, http::url_id(&r)?);
    let png: Vec<u8> = {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
        core.rate(format!("file:{}", u.0), 600., 120.)?;
        role(&mut c, u, g).await?;
        sqlx::query_scalar("SELECT png FROM role_icons WHERE guild_id=? AND role_id=?")
            .bind(id(g)?)
            .bind(id(r)?)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?
    };
    image_response(png, "image/png", q.contains_key("v"))
}
pub(super) fn image_response(bytes: Vec<u8>, mime: &str, versioned: bool) -> Result<Response> {
    Response::builder()
        .header("content-type", mime)
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", "default-src 'none'; sandbox")
        .header(
            "cache-control",
            if versioned {
                "private, max-age=86400, immutable"
            } else {
                "private, no-cache"
            },
        )
        .body(Body::from(bytes))
        .map_err(|_| Error::new(ErrorCode::Internal))
}
