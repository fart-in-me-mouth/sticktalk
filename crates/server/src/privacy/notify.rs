//! Notification controls and the one decision every notification path uses.
//!
//! Precedence for a channel: the server's setting, then each channel it is
//! nested in from the outside in, then the channel itself; the nearest
//! setting picks the level and the @everyone/@here and role-mention
//! suppression. A mute anywhere on that chain mutes the channel. Mobile push
//! is a server-wide switch. DMs and group DMs have their own setting.
//!
//! `decide` is used by the in-process notifications, Web Push and the app's
//! alerts (`observe`). Badges and the Inbox use `suppressed_mentions`
//! (written here when a mention is stored) and the ignore list, so a mention
//! that would not notify does not badge either.
use super::Deliveries;
use crate::{
    db::*,
    error::{require, Error, Result},
    state::{now, App, Core},
};
use sqlx::{Row, SqliteConnection};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use sticktalk_proto::*;

/// The settings in effect for one person in one conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Effective {
    pub level: NotificationLevel,
    pub muted: bool,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub mobile_push: bool,
}
fn active_mute(s: &NotificationSettings, t: u64) -> bool {
    s.muted && s.mute_until.is_none_or(|until| until > t)
}
/// Settings rows of `u` for these scopes, keyed by (scope, target).
async fn rows(
    c: &mut SqliteConnection,
    u: Id,
    guild: Option<Id>,
    channels: &[Id],
) -> Result<(
    BTreeMap<(&'static str, i64), NotificationSettings>,
    BTreeMap<(&'static str, i64), (bool, bool)>,
)> {
    let mut settings = BTreeMap::new();
    let mut extras = BTreeMap::new();
    let mut targets: Vec<(&'static str, i64)> =
        channels.iter().map(|c| ("channel", c.0 as i64)).collect();
    if let Some(g) = guild {
        targets.push(("guild", id(g)?));
    }
    for (scope, target) in targets {
        if let Some(json) = sqlx::query_scalar::<_, String>(
            "SELECT settings FROM notification_settings WHERE user_id=? AND scope=? AND target_id=?",
        )
        .bind(id(u)?)
        .bind(scope)
        .bind(target)
        .fetch_optional(&mut *c)
        .await?
        {
            if let Ok(s) = serde_json::from_str::<NotificationSettings>(&json) {
                settings.insert((scope, target), s);
            }
        }
        if let Some(r) = sqlx::query(
            "SELECT suppress_roles,mobile_push FROM notification_extras WHERE user_id=? AND scope=? AND target_id=?",
        )
        .bind(id(u)?)
        .bind(scope)
        .bind(target)
        .fetch_optional(&mut *c)
        .await?
        {
            extras.insert(
                (scope, target),
                (r.get::<i64, _>(0) != 0, r.get::<i64, _>(1) != 0),
            );
        }
    }
    Ok((settings, extras))
}
/// The chain of scopes for a conversation: its channels, outermost first.
pub(crate) async fn chain(c: &mut SqliteConnection, ch: &ChannelSummary) -> Result<Vec<Id>> {
    match ch.guild_id {
        Some(g) => crate::permissions::lineage(c, g, ch.id).await,
        None => Ok(vec![ch.id]),
    }
}
pub(crate) async fn effective_in(
    c: &mut SqliteConnection,
    u: Id,
    ch: &ChannelSummary,
    lineage: &[Id],
) -> Result<Effective> {
    let t = now();
    let (settings, extras) = rows(c, u, ch.guild_id, lineage).await?;
    let mut e = Effective {
        level: if ch.guild_id.is_some() {
            NotificationLevel::Mentions
        } else {
            NotificationLevel::All
        },
        muted: false,
        suppress_everyone: false,
        suppress_roles: false,
        mobile_push: true,
    };
    if let Some(g) = ch.guild_id {
        let key = ("guild", g.0 as i64);
        if let Some(s) = settings.get(&key) {
            e.level = s.level;
            e.suppress_everyone = s.suppress_everyone;
            e.muted |= active_mute(s, t);
        }
        if let Some((roles, push)) = extras.get(&key) {
            e.suppress_roles = *roles;
            e.mobile_push = *push;
        }
    }
    for scope in lineage {
        let key = ("channel", scope.0 as i64);
        if let Some(s) = settings.get(&key) {
            e.level = s.level;
            e.suppress_everyone = s.suppress_everyone;
            e.muted |= active_mute(s, t);
            if let Some((roles, _)) = extras.get(&key) {
                e.suppress_roles = *roles;
            } else {
                e.suppress_roles = false;
            }
        }
    }
    Ok(e)
}
pub(crate) async fn effective(
    c: &mut SqliteConnection,
    u: Id,
    ch: &ChannelSummary,
) -> Result<Effective> {
    let lineage = chain(c, ch).await?;
    effective_in(c, u, ch, &lineage).await
}

/// Whether and how `user` is told about `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub mentioned: bool,
    pub push: bool,
}
pub(crate) async fn suppressed(c: &mut SqliteConnection, m: Id, u: Id) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM suppressed_mentions WHERE message_id=? AND user_id=?)",
    )
    .bind(id(m)?)
    .bind(id(u)?)
    .fetch_one(c)
    .await?)
}
/// The single notification decision. None: say nothing. Checks the author
/// (blocked or ignored), a hidden message request, an unconfirmed
/// age-restricted channel, Do Not Disturb, mutes, the level and suppressed
/// mentions. Invisible people are notified like anyone else.
pub(crate) async fn decide(
    c: &mut SqliteConnection,
    u: Id,
    channel: &Channel,
    message: &Message,
) -> Result<Option<Decision>> {
    let author = message.author_id;
    if author == u || blocked(c, u, author).await? || super::people::ignores(c, u, author).await? {
        return Ok(None);
    }
    let ch = channel.summary.id;
    if channel.summary.guild_id.is_none() {
        if super::requests::hidden(c, ch).await?.contains(&u) {
            return Ok(None);
        }
    } else if super::gate::age_blocked(c, u, ch).await? {
        return Ok(None);
    }
    if me(c, u).await?.desired_presence == PresenceStatus::Dnd {
        return Ok(None);
    }
    let mut e = effective(c, u, &channel.summary).await?;
    // In a thread, members use their thread setting (Default keeps what the
    // parent channel gives); everyone else hears only about mentions.
    crate::threads::notify_level(c, u, channel, &[], &mut e.level, &mut e.muted).await?;
    let mut mentioned = message.mention_ids.contains(&u) && !suppressed(c, message.id, u).await?;
    if mentioned && (e.suppress_everyone || e.suppress_roles) {
        // The settings as they are now, for mentions made before they changed.
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT kind FROM message_broadcast_mentions WHERE message_id=? AND user_id=?",
        )
        .bind(id(message.id)?)
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?;
        mentioned = kinds.is_empty()
            || !kinds.iter().all(|k| match k.as_str() {
                "everyone" | "here" => e.suppress_everyone,
                "role" => e.suppress_roles,
                _ => false,
            });
    }
    if e.muted
        || e.level == NotificationLevel::Nothing
        || (e.level == NotificationLevel::Mentions && !mentioned)
    {
        return Ok(None);
    }
    Ok(Some(Decision {
        mentioned,
        push: e.mobile_push,
    }))
}

/// A notification that is not a message (messages use [`decide`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notice {
    /// A scheduled event the person is interested in is starting (`channel`:
    /// where it happens, if in a channel).
    EventStarting { guild: Id, channel: Option<Id> },
    /// A reminder they set on a message in `channel`.
    Reminder { channel: Id },
    /// A direct or group call in `channel` is ringing them.
    Ring { channel: Id, caller: Id },
}
/// The server-wide settings alone (an event with no channel).
async fn effective_guild(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Effective> {
    let (settings, extras) = rows(c, u, Some(g), &[]).await?;
    let mut e = Effective {
        level: NotificationLevel::Mentions,
        muted: false,
        suppress_everyone: false,
        suppress_roles: false,
        mobile_push: true,
    };
    let key = ("guild", g.0 as i64);
    if let Some(s) = settings.get(&key) {
        e.level = s.level;
        e.muted = active_mute(s, now());
    }
    if let Some((_, push)) = extras.get(&key) {
        e.mobile_push = *push;
    }
    Ok(e)
}
/// The single decision for notifications that are not messages, in the same
/// order as [`decide`]: who it is from (blocked or ignored), a hidden message
/// request or an unconfirmed age-restricted conversation, Do Not Disturb, then
/// the mute and level of the conversation (or of the server). A reminder
/// ignores mutes and levels: the person asked for it. `push` is the mobile
/// push switch of that server. Used by the event-start notice and its push,
/// reminder pushes and call-ring pushes. (Stage invitations to speak,
/// application decisions and report status are state shown where the person
/// looks, never an alert or a push; AutoMod alerts are messages and go
/// through [`decide`].)
pub(crate) async fn notice(c: &mut SqliteConnection, u: Id, n: Notice) -> Result<Option<Decision>> {
    if let Notice::Ring { caller, .. } = n {
        if caller == u
            || blocked(c, u, caller).await?
            || super::people::ignores(c, u, caller).await?
        {
            return Ok(None);
        }
    }
    let ch = match n {
        Notice::EventStarting { channel, .. } => channel,
        Notice::Reminder { channel } | Notice::Ring { channel, .. } => Some(channel),
    };
    let summary = match ch {
        Some(ch) => match crate::db::channel(c, ch).await {
            Ok(channel) => Some(channel.summary),
            Err(_) => return Ok(None),
        },
        None => None,
    };
    if let Some(s) = &summary {
        if s.guild_id.is_none() {
            if super::requests::hidden(c, s.id).await?.contains(&u) {
                return Ok(None);
            }
        } else if super::gate::age_blocked(c, u, s.id).await? {
            return Ok(None);
        }
    }
    if me(c, u).await?.desired_presence == PresenceStatus::Dnd {
        return Ok(None);
    }
    let e = match (&summary, n) {
        (Some(s), _) => effective(c, u, s).await?,
        (None, Notice::EventStarting { guild, .. }) => effective_guild(c, u, guild).await?,
        (None, _) => return Ok(None),
    };
    if !matches!(n, Notice::Reminder { .. }) && (e.muted || e.level == NotificationLevel::Nothing) {
        return Ok(None);
    }
    Ok(Some(Decision {
        mentioned: false,
        push: e.mobile_push,
    }))
}

/// A mention was stored (`mentions::persist`, on send and edit): record which
/// mentioned people must not count it: they ignore the author, or every way
/// they were mentioned (@everyone/@here, a role) is one they suppress.
pub(crate) async fn persist_suppressions(
    c: &mut SqliteConnection,
    m: Id,
    users: &[Id],
    origins: &[(Id, &'static str)],
) -> Result<()> {
    sqlx::query("DELETE FROM suppressed_mentions WHERE message_id=?")
        .bind(id(m)?)
        .execute(&mut *c)
        .await?;
    if users.is_empty() {
        return Ok(());
    }
    let r = sqlx::query("SELECT author_id,channel_id FROM messages WHERE id=?")
        .bind(id(m)?)
        .fetch_one(&mut *c)
        .await?;
    let author = Id(r.get::<i64, _>(0) as u64);
    let channel = crate::db::channel(c, Id(r.get::<i64, _>(1) as u64)).await?;
    let ignorers: HashSet<Id> =
        sqlx::query_scalar::<_, i64>("SELECT user_id FROM user_ignores WHERE ignored_id=?")
            .bind(id(author)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|v| Id(v as u64))
            .collect();
    let mut kinds: BTreeMap<Id, BTreeSet<&str>> = BTreeMap::new();
    for (u, kind) in origins {
        kinds.entry(*u).or_default().insert(kind);
    }
    let mut out: Vec<Id> = users
        .iter()
        .copied()
        .filter(|u| ignorers.contains(u))
        .collect();
    // Only people with a suppression setting somewhere need the full lookup.
    let candidates: Vec<Id> = kinds
        .keys()
        .copied()
        .filter(|u| !ignorers.contains(u))
        .collect();
    let mut flagged: HashSet<Id> = HashSet::new();
    for chunk in candidates.chunks(400) {
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT user_id FROM notification_settings WHERE json_extract(settings,'$.suppress_everyone')=1 AND user_id IN (");
        {
            let mut s = q.separated(",");
            for u in chunk {
                s.push_bind(id(*u)?);
            }
            s.push_unseparated(") UNION SELECT user_id FROM notification_extras WHERE suppress_roles=1 AND user_id IN (");
        }
        {
            let mut s = q.separated(",");
            for u in chunk {
                s.push_bind(id(*u)?);
            }
            s.push_unseparated(")");
        }
        for v in q.build_query_scalar::<i64>().fetch_all(&mut *c).await? {
            flagged.insert(Id(v as u64));
        }
    }
    if !flagged.is_empty() {
        let lineage = chain(c, &channel.summary).await?;
        for u in candidates.into_iter().filter(|u| flagged.contains(u)) {
            let e = effective_in(c, u, &channel.summary, &lineage).await?;
            let all = kinds.get(&u).is_some_and(|k| {
                k.iter().all(|kind| match *kind {
                    "everyone" | "here" => e.suppress_everyone,
                    "role" => e.suppress_roles,
                    _ => false,
                })
            });
            if all {
                out.push(u);
            }
        }
    }
    for u in out {
        sqlx::query("INSERT OR IGNORE INTO suppressed_mentions(message_id,user_id) VALUES(?,?)")
            .bind(id(m)?)
            .bind(id(u)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}

fn scope_of(s: &NotificationSettings) -> Result<(&'static str, Id)> {
    match (s.guild_id, s.channel_id) {
        (_, Some(ch)) => Ok(("channel", ch)),
        (Some(g), None) => Ok(("guild", g)),
        _ => Err(Error::new(ErrorCode::InvalidRequest)),
    }
}
pub(crate) async fn set(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    o: NotificationOptions,
    out: &mut Deliveries,
) -> Result<()> {
    let mut s = o.settings;
    let (scope, target) = scope_of(&s)?;
    if scope == "channel" {
        let ch = access(c, u, target, Permissions::VIEW_CHANNEL).await?;
        require(s.guild_id == ch.summary.guild_id, ErrorCode::InvalidRequest)?;
    } else {
        role(c, u, target).await?;
    }
    // A mute with a deadline ends by itself; without one it lasts until turned off.
    if !s.muted {
        s.mute_until = None;
    } else if let Some(t) = s.mute_until {
        require(t <= now() + MAX_MUTE_MS, ErrorCode::InvalidRequest)?;
        if t <= now() {
            s.muted = false;
            s.mute_until = None;
        }
    }
    let json = serde_json::to_string(&s).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO notification_settings VALUES(?,?,?,?) ON CONFLICT(user_id,scope,target_id) DO UPDATE SET settings=excluded.settings")
        .bind(id(u)?)
        .bind(scope)
        .bind(id(target)?)
        .bind(json)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO notification_extras(user_id,scope,target_id,suppress_roles,mobile_push,mute_until) VALUES(?,?,?,?,?,?) ON CONFLICT(user_id,scope,target_id) DO UPDATE SET suppress_roles=excluded.suppress_roles,mobile_push=excluded.mobile_push,mute_until=excluded.mute_until")
        .bind(id(u)?)
        .bind(scope)
        .bind(id(target)?)
        .bind(i64::from(o.suppress_roles))
        .bind(i64::from(o.mobile_push || scope == "channel"))
        .bind(s.mute_until.map(|t| t as i64))
        .execute(&mut *c)
        .await?;
    if s.mute_until.is_some() {
        core.privacy.wake.notify_one();
    }
    out.push((vec![u], Event::NotificationSettingsUpdated(s)));
    Ok(())
}
pub(crate) async fn extras(c: &mut SqliteConnection, u: Id) -> Result<Vec<NotificationExtra>> {
    let rows = sqlx::query("SELECT x.scope,x.target_id,x.suppress_roles,x.mobile_push,ch.guild_id FROM notification_extras x LEFT JOIN channels ch ON x.scope='channel' AND ch.id=x.target_id WHERE x.user_id=? ORDER BY x.scope,x.target_id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?;
    let mut out = vec![];
    for r in rows {
        let scope: String = r.get(0);
        let target = Id(r.get::<i64, _>(1) as u64);
        let (guild_id, channel_id) = if scope == "guild" {
            if role(c, u, target).await.is_err() {
                continue;
            }
            (Some(target), None)
        } else {
            if access(c, u, target, Permissions::VIEW_CHANNEL)
                .await
                .is_err()
            {
                continue;
            }
            (
                r.get::<Option<i64>, _>(4).map(|g| Id(g as u64)),
                Some(target),
            )
        };
        out.push(NotificationExtra {
            guild_id,
            channel_id,
            suppress_roles: r.get::<i64, _>(2) != 0,
            mobile_push: r.get::<i64, _>(3) != 0,
        });
    }
    Ok(out)
}
pub(crate) async fn global(c: &mut SqliteConnection, u: Id) -> Result<GlobalNotifications> {
    let json: Option<String> =
        sqlx::query_scalar("SELECT settings FROM global_notifications WHERE user_id=?")
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}
pub(crate) async fn set_global(
    c: &mut SqliteConnection,
    u: Id,
    g: GlobalNotifications,
) -> Result<()> {
    let json = serde_json::to_string(&g).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO global_notifications(user_id,settings) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET settings=excluded.settings")
        .bind(id(u)?)
        .bind(json)
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// Mutes whose time is up end by themselves. True when more remain.
pub(crate) async fn expire_mutes(c: &mut SqliteConnection, out: &mut Deliveries) -> Result<bool> {
    let t = now() as i64;
    let due = sqlx::query("SELECT user_id,scope,target_id FROM notification_extras WHERE mute_until<=? ORDER BY mute_until LIMIT 200")
        .bind(t)
        .fetch_all(&mut *c)
        .await?;
    let more = due.len() == 200;
    for r in due {
        let (u, scope, target): (i64, String, i64) = (r.get(0), r.get(1), r.get(2));
        sqlx::query("UPDATE notification_extras SET mute_until=NULL WHERE user_id=? AND scope=? AND target_id=?")
            .bind(u)
            .bind(&scope)
            .bind(target)
            .execute(&mut *c)
            .await?;
        let json: Option<String> = sqlx::query_scalar(
            "SELECT settings FROM notification_settings WHERE user_id=? AND scope=? AND target_id=?",
        )
        .bind(u)
        .bind(&scope)
        .bind(target)
        .fetch_optional(&mut *c)
        .await?;
        let Some(mut s) = json.and_then(|j| serde_json::from_str::<NotificationSettings>(&j).ok())
        else {
            continue;
        };
        if !s.muted || s.mute_until.is_none_or(|until| until as i64 > t) {
            continue;
        }
        s.muted = false;
        s.mute_until = None;
        sqlx::query("UPDATE notification_settings SET settings=? WHERE user_id=? AND scope=? AND target_id=?")
            .bind(serde_json::to_string(&s).map_err(|_| Error::new(ErrorCode::Internal))?)
            .bind(u)
            .bind(&scope)
            .bind(target)
            .execute(&mut *c)
            .await?;
        out.push((vec![Id(u as u64)], Event::NotificationSettingsUpdated(s)));
    }
    Ok(more)
}
pub(crate) async fn next_mute(c: &mut SqliteConnection) -> Result<Option<u64>> {
    Ok(sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MIN(mute_until) FROM notification_extras WHERE mute_until IS NOT NULL",
    )
    .fetch_one(c)
    .await?
    .map(|v| v.max(0) as u64))
}
/// Muted conversations stay out of the Inbox's unread list.
pub(crate) async fn inbox_hidden(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<bool> {
    let channel = crate::db::channel(c, ch).await?;
    if channel.summary.guild_id.is_some() && super::gate::age_blocked(c, u, ch).await? {
        return Ok(true);
    }
    Ok(effective(c, u, &channel.summary).await?.muted)
}

fn alert_streams(core: &Core, u: Id) -> bool {
    core.streams
        .values()
        .any(|s| s.user == u && s.connection.is_some() && core.privacy.subscribed.contains(&s.id))
}
/// Alerts for the app (sound and desktop notification), after a command
/// committed. Only people with a live, subscribed connection are considered.
pub(crate) async fn observe(
    _app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    e: &Event,
) -> Result<()> {
    match e {
        Event::MessageCreated { message, .. } => {
            let mut candidates: Vec<Id> = core
                .streams
                .values()
                .filter(|s| {
                    s.user != message.author_id
                        && s.connection.is_some()
                        && core.privacy.subscribed.contains(&s.id)
                })
                .map(|s| s.user)
                .collect();
            candidates.sort();
            candidates.dedup();
            if candidates.is_empty() {
                return Ok(());
            }
            let channel = crate::db::channel(c, message.channel_id).await?;
            let audience: HashSet<Id> =
                audience(c, message.channel_id).await?.into_iter().collect();
            let direct = channel.summary.guild_id.is_none();
            for u in candidates.into_iter().filter(|u| audience.contains(u)) {
                if let Some(d) = decide(c, u, &channel, message).await? {
                    let kind = if direct {
                        AlertKind::Direct
                    } else if d.mentioned {
                        AlertKind::Mention
                    } else {
                        AlertKind::Message
                    };
                    core.emit(
                        &[u],
                        Event::Privacy(PrivacyEvent::Alert(Alert {
                            kind,
                            channel_id: message.channel_id,
                            message_id: message.id,
                            user_id: message.author_id,
                            emoji: None,
                            at: now(),
                        })),
                    )?;
                }
            }
        }
        Event::ReactionUpdated(r) if r.added => {
            let author: Option<i64> = sqlx::query_scalar(
                "SELECT author_id FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL",
            )
            .bind(id(r.message_id)?)
            .bind(id(r.channel_id)?)
            .fetch_optional(&mut *c)
            .await?;
            let Some(author) = author.map(|v| Id(v as u64)) else {
                return Ok(());
            };
            if author == r.user_id || !alert_streams(core, author) {
                return Ok(());
            }
            let prefs = global(c, author).await?;
            let channel = match text(
                c,
                author,
                r.channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
            )
            .await
            {
                Ok(ch) => ch,
                Err(_) => return Ok(()),
            };
            let direct = channel.summary.guild_id.is_none();
            let wanted = match prefs.reactions {
                ReactionAlerts::Never => false,
                ReactionAlerts::Direct => direct,
                ReactionAlerts::All => true,
            };
            if !wanted
                || blocked(c, author, r.user_id).await?
                || super::people::ignores(c, author, r.user_id).await?
                || me(c, author).await?.desired_presence == PresenceStatus::Dnd
                || effective(c, author, &channel.summary).await?.muted
            {
                return Ok(());
            }
            core.emit(
                &[author],
                Event::Privacy(PrivacyEvent::Alert(Alert {
                    kind: AlertKind::Reaction,
                    channel_id: r.channel_id,
                    message_id: r.message_id,
                    user_id: r.user_id,
                    emoji: Some(r.reaction.emoji.clone()),
                    at: now(),
                })),
            )?;
        }
        _ => {}
    }
    Ok(())
}
