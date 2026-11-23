//! Stage channels: speakers and an audience (`docs/features/stage.md`).
//! Wire contracts: `sticktalk_proto::stage`.
//!
//! A Stage's media is the ordinary voice path. What this module adds is each
//! live Stage's roles: who may speak (`speakers`), raised hands, invitations,
//! and who is a Stage moderator. Audience sessions are SFU *listeners*: the
//! voice lease gives them no right to publish (`scope`) and marks them so the
//! SFU sends them nothing about other listeners and drops anything they send.
//! They never appear in `GuildState`, so a thousand listeners cost the rest
//! of the server nothing.
//!
//! Timing: roster changes are combined and sent at most every `FLUSH_MS`,
//! the channel list's counts at most every `COUNT_MS`, by one task that is
//! woken only when something changed (`touch`). With nothing pending it waits.
#[cfg(test)]
mod tests;

use crate::{
    db,
    error::{require, Error, Result},
    state::{now, App, Core},
    voice,
};
use sqlx::{Connection as _, SqliteConnection};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use sticktalk_proto::*;

/// Listener sessions in all Stages together (the SFU's listener cap).
pub(crate) const MAX_LISTENERS: usize = MAX_GUILD_MEMBERS;
/// Roster changes are combined over this long.
const FLUSH_MS: u64 = 400;
/// The channel list's counts change at most this often.
const COUNT_MS: u64 = 1000;
/// A speaker whose last device left speaks again if they rejoin this soon.
const SPEAKER_GRACE_MS: u64 = 120_000;
/// Discord's Stage moderator: all three on the Stage channel.
const MODERATOR: Permissions = Permissions::MANAGE_CHANNELS
    .union(Permissions::MUTE_MEMBERS)
    .union(Permissions::MOVE_MEMBERS);
/// What the audience may not do.
const PUBLISH: Permissions = Permissions::SPEAK
    .union(Permissions::STREAM)
    .union(Permissions::VIDEO)
    .union(Permissions::USE_VOICE_ACTIVITY)
    .union(Permissions::PRIORITY_SPEAKER)
    .union(Permissions::USE_SOUNDBOARD);

#[derive(Default)]
pub(crate) struct Stages {
    rooms: BTreeMap<Id, Room>,
    /// When the flush task should next run; None: nothing pending.
    due: Option<u64>,
    wake: Arc<tokio::sync::Notify>,
}

struct Room {
    guild: Id,
    live: Option<Live>,
    speakers: BTreeSet<Id>,
    /// Raised hands, oldest first.
    hands: Vec<(Id, u64)>,
    invited: BTreeSet<Id>,
    /// Everyone connected, by user.
    members: BTreeMap<Id, Member>,
    /// Speakers whose last device left, and when: a reconnect keeps the role.
    recent: BTreeMap<Id, u64>,
    next_seq: u64,
    /// Members whose entry changed since the last flush, and who left.
    dirty: BTreeSet<Id>,
    gone: BTreeSet<Id>,
    counts_dirty: bool,
    counts_at: u64,
    /// Connections that missed a change and get the whole roster again.
    resync: BTreeSet<Vec<u8>>,
}

struct Member {
    name: String,
    avatar: Option<Id>,
    moderator: bool,
    seq: u64,
}

struct Live {
    topic: String,
    note: Option<String>,
    started_by: Id,
    started_at: u64,
    event_id: Option<Id>,
    peak: u32,
    spoke: BTreeSet<Id>,
}

impl Room {
    fn new(guild: Id) -> Self {
        Self {
            guild,
            live: None,
            speakers: BTreeSet::new(),
            hands: vec![],
            invited: BTreeSet::new(),
            members: BTreeMap::new(),
            recent: BTreeMap::new(),
            next_seq: 0,
            dirty: BTreeSet::new(),
            gone: BTreeSet::new(),
            counts_dirty: false,
            counts_at: 0,
            resync: BTreeSet::new(),
        }
    }
    fn speaking(&self) -> u32 {
        self.members
            .keys()
            .filter(|u| self.speakers.contains(u))
            .count() as u32
    }
    fn listening(&self) -> u32 {
        self.members.len() as u32 - self.speaking()
    }
    fn instance(&self, channel: Id) -> Option<StageInstance> {
        self.live.as_ref().map(|l| StageInstance {
            channel_id: channel,
            guild_id: self.guild,
            topic: l.topic.clone(),
            privacy_note: l.note.clone(),
            started_by: l.started_by,
            started_at: l.started_at,
            speakers: self.speaking(),
            listeners: self.listening(),
            event_id: l.event_id,
        })
    }
    /// One person as `viewer` sees them: hands and invitations are private
    /// to moderators and to the person themselves.
    fn entry(&self, user: Id, viewer: Id, moderator_view: bool) -> Option<StageMember> {
        let m = self.members.get(&user)?;
        let private = moderator_view || viewer == user;
        Some(StageMember {
            user_id: user,
            name: m.name.clone(),
            avatar_id: m.avatar,
            role: if self.speakers.contains(&user) {
                StageRole::Speaker
            } else {
                StageRole::Audience
            },
            moderator: m.moderator,
            hand_raised_at: if private {
                self.hands.iter().find(|h| h.0 == user).map(|h| h.1)
            } else {
                None
            },
            invited: private && self.invited.contains(&user),
        })
    }
    fn roster(&self, channel: Id, viewer: Id) -> StageRoster {
        let moderator_view = self.members.get(&viewer).is_some_and(|m| m.moderator);
        let mut order: Vec<(bool, u64, Id)> = self
            .members
            .iter()
            .map(|(u, m)| (!self.speakers.contains(u), m.seq, *u))
            .collect();
        order.sort();
        StageRoster {
            channel_id: channel,
            instance: self.instance(channel),
            members: order
                .into_iter()
                .filter_map(|(_, _, u)| self.entry(u, viewer, moderator_view))
                .collect(),
        }
    }
    fn changed(&mut self, user: Id) {
        self.dirty.insert(user);
        self.counts_dirty = true;
    }
    fn expire_recent(&mut self, now: u64) {
        self.recent
            .retain(|_, at| now.saturating_sub(*at) < SPEAKER_GRACE_MS);
    }
}

/// The audience's rights: everything but publishing.
pub(crate) fn scope(p: Permissions, listener: bool) -> Permissions {
    if listener {
        p - PUBLISH
    } else {
        p
    }
}

/// The Stage channel `ch`, if `u` can see it. Anything else is `not_found`.
async fn stage_channel(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<(Id, Id)> {
    let channel = db::access(c, u, ch, Permissions::empty()).await?;
    require(
        channel.summary.kind == ChannelKind::Stage,
        ErrorCode::NotFound,
    )?;
    let guild = channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((ch, guild))
}
async fn is_moderator(c: &mut SqliteConnection, u: Id, ch: Id) -> bool {
    db::access(c, u, ch, MODERATOR).await.is_ok()
}
fn refused(code: ErrorCode, message: &str) -> Error {
    let mut e = Error::new(code);
    e.0.message = message.into();
    e
}
/// Trimmed text of 1..=max characters (0 allowed when `optional`), no controls.
fn text(s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    require(
        !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
fn note(s: Option<String>) -> Result<Option<String>> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) => text(v, MAX_STAGE_NOTE_CHARS).map(Some),
    }
}

/// Before a session joins a Stage: whether it joins as audience (a listener).
/// A Stage that is not live admits only its moderators, as audience until
/// they start it.
pub(crate) async fn admit(
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
) -> Result<bool> {
    let live = core.stage.rooms.get(&ch).is_some_and(|r| r.live.is_some());
    if !live && !is_moderator(c, u, ch).await {
        return Err(refused(
            ErrorCode::Forbidden,
            "This Stage hasn't started yet.",
        ));
    }
    let Some(room) = core.stage.rooms.get_mut(&ch) else {
        return Ok(true);
    };
    let now = now();
    room.expire_recent(now);
    if room.live.is_some()
        && room.recent.remove(&u).is_some()
        && room.speakers.len() < MAX_STAGE_SPEAKERS
    {
        room.speakers.insert(u);
        room.changed(u);
    }
    Ok(!room.speakers.contains(&u))
}

/// A session was added to `core.voice` for a Stage: the person is on it.
pub(crate) async fn joined(
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
) -> Result<()> {
    let Some(entry) = core.voice.get(&session) else {
        return Ok(());
    };
    let (guild, ch, key) = (
        entry.state.guild_id,
        entry.state.channel_id,
        entry.key().to_vec(),
    );
    let u = session.user_id;
    let present = core
        .stage
        .rooms
        .get(&ch)
        .is_some_and(|r| r.members.contains_key(&u));
    if !present {
        let (name, avatar) = display(c, guild, u).await?;
        let moderator = is_moderator(c, u, ch).await;
        let room = core
            .stage
            .rooms
            .entry(ch)
            .or_insert_with(|| Room::new(guild));
        room.next_seq += 1;
        let seq = room.next_seq;
        room.members.insert(
            u,
            Member {
                name,
                avatar,
                moderator,
                seq,
            },
        );
        room.gone.remove(&u);
        room.changed(u);
        let listening = room.listening();
        if let Some(live) = room.live.as_mut() {
            live.peak = live.peak.max(listening);
        }
    }
    let roster = core.stage.rooms[&ch].roster(ch, u);
    if voice::send(
        core,
        &key,
        0,
        &VoiceEvent::Stage(StageEvent::Roster(roster)),
    )
    .is_err()
    {
        if let Some(room) = core.stage.rooms.get_mut(&ch) {
            room.resync.insert(key);
        }
    }
    touch(core);
    Ok(())
}

/// A session left a Stage (`voice::remove`, after it left `core.voice`).
pub(crate) async fn left(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    u: Id,
) -> Result<()> {
    let still_here = core
        .voice
        .values()
        .any(|e| e.state.channel_id == ch && e.state.participant.session.user_id == u);
    let empty = !core.voice.values().any(|e| e.state.channel_id == ch);
    let Some(room) = core.stage.rooms.get_mut(&ch) else {
        return Ok(());
    };
    if !still_here && room.members.remove(&u).is_some() {
        room.dirty.remove(&u);
        room.gone.insert(u);
        room.counts_dirty = true;
        room.hands.retain(|h| h.0 != u);
        room.invited.remove(&u);
        if room.speakers.remove(&u) && room.live.is_some() {
            room.recent.insert(u, now());
            if room.recent.len() > 64 {
                if let Some(oldest) = room.recent.iter().min_by_key(|r| *r.1).map(|r| *r.0) {
                    room.recent.remove(&oldest);
                }
            }
        }
    }
    if empty {
        if room.live.is_some() {
            // Everyone left: the Stage is over.
            return end(app, core, c, ch, None).await;
        }
        core.stage.rooms.remove(&ch);
        return Ok(());
    }
    touch(core);
    Ok(())
}

/// The person's name in this server and their picture.
async fn display(c: &mut SqliteConnection, guild: Id, u: Id) -> Result<(String, Option<Id>)> {
    let row: (String, Option<i64>) = sqlx::query_as(
        "SELECT COALESCE(n.nickname,u.display_name,u.username),u.avatar_id FROM users u LEFT JOIN nicknames n ON n.guild_id=? AND n.user_id=u.id WHERE u.id=?",
    )
    .bind(db::id(guild)?)
    .bind(db::id(u)?)
    .fetch_one(&mut *c)
    .await?;
    Ok((row.0, row.1.map(|v| Id(v as u64))))
}

/// Something changed: make sure the flush task runs soon.
fn touch(core: &mut Core) {
    if core.stage.due.is_none() {
        core.stage.due = Some(now() + FLUSH_MS);
        core.stage.wake.notify_one();
    }
}

/// Connections on a Stage and whose they are (one entry per connection).
fn connections(core: &Core, ch: Id) -> Vec<(Vec<u8>, Id)> {
    let mut out: Vec<(Vec<u8>, Id)> = core
        .voice
        .values()
        .filter(|e| e.state.channel_id == ch)
        .map(|e| (e.key().to_vec(), e.state.participant.session.user_id))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A Stage's live state to everyone who can see its channel.
async fn announce(
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    event: StageEvent,
) -> Result<()> {
    let viewers = db::audience(c, ch).await?;
    let keys: Vec<Vec<u8>> = core
        .streams
        .iter()
        .filter(|(_, s)| s.connection.is_some() && viewers.contains(&s.user))
        .map(|(k, _)| k.clone())
        .collect();
    let event = VoiceEvent::Stage(event);
    for key in keys {
        let _ = voice::send(core, &key, 0, &event);
    }
    Ok(())
}

/// Sends what changed since the last flush; called by the flush task.
pub(crate) async fn flush(core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let now = now();
    core.stage.due = None;
    let channels: Vec<Id> = core.stage.rooms.keys().copied().collect();
    let mut next: Option<u64> = None;
    for ch in channels {
        let recipients = connections(core, ch);
        let Some(room) = core.stage.rooms.get_mut(&ch) else {
            continue;
        };
        let mut failed = vec![];
        if !room.dirty.is_empty() || !room.gone.is_empty() {
            let dirty = std::mem::take(&mut room.dirty);
            let removed: Vec<Id> = std::mem::take(&mut room.gone).into_iter().collect();
            let room = &core.stage.rooms[&ch];
            // Two views, each encoded once: moderators', and everyone else's.
            // Someone whose own entry changed gets theirs with their own details.
            let view = |viewer: Id, moderator: bool| {
                VoiceEvent::Stage(StageEvent::Members {
                    channel_id: ch,
                    upserts: dirty
                        .iter()
                        .filter_map(|u| room.entry(*u, viewer, moderator))
                        .collect(),
                    removed: removed.clone(),
                })
            };
            let encode = |e: &VoiceEvent| sticktalk_sfu::encode_event(0, e).ok();
            let public = encode(&view(Id(0), false));
            let moderators = encode(&view(Id(0), true));
            let mut sends = vec![];
            for (key, viewer) in &recipients {
                let moderator = room.members.get(viewer).is_some_and(|m| m.moderator);
                let payload = if moderator {
                    moderators.clone()
                } else if dirty.contains(viewer) {
                    encode(&view(*viewer, false))
                } else {
                    public.clone()
                };
                sends.push((key.clone(), payload));
            }
            for (key, payload) in sends {
                let sent =
                    payload.is_some_and(|p| voice::send_payload(core, &key, p, false).is_ok());
                if !sent {
                    failed.push(key);
                }
            }
        }
        let Some(room) = core.stage.rooms.get_mut(&ch) else {
            continue;
        };
        room.resync.extend(failed);
        let resync: Vec<Vec<u8>> = std::mem::take(&mut room.resync).into_iter().collect();
        for key in resync {
            let Some(viewer) = recipients.iter().find(|r| r.0 == key).map(|r| r.1) else {
                continue;
            };
            let roster = core.stage.rooms[&ch].roster(ch, viewer);
            if voice::send(
                core,
                &key,
                0,
                &VoiceEvent::Stage(StageEvent::Roster(roster)),
            )
            .is_err()
            {
                if let Some(room) = core.stage.rooms.get_mut(&ch) {
                    room.resync.insert(key);
                }
            }
        }
        let Some(room) = core.stage.rooms.get_mut(&ch) else {
            continue;
        };
        if !room.resync.is_empty() {
            next = Some(next.map_or(now + FLUSH_MS, |n: u64| n.min(now + FLUSH_MS)));
        }
        if room.counts_dirty && room.live.is_some() {
            if now >= room.counts_at + COUNT_MS {
                room.counts_dirty = false;
                room.counts_at = now;
                let listening = room.listening();
                if let Some(live) = room.live.as_mut() {
                    live.peak = live.peak.max(listening);
                }
                if let Some(instance) = room.instance(ch) {
                    announce(core, c, ch, StageEvent::Live(instance)).await?;
                }
            } else {
                let at = room.counts_at + COUNT_MS;
                next = Some(next.map_or(at, |n| n.min(at)));
            }
        } else {
            room.counts_dirty = false;
        }
    }
    if let Some(at) = next {
        core.stage.due = Some(at);
    }
    Ok(())
}

/// One task, woken only when a Stage changed.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = app.0.core.lock().await.stage.wake.clone();
        let stop = app.0.shutdown.clone();
        loop {
            let due = app.0.core.lock().await.stage.due;
            let wait = async {
                match due {
                    Some(at) => {
                        let ms = at.saturating_sub(now());
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
            let mut core = app.0.core.lock().await;
            if core.stage.due.is_some_and(|at| at <= now()) {
                if let Ok(mut c) = app.0.pool.acquire().await {
                    if let Err(error) = flush(&mut core, &mut c).await {
                        tracing::debug!(code = ?error.0.code, "stage flush failed");
                    }
                }
            }
        }
    });
}

/// Every live Stage `u` can see, to one connection (after Ready and Resumed).
pub(crate) async fn snapshot(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    u: Id,
) -> Result<()> {
    let live: Vec<(Id, StageInstance)> = core
        .stage
        .rooms
        .iter()
        .filter_map(|(ch, r)| r.instance(*ch).map(|i| (*ch, i)))
        .collect();
    let mut stages = vec![];
    for (ch, instance) in live {
        if db::access(c, u, ch, Permissions::VIEW_CHANNEL)
            .await
            .is_ok()
        {
            stages.push(instance);
        }
    }
    voice::send(
        core,
        key,
        0,
        &VoiceEvent::Stage(StageEvent::Snapshot { stages }),
    )
}

/// Roles or overwrites changed: moderators are recomputed in one batched
/// evaluation per Stage (`capacity::members_with`).
pub(crate) async fn reconcile(core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let rooms: Vec<(Id, Id)> = core
        .stage
        .rooms
        .iter()
        .map(|(ch, r)| (*ch, r.guild))
        .collect();
    for (ch, guild) in rooms {
        let moderators: BTreeSet<Id> = crate::capacity::members_with(
            c,
            guild,
            Some(ch),
            MODERATOR | Permissions::VIEW_CHANNEL,
        )
        .await?
        .into_iter()
        .collect();
        let recipients = connections(core, ch);
        let Some(room) = core.stage.rooms.get_mut(&ch) else {
            continue;
        };
        let changed: Vec<Id> = room
            .members
            .iter_mut()
            .filter_map(|(u, m)| {
                let now = moderators.contains(u);
                (m.moderator != now).then(|| {
                    m.moderator = now;
                    *u
                })
            })
            .collect();
        for u in changed {
            room.changed(u);
            // Their view of hands and invitations changed: the whole roster.
            for (key, viewer) in &recipients {
                if *viewer == u {
                    room.resync.insert(key.clone());
                }
            }
        }
    }
    touch(core);
    Ok(())
}

/// Speakers may hold at most `MAX_STAGE_SPEAKERS` people and the SFU's room
/// cap of publishing sessions.
fn room_to_speak(core: &Core, ch: Id, u: Id) -> Result<()> {
    let room = &core.stage.rooms[&ch];
    let sessions = core
        .voice
        .values()
        .filter(|e| {
            e.state.channel_id == ch && (!e.listener || e.state.participant.session.user_id == u)
        })
        .count();
    require(
        room.speakers.len() < MAX_STAGE_SPEAKERS && sessions <= voice::MAX_ROOM_SESSIONS,
        ErrorCode::LimitExceeded,
    )
}

/// Moves `u` onto the Stage or into the audience, with every device.
async fn set_role(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    u: Id,
    speaker: bool,
) -> Result<()> {
    if let Some(room) = core.stage.rooms.get_mut(&ch) {
        if speaker {
            room.speakers.insert(u);
            if let Some(live) = room.live.as_mut() {
                live.spoke.insert(u);
            }
        } else {
            room.speakers.remove(&u);
        }
        room.hands.retain(|h| h.0 != u);
        room.invited.remove(&u);
        room.changed(u);
    }
    let sessions: Vec<VoiceSession> = core
        .voice
        .iter()
        .filter(|(s, e)| s.user_id == u && e.state.channel_id == ch)
        .map(|(s, _)| *s)
        .collect();
    for session in sessions {
        Box::pin(voice::restage(app, core, c, session, !speaker)).await?;
    }
    touch(core);
    Ok(())
}

fn on_stage(core: &Core, ch: Id, u: Id) -> bool {
    core.stage
        .rooms
        .get(&ch)
        .is_some_and(|r| r.members.contains_key(&u))
}
fn live_room(core: &mut Core, ch: Id) -> Result<&mut Room> {
    core.stage
        .rooms
        .get_mut(&ch)
        .filter(|r| r.live.is_some())
        .ok_or_else(|| refused(ErrorCode::InvalidRequest, "This Stage isn't live."))
}
async fn moderator(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<()> {
    db::access(c, u, ch, MODERATOR).await.map(|_| ())
}

/// `VoiceSignal::Stage`, from an authenticated connection.
pub(crate) async fn signal(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    signal: StageSignal,
) -> Result<()> {
    core.rate(format!("stage:{}", u.0), 60., 10.)?;
    match signal {
        StageSignal::Start {
            channel_id: ch,
            topic,
            privacy_note,
            event_id,
        } => {
            let (_, guild) = stage_channel(c, u, ch).await?;
            moderator(c, u, ch).await?;
            let topic = text(&topic, MAX_STAGE_TOPIC_CHARS)?;
            crate::moderation::coverage::name_check(core, c, u, guild, "stage topic", &topic)
                .await?;
            let note = note(privacy_note)?;
            require(
                !core.stage.rooms.get(&ch).is_some_and(|r| r.live.is_some()),
                ErrorCode::Conflict,
            )?;
            let event_id = Box::pin(crate::schedule::stage_starting(
                core, c, guild, ch, event_id, u,
            ))
            .await?;
            audit(c, guild, u, AuditAction::StageStart, ch, &topic, vec![]).await?;
            let room = core
                .stage
                .rooms
                .entry(ch)
                .or_insert_with(|| Room::new(guild));
            room.live = Some(Live {
                topic,
                note,
                started_by: u,
                started_at: now(),
                event_id,
                peak: 0,
                spoke: BTreeSet::new(),
            });
            room.recent.clear();
            // The starter speaks, whether they joined before starting or join after.
            room.speakers.insert(u);
            room.counts_at = now();
            let listening = room.listening();
            room.live.as_mut().unwrap().peak = listening;
            let instance = room.instance(ch);
            let keys: Vec<Vec<u8>> = connections(core, ch).into_iter().map(|k| k.0).collect();
            core.stage.rooms.get_mut(&ch).unwrap().resync.extend(keys);
            if on_stage(core, ch, u) {
                set_role(app, core, c, ch, u, true).await?;
            }
            if let Some(instance) = instance {
                announce(core, c, ch, StageEvent::Live(instance)).await?;
            }
            Box::pin(crate::schedule::publish_channel(core, c, ch)).await?;
        }
        StageSignal::Update {
            channel_id: ch,
            topic,
            privacy_note,
        } => {
            let (_, guild) = stage_channel(c, u, ch).await?;
            moderator(c, u, ch).await?;
            let topic = text(&topic, MAX_STAGE_TOPIC_CHARS)?;
            crate::moderation::coverage::name_check(core, c, u, guild, "stage topic", &topic)
                .await?;
            let note = note(privacy_note)?;
            let room = live_room(core, ch)?;
            let live = room.live.as_mut().unwrap();
            let changes = vec![
                ("topic", Some(live.topic.clone()), Some(topic.clone())),
                ("privacy_note", live.note.clone(), note.clone()),
            ];
            live.topic = topic.clone();
            live.note = note;
            let instance = room.instance(ch);
            audit(c, guild, u, AuditAction::StageUpdate, ch, &topic, changes).await?;
            if let Some(instance) = instance {
                announce(core, c, ch, StageEvent::Live(instance)).await?;
            }
        }
        StageSignal::End { channel_id: ch } => {
            stage_channel(c, u, ch).await?;
            moderator(c, u, ch).await?;
            live_room(core, ch)?;
            end(app, core, c, ch, Some(u)).await?;
        }
        StageSignal::RaiseHand {
            channel_id: ch,
            raised,
        } => {
            require(on_stage(core, ch, u), ErrorCode::Forbidden)?;
            if raised {
                db::access(c, u, ch, Permissions::REQUEST_TO_SPEAK).await?;
            }
            let room = live_room(core, ch)?;
            require(!room.speakers.contains(&u), ErrorCode::InvalidRequest)?;
            let has = room.hands.iter().any(|h| h.0 == u);
            if raised && !has {
                require(
                    room.hands.len() < MAX_STAGE_REQUESTS,
                    ErrorCode::LimitExceeded,
                )?;
                room.hands.push((u, now()));
                room.changed(u);
            } else if !raised && has {
                room.hands.retain(|h| h.0 != u);
                room.changed(u);
            }
            touch(core);
        }
        StageSignal::Accept {
            channel_id: ch,
            user_id: v,
        } => {
            moderator(c, u, ch).await?;
            let room = live_room(core, ch)?;
            require(
                room.members.contains_key(&v) && room.hands.iter().any(|h| h.0 == v),
                ErrorCode::NotFound,
            )?;
            room_to_speak(core, ch, v)?;
            set_role(app, core, c, ch, v, true).await?;
        }
        StageSignal::Dismiss {
            channel_id: ch,
            user_id: v,
        } => {
            moderator(c, u, ch).await?;
            let room = live_room(core, ch)?;
            require(room.hands.iter().any(|h| h.0 == v), ErrorCode::NotFound)?;
            room.hands.retain(|h| h.0 != v);
            room.changed(v);
            touch(core);
        }
        StageSignal::Invite {
            channel_id: ch,
            user_id: v,
        } => {
            moderator(c, u, ch).await?;
            let room = live_room(core, ch)?;
            require(room.members.contains_key(&v), ErrorCode::NotFound)?;
            require(!room.speakers.contains(&v), ErrorCode::InvalidRequest)?;
            room.invited.insert(v);
            room.changed(v);
            touch(core);
        }
        StageSignal::AnswerInvite {
            channel_id: ch,
            accept,
        } => {
            let room = live_room(core, ch)?;
            require(room.invited.remove(&u), ErrorCode::NotFound)?;
            room.changed(u);
            if accept {
                if let Err(e) = room_to_speak(core, ch, u) {
                    touch(core);
                    return Err(e);
                }
                set_role(app, core, c, ch, u, true).await?;
            } else {
                touch(core);
            }
        }
        StageSignal::ToAudience {
            channel_id: ch,
            user_id: v,
        } => {
            if v != u {
                moderator(c, u, ch).await?;
            }
            let room = live_room(core, ch)?;
            require(room.speakers.contains(&v), ErrorCode::InvalidRequest)?;
            set_role(app, core, c, ch, v, false).await?;
        }
        StageSignal::BecomeSpeaker { channel_id: ch } => {
            moderator(c, u, ch).await?;
            require(on_stage(core, ch, u), ErrorCode::Forbidden)?;
            let room = live_room(core, ch)?;
            require(!room.speakers.contains(&u), ErrorCode::InvalidRequest)?;
            room_to_speak(core, ch, u)?;
            set_role(app, core, c, ch, u, true).await?;
        }
    }
    Ok(())
}

async fn audit(
    c: &mut SqliteConnection,
    guild: Id,
    actor: Id,
    action: AuditAction,
    ch: Id,
    topic: &str,
    changes: Vec<(&str, Option<String>, Option<String>)>,
) -> Result<()> {
    use crate::moderation::audit::Entry;
    let mut entry = Entry::new(guild, action, AuditTarget::Channel, Some(ch)).named(topic);
    for (field, before, after) in changes {
        entry = entry.change(field, before, after);
    }
    let mut tx = c.begin().await?;
    crate::moderation::audit::write(&mut tx, actor, entry).await?;
    tx.commit().await?;
    Ok(())
}

/// Ends a live Stage: the record in its chat, everyone out, the channel list
/// told, and its scheduled event (if any) ended. `actor` is None when the
/// last person left.
pub(crate) async fn end(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    actor: Option<Id>,
) -> Result<()> {
    let Some(room) = core.stage.rooms.get_mut(&ch) else {
        return Ok(());
    };
    let Some(live) = room.live.take() else {
        return Ok(());
    };
    let guild = room.guild;
    let summary = StageSummary {
        topic: live.topic.clone(),
        started_at: live.started_at,
        ended_at: now().max(live.started_at),
        peak_listeners: live.peak.max(room.listening()),
        speakers: live.spoke.len() as u32,
    };
    if let Some(actor) = actor {
        audit(
            c,
            guild,
            actor,
            AuditAction::StageEnd,
            ch,
            &live.topic,
            vec![],
        )
        .await?;
    }
    Box::pin(record(core, c, ch, live.started_by, &summary)).await?;
    let sessions: Vec<VoiceSession> = core
        .voice
        .iter()
        .filter(|(_, e)| e.state.channel_id == ch)
        .map(|(s, _)| *s)
        .collect();
    for session in sessions {
        Box::pin(voice::remove(app, core, c, session)).await?;
    }
    core.stage.rooms.remove(&ch);
    announce(
        core,
        c,
        ch,
        StageEvent::Ended {
            guild_id: guild,
            channel_id: ch,
        },
    )
    .await?;
    Box::pin(crate::schedule::stage_ended(core, c, ch, live.event_id)).await
}

/// "Stage ended" in the channel's chat, authored by whoever started it.
async fn record(
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    author: Id,
    summary: &StageSummary,
) -> Result<()> {
    let m = core.id().await?;
    let payload = rmp_serde::to_vec_named(summary).map_err(|_| Error::new(ErrorCode::Internal))?;
    {
        let mut tx = c.begin().await?;
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)",
        )
        .bind(db::id(m)?)
        .bind(db::id(ch)?)
        .bind(db::id(author)?)
        .bind(describe(summary))
        .bind(summary.ended_at as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO stage_messages(message_id,kind,payload) VALUES(?,'record',?)")
            .bind(db::id(m)?)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
        core.persist_id(&mut tx).await?;
        db::recount(&mut tx, ch).await?;
        tx.commit().await?;
    }
    let message = db::message(c, author, ch, m, false).await?;
    let recipients = db::audience(c, ch).await?;
    let author = db::user(c, author).await?;
    core.emit(&recipients, Event::MessageCreated { message, author })
}

/// The text of a Stage's record, for clients that do not read `extra.stage`.
pub(crate) fn describe(s: &StageSummary) -> String {
    let minutes = s.duration_ms() / 60_000;
    let time = match minutes {
        0 => "less than a minute".to_owned(),
        1 => "a minute".to_owned(),
        n if n < 60 => format!("{n} minutes"),
        n => format!(
            "{} hour{} {} minute{}",
            n / 60,
            if n / 60 == 1 { "" } else { "s" },
            n % 60,
            if n % 60 == 1 { "" } else { "s" }
        ),
    };
    format!(
        "The Stage “{}” ended after {time}. {} {} listening at most.",
        s.topic,
        s.peak_listeners,
        if s.peak_listeners == 1 {
            "person was"
        } else {
            "people were"
        }
    )
}

/// `extra.stage` on a message: a Stage record or a shared event's card.
pub(crate) async fn decorate(c: &mut SqliteConnection, m: &mut Message) -> Result<()> {
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT kind,payload FROM stage_messages WHERE message_id=?")
            .bind(db::id(m.id)?)
            .fetch_optional(&mut *c)
            .await?;
    let Some((kind, payload)) = row else {
        return Ok(());
    };
    let mut extra = StageMessage::default();
    match kind.as_str() {
        "record" => extra.record = rmp_serde::from_slice(&payload).ok(),
        _ => extra.event = rmp_serde::from_slice(&payload).ok(),
    }
    m.extra.stage = Some(Box::new(extra));
    Ok(())
}

/// Whether `u` is a speaker on Stage `ch` (the soundboard is for speakers).
pub(crate) fn is_live(core: &Core, ch: Id) -> bool {
    core.stage.rooms.get(&ch).is_some_and(|r| r.live.is_some())
}

/// How many people are on a Stage now (for tests and events).
#[cfg(test)]
pub(crate) fn counts(core: &Core, ch: Id) -> Option<(u32, u32)> {
    core.stage
        .rooms
        .get(&ch)
        .map(|r| (r.speaking(), r.listening()))
}
