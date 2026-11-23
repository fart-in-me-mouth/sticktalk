//! Moderation: timeouts, kick with reason, prune, safety settings (verification
//! level, invite and DM pauses, raid alerts), AutoMod, age-restricted channels
//! and the audit log. See `docs/features/moderation.md`.
//!
//! Hooks into the shared command pipeline (`commands::execute`):
//! `precheck` runs before a command's transaction (AutoMod, timeout,
//! verification and pause checks; it also reads what the audit log will
//! compare), `record` runs inside it after the command succeeded, and `after`
//! runs once it committed. Anything timed (timeouts, pauses, raid alerts) is
//! expired by one task sleeping until the next deadline, never by polling.
pub(crate) mod audit;
pub(crate) mod automod;
pub(crate) mod coverage;
pub(crate) mod feature_audit;
use crate::{
    db::{self, *},
    error::{require, Error, Result},
    http, permissions as acl,
    state::{now, App, Core},
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
    Router,
};
use sqlx::{Connection as _, Row, SqliteConnection};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use sticktalk_proto::*;
type Deliveries = Vec<(Vec<Id>, Event)>;

/// What a timed-out member keeps: they can read, but not talk, react or join voice.
pub(crate) const TIMED_OUT_KEEPS: Permissions = Permissions::from_bits_retain(
    Permissions::VIEW_CHANNEL.bits() | Permissions::READ_HISTORY.bits(),
);
/// The author of AutoMod alerts. Below every generated Snowflake (those start
/// at 2^22), and its username cannot be registered (registration lowercases).
pub(crate) const AUTOMOD_USER: Id = Id((1 << 22) - 1);
const RAID_ALERT_MS: u64 = 3_600_000;
const REPEAT_WINDOW_MS: u64 = 30_000;
const MODERATOR: Permissions = Permissions::from_bits_retain(
    Permissions::MANAGE_GUILD.bits()
        | Permissions::KICK_MEMBERS.bits()
        | Permissions::BAN_MEMBERS.bits()
        | Permissions::MODERATE_MEMBERS.bits(),
);

/// In-memory moderation state, owned by `Core` (so serialized with commands).
/// Every collection is bounded.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Event streams that opted in to `Event::Moderation`.
    pub(crate) subscribed: HashSet<String>,
    /// Wakes the expiry task when a new deadline is set.
    pub(crate) wake: Arc<tokio::sync::Notify>,
    rules: HashMap<Id, Arc<Vec<automod::Compiled>>>,
    /// Recent message fingerprints per (server, author), for repeated-message spam.
    recent: HashMap<(Id, Id), VecDeque<(u64, u64)>>,
    /// Recent join times per server, for raid detection.
    joins: HashMap<Id, VecDeque<u64>>,
    pub(crate) reconcile_voice: bool,
    rearm: bool,
}
impl Runtime {
    fn repeats(&mut self, g: Id, u: Id, print: u64) -> usize {
        let t = now();
        if self.recent.len() >= 4096 && !self.recent.contains_key(&(g, u)) {
            self.recent.retain(|_, q| {
                q.back()
                    .is_some_and(|(_, at)| t.saturating_sub(*at) < REPEAT_WINDOW_MS)
            });
            if self.recent.len() >= 4096 {
                self.recent.clear();
            }
        }
        let q = self.recent.entry((g, u)).or_default();
        while q
            .front()
            .is_some_and(|(_, at)| t.saturating_sub(*at) >= REPEAT_WINDOW_MS)
            || q.len() >= 10
        {
            q.pop_front();
        }
        let n = q.iter().filter(|(p, _)| *p == print).count();
        q.push_back((print, t));
        n
    }
    fn join(&mut self, g: Id) -> usize {
        let t = now();
        let window = u64::from(RAID_WINDOW_SECONDS) * 1000;
        if self.joins.len() >= 1024 && !self.joins.contains_key(&g) {
            self.joins
                .retain(|_, q| q.back().is_some_and(|at| t.saturating_sub(*at) < window));
            if self.joins.len() >= 1024 {
                self.joins.clear();
            }
        }
        let q = self.joins.entry(g).or_default();
        while q.front().is_some_and(|at| t.saturating_sub(*at) >= window) || q.len() >= 64 {
            q.pop_front();
        }
        q.push_back(t);
        q.len()
    }
}

fn denied(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::Forbidden);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}
fn reason(r: Option<String>) -> Result<Option<String>> {
    let r = r.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty());
    if let Some(r) = &r {
        require(
            r.chars().count() <= MAX_REASON_CHARS
                && !r.chars().any(|c| c.is_control() && c != '\n'),
            ErrorCode::InvalidRequest,
        )?;
    }
    Ok(r)
}
fn minutes(ms: u64) -> String {
    let m = ms.div_ceil(60_000);
    if m == 1 {
        "1 minute".into()
    } else {
        format!("{m} minutes")
    }
}
fn span(seconds: u64) -> String {
    match seconds {
        s if s >= 86_400 && s % 86_400 == 0 => plural(s / 86_400, "day"),
        s if s >= 3_600 && s % 3_600 == 0 => plural(s / 3_600, "hour"),
        s if s >= 60 && s % 60 == 0 => plural(s / 60, "minute"),
        s => plural(s, "second"),
    }
}
fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// Applies an active timeout to computed permissions (`permissions::evaluate`).
pub(crate) async fn restrict(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    p: Permissions,
) -> Result<Permissions> {
    Ok(
        if timeout(c, g, u).await?.is_some() || crate::community::pending(c, g, u).await? {
            p & TIMED_OUT_KEEPS
        } else {
            p
        },
    )
}
pub(crate) async fn timeout(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Option<TimeoutInfo>> {
    let r = sqlx::query(
        "SELECT until,reason FROM member_timeouts WHERE guild_id=? AND user_id=? AND until>?",
    )
    .bind(id(g)?)
    .bind(id(u)?)
    .bind(now() as i64)
    .fetch_optional(&mut *c)
    .await?;
    Ok(r.map(|r| TimeoutInfo {
        until: r.get::<i64, _>(0) as u64,
        reason: r.get(1),
    }))
}
/// For `db::member`: when the member's timeout ends, if they are timed out.
pub(crate) async fn timeout_until(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
) -> Result<Option<UnixMillis>> {
    Ok(timeout(c, g, u).await?.map(|t| t.until))
}

pub(crate) async fn safety(c: &mut SqliteConnection, g: Id) -> Result<GuildSafety> {
    let r = sqlx::query("SELECT * FROM guild_safety WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    let t = now();
    let Some(r) = r else {
        return Ok(GuildSafety {
            guild_id: g,
            ..Default::default()
        });
    };
    let active = |v: Option<i64>| v.map(|v| v as u64).filter(|v| *v > t);
    Ok(GuildSafety {
        guild_id: g,
        verification_level: en(r.get("verification_level"))?,
        invites_paused_until: active(r.get("invites_paused_until")),
        dms_paused_until: active(r.get("dms_paused_until")),
        raid: r
            .get::<Option<i64>, _>("raid_detected_at")
            .map(|v| v as u64)
            .filter(|at| at + RAID_ALERT_MS > t)
            .map(|detected_at| RaidAlert {
                detected_at,
                joins: r.get::<i64, _>("raid_joins") as u32,
                window_seconds: RAID_WINDOW_SECONDS,
            }),
    })
}
async fn save_safety(c: &mut SqliteConnection, s: &GuildSafety) -> Result<()> {
    sqlx::query("INSERT INTO guild_safety(guild_id,verification_level,invites_paused_until,dms_paused_until,raid_detected_at,raid_joins) VALUES(?,?,?,?,?,?) ON CONFLICT(guild_id) DO UPDATE SET verification_level=excluded.verification_level,invites_paused_until=excluded.invites_paused_until,dms_paused_until=excluded.dms_paused_until,raid_detected_at=excluded.raid_detected_at,raid_joins=excluded.raid_joins")
        .bind(id(s.guild_id)?)
        .bind(enum_str(s.verification_level))
        .bind(s.invites_paused_until.map(|v| v as i64))
        .bind(s.dms_paused_until.map(|v| v as i64))
        .bind(s.raid.as_ref().map(|r| r.detected_at as i64))
        .bind(s.raid.as_ref().map_or(0, |r| i64::from(r.joins)))
        .execute(c)
        .await?;
    Ok(())
}
async fn moderators(c: &mut SqliteConnection, g: Id) -> Result<Vec<Id>> {
    let mut users = vec![];
    for bit in MODERATOR.iter() {
        users.extend(crate::capacity::members_with(c, g, None, bit).await?);
    }
    users.sort();
    users.dedup();
    Ok(users)
}
/// Everyone gets the settings; only moderators see the raid alert.
async fn safety_events(c: &mut SqliteConnection, g: Id) -> Result<Deliveries> {
    let s = safety(c, g).await?;
    let mods = moderators(c, g).await?;
    let others: Vec<Id> = guild_users(c, g)
        .await?
        .into_iter()
        .filter(|u| !mods.contains(u))
        .collect();
    let mut public = s.clone();
    public.raid = None;
    Ok(vec![
        (mods, Event::Moderation(ModerationEvent::SafetyUpdated(s))),
        (
            others,
            Event::Moderation(ModerationEvent::SafetyUpdated(public)),
        ),
    ])
}

/// How long the verification level stops `u` sending in `g`, if it does.
async fn verification_wait(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Option<UnixMillis>> {
    let level = safety(c, g).await?.verification_level;
    if level == VerificationLevel::None {
        return Ok(None);
    }
    let r = sqlx::query("SELECT g.owner_id,m.joined_at,u.created_at,EXISTS(SELECT 1 FROM member_roles r WHERE r.guild_id=m.guild_id AND r.user_id=m.user_id AND r.role_id<>m.guild_id) AS roles FROM guild_members m JOIN guilds g ON g.id=m.guild_id JOIN users u ON u.id=m.user_id WHERE m.guild_id=? AND m.user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    let Some(r) = r else { return Ok(None) };
    // Members with any role, and the owner, are trusted (as on Discord).
    if r.get::<i64, _>("owner_id") == id(u)? || r.get::<bool, _>("roles") {
        return Ok(None);
    }
    let mut ready = (r.get::<i64, _>("created_at").max(0) as u64).saturating_add(ACCOUNT_AGE_MS);
    if level == VerificationLevel::MemberAge {
        ready = ready
            .max((r.get::<i64, _>("joined_at").max(0) as u64).saturating_add(MEMBERSHIP_AGE_MS));
    }
    Ok((ready > now()).then_some(ready))
}
async fn verification_error(c: &mut SqliteConnection, g: Id, until: UnixMillis) -> Result<Error> {
    let level = safety(c, g).await?.verification_level;
    let name: String = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let wait = minutes(until.saturating_sub(now()));
    Ok(denied(
        "verification",
        match level {
            VerificationLevel::MemberAge => format!("{name} lets new members talk once their account is 5 minutes old and they have been a member for 10 minutes. You can send messages in {wait}."),
            _ => format!("{name} lets accounts talk once they are 5 minutes old. You can send messages in {wait}."),
        },
    ))
}

/// What `after` must do once the command has committed.
pub(crate) struct Hook {
    audit: Option<audit::Pending>,
    join: Option<Id>,
    slowmode: Option<Id>,
}
/// Before a command's transaction: refuse what moderation forbids, apply
/// AutoMod, and read the state the audit log will compare.
pub(crate) async fn precheck(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    cmd: &Command,
) -> Result<Hook> {
    let mut hook = Hook {
        audit: audit::prepare(c, cmd).await,
        join: None,
        slowmode: None,
    };
    match cmd {
        Command::SendMessage(d) => {
            message_check(
                app,
                core,
                c,
                u,
                d.channel_id,
                &d.content,
                &d.mention_ids,
                true,
            )
            .await?
        }
        Command::EditMessage(e) => {
            message_check(
                app,
                core,
                c,
                u,
                e.channel_id,
                &e.content,
                &e.mention_ids,
                false,
            )
            .await?
        }
        Command::JoinGuild { invite_code } if invite_code.len() == 22 => {
            let g: Option<i64> =
                sqlx::query_scalar("SELECT guild_id FROM invites WHERE code_hash=?")
                    .bind(crate::state::hash(invite_code.as_bytes()))
                    .fetch_optional(&mut *c)
                    .await?;
            if let Some(g) = g.map(|g| Id(g as u64)) {
                if role(c, u, g).await.is_err() {
                    if safety(c, g).await?.invites_paused_until.is_some() {
                        return Err(denied(
                            "invites_paused",
                            "This server has paused invites. Try again later.",
                        ));
                    }
                    hook.join = Some(g);
                }
            }
        }
        Command::Launch(LaunchCommand::SetSlowmode { channel_id, .. }) => {
            hook.slowmode = Some(*channel_id);
        }
        // Joining from the server directory is paused and counted like an invite.
        Command::Community(cmd) => {
            if let Some(g) = crate::community::joining(cmd) {
                if role(c, u, g).await.is_err() {
                    if safety(c, g).await?.invites_paused_until.is_some() {
                        return Err(denied(
                            "invites_paused",
                            "This server has paused invites. Try again later.",
                        ));
                    }
                    hook.join = Some(g);
                }
            }
        }
        Command::Content(cmd) => content_check(app, core, c, u, cmd).await?,
        // Sticker messages get the same checks as `SendMessage`.
        Command::Expression(cmd) => {
            if let Some((ch, content, mentions)) = crate::expression::outgoing(cmd) {
                message_check(app, core, c, u, ch, content, mentions, true).await?
            }
        }
        Command::Threads(cmd) => coverage::thread_check(app, core, c, u, cmd).await?,
        _ => {}
    }
    Ok(hook)
}
/// Content commands that create a message get the same checks as `SendMessage`:
/// timeout, verification level and AutoMod.
async fn content_check(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    cmd: &ContentCommand,
) -> Result<()> {
    match cmd {
        ContentCommand::SendMessage { draft, .. } => {
            message_check(
                app,
                core,
                c,
                u,
                draft.channel_id,
                &draft.content,
                &draft.mention_ids,
                true,
            )
            .await
        }
        ContentCommand::CreatePoll(d) => {
            let mut text = d.question.clone();
            for a in &d.answers {
                text.push('\n');
                text.push_str(&a.text);
            }
            message_check(app, core, c, u, d.channel_id, &text, &[], true).await
        }
        ContentCommand::SendGif { channel_id, .. } => {
            message_check(app, core, c, u, *channel_id, "", &[], true).await
        }
        // Voting only needs Read History, which a timeout keeps.
        ContentCommand::VotePoll { channel_id, .. } => coverage::timed_out(c, u, *channel_id).await,
        ContentCommand::Forward {
            channel_id,
            message_id,
            destinations,
            comment,
        } => {
            // Nothing to check when the author may not forward it: the command refuses.
            let Some(snapshot) = crate::content::forward_text(c, u, *channel_id, *message_id).await
            else {
                return Ok(());
            };
            let text = format!("{comment}\n{snapshot}");
            for ch in destinations.iter().take(MAX_FORWARD_DESTINATIONS) {
                message_check(app, core, c, u, *ch, &text, &[], true).await?;
            }
            Ok(())
        }
        // Checked as an edit here (no repeat counting); the delivery is checked as a send.
        ContentCommand::ScheduleMessage {
            channel_id,
            content,
            mention_ids,
            ..
        } => message_check(app, core, c, u, *channel_id, content, mention_ids, false).await,
        ContentCommand::UpdateScheduled {
            id: scheduled,
            content,
            mention_ids,
            ..
        } => {
            let Some(ch) = scheduled_channel(c, u, *scheduled).await? else {
                return Ok(());
            };
            message_check(app, core, c, u, ch, content, mention_ids, false).await
        }
        ContentCommand::SendScheduledNow { id: scheduled } => {
            let row = sqlx::query(
                "SELECT channel_id,content FROM scheduled_messages WHERE id=? AND author_id=?",
            )
            .bind(id(*scheduled)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
            let Some(row) = row else {
                return Ok(());
            };
            let ch = Id(row.get::<i64, _>("channel_id") as u64);
            let content: String = row.get("content");
            message_check(app, core, c, u, ch, &content, &[], true).await
        }
        _ => Ok(()),
    }
}
async fn scheduled_channel(c: &mut SqliteConnection, u: Id, scheduled: Id) -> Result<Option<Id>> {
    let ch: Option<i64> =
        sqlx::query_scalar("SELECT channel_id FROM scheduled_messages WHERE id=? AND author_id=?")
            .bind(id(scheduled)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(ch.map(|ch| Id(ch as u64)))
}
/// Inside the command's transaction, after it succeeded.
pub(crate) async fn record(
    c: &mut SqliteConnection,
    u: Id,
    hook: &mut Hook,
    result: &CommandResult,
) -> Result<()> {
    audit::record(c, u, hook.audit.take(), result).await
}
/// After the command committed.
pub(crate) async fn after(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    hook: Hook,
) -> Result<()> {
    if let Some(g) = hook.join {
        if role(c, u, g).await.is_ok() {
            raid_check(core, c, g).await?;
        }
    }
    if let Some(ch) = hook.slowmode {
        let seconds: i64 = sqlx::query_scalar(
            "SELECT COALESCE((SELECT seconds FROM channel_slowmode WHERE channel_id=?),0)",
        )
        .bind(id(ch)?)
        .fetch_one(&mut *c)
        .await?;
        let users = audience(c, ch).await?;
        core.emit(
            &users,
            Event::Moderation(ModerationEvent::SlowmodeUpdated {
                channel_id: ch,
                seconds: seconds as u32,
            }),
        )?;
    }
    if std::mem::take(&mut core.moderation.reconcile_voice) {
        crate::voice::reconcile(app, core, c).await?;
    }
    if std::mem::take(&mut core.moderation.rearm) {
        core.moderation.wake.notify_one();
    }
    Ok(())
}
async fn raid_check(core: &mut Core, c: &mut SqliteConnection, g: Id) -> Result<()> {
    let joins = core.moderation.join(g) as u32;
    if joins < RAID_JOIN_THRESHOLD {
        return Ok(());
    }
    let mut s = safety(c, g).await?;
    // One alert per burst: an active alert only has its count raised.
    let fresh = s.raid.is_none();
    s.raid = Some(RaidAlert {
        detected_at: s.raid.as_ref().map_or(now(), |r| r.detected_at),
        joins: s.raid.as_ref().map_or(joins, |r| r.joins.max(joins)),
        window_seconds: RAID_WINDOW_SECONDS,
    });
    save_safety(c, &s).await?;
    if fresh {
        tracing::info!(guild = g.0, joins, "raid alert raised");
    }
    for (users, e) in safety_events(c, g).await? {
        core.emit(&users, e)?;
    }
    core.moderation.wake.notify_one();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn message_check(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    content: &str,
    mentions: &[Id],
    send: bool,
) -> Result<()> {
    screen(
        app,
        core,
        c,
        u,
        ch,
        content,
        mentions,
        send,
        Permissions::SEND_MESSAGES,
    )
    .await
}
/// `message_check` for text that needs another right than Send Messages
/// (a thread name needs Create Threads): AutoMod is skipped when the author
/// lacks `needed`, because the command refuses.
#[allow(clippy::too_many_arguments)]
async fn screen(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    content: &str,
    mentions: &[Id],
    send: bool,
    needed: Permissions,
) -> Result<()> {
    // Say nothing about channels the author cannot use; the command refuses them.
    let Ok(channel) = access(c, u, ch, Permissions::VIEW_CHANNEL).await else {
        return Ok(());
    };
    let Some(g) = channel.summary.guild_id else {
        if send && channel.summary.kind == ChannelKind::Direct {
            dm_check(c, u, &channel).await?;
        }
        return Ok(());
    };
    if let Some(t) = timeout(c, g, u).await? {
        return Err(denied(
            "timeout",
            format!(
                "You are timed out in this server for {} more. You can read, but not send messages, react or join voice.",
                minutes(t.until.saturating_sub(now()))
            ),
        ));
    }
    if let Some(e) = crate::community::talk_gate(c, g, u).await? {
        return Err(e);
    }
    if send {
        if let Some(until) = verification_wait(c, g, u).await? {
            return Err(verification_error(c, g, until).await?);
        }
    }
    let rights = acl::evaluate(c, u, g, Some(ch)).await?;
    if !rights.intersects(needed) && !needed.is_empty()
        || rights.contains(Permissions::MANAGE_GUILD)
    {
        return Ok(());
    }
    let rules = automod::rules(&mut core.moderation, c, g).await?;
    if !rules.iter().any(|r| r.rule.enabled) {
        return Ok(());
    }
    let counts_repeats = rules.iter().any(|r| {
        r.rule.enabled
            && matches!(r.rule.trigger, AutoModTrigger::Spam { duplicates, .. } if duplicates > 0)
    });
    let repeats = if send && counts_repeats {
        core.moderation.repeats(g, u, automod::fingerprint(content))
    } else {
        0
    };
    let roles = acl::role_ids(c, g, u).await?;
    let lineage = acl::lineage(c, g, ch).await?;
    let hits = automod::check(
        &rules,
        &roles,
        &lineage,
        &automod::Draft {
            content,
            mentions,
            repeats,
        },
    );
    if hits.is_empty() {
        return Ok(());
    }
    enforce(app, core, c, u, g, &channel, content, hits).await
}
async fn dm_check(c: &mut SqliteConnection, u: Id, channel: &Channel) -> Result<()> {
    let Some(peer) = channel
        .summary
        .recipient_ids
        .iter()
        .copied()
        .find(|v| *v != u)
    else {
        return Ok(());
    };
    dm_pair(c, u, peer).await
}
/// Paused DMs and the verification level, for a DM between `u` and `peer`
/// (also used when a first-contact message request is started).
pub(crate) async fn dm_pair(c: &mut SqliteConnection, u: Id, peer: Id) -> Result<()> {
    if friends(c, u, peer).await.is_ok() {
        return Ok(());
    }
    let shared: Vec<i64> = sqlx::query_scalar("SELECT a.guild_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=? ORDER BY a.guild_id")
        .bind(id(u)?)
        .bind(id(peer)?)
        .fetch_all(&mut *c)
        .await?;
    for g in shared.into_iter().map(|g| Id(g as u64)) {
        if safety(c, g).await?.dms_paused_until.is_some() {
            let name: String = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
                .bind(id(g)?)
                .fetch_one(&mut *c)
                .await?;
            return Err(denied(
                "dms_paused",
                format!("{name} has paused direct messages between members who are not friends."),
            ));
        }
        if let Some(until) = verification_wait(c, g, u).await? {
            return Err(verification_error(c, g, until).await?);
        }
        if let Some(e) = crate::community::talk_gate(c, g, u).await? {
            return Err(e);
        }
    }
    Ok(())
}
async fn ensure_automod_user(c: &mut SqliteConnection) -> Result<UserSummary> {
    sqlx::query("INSERT OR IGNORE INTO users(id,username,password_hash,display_name,desired_presence,created_at) VALUES(?,'AutoMod','','AutoMod','invisible',0)")
        .bind(id(AUTOMOD_USER)?)
        .execute(&mut *c)
        .await?;
    user(c, AUTOMOD_USER).await
}
/// AutoMod fired: alert, time out, log; then refuse the message if a rule blocks it.
#[allow(clippy::too_many_arguments)]
async fn enforce(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    channel: &Channel,
    content: &str,
    hits: Vec<automod::Hit>,
) -> Result<()> {
    let block = hits.iter().find(|h| h.rule.actions.block).map(|h| {
        h.rule.actions.message.clone().unwrap_or_else(|| {
            format!(
                "This server's AutoMod blocked your message because {}.",
                h.why
            )
        })
    });
    let channel_name = channel.summary.name.clone().unwrap_or_default();
    let mut out: Deliveries = vec![];
    let mut timed_out = false;
    let mut tx = c.begin().await?;
    let bot = ensure_automod_user(&mut tx).await?;
    let author = user(&mut tx, u).await?;
    for h in &hits {
        let a = &h.rule.actions;
        let base = |action| {
            audit::Entry::new(g, action, AuditTarget::User, Some(u))
                .named(author.username.clone())
                .change("rule", None, Some(h.rule.name.clone()))
                .change("channel", None, Some(format!("#{channel_name}")))
                .reason(Some(format!("The message was flagged because {}.", h.why)))
        };
        if a.block {
            audit::write(&mut tx, AUTOMOD_USER, base(AuditAction::AutomodBlock)).await?;
        }
        // At most one alert per member and rule every ten seconds.
        let alert = a.alert_channel_id.filter(|_| {
            core.rate(format!("automod-alert:{}:{}", u.0, h.rule.id.0), 6., 1.)
                .is_ok()
        });
        if let Some(alert) = alert {
            if let Ok(target) = channel_in(&mut tx, alert, g).await {
                let excerpt: String = content
                    .chars()
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .take(300)
                    .collect();
                let text = format!(
                    "**AutoMod** {} a message from @{} in #{}.\nRule: {}, because {}.{}\n> {}",
                    if a.block { "blocked" } else { "flagged" },
                    author.username,
                    channel_name,
                    h.rule.name,
                    h.why,
                    if a.timeout_seconds > 0 {
                        format!(
                            " They were timed out for {}.",
                            span(u64::from(a.timeout_seconds))
                        )
                    } else {
                        String::new()
                    },
                    excerpt
                );
                let m = core.id().await?;
                sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)")
                    .bind(id(m)?)
                    .bind(id(target)?)
                    .bind(id(AUTOMOD_USER)?)
                    .bind(text)
                    .bind(now() as i64)
                    .execute(&mut *tx)
                    .await?;
                let message = message(&mut tx, AUTOMOD_USER, target, m, false).await?;
                let audience = audience(&mut tx, target).await?;
                recount(&mut tx, target).await?;
                out.push((
                    audience.clone(),
                    Event::MessageCreated {
                        message,
                        author: bot.clone(),
                    },
                ));
                for v in audience {
                    if let Ok(r) = read(&mut tx, v, target).await {
                        out.push((vec![v], Event::ReadStateUpdated(r)));
                    }
                }
                audit::write(&mut tx, AUTOMOD_USER, base(AuditAction::AutomodAlert)).await?;
            }
        }
        if a.timeout_seconds > 0 {
            let until = now() + u64::from(a.timeout_seconds) * 1000;
            sqlx::query("INSERT INTO member_timeouts(guild_id,user_id,until,reason,actor_id) VALUES(?,?,?,?,?) ON CONFLICT(guild_id,user_id) DO UPDATE SET until=MAX(until,excluded.until),reason=excluded.reason,actor_id=excluded.actor_id")
                .bind(id(g)?)
                .bind(id(u)?)
                .bind(until as i64)
                .bind(format!("AutoMod: {}", h.rule.name))
                .bind(id(AUTOMOD_USER)?)
                .execute(&mut *tx)
                .await?;
            audit::write(
                &mut tx,
                AUTOMOD_USER,
                base(AuditAction::AutomodTimeout).change(
                    "duration",
                    None,
                    Some(span(u64::from(a.timeout_seconds))),
                ),
            )
            .await?;
            timed_out = true;
        }
    }
    if timed_out {
        timeout_events(&mut tx, &mut out, g, u).await?;
    }
    core.persist_id(&mut tx).await?;
    tx.commit().await?;
    for (users, e) in out {
        core.emit(&users, e)?;
    }
    if timed_out {
        crate::voice::reconcile(app, core, c).await?;
        core.moderation.wake.notify_one();
    }
    match block {
        Some(message) => Err(denied("automod", message)),
        None => Ok(()),
    }
}
async fn channel_in(c: &mut SqliteConnection, ch: Id, g: Id) -> Result<Id> {
    let channel = channel(c, ch).await?;
    require(
        channel.summary.guild_id == Some(g) && channel.summary.kind == ChannelKind::Text,
        ErrorCode::NotFound,
    )?;
    Ok(ch)
}
async fn timeout_events(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
    v: Id,
) -> Result<()> {
    if role(c, v, g).await.is_err() {
        return Ok(());
    }
    out.push((
        guild_users(c, g).await?,
        Event::MemberUpsert(member(c, g, v).await?),
    ));
    out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
    let t = timeout(c, g, v).await?;
    out.push((
        vec![v],
        Event::Moderation(ModerationEvent::TimeoutUpdated {
            guild_id: g,
            timeout: t,
        }),
    ));
    Ok(())
}
/// Remove members from a server (kick and prune), with the same cleanup and
/// events as leaving, batched so a prune costs one pass over the server.
async fn remove(c: &mut SqliteConnection, out: &mut Deliveries, g: Id, users: &[Id]) -> Result<()> {
    for v in users {
        sqlx::query("DELETE FROM voice_connect_denials WHERE user_id=? AND channel_id IN (SELECT id FROM channels WHERE guild_id=?)").bind(id(*v)?).bind(id(g)?).execute(&mut *c).await?;
        sqlx::query("DELETE FROM guild_members WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(*v)?)
            .execute(&mut *c)
            .await?;
        sqlx::query("DELETE FROM read_states WHERE user_id=? AND channel_id IN (SELECT id FROM channels WHERE guild_id=?)").bind(id(*v)?).bind(id(g)?).execute(&mut *c).await?;
        out.push((
            vec![*v],
            Event::GuildRemoved {
                guild_id: g,
                reason: RemovalReason::Kicked,
            },
        ));
    }
    let remaining = guild_users(c, g).await?;
    for v in users {
        out.push((
            remaining.clone(),
            Event::MemberRemoved {
                guild_id: g,
                user_id: *v,
                reason: RemovalReason::Kicked,
            },
        ));
    }
    for v in remaining {
        out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
    }
    Ok(())
}
/// Members a prune by `actor` would remove.
async fn prune_candidates(
    c: &mut SqliteConnection,
    core: &Core,
    g: Id,
    actor: Id,
    days: u32,
    roles: &[Id],
) -> Result<Vec<Id>> {
    require(PRUNE_DAYS.contains(&days), ErrorCode::InvalidRequest)?;
    unique(roles, MAX_ROLES_PER_GUILD)?;
    let all = acl::roles(c, g).await?;
    for r in roles {
        require(
            *r != g && all.iter().any(|v| v.id == *r),
            ErrorCode::InvalidRequest,
        )?;
    }
    let cutoff = now().saturating_sub(u64::from(days) * 86_400_000) as i64;
    let online: HashSet<Id> = core
        .streams
        .values()
        .filter(|s| s.connection.is_some())
        .map(|s| s.user)
        .collect();
    let rows = sqlx::query("SELECT m.user_id FROM guild_members m WHERE m.guild_id=? AND m.role<>'owner' AND m.user_id<>? AND m.joined_at<? AND COALESCE((SELECT MAX(s.last_used_at) FROM sessions s WHERE s.user_id=m.user_id),0)<? ORDER BY m.user_id")
        .bind(id(g)?)
        .bind(id(actor)?)
        .bind(cutoff)
        .bind(cutoff)
        .fetch_all(&mut *c)
        .await?;
    let mut out = vec![];
    for r in rows {
        let v = Id(r.get::<i64, _>(0) as u64);
        if online.contains(&v) {
            continue;
        }
        let has: Vec<Id> = acl::role_ids(c, g, v)
            .await?
            .into_iter()
            .filter(|r| *r != g)
            .collect();
        let selected = if roles.is_empty() {
            has.is_empty()
        } else {
            has.iter().any(|r| roles.contains(r))
        };
        if selected && acl::above_member(c, g, actor, v).await.is_ok() {
            out.push(v);
        }
    }
    Ok(out)
}

pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: ModerationCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use ModerationCommand::*;
    match cmd {
        Subscribe => {
            let stream = core
                .streams
                .get(key)
                .map(|s| s.id.clone())
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            let rt = &mut core.moderation;
            if rt.subscribed.len() >= 4096 {
                let live: HashSet<&String> = core.streams.values().map(|s| &s.id).collect();
                rt.subscribed.retain(|s| live.contains(s));
            }
            rt.subscribed.insert(stream);
            Ok(CommandResult::Ack)
        }
        Kick {
            guild_id: g,
            user_id: v,
            reason: r,
        } => {
            guild_perm(c, u, g, Permissions::KICK_MEMBERS).await?;
            let r = reason(r)?;
            require(v != u, ErrorCode::HierarchyViolation)?;
            role(c, v, g).await?;
            acl::above_member(c, g, u, v).await?;
            let name = user(c, v).await?.username;
            remove(c, out, g, &[v]).await?;
            audit::write(
                c,
                u,
                audit::Entry::new(g, AuditAction::MemberKick, AuditTarget::User, Some(v))
                    .named(name)
                    .reason(r),
            )
            .await?;
            core.moderation.reconcile_voice = true;
            Ok(CommandResult::Ack)
        }
        Timeout {
            guild_id: g,
            user_id: v,
            seconds,
            reason: r,
        } => {
            guild_perm(c, u, g, Permissions::MODERATE_MEMBERS).await?;
            let r = reason(r)?;
            require(seconds <= MAX_TIMEOUT_SECONDS, ErrorCode::InvalidRequest)?;
            require(v != u, ErrorCode::HierarchyViolation)?;
            role(c, v, g).await?;
            acl::above_member(c, g, u, v).await?;
            // Administrators cannot be timed out (their rights bypass it).
            require(
                !acl::evaluate(c, v, g, None)
                    .await?
                    .contains(Permissions::ADMINISTRATOR),
                ErrorCode::HierarchyViolation,
            )?;
            let name = user(c, v).await?.username;
            let previous = timeout(c, g, v).await?;
            if seconds > 0 {
                let until = now() + u64::from(seconds) * 1000;
                sqlx::query("INSERT INTO member_timeouts(guild_id,user_id,until,reason,actor_id) VALUES(?,?,?,?,?) ON CONFLICT(guild_id,user_id) DO UPDATE SET until=excluded.until,reason=excluded.reason,actor_id=excluded.actor_id")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .bind(until as i64)
                    .bind(&r)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?;
                audit::write(
                    c,
                    u,
                    audit::Entry::new(g, AuditAction::MemberTimeout, AuditTarget::User, Some(v))
                        .named(name)
                        .change("duration", None, Some(span(u64::from(seconds))))
                        .reason(r),
                )
                .await?;
            } else {
                sqlx::query("DELETE FROM member_timeouts WHERE guild_id=? AND user_id=?")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .execute(&mut *c)
                    .await?;
                if previous.is_none() {
                    return Ok(CommandResult::Member(member(c, g, v).await?));
                }
                audit::write(
                    c,
                    u,
                    audit::Entry::new(
                        g,
                        AuditAction::MemberTimeoutRemove,
                        AuditTarget::User,
                        Some(v),
                    )
                    .named(name)
                    .reason(r),
                )
                .await?;
            }
            timeout_events(c, out, g, v).await?;
            core.moderation.reconcile_voice = true;
            core.moderation.rearm = true;
            Ok(CommandResult::Member(member(c, g, v).await?))
        }
        Prune {
            guild_id: g,
            days,
            role_ids,
            reason: r,
        } => {
            guild_perm(c, u, g, Permissions::KICK_MEMBERS).await?;
            let r = reason(r)?;
            let users = prune_candidates(c, core, g, u, days, &role_ids).await?;
            if !users.is_empty() {
                remove(c, out, g, &users).await?;
                let mut names = vec![];
                for v in &role_ids {
                    names.push(
                        acl::roles(c, g)
                            .await?
                            .into_iter()
                            .find(|x| x.id == *v)
                            .map_or_else(String::new, |x| x.name),
                    );
                }
                audit::write(
                    c,
                    u,
                    audit::Entry::new(g, AuditAction::MemberPrune, AuditTarget::Guild, Some(g))
                        .change("members removed", None, Some(users.len().to_string()))
                        .change("inactive for", None, Some(plural(u64::from(days), "day")))
                        .change(
                            "roles",
                            None,
                            Some(if names.is_empty() {
                                "members without roles".into()
                            } else {
                                names.join(", ")
                            }),
                        )
                        .reason(r),
                )
                .await?;
                core.moderation.reconcile_voice = true;
            }
            Ok(CommandResult::Moderation(ModerationResult::Pruned {
                count: users.len() as u32,
            }))
        }
        SetVerificationLevel { guild_id: g, level } => {
            guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
            let mut s = safety(c, g).await?;
            let before = s.verification_level;
            s.verification_level = level;
            save_safety(c, &s).await?;
            if before != level {
                audit::write(
                    c,
                    u,
                    audit::Entry::new(
                        g,
                        AuditAction::VerificationUpdate,
                        AuditTarget::Guild,
                        Some(g),
                    )
                    .change(
                        "verification level",
                        Some(level_label(before)),
                        Some(level_label(level)),
                    ),
                )
                .await?;
                out.extend(safety_events(c, g).await?);
            }
            Ok(CommandResult::Moderation(ModerationResult::Safety(s)))
        }
        PauseInvites {
            guild_id: g,
            seconds,
        }
        | PauseDms {
            guild_id: g,
            seconds,
        } => {
            let invites = matches!(cmd, PauseInvites { .. });
            guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
            require(
                seconds == 0 || (MIN_PAUSE_SECONDS..=MAX_PAUSE_SECONDS).contains(&seconds),
                ErrorCode::InvalidRequest,
            )?;
            let mut s = safety(c, g).await?;
            let until = (seconds > 0).then(|| now() + u64::from(seconds) * 1000);
            let slot = if invites {
                &mut s.invites_paused_until
            } else {
                &mut s.dms_paused_until
            };
            let was = slot.is_some();
            *slot = until;
            save_safety(c, &s).await?;
            if was || until.is_some() {
                let what = if invites {
                    "invites"
                } else {
                    "direct messages"
                };
                audit::write(
                    c,
                    u,
                    audit::Entry::new(
                        g,
                        if invites {
                            AuditAction::InvitesPause
                        } else {
                            AuditAction::DmsPause
                        },
                        AuditTarget::Guild,
                        Some(g),
                    )
                    .change(
                        what,
                        Some(if was { "paused" } else { "allowed" }.into()),
                        Some(match until {
                            Some(_) => format!("paused for {}", span(u64::from(seconds))),
                            None => "allowed".into(),
                        }),
                    ),
                )
                .await?;
                out.extend(safety_events(c, g).await?);
                core.moderation.rearm = true;
            }
            Ok(CommandResult::Moderation(ModerationResult::Safety(s)))
        }
        DismissRaidAlert { guild_id: g } => {
            role(c, u, g).await?;
            require(
                acl::evaluate(c, u, g, None).await?.intersects(MODERATOR),
                ErrorCode::Forbidden,
            )?;
            let mut s = safety(c, g).await?;
            if s.raid.take().is_some() {
                save_safety(c, &s).await?;
                out.extend(safety_events(c, g).await?);
            }
            Ok(CommandResult::Moderation(ModerationResult::Safety(s)))
        }
        CreateAutoModRule(r) => save_rule(c, core, u, r, true, out).await,
        UpdateAutoModRule(r) => save_rule(c, core, u, r, false, out).await,
        DeleteAutoModRule {
            guild_id: g,
            rule_id,
        } => {
            guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
            let existing = automod::load(c, g).await?;
            let r = existing
                .into_iter()
                .find(|x| x.id == rule_id)
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            sqlx::query("DELETE FROM automod_rules WHERE id=? AND guild_id=?")
                .bind(id(rule_id)?)
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
            let e = audit::Entry::new(
                g,
                AuditAction::AutomodRuleDelete,
                AuditTarget::AutomodRule,
                Some(rule_id),
            )
            .named(r.name.clone());
            audit::write(c, u, describe_rule(e, Some(&r), None)).await?;
            core.moderation.rules.remove(&g);
            out.push((
                crate::capacity::members_with(c, g, None, Permissions::MANAGE_GUILD).await?,
                Event::Moderation(ModerationEvent::AutoModUpdated { guild_id: g }),
            ));
            Ok(CommandResult::Ack)
        }
        SetAgeRestricted {
            channel_id: ch,
            age_restricted,
        } => {
            let channel = access(c, u, ch, Permissions::MANAGE_CHANNELS).await?;
            let g = channel
                .summary
                .guild_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            require(
                matches!(channel.summary.kind, ChannelKind::Text | ChannelKind::Voice),
                ErrorCode::InvalidRequest,
            )?;
            let was = age_restricted_flag(c, ch).await?;
            if was != age_restricted {
                if age_restricted {
                    sqlx::query("INSERT OR IGNORE INTO channel_age_restricted VALUES(?)")
                        .bind(id(ch)?)
                        .execute(&mut *c)
                        .await?;
                } else {
                    sqlx::query("DELETE FROM channel_age_restricted WHERE channel_id=?")
                        .bind(id(ch)?)
                        .execute(&mut *c)
                        .await?;
                }
                audit::write(
                    c,
                    u,
                    audit::Entry::new(
                        g,
                        AuditAction::ChannelUpdate,
                        AuditTarget::Channel,
                        Some(ch),
                    )
                    .named(channel.summary.name.clone().unwrap_or_default())
                    .change(
                        "age-restricted",
                        Some(yes(was)),
                        Some(yes(age_restricted)),
                    ),
                )
                .await?;
                out.push((
                    audience(c, ch).await?,
                    Event::Moderation(ModerationEvent::ChannelAgeRestricted {
                        channel_id: ch,
                        age_restricted,
                    }),
                ));
            }
            Ok(CommandResult::Ack)
        }
        ConfirmAge { channel_id: ch } => {
            access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            sqlx::query("INSERT OR IGNORE INTO age_confirmations VALUES(?,?,?)")
                .bind(id(u)?)
                .bind(id(ch)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            Ok(CommandResult::Ack)
        }
    }
}
async fn save_rule(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    r: AutoModRule,
    creating: bool,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let g = r.guild_id;
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let mut r = automod::validate(c, g, r).await?;
    let existing = automod::load(c, g).await?;
    let before = if creating {
        require(existing.len() < MAX_AUTOMOD_RULES, ErrorCode::LimitExceeded)?;
        r.id = core.id().await?;
        None
    } else {
        Some(
            existing
                .iter()
                .find(|x| x.id == r.id)
                .cloned()
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?,
        )
    };
    // One mention-spam rule and one spam rule per server.
    let singleton = |t: &AutoModTrigger| match t {
        AutoModTrigger::Keyword { .. } => None,
        AutoModTrigger::MentionSpam { .. } => Some(1),
        AutoModTrigger::Spam { .. } => Some(2),
    };
    if let Some(kind) = singleton(&r.trigger) {
        require(
            !existing
                .iter()
                .any(|x| x.id != r.id && singleton(&x.trigger) == Some(kind)),
            ErrorCode::AlreadyExists,
        )?;
    }
    let json = serde_json::to_string(&r).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO automod_rules(id,guild_id,rule) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET rule=excluded.rule")
        .bind(id(r.id)?)
        .bind(id(g)?)
        .bind(json)
        .execute(&mut *c)
        .await?;
    let mut e = audit::Entry::new(
        g,
        if creating {
            AuditAction::AutomodRuleCreate
        } else {
            AuditAction::AutomodRuleUpdate
        },
        AuditTarget::AutomodRule,
        Some(r.id),
    )
    .named(r.name.clone());
    e = describe_rule(e, before.as_ref(), Some(&r));
    audit::write(c, u, e).await?;
    core.moderation.rules.remove(&g);
    out.push((
        crate::capacity::members_with(c, g, None, Permissions::MANAGE_GUILD).await?,
        Event::Moderation(ModerationEvent::AutoModUpdated { guild_id: g }),
    ));
    Ok(CommandResult::Moderation(ModerationResult::AutoModRule(r)))
}
fn level_label(l: VerificationLevel) -> String {
    match l {
        VerificationLevel::None => "None",
        VerificationLevel::AccountAge => "Low (account older than 5 minutes)",
        VerificationLevel::MemberAge => "Medium (member for 10 minutes)",
    }
    .into()
}
fn yes(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}
fn describe_rule(
    mut e: audit::Entry,
    before: Option<&AutoModRule>,
    after: Option<&AutoModRule>,
) -> audit::Entry {
    let trigger = |r: &AutoModRule| match &r.trigger {
        AutoModTrigger::Keyword { keywords, allow } => {
            format!("{} keywords, {} allowed", keywords.len(), allow.len())
        }
        AutoModTrigger::MentionSpam { limit } => format!("more than {limit} mentions"),
        AutoModTrigger::Spam { duplicates, links } => {
            format!("{duplicates} repeats, more than {links} links")
        }
    };
    let actions = |r: &AutoModRule| {
        let a = &r.actions;
        let mut v = vec![];
        if a.block {
            v.push("block".to_string());
        }
        if a.alert_channel_id.is_some() {
            v.push("alert".into());
        }
        if a.timeout_seconds > 0 {
            v.push(format!("timeout {}", span(u64::from(a.timeout_seconds))));
        }
        v.join(", ")
    };
    e = e
        .change(
            "name",
            before.map(|r| r.name.clone()),
            after.map(|r| r.name.clone()),
        )
        .change(
            "enabled",
            before.map(|r| yes(r.enabled)),
            after.map(|r| yes(r.enabled)),
        )
        .change("trigger", before.map(trigger), after.map(trigger))
        .change("actions", before.map(actions), after.map(actions))
        .change(
            "exemptions",
            before.map(|r| {
                format!(
                    "{} roles, {} channels",
                    r.exempt_roles.len(),
                    r.exempt_channels.len()
                )
            }),
            after.map(|r| {
                format!(
                    "{} roles, {} channels",
                    r.exempt_roles.len(),
                    r.exempt_channels.len()
                )
            }),
        );
    e
}
async fn age_restricted_flag(c: &mut SqliteConnection, ch: Id) -> Result<bool> {
    Ok(
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channel_age_restricted WHERE channel_id=?)",
        )
        .bind(id(ch)?)
        .fetch_one(&mut *c)
        .await?,
    )
}
/// History of an age-restricted channel stays hidden until the viewer confirms
/// they are an adult (`ModerationCommand::ConfirmAge`).
pub(crate) async fn age_gate(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<()> {
    let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_age_restricted WHERE channel_id=?1) AND NOT EXISTS(SELECT 1 FROM age_confirmations WHERE channel_id=?1 AND user_id=?2)")
        .bind(id(ch)?)
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    if blocked {
        return Err(denied(
            "age_restricted",
            "This channel is age-restricted. Confirm you are an adult to see it.",
        ));
    }
    Ok(())
}

/// Expire timeouts, pauses and raid alerts that are due; return the next deadline.
pub(crate) async fn expire(app: &App) -> Result<Option<UnixMillis>> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let t = now() as i64;
    let mut out: Deliveries = vec![];
    let due = sqlx::query(
        "SELECT guild_id,user_id FROM member_timeouts WHERE until<=? ORDER BY until LIMIT 200",
    )
    .bind(t)
    .fetch_all(&mut *c)
    .await?;
    let more = due.len() == 200;
    let mut tx = c.begin().await?;
    for r in due {
        let (g, v) = (Id(r.get::<i64, _>(0) as u64), Id(r.get::<i64, _>(1) as u64));
        sqlx::query("DELETE FROM member_timeouts WHERE guild_id=? AND user_id=? AND until<=?")
            .bind(id(g)?)
            .bind(id(v)?)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        timeout_events(&mut tx, &mut out, g, v).await?;
    }
    let guilds: Vec<i64> = sqlx::query_scalar("SELECT guild_id FROM guild_safety WHERE invites_paused_until<=?1 OR dms_paused_until<=?1 OR raid_detected_at<=?2")
        .bind(t)
        .bind(t - RAID_ALERT_MS as i64)
        .fetch_all(&mut *tx)
        .await?;
    for g in guilds {
        sqlx::query("UPDATE guild_safety SET invites_paused_until=CASE WHEN invites_paused_until<=?1 THEN NULL ELSE invites_paused_until END,dms_paused_until=CASE WHEN dms_paused_until<=?1 THEN NULL ELSE dms_paused_until END,raid_detected_at=CASE WHEN raid_detected_at<=?2 THEN NULL ELSE raid_detected_at END WHERE guild_id=?3")
            .bind(t)
            .bind(t - RAID_ALERT_MS as i64)
            .bind(g)
            .execute(&mut *tx)
            .await?;
        out.extend(safety_events(&mut tx, Id(g as u64)).await?);
    }
    let next: Option<i64> = sqlx::query_scalar("SELECT MIN(x) FROM (SELECT MIN(until) AS x FROM member_timeouts UNION ALL SELECT MIN(invites_paused_until) FROM guild_safety UNION ALL SELECT MIN(dms_paused_until) FROM guild_safety UNION ALL SELECT MIN(raid_detected_at)+?1 FROM guild_safety)")
        .bind(RAID_ALERT_MS as i64)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    for (users, e) in out {
        core.emit(&users, e)?;
    }
    Ok(if more {
        Some(now())
    } else {
        next.map(|v| v.max(0) as u64)
    })
}
/// One task per server process, asleep until the next deadline or a new one.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = app.0.core.lock().await.moderation.wake.clone();
        loop {
            let next = match expire(&app).await {
                Ok(next) => next,
                Err(e) => {
                    tracing::warn!(error = ?e.0.code, "moderation expiry failed; retrying in a minute");
                    Some(now() + 60_000)
                }
            };
            let sleep = async {
                match next {
                    Some(at) => {
                        tokio::time::sleep(Duration::from_millis(at.saturating_sub(now()).max(1)))
                            .await
                    }
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = wake.notified() => {}
                _ = sleep => {}
            }
        }
    });
}

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/guilds/{id}/audit-log", get(audit::endpoint))
        .route("/api/v1/guilds/{id}/moderation/me", get(state_endpoint))
        .route("/api/v1/guilds/{id}/automod/rules", get(rules_endpoint))
        .route("/api/v1/guilds/{id}/prune", get(prune_endpoint))
        .route("/api/v1/channels/{id}/access", get(access_endpoint))
}
type Rest<'a> = (
    tokio::sync::MutexGuard<'a, Core>,
    sqlx::pool::PoolConnection<sqlx::Sqlite>,
    Id,
);
async fn rest<'a>(app: &'a App, h: &HeaderMap) -> Result<Rest<'a>> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((core, c, u))
}
pub(crate) async fn state(c: &mut SqliteConnection, u: Id, g: Id) -> Result<ModerationState> {
    role(c, u, g).await?;
    let mut safety = safety(c, g).await?;
    if !acl::evaluate(c, u, g, None).await?.intersects(MODERATOR) {
        safety.raid = None;
    }
    let restricted: HashSet<Id> = sqlx::query_scalar::<_, i64>("SELECT r.channel_id FROM channel_age_restricted r JOIN channels ch ON ch.id=r.channel_id WHERE ch.guild_id=?")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect();
    let slowed: HashMap<Id, u32> = sqlx::query("SELECT s.channel_id,s.seconds FROM channel_slowmode s JOIN channels ch ON ch.id=s.channel_id WHERE ch.guild_id=? AND s.seconds>0")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| (Id(r.get::<i64, _>(0) as u64), r.get::<i64, _>(1) as u32))
        .collect();
    let mut age_restricted = vec![];
    let mut slowmode = vec![];
    if !restricted.is_empty() || !slowed.is_empty() {
        for (ch, visible) in crate::capacity::guild_channels(c, u, g).await? {
            if visible && restricted.contains(&ch.id) {
                age_restricted.push(ch.id);
            }
            if let Some(seconds) = slowed.get(&ch.id).filter(|_| visible) {
                slowmode.push(ChannelSlowmode {
                    channel_id: ch.id,
                    seconds: *seconds,
                });
            }
        }
    }
    let confirmed: HashSet<Id> =
        sqlx::query_scalar::<_, i64>("SELECT channel_id FROM age_confirmations WHERE user_id=?")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|v| Id(v as u64))
            .collect();
    let age_confirmed = age_restricted
        .iter()
        .copied()
        .filter(|ch| confirmed.contains(ch))
        .collect();
    Ok(ModerationState {
        safety,
        timeout: timeout(c, g, u).await?,
        send_blocked_until: verification_wait(c, g, u).await?,
        age_restricted,
        age_confirmed,
        server_time: now(),
        slowmode,
    })
}
async fn state_endpoint(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(state(&mut c, u, Id(g)).await?))
}
async fn rules_endpoint(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    guild_perm(&mut c, u, Id(g), Permissions::MANAGE_GUILD).await?;
    Ok(http::json(automod::load(&mut c, Id(g)).await?))
}
#[derive(serde::Deserialize)]
struct PruneQuery {
    days: u32,
    #[serde(default)]
    roles: String,
}
async fn prune_endpoint(
    State(app): State<App>,
    Path(g): Path<u64>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let q: PruneQuery = serde_urlencoded::from_str(raw.as_deref().unwrap_or_default())
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let roles = q
        .roles
        .split(',')
        .filter(|s| !s.is_empty())
        .map(http::url_id)
        .collect::<Result<Vec<_>>>()?;
    let (core, mut c, u) = rest(&app, &h).await?;
    guild_perm(&mut c, u, Id(g), Permissions::KICK_MEMBERS).await?;
    let count = prune_candidates(&mut c, &core, Id(g), u, q.days, &roles)
        .await?
        .len() as u32;
    Ok(http::json(PrunePreview {
        days: q.days,
        count,
    }))
}
pub(crate) async fn channel_access(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
) -> Result<ChannelAccess> {
    let channel = access(c, u, ch, Permissions::MANAGE_ROLES).await?;
    let g = channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let viewers = crate::capacity::members_with(c, g, Some(ch), Permissions::VIEW_CHANNEL).await?;
    let members = guild_users(c, g).await?.len() as u32;
    let roles: Vec<Id> = acl::roles(c, g)
        .await?
        .into_iter()
        .map(|r| r.id)
        .filter(|r| *r != g)
        .collect();
    let mut sets = vec![vec![]];
    sets.extend(roles.iter().map(|r| vec![*r]));
    let can = crate::capacity::role_can_view(c, g, ch, &sets).await?;
    let everyone = can[0];
    let mut sample = vec![];
    for v in viewers.iter().take(8) {
        sample.push(user(c, *v).await?);
    }
    Ok(ChannelAccess {
        channel_id: ch,
        members,
        viewers: viewers.len() as u32,
        everyone,
        roles: roles
            .into_iter()
            .zip(can.into_iter().skip(1))
            .filter(|(_, v)| *v)
            .map(|(r, _)| r)
            .collect(),
        sample,
    })
}
async fn access_endpoint(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(channel_access(&mut c, u, Id(ch)).await?))
}

#[cfg(test)]
mod tests;
