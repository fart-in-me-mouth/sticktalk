//! Vanity invite links and invite cards.
//!
//! A vanity code is stored as an invite whose hash is the lowercase code's,
//! with no expiry, so every invite path (join, preview, applications, raid
//! pauses, insights' join sources) works unchanged. The invites table caps
//! uses at 1,000; `rebase` moves the count into `vanity_codes.uses_base`
//! before it gets there, so the link never runs out and admins still see
//! every use.
use super::*;
use crate::{http, state::App};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use std::collections::HashMap;

/// A vanity invite never expires (the year 3000).
const NEVER: i64 = 32_503_680_000_000;
const MAX_USES: i64 = 1_000;
const REBASE_AT: i64 = 900;

pub(super) async fn rebase(c: &mut SqliteConnection, invite: i64) -> Result<()> {
    let uses: Option<i64> = sqlx::query_scalar("SELECT uses FROM invites WHERE id=?")
        .bind(invite)
        .fetch_optional(&mut *c)
        .await?;
    if uses.is_some_and(|n| n >= REBASE_AT) {
        sqlx::query("UPDATE vanity_codes SET uses_base=uses_base+? WHERE invite_id=?")
            .bind(uses)
            .bind(invite)
            .execute(&mut *c)
            .await?;
        sqlx::query("UPDATE invites SET uses=0 WHERE id=?")
            .bind(invite)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
pub(super) async fn info(c: &mut SqliteConnection, g: Id) -> Result<Option<Vanity>> {
    let r = sqlx::query("SELECT v.code,v.uses_base,v.claimed_at,COALESCE(i.uses,0) AS uses FROM vanity_codes v LEFT JOIN invites i ON i.id=v.invite_id WHERE v.guild_id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.map(|r| Vanity {
        code: r.get("code"),
        uses: (r.get::<i64, _>("uses_base") + r.get::<i64, _>("uses")).max(0) as u64,
        claimed_at: r.get::<i64, _>("claimed_at") as u64,
    }))
}
/// `g` gives up its code: the invite goes, the code is held for 30 days.
pub(super) async fn release(c: &mut SqliteConnection, g: Id) -> Result<()> {
    let invite: Option<Option<i64>> =
        sqlx::query_scalar("SELECT invite_id FROM vanity_codes WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_optional(&mut *c)
            .await?;
    let Some(invite) = invite else { return Ok(()) };
    sqlx::query("UPDATE vanity_codes SET guild_id=NULL,invite_id=NULL,held_by=?,held_until=? WHERE guild_id=?")
        .bind(id(g)?)
        .bind((now() + VANITY_HOLD_MS) as i64)
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
    if let Some(i) = invite {
        sqlx::query("DELETE FROM invites WHERE id=?")
            .bind(i)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
/// How long until `g` may change its vanity code: at most
/// `VANITY_CHANGES_PER_HOUR` changes in any hour, counted from the audit log
/// (which records every change in the same transaction).
pub(super) async fn wait_ms(c: &mut SqliteConnection, g: Id) -> Result<u64> {
    let hour = 3_600_000u64;
    let recent: Vec<i64> = sqlx::query_scalar("SELECT created_at FROM audit_log WHERE guild_id=? AND action='vanity_update' AND created_at>? ORDER BY created_at DESC LIMIT ?")
        .bind(id(g)?)
        .bind(now().saturating_sub(hour) as i64)
        .bind(VANITY_CHANGES_PER_HOUR as i64)
        .fetch_all(&mut *c)
        .await?;
    if recent.len() < VANITY_CHANGES_PER_HOUR as usize {
        return Ok(0);
    }
    let oldest = recent.last().copied().unwrap_or(0).max(0) as u64;
    Ok((oldest + hour).saturating_sub(now()))
}
fn refused(code: ErrorCode, message: &str) -> Error {
    let mut e = Error::new(code);
    e.0.field = Some("vanity".into());
    e.0.message = message.into();
    e
}

pub(super) async fn set(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    g: Id,
    code: Option<String>,
    _out: &mut Deliveries,
) -> Result<CommandResult> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let current = info(c, g).await?;
    let wanted = match code {
        Some(raw) => Some(vanity_code(&raw).ok_or_else(|| {
            invalid(
                "vanity",
                "Use 3 to 32 letters, numbers or single hyphens, not at the start or end.",
            )
        })?),
        None => None,
    };
    if current.as_ref().map(|v| &v.code) == wanted.as_ref() {
        return Ok(CommandResult::Identity(IdentityResult::Vanity(current)));
    }
    if let Some(w) = &wanted {
        if reserved_vanity(w) {
            return Err(refused(ErrorCode::Forbidden, "That link is reserved."));
        }
        let held = sqlx::query("SELECT guild_id,held_by,held_until FROM vanity_codes WHERE code=?")
            .bind(w)
            .fetch_optional(&mut *c)
            .await?;
        if let Some(h) = held {
            if h.get::<Option<i64>, _>("guild_id").is_some() {
                return Err(refused(
                    ErrorCode::AlreadyExists,
                    "Another server has that link.",
                ));
            }
            let until = h.get::<Option<i64>, _>("held_until").unwrap_or(0);
            if until > now() as i64 && h.get::<Option<i64>, _>("held_by") != Some(id(g)?) {
                return Err(refused(
                    ErrorCode::Conflict,
                    "That link was released recently and is held for 30 days. Try another one.",
                ));
            }
        }
        let clash: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invites WHERE code_hash=?)")
                .bind(hash(w.as_bytes()))
                .fetch_one(&mut *c)
                .await?;
        if clash {
            return Err(refused(ErrorCode::AlreadyExists, "That link is taken."));
        }
    }
    // Checked last, so a refused code does not use up a change.
    let wait = wait_ms(c, g).await?;
    if wait > 0 {
        return Err(Error::rate(wait.min(u64::from(u32::MAX)) as u32));
    }
    release(c, g).await?;
    if let Some(w) = &wanted {
        let invite = core.id().await?;
        sqlx::query("INSERT INTO invites(id,code_hash,code,guild_id,creator_id,expires_at,max_uses,uses) VALUES(?,?,?,?,?,?,?,0)")
            .bind(id(invite)?)
            .bind(hash(w.as_bytes()))
            .bind(w)
            .bind(id(g)?)
            .bind(id(u)?)
            .bind(NEVER)
            .bind(MAX_USES)
            .execute(&mut *c)
            .await?;
        sqlx::query("INSERT INTO vanity_codes(code,guild_id,invite_id,held_by,held_until,uses_base,claimed_at) VALUES(?,?,?,NULL,NULL,0,?) ON CONFLICT(code) DO UPDATE SET guild_id=excluded.guild_id,invite_id=excluded.invite_id,held_by=NULL,held_until=NULL,uses_base=0,claimed_at=excluded.claimed_at")
            .bind(w)
            .bind(id(g)?)
            .bind(id(invite)?)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?;
    }
    // A tag offering the old code as its way in no longer can.
    if let Some(old) = &current {
        if let Some(mut t) = tags::load(c, g).await? {
            if t.invite_code.as_deref() == Some(old.code.as_str()) {
                t.invite_code = None;
                sqlx::query("UPDATE server_tags SET tag=? WHERE guild_id=?")
                    .bind(json(&t)?)
                    .bind(id(g)?)
                    .execute(&mut *c)
                    .await?;
            }
        }
    }
    let name: String = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(g, AuditAction::VanityUpdate, AuditTarget::Guild, Some(g))
            .named(name)
            .change("vanity_code", current.map(|v| v.code), wanted),
    )
    .await?;
    Ok(CommandResult::Identity(IdentityResult::Vanity(
        info(c, g).await?,
    )))
}

/// The usable invite a code names, and its server.
async fn usable(c: &mut SqliteConnection, code: &str) -> Result<(i64, Id)> {
    let i = find_invite(c, code)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidInvite))?;
    let inv = invite(c, Id(i as u64)).await?;
    require(inv.expires_at > now(), ErrorCode::InviteExpired)?;
    require(inv.uses < inv.max_uses, ErrorCode::InviteExhausted)?;
    Ok((i, inv.guild_id))
}

pub(super) async fn card(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    code: &str,
) -> Result<InviteCard> {
    let (i, g) = usable(c, code).await?;
    let r = sqlx::query("SELECT name,(SELECT description FROM guild_profiles WHERE guild_id=guilds.id) AS d FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let members = guild_users(c, g).await?;
    let connected: std::collections::HashSet<Id> = core
        .streams
        .values()
        .filter(|s| s.connection.is_some())
        .map(|s| s.user)
        .collect();
    let mut online = 0u32;
    for m in members.iter().filter(|m| connected.contains(m)) {
        let p: Option<String> = sqlx::query_scalar("SELECT desired_presence FROM users WHERE id=?")
            .bind(id(*m)?)
            .fetch_optional(&mut *c)
            .await?;
        if p.as_deref() != Some("invisible") {
            online += 1;
        }
    }
    let look = guild_look(c, g).await?;
    let vanity: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM vanity_codes WHERE invite_id=?)")
            .bind(i)
            .fetch_one(&mut *c)
            .await?;
    Ok(InviteCard {
        code: code.to_owned(),
        guild_id: g,
        name: r.get("name"),
        description: r.get::<Option<String>, _>("d").unwrap_or_default(),
        member_count: members.len() as u32,
        online_count: online,
        icon: looks::icon(c, g).await?,
        banner: look.banner,
        background: looks::invite_background(c, g).await?,
        tag: tags::load(c, g).await?.map(|t| t.worn(g)),
        joined: members.contains(&u),
        apply: crate::community::admission_settings(c, g)
            .await?
            .applications,
        vanity,
    })
}

/// `GET /api/v1/invites/{code}/art/{icon|banner|background}[?still]`: a
/// server image for someone holding a usable invite. `still` asks for the
/// first-frame rendition of an animation.
pub(super) async fn art(
    State(app): State<App>,
    Path((code, slot)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    h: HeaderMap,
) -> Result<Response> {
    let (key, mime, size) = {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
        core.rate(format!("file:{}", u.0), 600., 120.)?;
        let (_, g) = usable(&mut c, &code).await?;
        let meta = match slot.as_str() {
            "icon" => looks::icon(&mut c, g).await?,
            "banner" => guild_look(&mut c, g).await?.banner,
            "background" => looks::invite_background(&mut c, g).await?,
            _ => None,
        }
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        let row = sqlx::query("SELECT storage_key,content_type,byte_size,thumbnail_key,thumbnail_type,thumbnail_size FROM uploads WHERE id=? AND expires_at IS NULL")
            .bind(id(meta.id)?)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        let thumb = q.contains_key("still") || slot == "icon" && !meta.animated;
        match (thumb, row.get::<Option<String>, _>("thumbnail_key")) {
            (true, Some(k)) => (
                k,
                row.get::<Option<String>, _>("thumbnail_type")
                    .unwrap_or_else(|| "image/jpeg".into()),
                row.get::<Option<i64>, _>("thumbnail_size").unwrap_or(0) as u64,
            ),
            _ => (
                row.get::<String, _>("storage_key"),
                row.get::<String, _>("content_type"),
                row.get::<i64, _>("byte_size") as u64,
            ),
        }
    };
    let body = app.0.tier.get(&app, &key, 0, size).await?;
    Response::builder()
        .header("content-type", mime)
        .header("content-length", size)
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", "default-src 'none'; sandbox")
        .header("cache-control", "private, max-age=3600")
        .body(body)
        .map_err(|_| Error::new(ErrorCode::Internal))
}

/// `GET /invite/{code}`: a shareable link that opens the app's join screen.
pub(super) async fn redirect(Path(code): Path<String>) -> Response {
    let ok = !code.is_empty()
        && code.len() <= MAX_VANITY_CHARS
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Redirect::to(&format!("/?invite={code}")).into_response()
    } else {
        Redirect::to("/").into_response()
    }
}
