//! Profiles and cosmetics (`docs/features/profiles.md`). Contracts:
//! `crates/proto/src/profiles.rs`. Everything is free for every user.
//!
//! The look of a name and avatar is stored on `users.style` and read with
//! every user summary; per-server overrides are a `member_profiles` row read
//! with every member. Banners and server avatars are ordinary avatar uploads,
//! listed in `profile_uploads` so that viewers may download them and upload
//! garbage collection keeps them. Badges are computed when a card is read,
//! except the ones the administrator awards with the admin CLI.
pub mod badges;
pub(crate) mod media;
#[cfg(test)]
mod tests;

use crate::{
    db::*,
    error::{require, Error, Result},
    http,
    moderation::audit,
    permissions as acl,
    state::{now, App, Core},
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use sqlx::{Row, SqliteConnection};
use std::collections::HashMap;
use sticktalk_proto::*;

type Deliveries = Vec<(Vec<Id>, Event)>;

fn invalid() -> Error {
    Error::new(ErrorCode::InvalidRequest)
}
/// Text a person typed: at most `max` characters, no control characters
/// other than line breaks.
fn text(s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    require(
        s.chars().count() <= max && !s.chars().any(|c| c.is_control() && c != '\n'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
fn optional_text(s: Option<String>, max: usize) -> Result<Option<String>> {
    Ok(s.map(|s| text(&s, max))
        .transpose()?
        .filter(|s| !s.is_empty()))
}
fn rgb(c: Option<u32>) -> Result<Option<u32>> {
    require(c.is_none_or(|c| c <= 0xFF_FFFF), ErrorCode::InvalidRequest)?;
    Ok(c)
}
fn item(catalog: &[Cosmetic], id: Option<String>) -> Result<Option<String>> {
    require(
        id.as_deref().is_none_or(|v| known(catalog, v)),
        ErrorCode::InvalidRequest,
    )?;
    Ok(id)
}
fn style(s: Option<NameStyle>) -> Result<Option<NameStyle>> {
    if let Some(problem) = s.as_ref().and_then(NameStyle::problem) {
        let mut e = invalid();
        e.0.message = problem.into();
        e.0.field = Some("name_style".into());
        return Err(e);
    }
    Ok(s)
}
fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| Error::new(ErrorCode::Internal))
}
fn parse<T: serde::de::DeserializeOwned + Default>(s: Option<String>) -> T {
    s.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The global name and avatar look of `u`.
pub(crate) async fn user_style(c: &mut SqliteConnection, u: Id) -> Result<UserStyle> {
    let s: Option<Option<String>> = sqlx::query_scalar("SELECT style FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(parse(s.flatten()))
}
/// Read `users.style` for `user_row`; a row without the column has no style.
pub(crate) fn style_column(r: &sqlx::sqlite::SqliteRow) -> UserStyle {
    parse(r.try_get::<Option<String>, _>("style").ok().flatten())
}
async fn save_style(c: &mut SqliteConnection, u: Id, s: &UserStyle) -> Result<()> {
    sqlx::query("UPDATE users SET style=? WHERE id=?")
        .bind(if s.is_empty() { None } else { Some(json(s)?) })
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}
pub(crate) async fn look(
    c: &mut SqliteConnection,
    u: Id,
) -> Result<(ProfileLook, Option<Vec<String>>)> {
    let r = sqlx::query("SELECT look,shown_badges FROM profile_looks WHERE user_id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(match r {
        Some(r) => (
            parse(Some(r.get("look"))),
            r.get::<Option<String>, _>("shown_badges")
                .and_then(|s| serde_json::from_str(&s).ok()),
        ),
        None => (ProfileLook::default(), None),
    })
}
async fn save_look(c: &mut SqliteConnection, u: Id, l: &ProfileLook) -> Result<()> {
    sqlx::query("INSERT INTO profile_looks(user_id,look) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET look=excluded.look")
        .bind(id(u)?)
        .bind(json(l)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}
/// `u`'s profile in server `g`, with the server nickname.
pub(crate) async fn server_profile(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
) -> Result<ServerProfile> {
    let s: Option<String> =
        sqlx::query_scalar("SELECT profile FROM member_profiles WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    let mut p: ServerProfile = parse(s);
    p.nickname =
        sqlx::query_scalar("SELECT nickname FROM nicknames WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(p)
}
async fn save_server_profile(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    p: &ServerProfile,
) -> Result<()> {
    let stored = ServerProfile {
        nickname: None,
        ..p.clone()
    };
    if stored.is_empty() {
        sqlx::query("DELETE FROM member_profiles WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .execute(&mut *c)
            .await?;
    } else {
        sqlx::query("INSERT INTO member_profiles VALUES(?,?,?) ON CONFLICT(guild_id,user_id) DO UPDATE SET profile=excluded.profile")
            .bind(id(g)?)
            .bind(id(u)?)
            .bind(json(&stored)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
/// What `Member::server_style` carries for `u` in `g`.
pub(crate) async fn member_style(c: &mut SqliteConnection, g: Id, u: Id) -> Result<UserStyle> {
    let s: Option<String> =
        sqlx::query_scalar("SELECT profile FROM member_profiles WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(parse::<ServerProfile>(s).style())
}
/// May `viewer` download upload `a`, because it is someone's banner or server
/// avatar? Global images go to anyone who may see the profile, server images
/// to the members of that server.
pub(crate) async fn file_access(c: &mut SqliteConnection, viewer: Id, a: Id) -> Result<bool> {
    let rows = sqlx::query("SELECT user_id,guild_id FROM profile_uploads WHERE upload_id=?")
        .bind(id(a)?)
        .fetch_all(&mut *c)
        .await?;
    for r in rows {
        let owner = Id(r.get::<i64, _>("user_id") as u64);
        let g = r.get::<i64, _>("guild_id");
        let ok = if g == 0 {
            profile_access(c, viewer, owner).await.is_ok()
        } else {
            viewer == owner || role(c, viewer, Id(g as u64)).await.is_ok()
        };
        if ok {
            return Ok(true);
        }
    }
    Ok(false)
}
/// Account erasure: profile data goes with the account.
pub(crate) async fn erase(c: &mut SqliteConnection, u: Id) -> Result<()> {
    for q in [
        "UPDATE users SET style=NULL WHERE id=?",
        "DELETE FROM profile_looks WHERE user_id=?",
        "DELETE FROM member_profiles WHERE user_id=?",
        "DELETE FROM profile_uploads WHERE user_id=?",
        "DELETE FROM badge_awards WHERE user_id=?",
    ] {
        sqlx::query(q).bind(id(u)?).execute(&mut *c).await?;
    }
    Ok(())
}

/// A user's look changed: tell them, everyone who can see them, and every
/// server they are in (their `Member` carries the user summary too).
pub(crate) async fn user_changed(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let me = me(c, u).await?;
    out.push((vec![u], Event::SelfUpdated(me.clone())));
    let mut viewers = vec![];
    let mut seen = std::collections::HashSet::new();
    for viewer in core.streams.values().map(|s| s.user) {
        if seen.insert(viewer) && profile_access(c, viewer, u).await.is_ok() {
            viewers.push(viewer);
        }
    }
    viewers.sort();
    out.push((viewers, Event::UserUpdated(me.user)));
    for g in sqlx::query_scalar::<_, i64>("SELECT guild_id FROM guild_members WHERE user_id=?")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
    {
        member_changed(c, Id(g as u64), u, out).await?;
    }
    Ok(())
}
async fn member_changed(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    out.push((
        guild_users(c, g).await?,
        Event::MemberUpsert(member(c, g, u).await?),
    ));
    Ok(())
}
async fn profile_viewers(c: &mut SqliteConnection, u: Id) -> Result<Vec<Id>> {
    let mut viewers = vec![u];
    for v in sqlx::query_scalar::<_,i64>("SELECT id FROM users WHERE id IN(SELECT other_id FROM relationships WHERE user_id=?) OR id IN(SELECT b.user_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=?) OR id IN(SELECT b.user_id FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=?)").bind(id(u)?).bind(id(u)?).bind(id(u)?).fetch_all(&mut *c).await? {
        viewers.push(Id(v as u64));
    }
    viewers.sort();
    viewers.dedup();
    Ok(viewers)
}

/// An uploaded image the person may use: their own avatar upload, either
/// fresh (not yet attached) or already one of their profile images.
async fn image(c: &mut SqliteConnection, u: Id, r: ImageRef) -> Result<ImageMeta> {
    require(r.crop.is_none_or(|c| c.valid()), ErrorCode::InvalidRequest)?;
    let row = sqlx::query("SELECT content_type,storage_key,expires_at FROM uploads WHERE id=? AND owner_id=? AND purpose='avatar'")
        .bind(id(r.id)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidAttachment))?;
    let fresh = row
        .get::<Option<i64>, _>("expires_at")
        .is_some_and(|t| t > now() as i64);
    let in_use: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND avatar_id=?2) OR EXISTS(SELECT 1 FROM profile_uploads WHERE user_id=?1 AND upload_id=?2)")
        .bind(id(u)?)
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
    let animated = media::animated(&row.get::<String, _>("storage_key"))
        .unwrap_or(matches!(mime.as_str(), "image/gif" | "image/apng"));
    Ok(ImageMeta {
        id: r.id,
        animated,
        crop: r.crop.filter(|c| !c.is_identity()),
    })
}
async fn track(
    c: &mut SqliteConnection,
    u: Id,
    g: i64,
    slot: ImageSlot,
    m: Option<&ImageMeta>,
) -> Result<()> {
    sqlx::query("DELETE FROM profile_uploads WHERE user_id=? AND guild_id=? AND slot=?")
        .bind(id(u)?)
        .bind(g)
        .bind(enum_str(slot))
        .execute(&mut *c)
        .await?;
    if let Some(m) = m {
        sqlx::query("INSERT INTO profile_uploads(user_id,guild_id,slot,upload_id) VALUES(?,?,?,?)")
            .bind(id(u)?)
            .bind(g)
            .bind(enum_str(slot))
            .bind(id(m.id)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}

pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    cmd: ProfilesCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    core.rate(format!("profiles:{}", u.0), 30., 10.)?;
    match cmd {
        ProfilesCommand::Update(e) => {
            let pronouns = text(&e.pronouns, MAX_PRONOUNS_CHARS)?;
            let about_me = text(&e.about_me, MAX_ABOUT_ME_CHARS)?;
            require(e.banner_color <= 0xFF_FFFF, ErrorCode::InvalidRequest)?;
            let old_style = user_style(c, u).await?;
            let new_style = UserStyle {
                avatar: old_style.avatar.clone(),
                name: style(e.name)?,
                decoration: item(DECORATIONS, e.decoration)?,
                nameplate: item(NAMEPLATES, e.nameplate)?,
                bot: old_style.bot,
                // The worn tag lives in its own column (identity.rs).
                tag: None,
            };
            let (old_look, _) = look(c, u).await?;
            let new_look = ProfileLook {
                banner: old_look.banner.clone(),
                primary: rgb(e.primary)?,
                accent: rgb(e.accent)?,
                effect: item(EFFECTS, e.effect)?,
                frame: item(FRAMES, e.frame)?,
            };
            sqlx::query("INSERT INTO user_profiles VALUES(?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET banner_color=excluded.banner_color,pronouns=excluded.pronouns,about_me=excluded.about_me").bind(id(u)?).bind(i64::from(e.banner_color)).bind(pronouns).bind(about_me).execute(&mut *c).await?;
            save_look(c, u, &new_look).await?;
            if new_style != old_style {
                save_style(c, u, &new_style).await?;
                user_changed(c, core, u, out).await?;
            }
            out.push((
                profile_viewers(c, u).await?,
                Event::ProfileUpdated(crate::launch::user_profile(c, u).await?),
            ));
            Ok(CommandResult::Ack)
        }
        ProfilesCommand::UpdateServer {
            guild_id: g,
            edit: e,
        } => {
            role(c, u, g).await?;
            let old = server_profile(c, g, u).await?;
            let new = ServerProfile {
                nickname: old.nickname.clone(),
                avatar: old.avatar.clone(),
                banner: old.banner.clone(),
                pronouns: optional_text(e.pronouns, MAX_PRONOUNS_CHARS)?,
                about_me: optional_text(e.about_me, MAX_ABOUT_ME_CHARS)?,
                primary: rgb(e.primary)?,
                accent: rgb(e.accent)?,
                name: style(e.name)?,
                decoration: item(DECORATIONS, e.decoration)?,
                nameplate: item(NAMEPLATES, e.nameplate)?,
                effect: item(EFFECTS, e.effect)?,
                frame: item(FRAMES, e.frame)?,
            };
            save_server_profile(c, g, u, &new).await?;
            if new.style() != old.style() {
                member_changed(c, g, u, out).await?;
            }
            Ok(CommandResult::Ack)
        }
        ProfilesCommand::SetImage {
            guild_id,
            slot,
            image: r,
        } => {
            if let Some(g) = guild_id {
                role(c, u, g).await?;
            }
            let meta = match r {
                Some(r) => Some(image(c, u, r).await?),
                None => None,
            };
            match (guild_id, slot) {
                (None, ImageSlot::Avatar) => {
                    sqlx::query("UPDATE users SET avatar_id=? WHERE id=?")
                        .bind(opt(meta.as_ref().map(|m| m.id))?)
                        .bind(id(u)?)
                        .execute(&mut *c)
                        .await?;
                    let mut s = user_style(c, u).await?;
                    s.avatar = meta;
                    save_style(c, u, &s).await?;
                    user_changed(c, core, u, out).await?;
                }
                (None, ImageSlot::Banner) => {
                    let (mut l, _) = look(c, u).await?;
                    track(c, u, 0, slot, meta.as_ref()).await?;
                    l.banner = meta;
                    save_look(c, u, &l).await?;
                }
                (Some(g), slot) => {
                    let mut p = server_profile(c, g, u).await?;
                    track(c, u, id(g)?, slot, meta.as_ref()).await?;
                    match slot {
                        ImageSlot::Avatar => p.avatar = meta,
                        ImageSlot::Banner => p.banner = meta,
                    }
                    save_server_profile(c, g, u, &p).await?;
                    if slot == ImageSlot::Avatar {
                        member_changed(c, g, u, out).await?;
                    }
                }
            }
            Ok(CommandResult::Ack)
        }
        ProfilesCommand::SetBadges { shown } => {
            require(shown.len() <= MAX_SHOWN_BADGES, ErrorCode::LimitExceeded)?;
            let earned = badges::earned(c, u).await?;
            let mut seen = std::collections::HashSet::new();
            require(
                shown
                    .iter()
                    .all(|b| seen.insert(b.as_str()) && earned.iter().any(|e| e.id == *b)),
                ErrorCode::InvalidRequest,
            )?;
            sqlx::query("INSERT INTO profile_looks(user_id,shown_badges) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET shown_badges=excluded.shown_badges")
                .bind(id(u)?)
                .bind(json(&shown)?)
                .execute(&mut *c)
                .await?;
            Ok(CommandResult::Ack)
        }
        ProfilesCommand::Reset {
            guild_id: g,
            user_id: v,
            avatar,
            banner,
            reason,
        } => {
            require(avatar || banner, ErrorCode::InvalidRequest)?;
            guild_perm(c, u, g, Permissions::MODERATE_MEMBERS).await?;
            acl::above_member(c, g, u, v).await?;
            role(c, v, g).await?;
            let reason = optional_text(reason, MAX_REASON_CHARS)?;
            let mut p = server_profile(c, g, v).await?;
            let mut entry = audit::Entry::new(
                g,
                AuditAction::MemberProfileReset,
                AuditTarget::User,
                Some(v),
            )
            .named(user(c, v).await?.username)
            .reason(reason);
            if avatar && p.avatar.is_some() {
                entry = entry.change("server_avatar", Some("custom".into()), None);
                p.avatar = None;
                track(c, v, id(g)?, ImageSlot::Avatar, None).await?;
            }
            if banner && p.banner.is_some() {
                entry = entry.change("server_banner", Some("custom".into()), None);
                p.banner = None;
                track(c, v, id(g)?, ImageSlot::Banner, None).await?;
            }
            save_server_profile(c, g, v, &p).await?;
            if avatar {
                member_changed(c, g, v, out).await?;
            }
            audit::write(c, u, entry).await?;
            Ok(CommandResult::Ack)
        }
    }
}

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/users/@me/profiles", get(mine_endpoint))
        .route("/api/v1/users/{id}/card", get(card_endpoint))
        .route("/api/v1/badges/{id}/icon", get(badges::icon_endpoint))
}
async fn rest<'a>(
    app: &'a App,
    h: &HeaderMap,
) -> Result<(
    tokio::sync::MutexGuard<'a, Core>,
    sqlx::pool::PoolConnection<sqlx::Sqlite>,
    Id,
)> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((core, c, u))
}
async fn mine_endpoint(State(app): State<App>, h: HeaderMap) -> Result<Json<MyProfiles>> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(Json(mine(&mut c, u).await?))
}
pub(crate) async fn mine(c: &mut SqliteConnection, u: Id) -> Result<MyProfiles> {
    let p = crate::launch::user_profile(c, u).await?;
    let (look, shown) = look(c, u).await?;
    let earned = badges::earned(c, u).await?;
    let chosen = shown.is_some();
    let shown = badges::shown(&earned, shown.as_deref())
        .into_iter()
        .map(|b| b.id)
        .collect::<Vec<_>>();
    let mut ordered: Vec<Badge> = shown
        .iter()
        .filter_map(|s| earned.iter().find(|b| b.id == *s).cloned())
        .collect();
    ordered.extend(earned.into_iter().filter(|b| !shown.contains(&b.id)));
    let mut servers = vec![];
    for g in sqlx::query_scalar::<_, i64>(
        "SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?
    {
        let g = Id(g as u64);
        let profile = server_profile(c, g, u).await?;
        if !profile.is_empty() {
            servers.push(GuildServerProfile {
                guild_id: g,
                profile,
            });
        }
    }
    Ok(MyProfiles {
        pronouns: p.pronouns,
        about_me: p.about_me,
        banner_color: p.banner_color,
        look,
        earned: ordered,
        shown,
        chosen,
        servers,
    })
}
async fn card_endpoint(
    State(app): State<App>,
    Path(target): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    h: HeaderMap,
) -> Result<Json<ProfileCard>> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    let v = http::url_id(&target)?;
    let g = q.get("guild").map(|g| http::url_id(g)).transpose()?;
    Ok(Json(card(&mut c, u, v, g).await?))
}
/// `v`'s profile card as `u` sees it, opened from server `g`.
pub(crate) async fn card(
    c: &mut SqliteConnection,
    u: Id,
    v: Id,
    g: Option<Id>,
) -> Result<ProfileCard> {
    profile_access(c, u, v).await?;
    let p = crate::launch::user_profile(c, v).await?;
    let (look, shown) = look(c, v).await?;
    let earned = badges::earned(c, v).await?;
    let (server, member_since) = match g {
        Some(g) if role(c, u, g).await.is_ok() => {
            let since: Option<i64> = sqlx::query_scalar(
                "SELECT joined_at FROM guild_members WHERE guild_id=? AND user_id=?",
            )
            .bind(id(g)?)
            .bind(id(v)?)
            .fetch_optional(&mut *c)
            .await?;
            match since {
                Some(t) => {
                    let s = server_profile(c, g, v).await?;
                    ((!s.is_empty()).then_some(s), Some(t as u64))
                }
                None => (None, None),
            }
        }
        Some(_) => return Err(Error::new(ErrorCode::NotFound)),
        None => (None, None),
    };
    Ok(ProfileCard {
        user: p.user,
        pronouns: p.pronouns,
        about_me: p.about_me,
        banner_color: p.banner_color,
        created_at: p.created_at,
        look,
        badges: badges::shown(&earned, shown.as_deref()),
        server,
        member_since,
    })
}
