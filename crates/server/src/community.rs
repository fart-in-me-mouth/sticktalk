//! Community tools: rules screening, member applications, onboarding, channel
//! opt-in, this instance's server directory, server templates and insights.
//! Wire contracts: `sticktalk_proto::community`. Behavior, routes and limits:
//! `docs/features/community.md`.
//!
//! Hooks into shared code (each one line):
//! - `moderation::restrict`, `capacity` and `moderation::message_check` /
//!   `dm_check`: a member who still has to accept the rules (or answer the
//!   required pre-join questions) keeps only View Channel and Read History,
//!   exactly like a timeout, and is told why.
//! - `commands::JoinGuild`: `gate` refuses servers that take applications and
//!   `admit` starts screening and onboarding and counts the join.
//! - `commands::execute`: `observe` counts messages and leaves for insights and
//!   ticks "say hello" guide tasks, after the command committed.
//! - `voice`: `voice_joined` / `voice_left` count voice minutes.
//!
//! Nothing here runs while unused: there is no timer, and every list is bounded.
mod admission;
mod directory;
mod insights;
mod onboarding;
mod routes;
mod templates;
#[cfg(test)]
mod tests;

pub(crate) use directory::{admin_cli, featured_emoji, listed_icon};
pub(crate) use routes::routes;

use crate::{
    db::*,
    error::{require, Error, Result},
    permissions as acl,
    state::{now, Core},
};
use serde::{de::DeserializeOwned, Serialize};
use sqlx::{Row, SqliteConnection};
use std::collections::{HashMap, HashSet};
use sticktalk_proto::*;

pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

/// In-memory community state, owned by `Core` (serialized with commands).
#[derive(Default)]
pub(crate) struct Runtime {
    /// Event streams that opted in to `Event::Community`.
    pub(crate) subscribed: HashSet<String>,
    /// Live guild voice sessions: server, channel and start time.
    voice: HashMap<VoiceSession, (Id, Id, u64)>,
    /// The day each server's insight rows were last trimmed to the window.
    trimmed: HashMap<Id, u32>,
    pub(crate) directory: directory::Cache,
}

fn denied(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::Forbidden);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}
fn invalid(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::InvalidRequest);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}
fn json<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| Error::new(ErrorCode::Internal))
}
fn parse<T: DeserializeOwned + Default>(s: Option<String>) -> T {
    s.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
/// Trimmed text of at most `max` characters with no control characters
/// (newlines allowed when `lines`).
fn text(s: &str, max: usize, lines: bool) -> Result<String> {
    let s = s.trim();
    require(
        s.chars().count() <= max && !s.chars().any(|c| c.is_control() && !(lines && c == '\n')),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
/// Like `text`, and not empty.
fn label(s: &str, max: usize) -> Result<String> {
    let s = text(s, max, false)?;
    require(!s.is_empty(), ErrorCode::InvalidRequest)?;
    Ok(s)
}
/// An optional emoji: a Unicode emoji or a `<:name:id>` token, at most 64 bytes.
fn emoji(e: Option<String>) -> Result<Option<String>> {
    let e = e.map(|e| e.trim().to_owned()).filter(|e| !e.is_empty());
    if let Some(e) = &e {
        require(
            e.len() <= 64 && !e.chars().any(|c| c.is_control() || c.is_whitespace()),
            ErrorCode::InvalidRequest,
        )?;
    }
    Ok(e)
}
fn emit(out: &mut Deliveries, users: Vec<Id>, e: CommunityEvent) {
    out.push((users, Event::Community(e)));
}

/// Dispatch for `Command::Community`, inside the command transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: CommunityCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use CommunityCommand::*;
    let result = match cmd {
        Subscribe => {
            let stream = core
                .streams
                .get(key)
                .map(|s| s.id.clone())
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            let rt = &mut core.community;
            if rt.subscribed.len() >= 4096 {
                let live: HashSet<&String> = core.streams.values().map(|s| &s.id).collect();
                rt.subscribed.retain(|s| live.contains(s));
            }
            rt.subscribed.insert(stream);
            CommunityResult::Ack
        }
        SetAdmission {
            guild_id,
            screening,
            description,
            rules,
            reaccept,
            applications,
            questions,
        } => {
            let next = Admission {
                guild_id,
                screening,
                description,
                rules,
                rules_version: 0,
                applications,
                questions,
            };
            admission::set(c, u, next, reaccept, out).await?
        }
        AcceptRules {
            guild_id,
            rules_version,
        } => admission::accept(c, u, guild_id, rules_version, out).await?,
        Apply {
            via,
            answers,
            accept_rules,
        } => admission::apply(c, core, u, via, answers, accept_rules, out).await?,
        Withdraw { guild_id } => admission::withdraw(c, u, guild_id, out).await?,
        Review {
            guild_id,
            user_id,
            approve,
            reason,
        } => admission::review(c, u, guild_id, user_id, approve, reason, out).await?,
        Interview { guild_id, user_id } => {
            return admission::interview(c, core, u, guild_id, user_id, out).await;
        }
        SetOnboarding(o) => onboarding::set(c, u, o, out).await?,
        Respond {
            guild_id,
            responses,
            finish,
        } => {
            core.rate(format!("onboarding:{}", u.0), 60., 20.)?;
            onboarding::respond(c, u, guild_id, responses, finish, out).await?
        }
        SetChannels {
            guild_id,
            opted_in,
            show_all,
        } => {
            core.rate(format!("onboarding:{}", u.0), 60., 20.)?;
            onboarding::channels_choice(c, u, guild_id, opted_in, show_all, out).await?
        }
        CompleteTask { guild_id, task_id } => {
            onboarding::complete(c, u, guild_id, task_id, out).await?
        }
        JoinDiscovered { guild_id } => {
            return directory::join(c, u, guild_id, out).await;
        }
        SetListing { guild_id, listing } => directory::set(c, u, guild_id, listing).await?,
        CreateTemplate {
            guild_id,
            name,
            description,
        } => templates::create(c, u, guild_id, name, description).await?,
        SyncTemplate { code } => templates::sync(c, u, &code).await?,
        DeleteTemplate { code } => templates::delete(c, u, &code).await?,
        CreateFromTemplate { template, name } => {
            return templates::build(c, core, u, template, name, out).await;
        }
    };
    Ok(CommandResult::Community(result))
}

/// Joins that go through the moderation pre-check (paused invites, raid count).
pub(crate) fn joining(cmd: &CommunityCommand) -> Option<Id> {
    match cmd {
        CommunityCommand::JoinDiscovered { guild_id } => Some(*guild_id),
        _ => None,
    }
}
/// Commands after which voice access must be checked again (they can take
/// rights away from members).
pub(crate) fn voice_acl(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Community(
            CommunityCommand::Respond { .. } | CommunityCommand::SetAdmission { .. }
        )
    )
}

pub(crate) async fn admission_settings(c: &mut SqliteConnection, g: Id) -> Result<Admission> {
    let raw: Option<Option<String>> =
        sqlx::query_scalar("SELECT admission FROM community_settings WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_optional(&mut *c)
            .await?;
    let mut a: Admission = parse(raw.flatten());
    a.guild_id = g;
    Ok(a)
}
pub(crate) async fn onboarding_settings(c: &mut SqliteConnection, g: Id) -> Result<Onboarding> {
    let raw: Option<Option<String>> =
        sqlx::query_scalar("SELECT onboarding FROM community_settings WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_optional(&mut *c)
            .await?;
    let mut o: Onboarding = parse(raw.flatten());
    o.guild_id = g;
    Ok(o)
}
async fn save_admission(c: &mut SqliteConnection, a: &Admission) -> Result<()> {
    sqlx::query("INSERT INTO community_settings(guild_id,admission,updated_at) VALUES(?,?,?) ON CONFLICT(guild_id) DO UPDATE SET admission=excluded.admission,updated_at=excluded.updated_at")
        .bind(id(a.guild_id)?)
        .bind(json(a)?)
        .bind(now() as i64)
        .execute(c)
        .await?;
    Ok(())
}
async fn save_onboarding(c: &mut SqliteConnection, o: &Onboarding, by: Id) -> Result<()> {
    sqlx::query("INSERT INTO community_settings(guild_id,onboarding,configured_by,updated_at) VALUES(?,?,?,?) ON CONFLICT(guild_id) DO UPDATE SET onboarding=excluded.onboarding,configured_by=excluded.configured_by,updated_at=excluded.updated_at")
        .bind(id(o.guild_id)?)
        .bind(json(o)?)
        .bind(id(by)?)
        .bind(now() as i64)
        .execute(c)
        .await?;
    Ok(())
}

/// A member's own state; members who joined before any of this have defaults.
pub(crate) async fn member_state(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
) -> Result<MemberCommunity> {
    let r = sqlx::query("SELECT * FROM community_members WHERE guild_id=? AND user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(match r {
        None => MemberCommunity {
            guild_id: g,
            onboarded: true,
            ..Default::default()
        },
        Some(r) => MemberCommunity {
            guild_id: g,
            rules_pending: r.get::<i64, _>("rules_pending") != 0,
            questions_pending: r.get::<i64, _>("questions_pending") != 0,
            rules_version: r.get::<i64, _>("rules_version") as u32,
            responses: parse(r.get("responses")),
            onboarded: r.get::<i64, _>("onboarded") != 0,
            opted_in: parse(r.get("opted_in")),
            show_all: r.get::<i64, _>("show_all") != 0,
            guide_done: parse(r.get("guide_done")),
        },
    })
}
/// Store `m` as user `u`'s state.
async fn store(c: &mut SqliteConnection, u: Id, m: &MemberCommunity) -> Result<()> {
    sqlx::query("INSERT INTO community_members(guild_id,user_id,rules_pending,questions_pending,rules_version,responses,opted_in,show_all,guide_done,onboarded) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(guild_id,user_id) DO UPDATE SET rules_pending=?3,questions_pending=?4,rules_version=?5,responses=?6,opted_in=?7,show_all=?8,guide_done=?9,onboarded=?10")
        .bind(id(m.guild_id)?)
        .bind(id(u)?)
        .bind(m.rules_pending)
        .bind(m.questions_pending)
        .bind(i64::from(m.rules_version))
        .bind(json(&m.responses)?)
        .bind(json(&m.opted_in)?)
        .bind(m.show_all)
        .bind(json(&m.guide_done)?)
        .bind(m.onboarded)
        .execute(c)
        .await?;
    Ok(())
}
fn me_event(out: &mut Deliveries, u: Id, m: &MemberCommunity) {
    emit(out, vec![u], CommunityEvent::Me(Box::new(m.clone())));
}

/// For `moderation::restrict`: the member still has to accept the rules or
/// answer the required pre-join questions.
pub(crate) async fn pending(c: &mut SqliteConnection, g: Id, u: Id) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM community_members WHERE guild_id=? AND user_id=? AND (rules_pending=1 OR questions_pending=1)")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_one(c)
        .await?
        > 0)
}
/// For `capacity`: the members of `g` (or only `only`) who are still pending.
pub(crate) async fn pending_users(
    c: &mut SqliteConnection,
    g: Id,
    only: Option<Id>,
) -> Result<Vec<Id>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT user_id FROM community_members WHERE guild_id=?1 AND (rules_pending=1 OR questions_pending=1) AND (?2 IS NULL OR user_id=?2)")
        .bind(id(g)?)
        .bind(only.map(|v| v.0 as i64))
        .fetch_all(c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect())
}
/// For the moderation message and DM checks: why a pending member cannot talk.
pub(crate) async fn talk_gate(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Option<Error>> {
    if !pending(c, g, u).await? {
        return Ok(None);
    }
    let name: String = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let m = member_state(c, g, u).await?;
    Ok(Some(denied(
        "screening",
        if m.rules_pending {
            format!("Accept the rules of {name} to send messages, react, join voice and message its members.")
        } else {
            format!("Answer the questions {name} asks new members to start talking.")
        },
    )))
}

/// For `JoinGuild`, before the member is added: servers that take
/// applications are joined by applying.
pub(crate) async fn gate(c: &mut SqliteConnection, g: Id) -> Result<()> {
    if admission_settings(c, g).await?.applications {
        return Err(denied(
            "application",
            "This server asks new members to apply. Fill in its application to join.",
        ));
    }
    Ok(())
}
/// A new member was added (by invite, the directory, an approved application
/// or a template): start rules screening and onboarding, and count the join.
pub(crate) async fn admit(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    source: JoinSource,
    code: Option<&str>,
    rules_accepted: bool,
) -> Result<()> {
    let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let a = admission_settings(c, g).await?;
    let o = onboarding_settings(c, g).await?;
    let is_owner = owner == id(u)?;
    let rules = a.screening && !a.rules.is_empty() && !is_owner;
    let m = MemberCommunity {
        guild_id: g,
        rules_pending: rules && !rules_accepted,
        questions_pending: !is_owner
            && o.enabled
            && o.questions.iter().any(|q| q.pre_join && q.required),
        rules_version: if rules && rules_accepted {
            a.rules_version
        } else {
            0
        },
        onboarded: is_owner || !(o.enabled || rules),
        ..Default::default()
    };
    store(c, u, &m).await?;
    insights::joined(c, g, u, source, code).await?;
    if o.enabled && !is_owner {
        onboarding::reach(c, g, u, "joined").await?;
    }
    Ok(())
}

/// After a command committed, for each event it produced: insights counts and
/// server guide progress. Never fails the command.
pub(crate) async fn observe(core: &mut Core, c: &mut SqliteConnection, e: &Event) {
    let r = match e {
        Event::MessageCreated { message, .. } => message_sent(core, c, message).await,
        Event::MemberRemoved {
            guild_id, user_id, ..
        } => left(c, *guild_id, *user_id).await,
        _ => Ok(()),
    };
    if let Err(error) = r {
        tracing::debug!(code = ?error.0.code, "community bookkeeping skipped");
    }
}
async fn message_sent(core: &mut Core, c: &mut SqliteConnection, m: &Message) -> Result<()> {
    let row = sqlx::query("SELECT guild_id,kind,parent_id FROM channels WHERE id=?")
        .bind(id(m.channel_id)?)
        .fetch_optional(&mut *c)
        .await?;
    let Some(row) = row else { return Ok(()) };
    let Some(g) = row.get::<Option<i64>, _>("guild_id").map(|g| Id(g as u64)) else {
        return Ok(());
    };
    if m.author_id == crate::moderation::AUTOMOD_USER {
        return Ok(());
    }
    // Threads count toward the channel they are in.
    let ch = if row.get::<String, _>("kind") == "thread" {
        row.get::<Option<i64>, _>("parent_id")
            .map_or(m.channel_id, |p| Id(p as u64))
    } else {
        m.channel_id
    };
    trim(core, c, g).await?;
    insights::message(c, g, ch, m.author_id).await?;
    if let Some(done) = onboarding::talked(c, g, m.author_id, m.channel_id).await? {
        let mut out = vec![];
        me_event(&mut out, m.author_id, &done);
        for (users, e) in out {
            core.emit(&users, e)?;
        }
    }
    Ok(())
}
async fn left(c: &mut SqliteConnection, g: Id, u: Id) -> Result<()> {
    // Still a member (a stale event): nothing left.
    if role(c, u, g).await.is_ok() {
        return Ok(());
    }
    sqlx::query("DELETE FROM community_members WHERE guild_id=? AND user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    insights::left(c, g, u).await
}
/// Drop insight rows older than the window, once per server and day.
async fn trim(core: &mut Core, c: &mut SqliteConnection, g: Id) -> Result<()> {
    let today = day_of(now());
    if core.community.trimmed.get(&g) == Some(&today) {
        return Ok(());
    }
    if core.community.trimmed.len() >= 4096 {
        core.community.trimmed.clear();
    }
    core.community.trimmed.insert(g, today);
    insights::trim(c, g, today).await
}

/// A guild voice session's media connected (`voice::signal`).
pub(crate) fn voice_joined(core: &mut Core, session: VoiceSession) {
    let Some(entry) = core.voice.get(&session) else {
        return;
    };
    if entry.dm() {
        return;
    }
    let (g, ch) = (entry.state.guild_id, entry.state.channel_id);
    // Bounded by the live sessions: entries leave with their session.
    if core.community.voice.len() < 65_536 {
        core.community.voice.insert(session, (g, ch, now()));
    }
}
/// A voice session ended (`voice::remove`): count its minutes.
pub(crate) async fn voice_left(core: &mut Core, c: &mut SqliteConnection, session: VoiceSession) {
    let Some((g, ch, at)) = core.community.voice.remove(&session) else {
        return;
    };
    let minutes = now().saturating_sub(at).div_ceil(60_000);
    if let Err(error) = insights::voice(c, g, ch, minutes).await {
        tracing::debug!(code = ?error.0.code, "voice insights skipped");
    }
}

/// The member's rights changed (rules accepted, roles granted): refresh their
/// server summary and their entry in the member list.
async fn rights_changed(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
    u: Id,
) -> Result<()> {
    out.push((
        guild_users(c, g).await?,
        Event::MemberUpsert(member(c, g, u).await?),
    ));
    out.push((vec![u], Event::GuildUpsert(guild(c, u, g).await?)));
    Ok(())
}
/// Members with any of `rights` (owner and administrators included).
async fn with_right(c: &mut SqliteConnection, g: Id, rights: Permissions) -> Result<Vec<Id>> {
    let mut users = vec![];
    for bit in rights.iter() {
        users.extend(crate::capacity::members_with(c, g, None, bit).await?);
    }
    users.sort();
    users.dedup();
    Ok(users)
}
async fn guild_name(c: &mut SqliteConnection, g: Id) -> Result<String> {
    sqlx::query_scalar::<_, String>("SELECT name FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
/// What one member may see of the server's community state.
pub(crate) async fn state(c: &mut SqliteConnection, u: Id, g: Id) -> Result<CommunityState> {
    role(c, u, g).await?;
    let rights = acl::evaluate(c, u, g, None).await?;
    let pending_applications = if rights.contains(Permissions::KICK_MEMBERS) {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM community_applications WHERE guild_id=? AND status='pending'",
        )
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await? as u32
    } else {
        0
    };
    Ok(CommunityState {
        admission: admission_settings(c, g).await?,
        onboarding: onboarding_settings(c, g).await?,
        me: member_state(c, g, u).await?,
        pending_applications,
    })
}
