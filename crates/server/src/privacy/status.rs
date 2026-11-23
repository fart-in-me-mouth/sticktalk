//! The status a person chose (optionally for a while), their custom status
//! (text, one emoji, an optional clearing time) and the automatic idle.
//!
//! The automatic idle is driven by the `Focus` messages every connection
//! already sends from window focus and visibility events: when none of a
//! person's windows has been focused for `AUTO_IDLE_MS`, Online shows as Idle
//! until one is focused again. A manual choice is never changed by it. No
//! client timer runs; the deadline task wakes once, at the deadline.
use super::Deliveries;
use crate::{
    db::*,
    error::{require, Result},
    state::{now, App, Core},
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
use unicode_segmentation::UnicodeSegmentation;
const MAX_TRACKED: usize = 8192;
const BATCH: i64 = 100;

pub(crate) async fn choice(c: &mut SqliteConnection, u: Id) -> Result<StatusChoice> {
    let r = sqlx::query("SELECT desired_presence,presence_until FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(StatusChoice {
        status: en(r.get("desired_presence"))?,
        until: r
            .get::<Option<i64>, _>("presence_until")
            .map(|v| v.max(0) as u64),
    })
}
pub(crate) async fn custom(c: &mut SqliteConnection, u: Id) -> Result<CustomStatus> {
    let r = sqlx::query("SELECT status_text,status_emoji,status_clear_at FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(CustomStatus {
        text: r.get("status_text"),
        emoji: r.get("status_emoji"),
        clear_at: r
            .get::<Option<i64>, _>("status_clear_at")
            .map(|v| v.max(0) as u64),
    })
}

/// What others see: Online becomes Idle while the automatic idle is on.
pub(crate) fn shown(core: &Core, u: Id, status: PresenceStatus) -> PresenceStatus {
    if status == PresenceStatus::Online && core.privacy.idle.contains(&u) {
        PresenceStatus::Idle
    } else {
        status
    }
}

pub(crate) async fn set(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    status: PresenceStatus,
    until: Option<UnixMillis>,
    out: &mut Deliveries,
) -> Result<()> {
    require(status != PresenceStatus::Offline, ErrorCode::InvalidRequest)?;
    if let Some(t) = until {
        // Online is the state a timed status returns to; it has no end itself.
        require(
            status != PresenceStatus::Online
                && t > now()
                && t <= now() + MAX_STATUS_DURATION_MS + 60_000,
            ErrorCode::InvalidRequest,
        )?;
    }
    let before = choice(c, u).await?;
    sqlx::query("UPDATE users SET desired_presence=?,presence_until=? WHERE id=?")
        .bind(enum_str(status))
        .bind(until.map(|t| t as i64))
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    if before.status != status {
        out.push((vec![u], Event::SelfUpdated(me(c, u).await?)));
    }
    if until.is_some() {
        core.privacy.wake.notify_one();
    }
    Ok(())
}

/// One emoji: a single Unicode grapheme that is not plain text, or a custom
/// emoji reference `<:name:id>` / `<a:name:id>`.
pub(crate) fn valid_emoji(e: &str) -> bool {
    if e.is_empty() || e.chars().count() > MAX_STATUS_EMOJI_CHARS {
        return false;
    }
    if let Some(inner) = e
        .strip_prefix("<a:")
        .or_else(|| e.strip_prefix("<:"))
        .and_then(|v| v.strip_suffix('>'))
    {
        let Some((name, number)) = inner.split_once(':') else {
            return false;
        };
        return (2..=32).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && (1..=20).contains(&number.len())
            && number.bytes().all(|b| b.is_ascii_digit());
    }
    e.graphemes(true).count() == 1
        && !e
            .chars()
            .any(|c| c.is_ascii() || c.is_control() || c.is_whitespace())
}
fn clean_text(text: Option<String>) -> Result<Option<String>> {
    let Some(t) = text else { return Ok(None) };
    let t = t.trim();
    if t.is_empty() {
        return Ok(None);
    }
    require(
        t.chars().count() <= MAX_STATUS_TEXT_CHARS && !t.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(Some(t.to_owned()))
}

pub(crate) async fn set_custom(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    custom: CustomStatus,
    out: &mut Deliveries,
) -> Result<()> {
    let text = clean_text(custom.text)?;
    let emoji = custom
        .emoji
        .map(|e| e.trim().to_owned())
        .filter(|e| !e.is_empty());
    if let Some(e) = &emoji {
        require(valid_emoji(e), ErrorCode::InvalidRequest)?;
    }
    let clear_at = if text.is_none() && emoji.is_none() {
        None
    } else {
        custom.clear_at
    };
    if let Some(t) = clear_at {
        require(
            t > now() && t <= now() + MAX_STATUS_CLEAR_MS,
            ErrorCode::InvalidRequest,
        )?;
    }
    let before = self::custom(c, u).await?;
    sqlx::query("UPDATE users SET status_text=?,status_emoji=?,status_clear_at=? WHERE id=?")
        .bind(&text)
        .bind(&emoji)
        .bind(clear_at.map(|t| t as i64))
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    if before.text != text || before.emoji != emoji {
        fan_out(c, core, u, out).await?;
    }
    if clear_at.is_some() {
        core.privacy.wake.notify_one();
    }
    Ok(())
}

/// The person's summary changed: tell them, everyone who may see their
/// profile, and the servers they are in (like a profile edit).
pub(crate) async fn fan_out(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    out.push((vec![u], Event::SelfUpdated(me(c, u).await?)));
    let mut viewers = vec![];
    let mut online: Vec<Id> = core.streams.values().map(|s| s.user).collect();
    online.sort();
    online.dedup();
    for viewer in online {
        if profile_access(c, viewer, u).await.is_ok() {
            viewers.push(viewer);
        }
    }
    out.push((viewers, Event::UserUpdated(user(c, u).await?)));
    for g in sqlx::query_scalar::<_, i64>("SELECT guild_id FROM guild_members WHERE user_id=?")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
    {
        let g = Id(g as u64);
        out.push((
            guild_users(c, g).await?,
            Event::MemberUpsert(member(c, g, u).await?),
        ));
    }
    Ok(())
}

/// Timed statuses and custom statuses that are due. True when more remain.
pub(crate) async fn expire(
    c: &mut SqliteConnection,
    core: &mut Core,
    out: &mut Deliveries,
    presence: &mut Vec<Id>,
) -> Result<bool> {
    let t = now() as i64;
    let due: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE presence_until<=? ORDER BY presence_until LIMIT ?",
    )
    .bind(t)
    .bind(BATCH)
    .fetch_all(&mut *c)
    .await?;
    let more = due.len() as i64 == BATCH;
    for u in due.into_iter().map(|v| Id(v as u64)) {
        sqlx::query(
            "UPDATE users SET desired_presence='online',presence_until=NULL WHERE id=? AND presence_until<=?",
        )
        .bind(id(u)?)
        .bind(t)
        .execute(&mut *c)
        .await?;
        out.push((vec![u], Event::SelfUpdated(me(c, u).await?)));
        super::settings_event(c, core, out, u).await?;
        presence.push(u);
    }
    let due: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE status_clear_at<=? ORDER BY status_clear_at LIMIT ?",
    )
    .bind(t)
    .bind(BATCH)
    .fetch_all(&mut *c)
    .await?;
    let more = more || due.len() as i64 == BATCH;
    for u in due.into_iter().map(|v| Id(v as u64)) {
        sqlx::query("UPDATE users SET status_text=NULL,status_emoji=NULL,status_clear_at=NULL WHERE id=? AND status_clear_at<=?")
            .bind(id(u)?)
            .bind(t)
            .execute(&mut *c)
            .await?;
        fan_out(c, core, u, out).await?;
        super::settings_event(c, core, out, u).await?;
    }
    Ok(more)
}

fn live(core: &Core, u: Id) -> (bool, bool) {
    let mut any = false;
    let mut focused = false;
    for s in core.streams.values().filter(|s| s.user == u) {
        if let Some(c) = &s.connection {
            any = true;
            focused |= c.focused;
        }
    }
    (any, focused)
}
/// Users whose windows have all been unfocused long enough become Idle.
pub(crate) fn expire_idle(core: &mut Core, presence: &mut Vec<Id>) {
    let t = now();
    let tracked: Vec<(Id, u64)> = core
        .privacy
        .unfocused
        .iter()
        .map(|(u, at)| (*u, *at))
        .collect();
    for (u, since) in tracked {
        let (any, focused) = live(core, u);
        if !any || focused {
            core.privacy.unfocused.remove(&u);
            if core.privacy.idle.remove(&u) && any {
                presence.push(u);
            }
            continue;
        }
        if since + AUTO_IDLE_MS <= t && core.privacy.idle.insert(u) {
            presence.push(u);
        }
    }
    core.privacy
        .idle
        .retain(|u| core.privacy.unfocused.contains_key(u));
}
/// The nearest deadline: a timed status, a custom status, or an automatic idle.
pub(crate) async fn next(c: &mut SqliteConnection, core: &Core) -> Result<Option<u64>> {
    let stored: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(x) FROM (SELECT MIN(presence_until) AS x FROM users UNION ALL SELECT MIN(status_clear_at) FROM users)",
    )
    .fetch_one(&mut *c)
    .await?;
    let idle = core
        .privacy
        .unfocused
        .iter()
        .filter(|(u, _)| !core.privacy.idle.contains(u))
        .map(|(_, at)| at + AUTO_IDLE_MS)
        .min();
    Ok(match (stored.map(|v| v.max(0) as u64), idle) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
}

/// A connection attached: it starts unfocused until its window reports focus.
pub(crate) fn connected(core: &mut Core, u: Id) {
    let (_, focused) = live(core, u);
    if !focused
        && !core.privacy.unfocused.contains_key(&u)
        && core.privacy.unfocused.len() < MAX_TRACKED
    {
        core.privacy.unfocused.insert(u, now());
        core.privacy.wake.notify_one();
    }
}
/// A connection reported focus or blur (`ClientMsg::Focus`).
pub(crate) async fn focus(app: &App, core: &mut Core, u: Id) {
    let (any, focused) = live(core, u);
    if !any {
        core.privacy.unfocused.remove(&u);
        core.privacy.idle.remove(&u);
        return;
    }
    if focused {
        core.privacy.unfocused.remove(&u);
        if core.privacy.idle.remove(&u) {
            if let Ok(mut c) = app.0.pool.acquire().await {
                let _ = crate::commands::broadcast_presence(&mut c, core, u).await;
                let mut out = vec![];
                if super::settings_event(&mut c, core, &mut out, u)
                    .await
                    .is_ok()
                {
                    let _ = super::deliver(core, &mut c, out).await;
                }
            }
        }
    } else if !core.privacy.unfocused.contains_key(&u) && core.privacy.unfocused.len() < MAX_TRACKED
    {
        core.privacy.unfocused.insert(u, now());
        core.privacy.wake.notify_one();
    }
}
