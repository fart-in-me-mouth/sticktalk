//! Shared activities (`docs/features/activities.md`, `docs/ACTIVITIES.md`).
//!
//! A session belongs to one call and lives only in memory: its participants,
//! its bounded key-value document and the files shared into it. The server
//! relays writes and ephemeral messages to the participants, runs the rules
//! of the built-in games (`games.rs`) so a tampered client cannot cheat, and
//! forgets everything when the last participant leaves or the call ends.
//! Only invite cards (`cards.rs`) and a short result outlive a session.
//!
//! Someone who is not in an activity costs nothing: no timer, no task, no
//! memory beyond the call's one optional session.
use crate::{
    db,
    error::{require, Error, Result},
    state::{now, App, Core},
    voice,
};
use sqlx::SqliteConnection;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use sticktalk_proto::*;

mod cards;
pub(crate) mod catalog;
pub(crate) mod frames;
mod games;
#[cfg(test)]
mod tests;

pub(crate) use cards::decorate;
pub(crate) use catalog::{run_admin, ActivityAdmin};

/// Recent text a participant showed in a session, kept for reports.
const RECENT_TEXT: usize = 5;

/// Every live session, by the call it belongs to.
#[derive(Default)]
pub(crate) struct Activities {
    sessions: HashMap<Id, Session>,
}

struct Participant {
    pid: u16,
    session: VoiceSession,
    key: Vec<u8>,
    name: String,
    avatar: Option<Id>,
    joined_at: u64,
    said: VecDeque<String>,
}

struct Value {
    text: String,
    version: u64,
    by: u16,
}

struct Session {
    id: Id,
    channel: Id,
    guild: Option<Id>,
    info: ActivityInfo,
    started_by: Id,
    started_at: u64,
    host: u16,
    shared: bool,
    next_pid: u16,
    participants: Vec<Participant>,
    doc: BTreeMap<String, Value>,
    bytes: usize,
    version: u64,
    media: Vec<Id>,
    /// Invite cards: (message, conversation, who posted it).
    cards: Vec<(Id, Id, Id)>,
    people: BTreeSet<Id>,
    result: Option<String>,
    engine: Option<Box<dyn games::Engine>>,
}

impl Session {
    fn participant(&self, voice: VoiceSession) -> Option<&Participant> {
        self.participants.iter().find(|p| p.session == voice)
    }
    fn pid(&self, voice: VoiceSession) -> Option<u16> {
        self.participant(voice).map(|p| p.pid)
    }
    fn view(&self, you: Option<u16>) -> ActivitySession {
        ActivitySession {
            session_id: self.id,
            channel_id: self.channel,
            guild_id: self.guild,
            activity: self.info.clone(),
            started_by: self.started_by,
            started_at: self.started_at,
            host: self.host,
            shared_control: self.shared,
            participants: self
                .participants
                .iter()
                .map(|p| ActivityParticipant {
                    pid: p.pid,
                    session: p.session,
                    name: p.name.clone(),
                    avatar_id: p.avatar,
                    joined_at: p.joined_at,
                })
                .collect(),
            you,
        }
    }
    fn summary(&self) -> ActivitySummary {
        ActivitySummary {
            activity_id: self.info.id.clone(),
            name: self.info.name.clone(),
            started_at: self.started_at,
            ended_at: now(),
            people: self.people.len().min(u16::MAX as usize) as u16,
            result: self.result.clone(),
        }
    }
    /// Apply checked changes; returns what to tell the participants.
    fn apply(&mut self, by: u16, ops: Vec<(String, Option<String>)>) -> Vec<ActivityEntry> {
        let mut out = Vec::with_capacity(ops.len());
        for (key, value) in ops {
            self.version += 1;
            if let Some(old) = self.doc.remove(&key) {
                self.bytes -= key.len() + old.text.len();
            }
            match value {
                Some(text) => {
                    self.bytes += key.len() + text.len();
                    self.doc.insert(
                        key.clone(),
                        Value {
                            text: text.clone(),
                            version: self.version,
                            by,
                        },
                    );
                    out.push(ActivityEntry {
                        key,
                        value: Some(text),
                        version: self.version,
                        by,
                    });
                }
                None => out.push(ActivityEntry {
                    key,
                    value: None,
                    version: self.version,
                    by,
                }),
            }
        }
        out
    }
    /// Whether these changes fit the document's limits.
    fn fits(&self, ops: &[(String, Option<String>)]) -> bool {
        let mut bytes = self.bytes as i64;
        let mut keys = self.doc.len() as i64;
        // The value each touched key holds after the operations so far.
        let mut touched: BTreeMap<&str, Option<usize>> = BTreeMap::new();
        for (key, value) in ops {
            let current = match touched.get(key.as_str()) {
                Some(len) => *len,
                None => self.doc.get(key).map(|v| v.text.len()),
            };
            if let Some(len) = current {
                bytes -= (key.len() + len) as i64;
                keys -= 1;
            }
            if let Some(v) = value {
                bytes += (key.len() + v.len()) as i64;
                keys += 1;
            }
            touched.insert(key, value.as_ref().map(String::len));
        }
        bytes <= MAX_ACTIVITY_STATE_BYTES as i64 && keys <= MAX_ACTIVITY_KEYS as i64
    }
}

/// Encode once for many recipients.
fn encode(event: ActivityEvent) -> Result<VoicePayload> {
    sticktalk_sfu::encode_event(0, &VoiceEvent::Activities(event))
        .map_err(|_| Error::new(ErrorCode::Internal))
}

/// The joined voice sessions of a call, with their stream keys.
fn call(core: &Core, channel: Id) -> Vec<(VoiceSession, Vec<u8>)> {
    core.voice
        .iter()
        .filter(|(_, e)| e.joined && e.state.channel_id == channel && !e.listener)
        .map(|(s, e)| (*s, e.key().to_vec()))
        .collect()
}

/// Tell everyone in the call about the session (each participant learns its
/// own number). Slow connections miss it; the next change repeats the state.
fn announce(core: &mut Core, channel: Id) {
    let Some(s) = core.activities.sessions.get(&channel) else {
        return;
    };
    let mut out = vec![];
    let shared = encode(ActivityEvent::Session(s.view(None))).ok();
    for (voice, key) in call(core, channel) {
        let payload = match s.pid(voice) {
            Some(you) => encode(ActivityEvent::Session(s.view(Some(you)))).ok(),
            None => shared.clone(),
        };
        if let Some(p) = payload {
            out.push((key, p));
        }
    }
    for (key, payload) in out {
        let _ = voice::send_payload(core, &key, payload, false);
    }
}

/// One event to the participants (except `skip`).
fn tell(core: &mut Core, channel: Id, event: ActivityEvent, skip: Option<u16>, only: &[u16]) {
    let Some(s) = core.activities.sessions.get(&channel) else {
        return;
    };
    let keys: Vec<Vec<u8>> = s
        .participants
        .iter()
        .filter(|p| Some(p.pid) != skip && (only.is_empty() || only.contains(&p.pid)))
        .map(|p| p.key.clone())
        .collect();
    if keys.is_empty() {
        return;
    }
    let Ok(payload) = encode(event) else { return };
    for key in keys {
        let _ = voice::send_payload(core, &key, payload.clone(), false);
    }
}

fn patch(core: &mut Core, channel: Id, entries: Vec<ActivityEntry>) {
    let Some(s) = core.activities.sessions.get(&channel) else {
        return;
    };
    if entries.is_empty() {
        return;
    }
    let event = ActivityEvent::State {
        session_id: s.id,
        reset: false,
        done: true,
        version: s.version,
        server_time: now(),
        entries,
    };
    tell(core, channel, event, None, &[]);
}

/// The whole document to one participant, in parts.
fn snapshot(core: &mut Core, channel: Id, key: &[u8]) {
    let Some(s) = core.activities.sessions.get(&channel) else {
        return;
    };
    let mut chunks: Vec<Vec<ActivityEntry>> = vec![vec![]];
    let mut size = 0;
    for (k, v) in &s.doc {
        let n = k.len() + v.text.len() + 24;
        if size + n > MAX_ACTIVITY_STATE_CHUNK && !chunks.last().unwrap().is_empty() {
            chunks.push(vec![]);
            size = 0;
        }
        size += n;
        chunks.last_mut().unwrap().push(ActivityEntry {
            key: k.clone(),
            value: Some(v.text.clone()),
            version: v.version,
            by: v.by,
        });
    }
    let (id, version, last) = (s.id, s.version, chunks.len() - 1);
    for (i, entries) in chunks.into_iter().enumerate() {
        if let Ok(p) = encode(ActivityEvent::State {
            session_id: id,
            reset: i == 0,
            done: i == last,
            version,
            server_time: now(),
            entries,
        }) {
            let _ = voice::send_payload(core, key, p, false);
        }
    }
}

/// The participant's name in this server (nickname, display name, username).
async fn person(
    c: &mut SqliteConnection,
    guild: Option<Id>,
    u: Id,
) -> Result<(String, Option<Id>)> {
    let row: Option<(String, Option<i64>)> = sqlx::query_as(
        "SELECT COALESCE(n.nickname,u.display_name,u.username),u.avatar_id FROM users u LEFT JOIN nicknames n ON n.guild_id=? AND n.user_id=u.id WHERE u.id=?",
    )
    .bind(db::opt(guild)?)
    .bind(db::id(u)?)
    .fetch_optional(&mut *c)
    .await?;
    let (name, avatar) = row.ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((name, avatar.map(|a| Id(a as u64))))
}

/// Use Activities in a guild voice channel; always in direct and group calls.
async fn may_use(c: &mut SqliteConnection, u: Id, guild: Option<Id>, ch: Id) -> Result<()> {
    if let Some(g) = guild {
        let p = crate::permissions::evaluate(c, u, g, Some(ch)).await?;
        require(
            p.contains(Permissions::USE_ACTIVITIES),
            ErrorCode::Forbidden,
        )?;
    }
    Ok(())
}

/// Strings under a `text` property anywhere in a JSON value: what an
/// activity shows other people as text (`docs/ACTIVITIES.md`).
fn visible_text(v: &serde_json::Value, out: &mut String, depth: u8) {
    if depth > 16 || out.len() > 4096 {
        return;
    }
    match v {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                match (k.as_str(), v) {
                    ("text", serde_json::Value::String(s)) => {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(s);
                    }
                    _ => visible_text(v, out, depth + 1),
                }
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                visible_text(v, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// Parse an activity's JSON text, and screen its visible text.
async fn screen(
    core: &mut Core,
    c: &mut SqliteConnection,
    channel: Id,
    voice: VoiceSession,
    raw: &str,
) -> Result<serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let mut text = String::new();
    visible_text(&value, &mut text, 0);
    if !text.trim().is_empty() {
        let guild = core.activities.sessions.get(&channel).and_then(|s| s.guild);
        if let Some(g) = guild {
            crate::moderation::coverage::activity_text_check(
                core,
                c,
                voice.user_id,
                g,
                channel,
                &text,
            )
            .await?;
        }
        if let Some(p) = core
            .activities
            .sessions
            .get_mut(&channel)
            .and_then(|s| s.participants.iter_mut().find(|p| p.session == voice))
        {
            if p.said.len() >= RECENT_TEXT {
                p.said.pop_front();
            }
            p.said.push_back(text.chars().take(300).collect());
        }
    }
    Ok(value)
}

fn single_line(text: &str, max: usize) -> Result<String> {
    let t = text.trim();
    require(
        !t.is_empty() && t.chars().count() <= max && !t.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(t.to_owned())
}

/// Find the session `id` that `voice` (the caller's session in a call) is in,
/// returning its call channel.
fn session_of(core: &Core, id: Id, voice: Option<VoiceSession>) -> Result<(Id, VoiceSession)> {
    let voice = voice
        .filter(|v| core.voice.get(v).is_some_and(|e| e.joined))
        .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
    let channel = core.voice[&voice].state.channel_id;
    let s = core
        .activities
        .sessions
        .get(&channel)
        .filter(|s| s.id == id)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((s.channel, voice))
}

fn participant_of(
    core: &Core,
    id: Id,
    voice: Option<VoiceSession>,
) -> Result<(Id, VoiceSession, u16)> {
    let (channel, voice) = session_of(core, id, voice)?;
    let pid = core.activities.sessions[&channel]
        .pid(voice)
        .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
    Ok((channel, voice, pid))
}

/// Run what a game's rules decided.
fn effects(core: &mut Core, channel: Id, fx: games::Effects) {
    let Some(s) = core.activities.sessions.get_mut(&channel) else {
        return;
    };
    let mut ops = fx.writes;
    if let Some(prefix) = &fx.clear {
        for k in s.doc.keys().filter(|k| k.starts_with(prefix.as_str())) {
            if !ops.iter().any(|(o, _)| o == k) {
                ops.insert(0, (k.clone(), None));
            }
        }
    }
    if let Some(r) = fx.result {
        s.result = Some(r);
    }
    // The games' own state is small; keep it within the limits regardless.
    ops.retain(|(k, v)| {
        v.as_ref()
            .is_none_or(|v| v.len() <= MAX_ACTIVITY_VALUE_BYTES)
            && valid_activity_key(k)
    });
    let entries = if s.fits(&ops) {
        s.apply(0, ops)
    } else {
        vec![]
    };
    let id = s.id;
    patch(core, channel, entries);
    for (to, data) in fx.private {
        tell(
            core,
            channel,
            ActivityEvent::Message {
                session_id: id,
                from: 0,
                data,
            },
            None,
            &[to],
        );
    }
}

/// `VoiceSignal::Activities`, from the connection whose call session is `voice`.
pub(crate) async fn signal(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    user: Id,
    voice: Option<VoiceSession>,
    signal: ActivitySignal,
) -> Result<()> {
    use ActivitySignal::*;
    match signal {
        Launch {
            channel_id,
            activity_id,
        } => {
            core.rate(format!("activity-launch:{}", user.0), 6., 2.)?;
            let voice = voice
                .filter(|v| core.voice.get(v).is_some_and(|e| e.joined && !e.listener))
                .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
            let entry = &core.voice[&voice];
            require(entry.state.channel_id == channel_id, ErrorCode::Forbidden)?;
            // A Stage is one-to-many audio: activities are for calls.
            require(!entry.stage, ErrorCode::InvalidRequest)?;
            let guild = (!entry.dm()).then_some(entry.state.guild_id);
            may_use(c, user, guild, channel_id).await?;
            require(
                !core.activities.sessions.contains_key(&channel_id),
                ErrorCode::Conflict,
            )?;
            let info = catalog::available(c, guild, &activity_id).await?;
            let (name, avatar) = person(c, guild, user).await?;
            let id = core.id().await?;
            core.persist_id(&mut *c).await?;
            if let Some(g) = guild {
                cards::audit(
                    c,
                    g,
                    user,
                    AuditAction::ActivityStart,
                    channel_id,
                    &info.name,
                    None,
                )
                .await?;
            }
            let mut engine = games::engine(&info.id);
            let mut s = Session {
                id,
                channel: channel_id,
                guild,
                info,
                started_by: user,
                started_at: now(),
                host: 1,
                shared: false,
                next_pid: 2,
                participants: vec![Participant {
                    pid: 1,
                    session: voice,
                    key: key.to_vec(),
                    name: name.clone(),
                    avatar,
                    joined_at: now(),
                    said: VecDeque::new(),
                }],
                doc: BTreeMap::new(),
                bytes: 0,
                version: 0,
                media: vec![],
                cards: vec![],
                people: BTreeSet::from([user]),
                result: None,
                engine: None,
            };
            let fx = engine.as_mut().map(|e| e.joined(1, &name));
            s.engine = engine;
            core.activities.sessions.insert(channel_id, s);
            announce(core, channel_id);
            snapshot(core, channel_id, key);
            if let Some(fx) = fx {
                effects(core, channel_id, fx);
            }
        }
        Join { session_id } => {
            core.rate(format!("activity-join:{}", user.0), 30., 6.)?;
            let (channel, voice) = session_of(core, session_id, voice)?;
            let s = &core.activities.sessions[&channel];
            require(s.pid(voice).is_none(), ErrorCode::Conflict)?;
            require(
                s.participants.len() < MAX_ACTIVITY_PARTICIPANTS,
                ErrorCode::LimitExceeded,
            )?;
            let guild = s.guild;
            may_use(c, user, guild, channel).await?;
            let (name, avatar) = person(c, guild, user).await?;
            let s = core.activities.sessions.get_mut(&channel).unwrap();
            let pid = s.next_pid;
            s.next_pid = s.next_pid.wrapping_add(1).max(1);
            s.participants.push(Participant {
                pid,
                session: voice,
                key: key.to_vec(),
                name: name.clone(),
                avatar,
                joined_at: now(),
                said: VecDeque::new(),
            });
            s.people.insert(user);
            if s.host == 0 {
                s.host = pid;
            }
            let fx = s.engine.as_mut().map(|e| e.joined(pid, &name));
            announce(core, channel);
            snapshot(core, channel, key);
            if let Some(fx) = fx {
                effects(core, channel, fx);
            }
            cards::refresh(core, c, channel).await?;
        }
        Leave { session_id } => {
            let (channel, voice) = session_of(core, session_id, voice)?;
            leave(app, core, c, channel, voice).await?;
        }
        End { session_id } => {
            core.rate(format!("activity-end:{}", user.0), 12., 4.)?;
            // The host, or a moderator of the channel from anywhere.
            let found = core
                .activities
                .sessions
                .values()
                .find(|s| s.id == session_id)
                .map(|s| {
                    (
                        s.channel,
                        s.guild,
                        s.participants
                            .iter()
                            .find(|p| p.pid == s.host)
                            .map(|p| p.session),
                    )
                });
            let (channel, guild, host) = found.ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            let is_host = voice.is_some() && host == voice;
            if !is_host {
                let g = guild.ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
                let rights = crate::permissions::evaluate(c, user, g, Some(channel)).await?;
                require(
                    rights.contains(Permissions::VIEW_CHANNEL)
                        && rights.contains(Permissions::MOVE_MEMBERS),
                    if rights.contains(Permissions::VIEW_CHANNEL) {
                        ErrorCode::Forbidden
                    } else {
                        ErrorCode::NotFound
                    },
                )?;
            }
            end(core, c, channel, Some(user)).await?;
        }
        SetHost { session_id, pid } => {
            let (channel, _, me) = participant_of(core, session_id, voice)?;
            let s = core.activities.sessions.get_mut(&channel).unwrap();
            require(s.host == me, ErrorCode::Forbidden)?;
            require(
                s.participants.iter().any(|p| p.pid == pid),
                ErrorCode::NotFound,
            )?;
            s.host = pid;
            announce(core, channel);
        }
        SetShared { session_id, shared } => {
            let (channel, _, me) = participant_of(core, session_id, voice)?;
            let s = core.activities.sessions.get_mut(&channel).unwrap();
            require(s.host == me, ErrorCode::Forbidden)?;
            s.shared = shared;
            announce(core, channel);
        }
        Write { session_id, ops } => {
            let (channel, voice, me) = participant_of(core, session_id, voice)?;
            core.rate(
                format!("activity-write:{}", voice.connection_id.0),
                600.,
                60.,
            )?;
            core.rate(format!("activity-writes:{}", session_id.0), 1800., 120.)?;
            require(
                !ops.is_empty() && ops.len() <= MAX_ACTIVITY_OPS,
                ErrorCode::InvalidRequest,
            )?;
            let mut checked = Vec::with_capacity(ops.len());
            for op in &ops {
                require(valid_activity_key(&op.key), ErrorCode::InvalidRequest)?;
                let s = &core.activities.sessions[&channel];
                let allowed = match s.engine.as_ref() {
                    // A game's rules decide which keys a player may write.
                    Some(e) => e.may_write(me, &op.key),
                    None => {
                        if op.key.starts_with("s/") {
                            false
                        } else if op.key.starts_with("h/") {
                            s.host == me || s.shared
                        } else if let Some(rest) = op.key.strip_prefix("p/") {
                            rest.split('/').next() == Some(me.to_string().as_str())
                        } else {
                            true
                        }
                    }
                };
                require(allowed, ErrorCode::Forbidden)?;
                if let Some(expect) = op.expect {
                    let current = s.doc.get(&op.key).map_or(0, |v| v.version);
                    require(current == expect, ErrorCode::Conflict)?;
                }
                if let Some(v) = &op.value {
                    require(
                        v.len() <= MAX_ACTIVITY_VALUE_BYTES,
                        ErrorCode::PayloadTooLarge,
                    )?;
                }
            }
            for op in ops {
                if let Some(v) = &op.value {
                    screen(core, c, channel, voice, v).await?;
                }
                checked.push((op.key, op.value));
            }
            let s = core
                .activities
                .sessions
                .get_mut(&channel)
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            require(s.fits(&checked), ErrorCode::LimitExceeded)?;
            let entries = s.apply(me, checked);
            patch(core, channel, entries);
        }
        Send {
            session_id,
            to,
            data,
        } => {
            let (channel, voice, me) = participant_of(core, session_id, voice)?;
            core.rate(
                format!("activity-send:{}", voice.connection_id.0),
                1200.,
                40.,
            )?;
            core.rate(format!("activity-sends:{}", session_id.0), 3600., 120.)?;
            require(
                data.len() <= MAX_ACTIVITY_MESSAGE_BYTES && to.len() <= MAX_ACTIVITY_PARTICIPANTS,
                ErrorCode::PayloadTooLarge,
            )?;
            {
                let s = &core.activities.sessions[&channel];
                require(
                    to.iter()
                        .all(|p| s.participants.iter().any(|q| q.pid == *p)),
                    ErrorCode::NotFound,
                )?;
            }
            screen(core, c, channel, voice, &data).await?;
            tell(
                core,
                channel,
                ActivityEvent::Message {
                    session_id,
                    from: me,
                    data,
                },
                Some(me),
                &to,
            );
        }
        Action { session_id, data } => {
            let (channel, voice, me) = participant_of(core, session_id, voice)?;
            core.rate(
                format!("activity-action:{}", voice.connection_id.0),
                240.,
                20.,
            )?;
            require(
                data.len() <= MAX_ACTIVITY_MESSAGE_BYTES,
                ErrorCode::PayloadTooLarge,
            )?;
            require(
                core.activities.sessions[&channel].engine.is_some(),
                ErrorCode::InvalidRequest,
            )?;
            let value = screen(core, c, channel, voice, &data).await?;
            let s = core.activities.sessions.get_mut(&channel).unwrap();
            let name = s
                .participants
                .iter()
                .find(|p| p.pid == me)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let fx = s
                .engine
                .as_mut()
                .unwrap()
                .action(me, &name, &value)
                .map_err(|why| {
                    let mut e = Error::new(ErrorCode::InvalidRequest);
                    e.0.message = why.into();
                    e
                })?;
            effects(core, channel, fx);
        }
        ShareMedia {
            session_id,
            attachment_id,
        } => {
            let (channel, voice, me) = participant_of(core, session_id, voice)?;
            core.rate(format!("activity-media:{}", user.0), 30., 6.)?;
            let s = &core.activities.sessions[&channel];
            if s.media.contains(&attachment_id) {
                return Ok(());
            }
            require(s.media.len() < MAX_ACTIVITY_MEDIA, ErrorCode::LimitExceeded)?;
            let item = catalog::shareable(c, voice.user_id, attachment_id).await?;
            let value = serde_json::json!({
                "name": item.filename,
                "type": item.content_type,
                "size": item.byte_size,
                "by": me,
            })
            .to_string();
            let s = core.activities.sessions.get_mut(&channel).unwrap();
            s.media.push(attachment_id);
            let op = vec![(format!("s/media/{}", attachment_id.0), Some(value))];
            let entries = if s.fits(&op) { s.apply(0, op) } else { vec![] };
            patch(core, channel, entries);
        }
        SaveResult { session_id, text } => {
            let (channel, voice, me) = participant_of(core, session_id, voice)?;
            core.rate(format!("activity-result:{}", user.0), 10., 3.)?;
            let s = &core.activities.sessions[&channel];
            require(s.host == me && s.engine.is_none(), ErrorCode::Forbidden)?;
            let text = single_line(&text, MAX_ACTIVITY_RESULT_CHARS)?;
            screen(
                core,
                c,
                channel,
                voice,
                &serde_json::json!({ "text": text }).to_string(),
            )
            .await?;
            core.activities.sessions.get_mut(&channel).unwrap().result = Some(text);
        }
        Invite {
            session_id,
            channel_id,
        } => {
            let (channel, voice, _) = participant_of(core, session_id, voice)?;
            core.rate(format!("activity-invite:{}", user.0), 6., 2.)?;
            cards::post(core, c, channel, voice.user_id, channel_id).await?;
        }
    }
    Ok(())
}

/// A participant leaves (or its call session ended).
async fn leave(
    _app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    channel: Id,
    voice: VoiceSession,
) -> Result<()> {
    let Some(s) = core.activities.sessions.get_mut(&channel) else {
        return Ok(());
    };
    let Some(i) = s.participants.iter().position(|p| p.session == voice) else {
        return Ok(());
    };
    let gone = s.participants.remove(i);
    if s.participants.is_empty() {
        return end(core, c, channel, None).await;
    }
    if s.host == gone.pid {
        s.host = s.participants[0].pid;
    }
    let fx = s.engine.as_mut().map(|e| e.left(gone.pid));
    announce(core, channel);
    if let Some(fx) = fx {
        effects(core, channel, fx);
    }
    cards::refresh(core, c, channel).await
}

/// Ends a session: everyone in the call is told, the invite cards show the
/// summary, the result is kept, the audit log notes it (guild channels).
pub(crate) async fn end(
    core: &mut Core,
    c: &mut SqliteConnection,
    channel: Id,
    actor: Option<Id>,
) -> Result<()> {
    let Some(s) = core.activities.sessions.remove(&channel) else {
        return Ok(());
    };
    let summary = s.summary();
    if let Some(g) = s.guild {
        let reason = if actor.is_none() {
            Some("Everyone left".to_owned())
        } else {
            None
        };
        cards::audit(
            c,
            g,
            actor.unwrap_or(s.started_by),
            AuditAction::ActivityEnd,
            channel,
            &s.info.name,
            reason,
        )
        .await?;
    }
    cards::ended(core, c, &s.cards, &s.info, &summary).await?;
    if let Some(text) = &summary.result {
        cards::keep_result(c, channel, &summary.name, text).await?;
    }
    let keys: Vec<Vec<u8>> = call(core, channel).into_iter().map(|(_, k)| k).collect();
    // Participants who already left the call still hear that it ended.
    let mut keys = keys;
    for p in &s.participants {
        if !keys.contains(&p.key) {
            keys.push(p.key.clone());
        }
    }
    if let Ok(payload) = encode(ActivityEvent::Ended {
        session_id: s.id,
        channel_id: channel,
        summary,
    }) {
        for key in keys {
            let _ = voice::send_payload(core, &key, payload.clone(), false);
        }
    }
    Ok(())
}

/// A call session's media connected: show it the call's activity, if any.
pub(crate) fn joined(core: &mut Core, voice: VoiceSession) {
    let Some(entry) = core.voice.get(&voice) else {
        return;
    };
    let (channel, key) = (entry.state.channel_id, entry.key().to_vec());
    let Some(s) = core.activities.sessions.get(&channel) else {
        return;
    };
    if let Ok(p) = encode(ActivityEvent::Session(s.view(None))) {
        let _ = voice::send_payload(core, &key, p, false);
    }
}

/// A call session ended (`voice::remove`): it leaves the activity, and the
/// activity ends with the call.
pub(crate) async fn voice_left(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    voice: VoiceSession,
    channel: Id,
) -> Result<()> {
    if !core.activities.sessions.contains_key(&channel) {
        return Ok(());
    }
    if !core.voice.values().any(|e| e.state.channel_id == channel) {
        return end(core, c, channel, None).await;
    }
    Box::pin(leave(app, core, c, channel, voice)).await
}

/// Rights changed somewhere: participants of guild sessions who lost Use
/// Activities leave them.
pub(crate) async fn reconcile(app: &App, core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let mut out = vec![];
    for s in core.activities.sessions.values() {
        let Some(g) = s.guild else { continue };
        for p in &s.participants {
            out.push((s.channel, g, p.session));
        }
    }
    for (channel, g, voice) in out {
        let ok = crate::permissions::evaluate(c, voice.user_id, g, Some(channel))
            .await
            .is_ok_and(|p| p.contains(Permissions::USE_ACTIVITIES));
        if !ok {
            Box::pin(leave(app, core, c, channel, voice)).await?;
        }
    }
    Ok(())
}

/// A file shared into a session this person takes part in (Watch Together):
/// they may stream it while the session lasts.
pub(crate) fn file_access(core: &Core, u: Id, attachment: Id) -> bool {
    core.activities.sessions.values().any(|s| {
        s.media.contains(&attachment) && s.participants.iter().any(|p| p.session.user_id == u)
    })
}

/// A server turned an activity off: its sessions there end.
pub(crate) async fn disabled(
    core: &mut Core,
    c: &mut SqliteConnection,
    guild: Id,
    activity: &str,
    actor: Id,
) -> Result<()> {
    let channels: Vec<Id> = core
        .activities
        .sessions
        .values()
        .filter(|s| s.guild == Some(guild) && s.info.id == activity)
        .map(|s| s.channel)
        .collect();
    for ch in channels {
        end(core, c, ch, Some(actor)).await?;
    }
    Ok(())
}

/// Evidence for a report about `user`'s part in session `session_id`: the
/// reporter must be in that call. Returns the queue, guild and evidence.
pub(crate) async fn report(
    core: &Core,
    c: &mut SqliteConnection,
    reporter: Id,
    channel: Id,
    session_id: Id,
    user: Id,
) -> Result<(Option<Id>, ReportEvidence)> {
    require(reporter != user, ErrorCode::InvalidRequest)?;
    let s = core
        .activities
        .sessions
        .get(&channel)
        .filter(|s| s.id == session_id)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(
        core.voice.values().any(|e| {
            e.state.channel_id == channel && e.state.participant.session.user_id == reporter
        }),
        ErrorCode::NotFound,
    )?;
    let said: Vec<String> = s
        .participants
        .iter()
        .filter(|p| p.session.user_id == user)
        .flat_map(|p| p.said.iter().cloned())
        .collect();
    require(s.people.contains(&user), ErrorCode::NotFound)?;
    let mut e = ReportEvidence {
        user: Some(db::user(c, user).await?),
        content: format!(
            "In {}:\n{}",
            s.info.name,
            if said.is_empty() {
                "(nothing typed in the activity)".to_owned()
            } else {
                said.join("\n")
            }
        )
        .chars()
        .take(MAX_REPORT_EVIDENCE_CHARS)
        .collect(),
        sent_at: Some(now()),
        ..Default::default()
    };
    if let Some(g) = s.guild {
        let ch = db::channel(c, channel).await?;
        e.channel_name = ch.summary.name.clone();
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM guilds WHERE id=?")
            .bind(db::id(g)?)
            .fetch_optional(&mut *c)
            .await?;
        e.guild_name = name;
    }
    Ok((s.guild, e))
}

#[cfg(test)]
pub(crate) fn session_for_test(core: &Core, channel: Id) -> Option<ActivitySession> {
    core.activities.sessions.get(&channel).map(|s| s.view(None))
}
