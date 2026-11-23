//! Scheduled server events (`docs/features/stage.md`, "Scheduled events").
//! Wire contracts: `sticktalk_proto::schedule`.
//!
//! The model: one `scheduled_events` row per series, with a rule computed in
//! the creator's time zone (`recur.rs`). "Edit this event" stores an
//! exception for one occurrence; "this and following" ends the series there
//! (`until_at`) and starts a new one with the changes.
//!
//! Timing: each series keeps `next_at`, the next time anything is due for it
//! (its next start announcement, an external event's end, an unstarted
//! occurrence lapsing, its deletion after it ended). One task sleeps until the
//! earliest `next_at` (an indexed MIN), handles the rows that are due and
//! re-arms; it is woken when a command sets an earlier deadline. With no
//! events it waits with no deadline.
mod recur;
mod routes;
#[cfg(test)]
mod tests;

pub(crate) use routes::routes;

use crate::{
    db::{self, id},
    error::{require, Error, Result},
    state::{now, App, Core},
};
use recur::Rule;
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection};
use std::sync::Arc;
use sticktalk_proto::*;

pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

/// Exceptions ("edit this event") one series may hold.
const MAX_EXCEPTIONS: i64 = 100;
/// A Stage or voice event goes live with its channel's session from this
/// long before its start.
const EARLY_START_MS: u64 = 30 * 60_000;
/// How long a live event may stay live at most, whatever happens.
const MAX_LIVE_MS: u64 = MAX_EVENT_DURATION_MS + EVENT_LAPSE_MS;

#[derive(Default)]
pub(crate) struct Schedule {
    wake: Arc<tokio::sync::Notify>,
}
impl Schedule {
    /// A deadline may have moved earlier: the timer looks again.
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }
}

/// One series, as stored.
#[derive(Clone)]
pub(crate) struct Series {
    pub id: Id,
    pub guild: Id,
    pub creator: Id,
    pub name: String,
    pub description: String,
    pub location: EventLocation,
    pub rule: Rule,
    pub time_zone: String,
    pub duration: Option<u64>,
    pub status: EventStatus,
    pub live: Option<u64>,
    pub live_since: Option<u64>,
    pub notified_through: u64,
    pub done_through: u64,
    pub ended_at: Option<u64>,
    pub cover: u32,
    pub interested: u32,
}

/// One occurrence with its exception applied.
#[derive(Clone)]
pub(crate) struct Occurrence {
    pub key: u64,
    pub name: String,
    pub description: String,
    pub start: u64,
    pub end: Option<u64>,
    pub location: EventLocation,
    pub cancelled: bool,
}

fn location_of(kind: &str, channel: Option<i64>, place: Option<String>) -> Option<EventLocation> {
    match kind {
        "stage" => channel.map(|c| EventLocation::Stage {
            channel_id: Id(c as u64),
        }),
        "voice" => channel.map(|c| EventLocation::Voice {
            channel_id: Id(c as u64),
        }),
        _ => place.map(|place| EventLocation::External { place }),
    }
}
fn location_columns(l: &EventLocation) -> (&'static str, Option<i64>, Option<&str>) {
    match l {
        EventLocation::Stage { channel_id } => ("stage", Some(channel_id.0 as i64), None),
        EventLocation::Voice { channel_id } => ("voice", Some(channel_id.0 as i64), None),
        EventLocation::External { place } => ("external", None, Some(place.as_str())),
    }
}
fn status_of(s: &str) -> EventStatus {
    match s {
        "live" => EventStatus::Live,
        "ended" => EventStatus::Ended,
        "cancelled" => EventStatus::Cancelled,
        _ => EventStatus::Scheduled,
    }
}
fn series_row(r: &SqliteRow) -> Result<Series> {
    let recurrence: EventRecurrence = db::en(r.get("recurrence"))?;
    let time_zone: String = r.get("time_zone");
    let rule = Rule::new(
        &r.get::<String, _>("start_local"),
        &time_zone,
        recurrence,
        r.get::<Option<i64>, _>("until_at").map(|v| v as u64),
    )
    .map_err(|_| Error::new(ErrorCode::Internal))?;
    Ok(Series {
        id: Id(r.get::<i64, _>("id") as u64),
        guild: Id(r.get::<i64, _>("guild_id") as u64),
        creator: Id(r.get::<i64, _>("creator_id") as u64),
        name: r.get("name"),
        description: r.get("description"),
        location: location_of(
            &r.get::<String, _>("location_kind"),
            r.get("channel_id"),
            r.get("place"),
        )
        .ok_or_else(|| Error::new(ErrorCode::Internal))?,
        rule,
        time_zone,
        duration: r.get::<Option<i64>, _>("duration_ms").map(|v| v as u64),
        status: status_of(&r.get::<String, _>("status")),
        live: r.get::<Option<i64>, _>("live_occurrence").map(|v| v as u64),
        live_since: r.get::<Option<i64>, _>("live_since").map(|v| v as u64),
        notified_through: r.get::<i64, _>("notified_through").max(0) as u64,
        done_through: r.get::<i64, _>("done_through").max(0) as u64,
        ended_at: r.get::<Option<i64>, _>("ended_at").map(|v| v as u64),
        cover: r.get::<i64, _>("cover_version").clamp(0, u32::MAX as i64) as u32,
        interested: r
            .get::<i64, _>("interested_count")
            .clamp(0, u32::MAX as i64) as u32,
    })
}
pub(crate) async fn load(c: &mut SqliteConnection, e: Id) -> Result<Series> {
    let r = sqlx::query("SELECT * FROM scheduled_events WHERE id=?")
        .bind(id(e)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    series_row(&r)
}

impl Series {
    /// Occurrence `key` with its exception (if any).
    pub(crate) async fn occurrence(
        &self,
        c: &mut SqliteConnection,
        key: u64,
    ) -> Result<Occurrence> {
        let mut o = Occurrence {
            key,
            name: self.name.clone(),
            description: self.description.clone(),
            start: key,
            end: self.duration.map(|d| key.saturating_add(d)),
            location: self.location.clone(),
            cancelled: self.status == EventStatus::Cancelled,
        };
        if let Some(r) =
            sqlx::query("SELECT * FROM event_exceptions WHERE event_id=? AND occurrence=?")
                .bind(id(self.id)?)
                .bind(key as i64)
                .fetch_optional(&mut *c)
                .await?
        {
            o.cancelled |= r.get::<i64, _>("cancelled") != 0;
            if let Some(v) = r.get::<Option<String>, _>("name") {
                o.name = v;
            }
            if let Some(v) = r.get::<Option<String>, _>("description") {
                o.description = v;
            }
            if let Some(v) = r.get::<Option<i64>, _>("start_at") {
                let moved = v as u64;
                o.end = o
                    .end
                    .map(|e| e.saturating_sub(o.start).saturating_add(moved));
                o.start = moved;
            }
            if let Some(v) = r.get::<Option<i64>, _>("end_at") {
                o.end = Some(v as u64);
            }
            if r.get::<i64, _>("end_cleared") != 0 {
                o.end = None;
            }
            if let Some(kind) = r.get::<Option<String>, _>("location_kind") {
                if let Some(l) = location_of(&kind, r.get("channel_id"), r.get("place")) {
                    o.location = l;
                }
            }
        }
        Ok(o)
    }
    /// The occurrence the event is about now: the live one, else the next
    /// one that has not lapsed or been cancelled. None: the series is over.
    pub(crate) async fn current(
        &self,
        c: &mut SqliteConnection,
        now: u64,
    ) -> Result<Option<Occurrence>> {
        if let Some(live) = self.live {
            return Ok(Some(self.occurrence(c, live).await?));
        }
        if matches!(self.status, EventStatus::Ended | EventStatus::Cancelled) {
            return Ok(None);
        }
        let mut from = now.saturating_sub(MAX_LIVE_MS);
        for _ in 0..64 {
            let Some((_, key)) = self.rule.next_from(from) else {
                return Ok(None);
            };
            let o = self.occurrence(c, key).await?;
            if !o.cancelled && now < lapse(&o) && key > self.done_through {
                return Ok(Some(o));
            }
            from = key + 1;
        }
        Ok(None)
    }
    pub(crate) fn wire(&self, o: &Occurrence) -> ScheduledEvent {
        ScheduledEvent {
            id: self.id,
            guild_id: self.guild,
            creator_id: self.creator,
            name: o.name.clone(),
            description: o.description.clone(),
            cover: (self.cover > 0).then_some(self.cover),
            occurrence: o.key,
            start_at: o.start,
            end_at: o.end,
            location: o.location.clone(),
            status: if self.live == Some(o.key) {
                EventStatus::Live
            } else if o.cancelled {
                EventStatus::Cancelled
            } else if self.status == EventStatus::Ended {
                EventStatus::Ended
            } else {
                EventStatus::Scheduled
            },
            recurrence: self.rule.recurrence(),
            time_zone: self.time_zone.clone(),
            interested: self.interested,
        }
    }
    /// What the wire shows for the event now (its current occurrence, or
    /// the last one when it is over).
    pub(crate) async fn view(&self, c: &mut SqliteConnection, now: u64) -> Result<ScheduledEvent> {
        match self.current(c, now).await? {
            Some(o) => Ok(self.wire(&o)),
            None => {
                let last = self
                    .done_through
                    .max(self.notified_through)
                    .max(self.rule.first_at);
                let o = self.occurrence(c, last).await?;
                let mut e = self.wire(&o);
                if e.status == EventStatus::Scheduled {
                    e.status = EventStatus::Ended;
                }
                Ok(e)
            }
        }
    }
}

/// When an occurrence that never went live stops being "the next one".
fn lapse(o: &Occurrence) -> u64 {
    match o.location {
        EventLocation::External { .. } => o.end.unwrap_or(o.start).max(o.start),
        _ => o.end.unwrap_or(o.start).saturating_add(EVENT_LAPSE_MS),
    }
}

// ---- permissions --------------------------------------------------------------

/// Whether `u` may see `s` at all: a member, and able to see its channel.
pub(crate) async fn visible(c: &mut SqliteConnection, u: Id, s: &Series) -> Result<()> {
    db::role(c, u, s.guild)
        .await
        .map_err(|_| Error::new(ErrorCode::NotFound))?;
    if let Some(ch) = s.location.channel() {
        db::access(c, u, ch, Permissions::VIEW_CHANNEL)
            .await
            .map_err(|_| Error::new(ErrorCode::NotFound))?;
    }
    Ok(())
}
async fn guild_rights(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Permissions> {
    db::role(c, u, g).await?;
    crate::permissions::evaluate(c, u, g, None).await
}
/// Manage Events, or Create Events for the creator's own.
pub(crate) async fn can_manage(c: &mut SqliteConnection, u: Id, s: &Series) -> Result<bool> {
    let p = guild_rights(c, u, s.guild).await?;
    Ok(p.contains(Permissions::MANAGE_EVENTS)
        || (s.creator == u && p.contains(Permissions::CREATE_EVENTS)))
}
async fn manage(c: &mut SqliteConnection, u: Id, s: &Series) -> Result<()> {
    visible(c, u, s).await?;
    require(can_manage(c, u, s).await?, ErrorCode::Forbidden)
}

// ---- validation -----------------------------------------------------------------

fn text(s: &str, max: usize, empty: bool) -> Result<String> {
    let s = s.trim();
    require(
        (empty || !s.is_empty())
            && s.chars().count() <= max
            && !s.chars().any(|c| c.is_control() && c != '\n'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
/// A location the creator can use: a Stage or voice channel of this server
/// they can see and connect to, or a short free-text place.
async fn location(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    l: EventLocation,
) -> Result<EventLocation> {
    match l {
        EventLocation::External { place } => Ok(EventLocation::External {
            place: text(&place, MAX_EVENT_PLACE_CHARS, false)?,
        }),
        EventLocation::Stage { channel_id } | EventLocation::Voice { channel_id } => {
            let ch = db::access(
                c,
                u,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::CONNECT,
            )
            .await?;
            require(ch.summary.guild_id == Some(g), ErrorCode::InvalidRequest)?;
            match (&l, ch.summary.kind) {
                (EventLocation::Stage { .. }, ChannelKind::Stage)
                | (EventLocation::Voice { .. }, ChannelKind::Voice) => Ok(l),
                _ => Err(Error::new(ErrorCode::InvalidRequest)),
            }
        }
    }
}
/// Start and end times: in the future (for new times), the end after the
/// start, at most `MAX_EVENT_DURATION_MS` long; external events need an end.
fn times(start: u64, end: Option<u64>, l: &EventLocation, future: bool) -> Result<Option<u64>> {
    let now = now();
    require(
        (!future || start + 60_000 >= now) && start <= now + MAX_EVENT_LEAD_MS,
        ErrorCode::InvalidRequest,
    )?;
    if matches!(l, EventLocation::External { .. }) {
        require(end.is_some(), ErrorCode::InvalidRequest)?;
    }
    if let Some(end) = end {
        require(
            end > start && end - start <= MAX_EVENT_DURATION_MS,
            ErrorCode::InvalidRequest,
        )?;
    }
    Ok(end.map(|e| e - start))
}

// ---- deliveries -------------------------------------------------------------------

/// Members who can see the event.
async fn audience(c: &mut SqliteConnection, s: &Series) -> Result<Vec<Id>> {
    match s.location.channel() {
        Some(ch) => db::audience(c, ch).await,
        None => db::guild_users(c, s.guild).await,
    }
}
/// A server's events that are scheduled or live, oldest first.
async fn active(c: &mut SqliteConnection, g: Id) -> Result<Vec<Series>> {
    sqlx::query(
        "SELECT * FROM scheduled_events WHERE guild_id=? AND status IN ('scheduled','live') ORDER BY id",
    )
    .bind(id(g)?)
    .fetch_all(&mut *c)
    .await?
    .iter()
    .map(series_row)
    .collect()
}
/// What the channel list needs for one server, as `u` sees it.
pub(crate) async fn summary(c: &mut SqliteConnection, u: Id, g: Id) -> Result<GuildEvents> {
    let now = now();
    let rows = active(c, g).await?;
    let mut out = GuildEvents {
        guild_id: g,
        count: 0,
        live: vec![],
    };
    for s in rows {
        if visible(c, u, &s).await.is_err() || s.current(c, now).await?.is_none() {
            continue;
        }
        out.count += 1;
        if s.live.is_some() {
            out.live.push(s.id);
        }
    }
    Ok(out)
}
/// The event changed: its next occurrence to everyone who can see it, and
/// each one's server summary.
async fn changed(c: &mut SqliteConnection, s: &Series, out: &mut Deliveries) -> Result<()> {
    let users = audience(c, s).await?;
    let view = s.view(c, now()).await?;
    out.push((users.clone(), Event::Schedule(ScheduleUpdate::Upsert(view))));
    summaries(c, s.guild, users, out).await
}
/// Each user's summary of one server, with one batched audience per event
/// (not one permission check per user and event).
async fn summaries(
    c: &mut SqliteConnection,
    g: Id,
    users: Vec<Id>,
    out: &mut Deliveries,
) -> Result<()> {
    let now = now();
    let mut each: std::collections::BTreeMap<Id, GuildEvents> = users
        .iter()
        .map(|u| {
            (
                *u,
                GuildEvents {
                    guild_id: g,
                    count: 0,
                    live: vec![],
                },
            )
        })
        .collect();
    for s in active(c, g).await? {
        if s.current(c, now).await?.is_none() {
            continue;
        }
        for v in audience(c, &s).await? {
            if let Some(e) = each.get_mut(&v) {
                e.count += 1;
                if s.live.is_some() {
                    e.live.push(s.id);
                }
            }
        }
    }
    for (u, summary) in each {
        out.push((vec![u], Event::Schedule(ScheduleUpdate::Summary(summary))));
    }
    Ok(())
}

/// Recomputes `next_at` for a series (call after any change to it).
pub(crate) async fn rearm(c: &mut SqliteConnection, s: &Series) -> Result<Option<u64>> {
    let now = now();
    let next = match s.status {
        EventStatus::Ended | EventStatus::Cancelled => {
            Some(s.ended_at.unwrap_or(now).saturating_add(EVENT_RETENTION_MS))
        }
        EventStatus::Live => {
            let o = s.occurrence(c, s.live.unwrap_or(s.rule.first_at)).await?;
            let hard = s.live_since.unwrap_or(now).saturating_add(MAX_LIVE_MS);
            let end = o.end.filter(|e| *e > now);
            Some(end.map_or(hard, |e| e.min(hard)))
        }
        EventStatus::Scheduled => match s.current(c, now).await? {
            Some(o) if o.key > s.notified_through => Some(o.start.max(now)),
            Some(o) => Some(lapse(&o)),
            None => Some(now),
        },
    };
    sqlx::query("UPDATE scheduled_events SET next_at=? WHERE id=?")
        .bind(next.map(|v| v as i64))
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    Ok(next)
}

// ---- commands -----------------------------------------------------------------------

pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: ScheduleCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    if !matches!(
        cmd,
        ScheduleCommand::Subscribe | ScheduleCommand::SetInterested { .. }
    ) {
        core.rate(format!("events:{}", u.0), 30., 10.)?;
    }
    let result = match cmd {
        ScheduleCommand::Subscribe => {
            let mut guilds = vec![];
            let ids: Vec<i64> = sqlx::query_scalar(
                "SELECT DISTINCT e.guild_id FROM scheduled_events e JOIN guild_members m ON m.guild_id=e.guild_id AND m.user_id=? WHERE e.status IN ('scheduled','live')",
            )
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?;
            for g in ids {
                let s = summary(c, u, Id(g as u64)).await?;
                if s.count > 0 {
                    guilds.push(s);
                }
            }
            let _ = key;
            ScheduleResult::Summary { guilds }
        }
        ScheduleCommand::Create(e) => ScheduleResult::Event(create(c, core, u, e, out).await?),
        ScheduleCommand::Update {
            event_id,
            occurrence,
            scope,
            patch,
        } => ScheduleResult::Event(
            update(c, core, u, event_id, occurrence, scope, patch, out).await?,
        ),
        ScheduleCommand::Cancel {
            event_id,
            occurrence,
            scope,
        } => {
            cancel(c, u, event_id, occurrence, scope, out).await?;
            ScheduleResult::Ack
        }
        ScheduleCommand::Delete { event_id } => {
            let s = load(c, event_id).await?;
            manage(c, u, &s).await?;
            let users = audience(c, &s).await?;
            sqlx::query("DELETE FROM scheduled_events WHERE id=?")
                .bind(id(event_id)?)
                .execute(&mut *c)
                .await?;
            audit(c, u, &s, AuditAction::EventDelete, vec![]).await?;
            out.push((
                users.clone(),
                Event::Schedule(ScheduleUpdate::Removed {
                    guild_id: s.guild,
                    event_id,
                }),
            ));
            summaries(c, s.guild, users, out).await?;
            ScheduleResult::Ack
        }
        ScheduleCommand::SetInterested {
            event_id,
            interested,
        } => {
            core.rate(format!("event-interest:{}", u.0), 60., 20.)?;
            let s = load(c, event_id).await?;
            visible(c, u, &s).await?;
            if interested {
                require(
                    !matches!(s.status, EventStatus::Ended | EventStatus::Cancelled),
                    ErrorCode::InvalidRequest,
                )?;
                sqlx::query("INSERT OR IGNORE INTO event_interest(event_id,user_id,created_at) VALUES(?,?,?)")
                    .bind(id(event_id)?)
                    .bind(id(u)?)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
            } else {
                sqlx::query("DELETE FROM event_interest WHERE event_id=? AND user_id=?")
                    .bind(id(event_id)?)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?;
            }
            out.push((
                vec![u],
                Event::Schedule(ScheduleUpdate::Interest {
                    event_id,
                    interested,
                }),
            ));
            ScheduleResult::Ack
        }
        ScheduleCommand::Start { event_id } => {
            let s = load(c, event_id).await?;
            manage(c, u, &s).await?;
            require(
                s.status == EventStatus::Scheduled,
                ErrorCode::InvalidRequest,
            )?;
            let o = s
                .current(c, now())
                .await?
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            go_live(c, &s, o.key).await?;
            let s = load(c, event_id).await?;
            changed(c, &s, out).await?;
            ScheduleResult::Event(s.view(c, now()).await?)
        }
        ScheduleCommand::End { event_id } => {
            let s = load(c, event_id).await?;
            manage(c, u, &s).await?;
            require(s.status == EventStatus::Live, ErrorCode::InvalidRequest)?;
            finish(c, &s).await?;
            let s = load(c, event_id).await?;
            changed(c, &s, out).await?;
            ScheduleResult::Event(s.view(c, now()).await?)
        }
        ScheduleCommand::Share {
            event_id,
            channel_id,
        } => ScheduleResult::Message(share(c, core, key, u, event_id, channel_id, out).await?),
        ScheduleCommand::CreateInvite {
            event_id,
            ttl_seconds,
            max_uses,
        } => {
            let s = load(c, event_id).await?;
            visible(c, u, &s).await?;
            require(
                !matches!(s.status, EventStatus::Ended | EventStatus::Cancelled),
                ErrorCode::InvalidRequest,
            )?;
            let CommandResult::Invite(invite) = Box::pin(crate::commands::mutate(
                c,
                core,
                key,
                u,
                Command::CreateInvite {
                    guild_id: s.guild,
                    ttl_seconds,
                    max_uses,
                },
                out,
            ))
            .await?
            else {
                return Err(Error::new(ErrorCode::Internal));
            };
            sqlx::query("INSERT INTO invite_events(invite_id,event_id) VALUES(?,?)")
                .bind(id(invite.id)?)
                .bind(id(event_id)?)
                .execute(&mut *c)
                .await?;
            // A top-level CreateInvite is audited by `moderation::record`; this one is here.
            let entry = crate::moderation::audit::Entry::new(
                s.guild,
                AuditAction::InviteCreate,
                AuditTarget::Invite,
                Some(invite.id),
            )
            .named(invite.code.clone())
            .change("event", None, Some(s.name.clone()));
            crate::moderation::audit::write(c, u, entry).await?;
            ScheduleResult::Invite(invite)
        }
    };
    core.schedule.wake();
    Ok(CommandResult::Schedule(result))
}

async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    e: NewEvent,
    out: &mut Deliveries,
) -> Result<ScheduledEvent> {
    let g = e.guild_id;
    let p = guild_rights(c, u, g).await?;
    require(
        p.intersects(Permissions::MANAGE_EVENTS | Permissions::CREATE_EVENTS),
        ErrorCode::Forbidden,
    )?;
    let name = text(&e.name, MAX_EVENT_NAME_CHARS, false)?;
    let description = text(&e.description, MAX_EVENT_DESCRIPTION_CHARS, true)?;
    let location = location(c, u, g, e.location).await?;
    let duration = times(e.start_at, e.end_at, &location, true)?;
    let (rule, start_local) = Rule::starting(e.start_at, &e.time_zone, e.recurrence)?;
    // Ended and cancelled events wait for deletion; they make room if needed.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_events WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    if stored >= 2 * MAX_GUILD_EVENTS as i64 {
        sqlx::query("DELETE FROM scheduled_events WHERE id=(SELECT id FROM scheduled_events WHERE guild_id=? AND status IN ('ended','cancelled') ORDER BY ended_at LIMIT 1)")
            .bind(id(g)?)
            .execute(&mut *c)
            .await?;
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scheduled_events WHERE guild_id=? AND status IN ('scheduled','live')",
    )
    .bind(id(g)?)
    .fetch_one(&mut *c)
    .await?;
    require(active < MAX_GUILD_EVENTS as i64, ErrorCode::LimitExceeded)?;
    let event = core.id().await?;
    insert(
        c,
        event,
        g,
        u,
        &name,
        &description,
        &location,
        &start_local,
        &e.time_zone,
        rule.first_at,
        duration,
        e.recurrence,
        None,
    )
    .await?;
    let s = load(c, event).await?;
    rearm(c, &s).await?;
    audit(c, u, &s, AuditAction::EventCreate, vec![]).await?;
    changed(c, &s, out).await?;
    s.view(c, now()).await
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    c: &mut SqliteConnection,
    event: Id,
    g: Id,
    creator: Id,
    name: &str,
    description: &str,
    location: &EventLocation,
    start_local: &str,
    time_zone: &str,
    first_at: u64,
    duration: Option<u64>,
    recurrence: EventRecurrence,
    until: Option<u64>,
) -> Result<()> {
    let (kind, channel, place) = location_columns(location);
    sqlx::query("INSERT INTO scheduled_events(id,guild_id,creator_id,name,description,location_kind,channel_id,place,start_local,time_zone,first_at,duration_ms,recurrence,until_at,status,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,'scheduled',?)")
        .bind(id(event)?)
        .bind(id(g)?)
        .bind(id(creator)?)
        .bind(name)
        .bind(description)
        .bind(kind)
        .bind(channel)
        .bind(place)
        .bind(start_local)
        .bind(time_zone)
        .bind(first_at as i64)
        .bind(duration.map(|d| d as i64))
        .bind(db::enum_str(recurrence))
        .bind(until.map(|u| u as i64))
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn update(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    event: Id,
    occurrence: u64,
    scope: EditScope,
    patch: EventPatch,
    out: &mut Deliveries,
) -> Result<ScheduledEvent> {
    let s = load(c, event).await?;
    manage(c, u, &s).await?;
    require(
        matches!(s.status, EventStatus::Scheduled | EventStatus::Live),
        ErrorCode::InvalidRequest,
    )?;
    require(s.rule.contains(occurrence), ErrorCode::NotFound)?;
    let o = s.occurrence(c, occurrence).await?;
    // A live event keeps its time and rule until it ends.
    require(
        s.live.is_none()
            || (patch.start_at.is_none()
                && patch.recurrence.is_none()
                && patch.time_zone.is_none()),
        ErrorCode::InvalidRequest,
    )?;
    let name = patch
        .name
        .as_deref()
        .map(|n| text(n, MAX_EVENT_NAME_CHARS, false))
        .transpose()?;
    let description = patch
        .description
        .as_deref()
        .map(|d| text(d, MAX_EVENT_DESCRIPTION_CHARS, true))
        .transpose()?;
    let new_location = match patch.location.clone() {
        Some(l) => Some(location(c, u, s.guild, l).await?),
        None => None,
    };
    let start = patch.start_at.unwrap_or(o.start);
    let end = match patch.end_at {
        Patch::Keep => o
            .end
            .map(|e| e.saturating_sub(o.start).saturating_add(start)),
        Patch::Set(e) => Some(e),
        Patch::Clear => None,
    };
    let loc = new_location.clone().unwrap_or(o.location.clone());
    times(start, end, &loc, patch.start_at.is_some())?;
    let one_off = s.rule.recurrence() == EventRecurrence::None;
    let mut changes = vec![];
    if let Some(n) = &name {
        changes.push(("name", Some(o.name.clone()), Some(n.clone())));
    }
    if patch.start_at.is_some() {
        changes.push(("start", Some(o.start.to_string()), Some(start.to_string())));
    }
    if one_off || scope == EditScope::Following {
        // The series itself changes from here on.
        let recurrence = patch.recurrence.unwrap_or(s.rule.recurrence());
        let time_zone = patch.time_zone.clone().unwrap_or(s.time_zone.clone());
        let (rule, start_local) = Rule::starting(start, &time_zone, recurrence)?;
        let duration = end.map(|e| e.saturating_sub(start));
        let (kind, channel, place) = location_columns(&loc);
        if occurrence == s.rule.first_at || one_off {
            sqlx::query("UPDATE scheduled_events SET name=?,description=?,location_kind=?,channel_id=?,place=?,start_local=?,time_zone=?,first_at=?,duration_ms=?,recurrence=?,notified_through=CASE WHEN live_occurrence IS NULL AND ?<>first_at THEN 0 ELSE notified_through END,done_through=CASE WHEN live_occurrence IS NULL THEN 0 ELSE done_through END WHERE id=?")
                .bind(name.clone().unwrap_or(s.name.clone()))
                .bind(description.clone().unwrap_or(s.description.clone()))
                .bind(kind)
                .bind(channel)
                .bind(place)
                .bind(&start_local)
                .bind(&time_zone)
                .bind(rule.first_at as i64)
                .bind(duration.map(|d| d as i64))
                .bind(db::enum_str(recurrence))
                .bind(rule.first_at as i64)
                .bind(id(event)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("DELETE FROM event_exceptions WHERE event_id=?")
                .bind(id(event)?)
                .execute(&mut *c)
                .await?;
            let s = load(c, event).await?;
            rearm(c, &s).await?;
            audit(c, u, &s, AuditAction::EventUpdate, changes).await?;
            changed(c, &s, out).await?;
            return s.view(c, now()).await;
        }
        // "This and following": the old series stops here; a new one takes
        // the changes and the interested members.
        require(s.live != Some(occurrence), ErrorCode::InvalidRequest)?;
        sqlx::query("UPDATE scheduled_events SET until_at=? WHERE id=?")
            .bind(occurrence as i64)
            .bind(id(event)?)
            .execute(&mut *c)
            .await?;
        let next = core.id().await?;
        insert(
            c,
            next,
            s.guild,
            s.creator,
            &name.unwrap_or(s.name.clone()),
            &description.unwrap_or(s.description.clone()),
            &loc,
            &start_local,
            &time_zone,
            rule.first_at,
            duration,
            recurrence,
            None,
        )
        .await?;
        sqlx::query("INSERT INTO event_interest(event_id,user_id,created_at) SELECT ?,user_id,created_at FROM event_interest WHERE event_id=?")
            .bind(id(next)?)
            .bind(id(event)?)
            .execute(&mut *c)
            .await?;
        sqlx::query("UPDATE scheduled_events SET cover_version=? WHERE id=?")
            .bind(i64::from(s.cover))
            .bind(id(next)?)
            .execute(&mut *c)
            .await?;
        sqlx::query("INSERT INTO event_covers(event_id,content_type,bytes) SELECT ?,content_type,bytes FROM event_covers WHERE event_id=?")
            .bind(id(next)?)
            .bind(id(event)?)
            .execute(&mut *c)
            .await?;
        let old = load(c, event).await?;
        if old.current(c, now()).await?.is_none() && old.live.is_none() {
            end_series(c, &old, EventStatus::Ended).await?;
        }
        let old = load(c, event).await?;
        rearm(c, &old).await?;
        changed(c, &old, out).await?;
        let s = load(c, next).await?;
        rearm(c, &s).await?;
        audit(c, u, &s, AuditAction::EventUpdate, changes).await?;
        changed(c, &s, out).await?;
        return s.view(c, now()).await;
    }
    // "This event": an exception for the one occurrence.
    require(
        patch.recurrence.is_none() && patch.time_zone.is_none(),
        ErrorCode::InvalidRequest,
    )?;
    exception_room(c, event, occurrence).await?;
    let (kind, channel, place) = match &new_location {
        Some(l) => {
            let (k, ch, p) = location_columns(l);
            (Some(k), ch, p.map(str::to_owned))
        }
        None => (None, None, None),
    };
    sqlx::query("INSERT INTO event_exceptions(event_id,occurrence,name,description,start_at,end_at,end_cleared,location_kind,channel_id,place) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(event_id,occurrence) DO UPDATE SET name=COALESCE(excluded.name,name),description=COALESCE(excluded.description,description),start_at=COALESCE(excluded.start_at,start_at),end_at=CASE WHEN excluded.end_cleared THEN NULL ELSE COALESCE(excluded.end_at,end_at) END,end_cleared=excluded.end_cleared,location_kind=COALESCE(excluded.location_kind,location_kind),channel_id=CASE WHEN excluded.location_kind IS NULL THEN channel_id ELSE excluded.channel_id END,place=CASE WHEN excluded.location_kind IS NULL THEN place ELSE excluded.place END")
        .bind(id(event)?)
        .bind(occurrence as i64)
        .bind(name)
        .bind(description)
        .bind(patch.start_at.map(|v| v as i64))
        .bind(match patch.end_at {
            Patch::Set(e) => Some(e as i64),
            Patch::Keep if patch.start_at.is_some() => end.map(|e| e as i64),
            _ => None,
        })
        .bind(matches!(patch.end_at, Patch::Clear))
        .bind(kind)
        .bind(channel)
        .bind(place)
        .execute(&mut *c)
        .await?;
    let s = load(c, event).await?;
    rearm(c, &s).await?;
    audit(c, u, &s, AuditAction::EventUpdate, changes).await?;
    changed(c, &s, out).await?;
    let o = s.occurrence(c, occurrence).await?;
    Ok(s.wire(&o))
}

async fn exception_room(c: &mut SqliteConnection, event: Id, occurrence: u64) -> Result<()> {
    // Exceptions for occurrences long past are no longer needed.
    sqlx::query("DELETE FROM event_exceptions WHERE event_id=? AND occurrence<?")
        .bind(id(event)?)
        .bind(now().saturating_sub(MAX_LIVE_MS + EVENT_RETENTION_MS) as i64)
        .execute(&mut *c)
        .await?;
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM event_exceptions WHERE event_id=? AND occurrence<>?",
    )
    .bind(id(event)?)
    .bind(occurrence as i64)
    .fetch_one(&mut *c)
    .await?;
    require(n < MAX_EXCEPTIONS, ErrorCode::LimitExceeded)
}

async fn cancel(
    c: &mut SqliteConnection,
    u: Id,
    event: Id,
    occurrence: u64,
    scope: EditScope,
    out: &mut Deliveries,
) -> Result<()> {
    let s = load(c, event).await?;
    manage(c, u, &s).await?;
    require(
        s.status == EventStatus::Scheduled,
        ErrorCode::InvalidRequest,
    )?;
    require(s.rule.contains(occurrence), ErrorCode::NotFound)?;
    let whole = s.rule.recurrence() == EventRecurrence::None
        || (scope == EditScope::Following && occurrence <= s.rule.first_at);
    if whole {
        end_series(c, &s, EventStatus::Cancelled).await?;
    } else if scope == EditScope::Following {
        sqlx::query("UPDATE scheduled_events SET until_at=? WHERE id=?")
            .bind(occurrence as i64)
            .bind(id(event)?)
            .execute(&mut *c)
            .await?;
    } else {
        exception_room(c, event, occurrence).await?;
        sqlx::query("INSERT INTO event_exceptions(event_id,occurrence,cancelled) VALUES(?,?,1) ON CONFLICT(event_id,occurrence) DO UPDATE SET cancelled=1")
            .bind(id(event)?)
            .bind(occurrence as i64)
            .execute(&mut *c)
            .await?;
    }
    let s = load(c, event).await?;
    if s.status == EventStatus::Scheduled && s.current(c, now()).await?.is_none() {
        end_series(c, &s, EventStatus::Ended).await?;
    }
    let s = load(c, event).await?;
    rearm(c, &s).await?;
    audit(
        c,
        u,
        &s,
        AuditAction::EventCancel,
        vec![(
            "occurrence",
            None,
            Some(if whole {
                "all".into()
            } else {
                occurrence.to_string()
            }),
        )],
    )
    .await?;
    changed(c, &s, out).await
}

async fn end_series(c: &mut SqliteConnection, s: &Series, status: EventStatus) -> Result<()> {
    sqlx::query("UPDATE scheduled_events SET status=?,live_occurrence=NULL,live_since=NULL,ended_at=? WHERE id=?")
        .bind(db::enum_str(status))
        .bind(now() as i64)
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}

async fn go_live(c: &mut SqliteConnection, s: &Series, occurrence: u64) -> Result<()> {
    sqlx::query("UPDATE scheduled_events SET status='live',live_occurrence=?,live_since=?,notified_through=MAX(notified_through,?) WHERE id=?")
        .bind(occurrence as i64)
        .bind(now() as i64)
        .bind(occurrence as i64)
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    let s = load(c, s.id).await?;
    rearm(c, &s).await?;
    Ok(())
}

/// The live occurrence is over: on to the next one, or the series ends.
async fn finish(c: &mut SqliteConnection, s: &Series) -> Result<()> {
    let done = s.live.unwrap_or(0) as i64;
    sqlx::query("UPDATE scheduled_events SET status='scheduled',live_occurrence=NULL,live_since=NULL,notified_through=MAX(notified_through,?1),done_through=MAX(done_through,?1) WHERE id=?2")
        .bind(done)
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    let next = load(c, s.id).await?;
    if next.current(c, now()).await?.is_none() {
        end_series(c, &next, EventStatus::Ended).await?;
    }
    let next = load(c, s.id).await?;
    rearm(c, &next).await?;
    Ok(())
}

async fn share(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    event: Id,
    ch: Id,
    out: &mut Deliveries,
) -> Result<Message> {
    let s = load(c, event).await?;
    visible(c, u, &s).await?;
    let target = db::text(
        c,
        u,
        ch,
        Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
    )
    .await?;
    require(
        target.summary.guild_id == Some(s.guild),
        ErrorCode::InvalidRequest,
    )?;
    let view = s.view(c, now()).await?;
    let location_name = match view.location.channel() {
        Some(lc) => db::channel(c, lc).await.ok().and_then(|ch| ch.summary.name),
        None => None,
    };
    let card = EventCard {
        event_id: s.id,
        guild_id: s.guild,
        name: view.name.clone(),
        start_at: view.start_at,
        end_at: view.end_at,
        location: view.location.clone(),
        location_name,
        cover: view.cover,
        recurrence: view.recurrence,
    };
    let CommandResult::Message(message) = Box::pin(crate::commands::mutate(
        c,
        core,
        key,
        u,
        Command::SendMessage(MessageDraft {
            channel_id: ch,
            content: format!("Event: {}", view.name),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        }),
        out,
    ))
    .await?
    else {
        return Err(Error::new(ErrorCode::Internal));
    };
    let payload = rmp_serde::to_vec_named(&card).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO stage_messages(message_id,kind,payload) VALUES(?,'event',?)")
        .bind(id(message.id)?)
        .bind(payload)
        .execute(&mut *c)
        .await?;
    let message = db::message(c, u, ch, message.id, false).await?;
    // The send's own MessageCreated predates the card: carry it there too.
    for (_, e) in out.iter_mut() {
        if let Event::MessageCreated { message: m, .. } = e {
            if m.id == message.id {
                *m = message.clone();
            }
        }
    }
    Ok(message)
}

async fn audit(
    c: &mut SqliteConnection,
    actor: Id,
    s: &Series,
    action: AuditAction,
    changes: Vec<(&str, Option<String>, Option<String>)>,
) -> Result<()> {
    use crate::moderation::audit::Entry;
    let mut entry =
        Entry::new(s.guild, action, AuditTarget::Event, Some(s.id)).named(s.name.clone());
    for (field, before, after) in changes {
        entry = entry.change(field, before, after);
    }
    crate::moderation::audit::write(c, actor, entry).await
}

// ---- Stage and voice sessions -------------------------------------------------------

/// A Stage or voice event at `ch` that is due now (scheduled, its current
/// occurrence within `EARLY_START_MS` of starting and not lapsed).
async fn due_at_channel(c: &mut SqliteConnection, ch: Id) -> Result<Option<(Series, u64)>> {
    let now = now();
    let rows = sqlx::query(
        "SELECT * FROM scheduled_events WHERE channel_id=? AND status='scheduled' ORDER BY id",
    )
    .bind(id(ch)?)
    .fetch_all(&mut *c)
    .await?;
    for r in rows {
        let s = series_row(&r)?;
        if let Some(o) = s.current(c, now).await? {
            if o.start <= now + EARLY_START_MS && o.location.channel() == Some(ch) {
                return Ok(Some((s, o.key)));
            }
        }
    }
    Ok(None)
}

/// A Stage is starting. With `event_id`, that event (a Stage event at this
/// channel) goes live; without, the event due at this channel does, if any.
pub(crate) async fn stage_starting(
    core: &mut Core,
    c: &mut SqliteConnection,
    guild: Id,
    ch: Id,
    event_id: Option<Id>,
    u: Id,
) -> Result<Option<Id>> {
    let found = match event_id {
        Some(e) => {
            let s = load(c, e).await?;
            require(s.guild == guild, ErrorCode::NotFound)?;
            visible(c, u, &s).await?;
            require(
                s.status == EventStatus::Scheduled,
                ErrorCode::InvalidRequest,
            )?;
            let o = s
                .current(c, now())
                .await?
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            require(
                o.location == (EventLocation::Stage { channel_id: ch }),
                ErrorCode::InvalidRequest,
            )?;
            Some((s, o.key))
        }
        None => due_at_channel(c, ch).await?,
    };
    let Some((s, key)) = found else {
        return Ok(None);
    };
    go_live(c, &s, key).await?;
    core.schedule.wake();
    Ok(Some(s.id))
}

/// Someone joined a voice channel: an event due there goes live.
pub(crate) async fn voice_joined(core: &mut Core, c: &mut SqliteConnection, ch: Id) -> Result<()> {
    if core
        .voice
        .values()
        .filter(|e| e.state.channel_id == ch)
        .count()
        > 1
    {
        return Ok(());
    }
    if let Some((s, key)) = due_at_channel(c, ch).await? {
        if matches!(s.location, EventLocation::Voice { .. }) {
            go_live(c, &s, key).await?;
            publish(core, c, s.id).await?;
        }
    }
    Ok(())
}

/// A voice channel emptied: its live event ends if its end time has passed
/// (or it has none).
pub(crate) async fn voice_emptied(core: &mut Core, c: &mut SqliteConnection, ch: Id) -> Result<()> {
    let rows = sqlx::query("SELECT * FROM scheduled_events WHERE channel_id=? AND status='live'")
        .bind(id(ch)?)
        .fetch_all(&mut *c)
        .await?;
    for r in rows {
        let s = series_row(&r)?;
        let o = s.occurrence(c, s.live.unwrap_or(s.rule.first_at)).await?;
        if o.end.is_none_or(|e| e <= now()) {
            finish(c, &s).await?;
            publish(core, c, s.id).await?;
        }
    }
    Ok(())
}

/// A Stage ended: its event's occurrence is over.
pub(crate) async fn stage_ended(
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    event_id: Option<Id>,
) -> Result<()> {
    let Some(e) = event_id else {
        return voice_emptied(core, c, ch).await;
    };
    if let Ok(s) = load(c, e).await {
        if s.status == EventStatus::Live {
            finish(c, &s).await?;
        }
        publish(core, c, e).await?;
    }
    Ok(())
}

/// The events at a channel changed outside a command: tell who can see them.
pub(crate) async fn publish_channel(
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
) -> Result<()> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM scheduled_events WHERE channel_id=?")
        .bind(id(ch)?)
        .fetch_all(&mut *c)
        .await?;
    for e in ids {
        publish(core, c, Id(e as u64)).await?;
    }
    Ok(())
}
async fn publish(core: &mut Core, c: &mut SqliteConnection, e: Id) -> Result<()> {
    let Ok(s) = load(c, e).await else {
        return Ok(());
    };
    let mut out = vec![];
    changed(c, &s, &mut out).await?;
    for (users, event) in out {
        core.emit(&users, event)?;
    }
    core.schedule.wake();
    Ok(())
}

/// Someone joined the server through an invite: an event invite marks them
/// Interested, and they get the server's events summary.
pub(crate) async fn joined_by_invite(
    c: &mut SqliteConnection,
    invite: Id,
    g: Id,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO event_interest(event_id,user_id,created_at) SELECT e.id,?,? FROM invite_events i JOIN scheduled_events e ON e.id=i.event_id WHERE i.invite_id=? AND e.status IN ('scheduled','live')")
        .bind(id(u)?)
        .bind(now() as i64)
        .bind(id(invite)?)
        .execute(&mut *c)
        .await?;
    let s = summary(c, u, g).await?;
    out.push((vec![u], Event::Schedule(ScheduleUpdate::Summary(s))));
    Ok(())
}

// ---- the timer -----------------------------------------------------------------------

pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = app.0.core.lock().await.schedule.wake.clone();
        let stop = app.0.shutdown.clone();
        loop {
            // Under the gate: a command that just armed a deadline has committed.
            let next = {
                let _core = app.0.core.lock().await;
                match app.0.pool.acquire().await {
                    Ok(mut c) => sqlx::query_scalar::<_, Option<i64>>(
                        "SELECT MIN(next_at) FROM scheduled_events WHERE next_at IS NOT NULL",
                    )
                    .fetch_one(&mut *c)
                    .await
                    .ok()
                    .flatten(),
                    Err(_) => None,
                }
            };
            let wait = async {
                match next {
                    Some(at) => {
                        let ms = (at.max(0) as u64).saturating_sub(now());
                        tokio::time::sleep(std::time::Duration::from_millis(ms)).await
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = wake.notified() => continue,
                _ = wait => {}
            }
            if let Err(error) = tick(&app).await {
                tracing::debug!(code = ?error.0.code, "event timer");
                // A failing row must not spin: wait for the next change.
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    });
}

/// Handles every series that is due, at most 100 at a time.
pub(crate) async fn tick(app: &App) -> Result<()> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let now = now();
    let rows = sqlx::query("SELECT * FROM scheduled_events WHERE next_at IS NOT NULL AND next_at<=? ORDER BY next_at LIMIT 100")
        .bind(now as i64)
        .fetch_all(&mut *c)
        .await?;
    let mut starting = vec![];
    for r in rows {
        let s = series_row(&r)?;
        match s.status {
            EventStatus::Ended | EventStatus::Cancelled => {
                let users = audience(&mut c, &s).await.unwrap_or_default();
                sqlx::query("DELETE FROM scheduled_events WHERE id=?")
                    .bind(id(s.id)?)
                    .execute(&mut *c)
                    .await?;
                core.emit(
                    &users,
                    Event::Schedule(ScheduleUpdate::Removed {
                        guild_id: s.guild,
                        event_id: s.id,
                    }),
                )?;
                continue;
            }
            EventStatus::Live => {
                let o = s
                    .occurrence(&mut c, s.live.unwrap_or(s.rule.first_at))
                    .await?;
                let ended = o.end.is_some_and(|e| e <= now);
                let hard = s.live_since.unwrap_or(now) + MAX_LIVE_MS <= now;
                let occupied = o
                    .location
                    .channel()
                    .is_some_and(|ch| core.voice.values().any(|e| e.state.channel_id == ch))
                    || o.location
                        .channel()
                        .is_some_and(|ch| crate::stage::is_live(&core, ch));
                if hard || (ended && !occupied) {
                    finish(&mut c, &s).await?;
                } else {
                    // Live past its end with people still there: it ends when
                    // the channel empties (an event, not a timer).
                    let at = if ended {
                        s.live_since.unwrap_or(now) + MAX_LIVE_MS
                    } else {
                        o.end.unwrap_or(now)
                    };
                    sqlx::query("UPDATE scheduled_events SET next_at=? WHERE id=?")
                        .bind(at.max(now + 1) as i64)
                        .bind(id(s.id)?)
                        .execute(&mut *c)
                        .await?;
                    continue;
                }
            }
            EventStatus::Scheduled => match s.current(&mut c, now).await? {
                None => end_series(&mut c, &s, EventStatus::Ended).await?,
                Some(o) if o.start <= now && o.key > s.notified_through => {
                    sqlx::query("UPDATE scheduled_events SET notified_through=? WHERE id=?")
                        .bind(o.key as i64)
                        .bind(id(s.id)?)
                        .execute(&mut *c)
                        .await?;
                    if matches!(o.location, EventLocation::External { .. }) {
                        go_live(&mut c, &s, o.key).await?;
                    }
                    starting.push(s.id);
                }
                Some(_) => {}
            },
        }
        let s = load(&mut c, s.id).await?;
        rearm(&mut c, &s).await?;
        let mut out = vec![];
        changed(&mut c, &s, &mut out).await?;
        for (users, event) in out {
            core.emit(&users, event)?;
        }
    }
    for e in starting {
        announce(app, &mut core, &mut c, e).await?;
    }
    Ok(())
}

/// An event is starting: interested members are told in the app, and by
/// Web Push on devices they are not using (never in Do Not Disturb, nor
/// when they muted the server or turned its notifications off).
async fn announce(app: &App, core: &mut Core, c: &mut SqliteConnection, e: Id) -> Result<()> {
    let s = load(c, e).await?;
    let view = s.view(c, now()).await?;
    let interested: Vec<Id> = sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM event_interest WHERE event_id=? ORDER BY user_id LIMIT 1000",
    )
    .bind(id(e)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect();
    let channel = view.location.channel();
    let mut told = vec![];
    let mut pushed = vec![];
    for u in interested {
        if visible(c, u, &s).await.is_err() {
            continue;
        }
        // The one notification decision (`privacy::notify::notice`).
        let notice = crate::privacy::notify::Notice::EventStarting {
            guild: s.guild,
            channel,
        };
        if let Some(d) = crate::privacy::notify::notice(c, u, notice).await? {
            told.push(u);
            if d.push {
                pushed.push(u);
            }
        }
    }
    core.emit(&told, Event::Schedule(ScheduleUpdate::Starting(view)))?;
    for u in pushed {
        crate::push::event_starting(app, u, s.guild, s.id);
    }
    Ok(())
}

/// Whether the start of event `e` may be pushed to `u` (asked again when the
/// push goes out: settings can change in between).
pub(crate) async fn push_allowed(c: &mut SqliteConnection, u: Id, g: Id, e: Id) -> Result<bool> {
    let channel = match load(c, e).await {
        Ok(s) => s.view(c, now()).await?.location.channel(),
        Err(_) => return Ok(false),
    };
    let notice = crate::privacy::notify::Notice::EventStarting { guild: g, channel };
    Ok(crate::privacy::notify::notice(c, u, notice)
        .await?
        .is_some_and(|d| d.push))
}
