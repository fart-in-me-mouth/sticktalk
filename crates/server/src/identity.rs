//! Server identity (`docs/features/identity.md`): server tags, role styles
//! and icons, the server banner, animated server icon and invite background,
//! rich invite cards and vanity invite links. Wire contracts:
//! `sticktalk_proto::identity`. Everything is free for every server.
//!
//! Storage: a worn tag is copied onto `users.tag` (with `users.tag_guild`),
//! so every user summary carries it with no extra query; a trigger on
//! `guild_members` clears it the moment its wearer leaves or is removed and
//! queues the user in `tag_drops`, which `settle` turns into profile events
//! inside the same command. A server's look is `guild_looks`, a role's is
//! `role_looks`, both read by the existing list queries through a subselect.
//! A vanity code is an ordinary invite row (hash of the lowercase code, no
//! expiry), so joining, previews, applications, raid pauses and insights treat
//! it as an invite; `vanity_codes` remembers who holds it and for how long a
//! released one stays held.
//!
//! Hooks into shared code (each one line or one clause):
//! - `db::user_row` reads the tag; `db::guild` and `permissions::roles` read
//!   the looks; `db::gc_uploads` keeps guild images; `files::download` asks
//!   `file_access`.
//! - Invite lookups (`JoinGuild`, the preview, community's join gate and
//!   moderation's pause check) call `find_invite`, which also accepts vanity
//!   codes; `RevokeInvite` refuses the vanity invite (`not_vanity`).
//! - `commands::execute` calls `settle` before committing.
//!
//! Nothing here runs while unused: there is no timer; holds expire when a code
//! is next claimed.
pub(crate) mod looks;
mod routes;
mod tags;
#[cfg(test)]
mod tests;
mod vanity;

pub(crate) use routes::routes;

/// In-memory state, under the command gate (`Core::identity`).
#[derive(Default)]
pub(crate) struct Runtime {
    /// Wakes the fan-out task: wearers are waiting in `tag_announce`.
    pub(crate) wake: std::sync::Arc<tokio::sync::Notify>,
    /// Invite cards' online counts by server: (counted at, online).
    counts: std::collections::HashMap<Id, (u64, u32)>,
}
/// How long an invite card's online count is reused, and how many servers'
/// counts are kept.
const COUNT_TTL_MS: u64 = 30_000;
const MAX_COUNTS: usize = 256;

/// How many of `g`'s members are online and not invisible: from a cache
/// refreshed at most every 30 s per server. The member query runs outside the
/// command gate; the gate is held only to read who is connected. It takes its
/// own connection for that query, so call it with none in hand: the gate is
/// never waited for while a pooled connection is held (`http::budgeted`).
pub(crate) async fn online_count(app: &crate::state::App, g: Id) -> Result<u32> {
    if let Some((at, n)) = app.0.core.lock().await.identity.counts.get(&g) {
        if now().saturating_sub(*at) < COUNT_TTL_MS {
            return Ok(*n);
        }
    }
    let present: Vec<i64> = sqlx::query_scalar("SELECT m.user_id FROM guild_members m JOIN users u ON u.id=m.user_id WHERE m.guild_id=? AND u.desired_presence NOT IN('invisible','offline')")
        .bind(id(g)?)
        .fetch_all(&mut *app.0.pool.acquire().await?)
        .await?;
    let mut core = app.0.core.lock().await;
    let connected: std::collections::HashSet<Id> = core
        .streams
        .values()
        .filter(|s| s.connection.is_some())
        .map(|s| s.user)
        .collect();
    let n = present
        .iter()
        .filter(|u| connected.contains(&Id(**u as u64)))
        .count() as u32;
    let counts = &mut core.identity.counts;
    if counts.len() >= MAX_COUNTS && !counts.contains_key(&g) {
        let t = now();
        counts.retain(|_, (at, _)| t.saturating_sub(*at) < COUNT_TTL_MS);
        if counts.len() >= MAX_COUNTS {
            if let Some(oldest) = counts
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| *k)
            {
                counts.remove(&oldest);
            }
        }
    }
    counts.insert(g, (now(), n));
    Ok(n)
}

/// Tells the viewers of every wearer queued in `tag_announce` (a tag edited
/// or removed, revoked by the administrator, a big server's members leaving)
/// in batches of `tags::ANNOUNCE_BATCH`: each batch holds the command gate
/// only while it is read and sent, and no transaction stays open.
pub(crate) async fn fanout(app: &crate::state::App) -> Result<()> {
    loop {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let batch: Vec<i64> = sqlx::query_scalar("DELETE FROM tag_announce WHERE user_id IN (SELECT user_id FROM tag_announce ORDER BY user_id LIMIT ?) RETURNING user_id")
            .bind(tags::ANNOUNCE_BATCH)
            .fetch_all(&mut *c)
            .await?;
        if batch.is_empty() {
            return Ok(());
        }
        let mut out = vec![];
        for u in batch {
            let u = Id(u as u64);
            if user(&mut c, u).await.is_ok() {
                tags::announce(&mut c, &core, u, &mut out).await?;
            }
        }
        for (to, event) in out {
            core.emit(&to, event)?;
        }
        drop(c);
        drop(core);
        tokio::task::yield_now().await;
    }
}
/// The fan-out task: runs when woken (a tag change, the housekeeping pass
/// finding wearers the admin CLI queued), and once at start for wearers left
/// queued by a restart. It sleeps with nothing to do.
pub(crate) fn spawn(app: &crate::state::App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = app.0.core.lock().await.identity.wake.clone();
        loop {
            if let Err(error) = fanout(&app).await {
                tracing::warn!(?error, "server tag fan-out failed");
            }
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = wake.notified() => {}
            }
        }
    });
}
/// The administrator CLI's changes to a server's identity are in that
/// server's audit log. The administrator has no account there, and an entry
/// needs an actor: it is the server's owner, with the reason saying who did it.
async fn admin_audit(
    c: &mut SqliteConnection,
    g: Id,
    action: AuditAction,
    field: &str,
    before: String,
    reason: &str,
) -> Result<()> {
    let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let entry = audit::Entry::new(g, action, AuditTarget::Guild, Some(g))
        .change(field, Some(before), None)
        .reason(Some(reason.to_owned()));
    audit::write(c, Id(owner as u64), entry).await
}
/// The housekeeping pass: wearers the admin CLI queued while the server ran.
pub(crate) async fn housekeeping(c: &mut SqliteConnection, core: &Core) -> Result<()> {
    let queued: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tag_announce)")
        .fetch_one(&mut *c)
        .await?;
    if queued {
        core.identity.wake.notify_one();
    }
    Ok(())
}

use crate::{
    db::*,
    error::{require, Error, Result},
    moderation::audit,
    state::{hash, now, Core},
};
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection};
use sticktalk_proto::*;

type Deliveries = Vec<(Vec<Id>, Event)>;

fn invalid(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::InvalidRequest);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}
fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| Error::new(ErrorCode::Internal))
}
fn parse<T: serde::de::DeserializeOwned>(s: Option<String>) -> Option<T> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

// ------------------------------------------------------------------ columns

/// `users.tag` of a `SELECT * FROM users` row (none on rows without it).
pub(crate) fn tag_column(r: &SqliteRow) -> Option<WornTag> {
    parse(r.try_get::<Option<String>, _>("tag").ok().flatten())
}
/// The `look` subselect of `db::guild`.
pub(crate) fn guild_look_column(r: &SqliteRow) -> Option<Box<GuildLook>> {
    parse::<GuildLook>(r.try_get::<Option<String>, _>("look").ok().flatten())
        .filter(|l| !l.is_empty())
        .map(Box::new)
}
/// The `look` subselect of `permissions::roles`.
pub(crate) fn role_look_column(r: &SqliteRow) -> Option<Box<RoleLook>> {
    parse::<RoleLook>(r.try_get::<Option<String>, _>("look").ok().flatten())
        .filter(|l| !l.is_empty())
        .map(Box::new)
}
pub(crate) async fn guild_look(c: &mut SqliteConnection, g: Id) -> Result<GuildLook> {
    let s: Option<String> = sqlx::query_scalar("SELECT look FROM guild_looks WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(parse(s).unwrap_or_default())
}
async fn save_guild_look(c: &mut SqliteConnection, g: Id, l: &GuildLook) -> Result<()> {
    sqlx::query("INSERT INTO guild_looks(guild_id,look) VALUES(?,?) ON CONFLICT(guild_id) DO UPDATE SET look=excluded.look")
        .bind(id(g)?)
        .bind(json(l)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}

// ------------------------------------------------------------ invite codes

/// The invite row a typed code names: a 22-character invite code exactly as
/// given, or else a vanity code (any case). None for anything else.
pub(crate) async fn find_invite(c: &mut SqliteConnection, code: &str) -> Result<Option<i64>> {
    let found = lookup_invite(c, code).await?;
    if let Some(i) = found {
        vanity::rebase(c, i).await?;
    }
    Ok(found)
}
/// The invite row a code names, without moving a vanity link's use count
/// (for read-only routes, which run outside the command gate).
async fn lookup_invite(c: &mut SqliteConnection, code: &str) -> Result<Option<i64>> {
    if code.len() == 22 {
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM invites WHERE code_hash=?")
            .bind(hash(code.as_bytes()))
            .fetch_optional(&mut *c)
            .await?;
        if found.is_some() {
            return Ok(found);
        }
    }
    let Some(v) = vanity_code(code) else {
        return Ok(None);
    };
    let found: Option<i64> = sqlx::query_scalar("SELECT i.id FROM invites i JOIN vanity_codes v ON v.invite_id=i.id WHERE i.code_hash=? AND v.guild_id=i.guild_id")
        .bind(hash(v.as_bytes()))
        .fetch_optional(&mut *c)
        .await?;
    Ok(found)
}
/// `RevokeInvite` may not remove a vanity link; it is released on its page.
pub(crate) async fn not_vanity(c: &mut SqliteConnection, invite: i64) -> Result<()> {
    let held: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM vanity_codes WHERE invite_id=?)")
            .bind(invite)
            .fetch_one(&mut *c)
            .await?;
    if held {
        let mut e = Error::new(ErrorCode::Forbidden);
        e.0.field = Some("vanity".into());
        e.0.message = "This is the server’s vanity link. Release it on the Vanity URL page.".into();
        return Err(e);
    }
    Ok(())
}

// ------------------------------------------------------------------ files

/// May `viewer` download upload `a` because it is a server image? Members
/// may; a listed server's banner and icon are public to anyone signed in, and
/// so is the icon of a server with a tag (its tag card shows it).
pub(crate) async fn file_access(c: &mut SqliteConnection, viewer: Id, a: Id) -> Result<bool> {
    let rows = sqlx::query("SELECT guild_id,slot FROM guild_uploads WHERE upload_id=?")
        .bind(id(a)?)
        .fetch_all(&mut *c)
        .await?;
    for r in rows {
        let g = Id(r.get::<i64, _>("guild_id") as u64);
        let slot: String = r.get("slot");
        if role(c, viewer, g).await.is_ok() {
            return Ok(true);
        }
        if slot != "invite_background" && crate::community::listed_icon(c, g).await? {
            return Ok(true);
        }
        if slot == "icon" && tags::has_tag(c, g).await? {
            return Ok(true);
        }
    }
    let icon_of_tagged: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM guilds g JOIN server_tags t ON t.guild_id=g.id WHERE g.icon_id=?)",
    )
    .bind(id(a)?)
    .fetch_one(&mut *c)
    .await?;
    Ok(icon_of_tagged)
}

// --------------------------------------------------------------- commands

pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    cmd: IdentityCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    core.rate(format!("identity:{}", u.0), 30., 10.)?;
    match cmd {
        IdentityCommand::SetTag { guild_id, tag } => {
            tags::set(c, core, u, guild_id, tag, out).await
        }
        IdentityCommand::WearTag { guild_id } => tags::wear(c, core, u, guild_id, out).await,
        IdentityCommand::SetHideTags { guild_id, hide } => {
            guild_perm(c, u, guild_id, Permissions::MANAGE_GUILD).await?;
            let mut look = guild_look(c, guild_id).await?;
            if look.hide_tags != hide {
                look.hide_tags = hide;
                save_guild_look(c, guild_id, &look).await?;
                let entry = audit::Entry::new(
                    guild_id,
                    AuditAction::GuildUpdate,
                    AuditTarget::Guild,
                    Some(guild_id),
                )
                .change(
                    "other_server_tags",
                    Some(if hide { "shown" } else { "hidden" }.into()),
                    Some(if hide { "hidden" } else { "shown" }.into()),
                );
                audit::write(c, u, entry).await?;
                guild_changed(c, guild_id, out).await?;
            }
            Ok(CommandResult::Ack)
        }
        IdentityCommand::SetImage {
            guild_id,
            slot,
            image,
        } => looks::set_image(c, u, guild_id, slot, image, out).await,
        IdentityCommand::SetRoleStyle {
            guild_id,
            role_id,
            style,
        } => looks::set_role_style(c, u, guild_id, role_id, style, out).await,
        IdentityCommand::SetRoleIcon {
            guild_id,
            role_id,
            icon,
        } => looks::set_role_icon(c, u, guild_id, role_id, icon, out).await,
        IdentityCommand::SetVanity { guild_id, code } => {
            vanity::set(c, core, u, guild_id, code, out).await
        }
    }
}

/// Every member gets the server's summary again (its look changed).
async fn guild_changed(c: &mut SqliteConnection, g: Id, out: &mut Deliveries) -> Result<()> {
    for v in guild_users(c, g).await? {
        out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
    }
    Ok(())
}
/// Every member gets the server's roles again (a role's look changed).
async fn roles_changed(c: &mut SqliteConnection, g: Id, out: &mut Deliveries) -> Result<()> {
    out.push((
        guild_users(c, g).await?,
        Event::RolesUpdated {
            guild_id: g,
            roles: crate::permissions::roles(c, g).await?,
        },
    ));
    Ok(())
}

/// Inside every command's transaction, after it ran: people who left or were
/// removed from a server whose tag they wore lost it (the trigger cleared it);
/// tell them and everyone who sees them.
pub(crate) async fn settle(
    c: &mut SqliteConnection,
    core: &Core,
    out: &mut Deliveries,
) -> Result<()> {
    if !out
        .iter()
        .any(|(_, e)| matches!(e, Event::MemberRemoved { .. } | Event::GuildRemoved { .. }))
    {
        return Ok(());
    }
    // A few are told at once; a big server's members (deleting it) in
    // batches once this commits (`fanout`).
    let dropped: Vec<i64> = sqlx::query_scalar("DELETE FROM tag_drops WHERE user_id IN (SELECT user_id FROM tag_drops ORDER BY user_id LIMIT ?) RETURNING user_id")
        .bind(tags::ANNOUNCE_BATCH)
        .fetch_all(&mut *c)
        .await?;
    for u in dropped {
        let u = Id(u as u64);
        if user(c, u).await.is_ok() {
            tags::announce(c, core, u, out).await?;
        }
    }
    let queued =
        sqlx::query("INSERT OR IGNORE INTO tag_announce(user_id) SELECT user_id FROM tag_drops")
            .execute(&mut *c)
            .await?
            .rows_affected();
    if queued > 0 {
        sqlx::query("DELETE FROM tag_drops")
            .execute(&mut *c)
            .await?;
        core.identity.wake.notify_one();
    }
    Ok(())
}

/// Account erasure: the worn tag goes with the account.
pub(crate) async fn erase(c: &mut SqliteConnection, u: Id) -> Result<()> {
    sqlx::query("UPDATE users SET tag=NULL,tag_guild=NULL WHERE id=?")
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}
/// For the data export (`settings.json`, `worn_tag`): the tag the account
/// wears, from a row with the `tag` column.
pub(crate) fn export_row(r: &SqliteRow) -> serde_json::Value {
    tag_column(r).map_or(serde_json::Value::Null, |t| {
        serde_json::json!({
            "server_id": t.guild_id.0.to_string(),
            "text": t.text,
            "glyph": t.glyph,
            "color": format!("#{:06x}", t.color),
            "color2": format!("#{:06x}", t.color2),
        })
    })
}

// ------------------------------------------------------------- admin CLI

/// `sticktalk-server admin identity ...` (`admin::IdentityCli`).
pub(crate) async fn admin_cli(
    c: &mut SqliteConnection,
    command: &crate::admin::IdentityCli,
) -> Result<()> {
    use crate::admin::IdentityCli as IdentityAdmin;
    match command {
        IdentityAdmin::List => {
            for r in sqlx::query("SELECT t.guild_id,g.name,t.tag,(SELECT COUNT(*) FROM users WHERE tag_guild=t.guild_id) AS n FROM server_tags t JOIN guilds g ON g.id=t.guild_id ORDER BY t.guild_id").fetch_all(&mut *c).await? {
                let tag: Option<TagEdit> = parse(Some(r.get("tag")));
                println!("tag\t{}\t{}\t{}\t{} wearing", r.get::<i64, _>("guild_id"), tag.map(|t| t.text).unwrap_or_default(), r.get::<String, _>("name"), r.get::<i64, _>("n"));
            }
            for r in sqlx::query("SELECT code,guild_id,held_until FROM vanity_codes ORDER BY code")
                .fetch_all(&mut *c)
                .await?
            {
                match r.get::<Option<i64>, _>("guild_id") {
                    Some(g) => println!("vanity\t{}\t{g}", r.get::<String, _>("code")),
                    None => println!(
                        "vanity\t{}\theld until {}",
                        r.get::<String, _>("code"),
                        r.get::<Option<i64>, _>("held_until").unwrap_or(0)
                    ),
                }
            }
            for k in
                sqlx::query_scalar::<_, String>("SELECT tag_key FROM blocked_tags ORDER BY tag_key")
                    .fetch_all(&mut *c)
                    .await?
            {
                println!("blocked\t{k}");
            }
            Ok(())
        }
        IdentityAdmin::RevokeTag { guild_id } => {
            let g = Id(*guild_id);
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            let key: Option<String> =
                sqlx::query_scalar("DELETE FROM server_tags WHERE guild_id=? RETURNING tag_key")
                    .bind(id(g)?)
                    .fetch_optional(&mut *tx)
                    .await?;
            let key = key.ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            admin_audit(
                &mut tx,
                g,
                AuditAction::ServerTagUpdate,
                "tag",
                key.clone(),
                "Revoked by the instance administrator.",
            )
            .await?;
            sqlx::query("INSERT OR IGNORE INTO blocked_tags(tag_key,created_at) VALUES(?,?)")
                .bind(&key)
                .bind(now() as i64)
                .execute(&mut *tx)
                .await?;
            // A running server tells the wearers' viewers at its next
            // housekeeping pass (within a minute), in batches (`fanout`).
            sqlx::query("INSERT OR IGNORE INTO tag_announce(user_id) SELECT id FROM users WHERE tag_guild=?")
                .bind(id(g)?)
                .execute(&mut *tx)
                .await?;
            let n = sqlx::query("UPDATE users SET tag=NULL,tag_guild=NULL WHERE tag_guild=?")
                .bind(id(g)?)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            tx.commit().await?;
            println!("Revoked tag {key} of server {guild_id}; {n} members stopped wearing it. Open apps show the change within a minute.");
            Ok(())
        }
        IdentityAdmin::UnblockTag { text } => {
            let n = sqlx::query("DELETE FROM blocked_tags WHERE tag_key=?")
                .bind(tag_key(text.trim()))
                .execute(&mut *c)
                .await?
                .rows_affected();
            require(n == 1, ErrorCode::NotFound)?;
            println!("{} may be claimed again.", tag_key(text.trim()));
            Ok(())
        }
        IdentityAdmin::ReleaseVanity { code } => {
            let code = vanity_code(code).ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            let g: Option<Option<i64>> =
                sqlx::query_scalar("SELECT guild_id FROM vanity_codes WHERE code=?")
                    .bind(&code)
                    .fetch_optional(&mut *tx)
                    .await?;
            let g = g.flatten().ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            vanity::release(&mut tx, Id(g as u64)).await?;
            admin_audit(
                &mut tx,
                Id(g as u64),
                AuditAction::VanityUpdate,
                "code",
                code.clone(),
                "Released by the instance administrator.",
            )
            .await?;
            tx.commit().await?;
            println!("Released {code} from server {g}; it is held for 30 days.");
            Ok(())
        }
    }
}
