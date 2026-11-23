//! Direct and group calls. A call lives in a DM or group DM; its media is the
//! ordinary voice path (the SFU room is the conversation's channel ID). This
//! module owns what a guild voice channel does not have: who is being rung,
//! when ringing stops, and the system message a finished call leaves.
//!
//! Nothing here runs while there is no call: the ring timer task waits on a
//! `Notify` with no deadline, and the call table is empty.
use crate::{
    db,
    error::{require, Error, Result},
    state::{now, App, Core},
};
use sqlx::{Connection as _, SqliteConnection};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use sticktalk_proto::*;

/// What a member of a conversation may do in its call. There are no roles in
/// a DM, so nobody can moderate anybody else.
pub(crate) fn rights() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::VIDEO
        | Permissions::STREAM
        | Permissions::USE_VOICE_ACTIVITY
}

#[derive(Default)]
pub(crate) struct Calls {
    /// At most one call per conversation, and only while it has a voice
    /// session, so this is bounded by the voice session cap.
    live: HashMap<Id, Call>,
    /// Wakes the ring timer when a new ring deadline is set.
    wake: Arc<tokio::sync::Notify>,
}

struct Call {
    started_by: Id,
    started_at: u64,
    /// Distinct people whose media connected at some point.
    joined: BTreeSet<Id>,
    /// Who is being rung, and until when.
    ringing: BTreeMap<Id, u64>,
}

impl Calls {
    fn next_deadline(&self) -> Option<u64> {
        self.live
            .values()
            .flat_map(|c| c.ringing.values())
            .min()
            .copied()
    }
    #[cfg(test)]
    pub(crate) fn ringing(&self, ch: Id) -> Vec<Id> {
        self.live
            .get(&ch)
            .map_or(vec![], |c| c.ringing.keys().copied().collect())
    }
    #[cfg(test)]
    pub(crate) fn active(&self, ch: Id) -> bool {
        self.live.contains_key(&ch)
    }
    #[cfg(test)]
    pub(crate) fn age_rings(&mut self, ch: Id) {
        if let Some(c) = self.live.get_mut(&ch) {
            c.ringing.values_mut().for_each(|d| *d = 0);
        }
    }
}

/// Whether `user` may be in this conversation's call. `ch` is already known to
/// be visible to them (they are a member).
pub(crate) async fn admit(c: &mut SqliteConnection, user: Id, ch: &Channel) -> Result<()> {
    match ch.summary.kind {
        ChannelKind::Direct => {
            let peer = *ch
                .summary
                .recipient_ids
                .iter()
                .find(|v| **v != user)
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            require(!db::blocked(c, user, peer).await?, ErrorCode::Blocked)
        }
        ChannelKind::GroupDm => Ok(()),
        _ => Err(Error::new(ErrorCode::InvalidRequest)),
    }
}

fn online(core: &Core, u: Id) -> bool {
    core.streams.values().any(|s| {
        s.user == u
            && s.connection
                .as_ref()
                .is_some_and(|c| c.alive.load(std::sync::atomic::Ordering::Acquire))
    })
}

fn deliver(core: &mut Core, users: &[Id], event: &VoiceEvent) {
    let keys: Vec<_> = core
        .streams
        .iter()
        .filter(|(_, s)| s.connection.is_some() && users.contains(&s.user))
        .map(|(k, _)| k.clone())
        .collect();
    for key in keys {
        let _ = crate::voice::send(core, &key, 0, event);
    }
}

fn state(core: &Core, ch: Id) -> Option<DmCall> {
    let call = core.calls.live.get(&ch)?;
    let mut participants: Vec<_> = core
        .voice
        .values()
        .filter(|v| v.joined && v.state.channel_id == ch)
        .map(|v| v.state.participant.clone())
        .collect();
    participants.sort_by_key(|p| p.session);
    Some(DmCall {
        channel_id: ch,
        started_by: call.started_by,
        started_at: call.started_at,
        participants,
        ringing: call.ringing.keys().copied().collect(),
    })
}

/// Sends the call's current state to every member of its conversation.
pub(crate) async fn publish(core: &mut Core, c: &mut SqliteConnection, ch: Id) -> Result<()> {
    if !core.calls.live.contains_key(&ch) {
        return Ok(());
    }
    let members = db::channel(c, ch).await?.summary.recipient_ids;
    if let Some(call) = core.calls.live.get_mut(&ch) {
        // Someone who left the group is not rung any more.
        call.ringing.retain(|u, _| members.contains(u));
    }
    if let Some(state) = state(core, ch) {
        deliver(core, &members, &VoiceEvent::CallState(state));
    }
    Ok(())
}

/// A member's voice session for this conversation now exists (not yet
/// connected). The first one starts the call and rings everyone else.
pub(crate) async fn joining(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    user: Id,
    ch: Id,
) -> Result<()> {
    if let Some(call) = core.calls.live.get_mut(&ch) {
        call.ringing.remove(&user);
        return publish(core, c, ch).await;
    }
    let channel = db::channel(c, ch).await?;
    let t = now();
    let mut ringing = BTreeMap::new();
    for v in channel.summary.recipient_ids.iter().filter(|v| **v != user) {
        if !db::blocked(c, user, *v).await? {
            ringing.insert(*v, t + CALL_RING_TIMEOUT_MS);
        }
    }
    let offline: Vec<Id> = ringing
        .keys()
        .copied()
        .filter(|v| !online(core, *v))
        .collect();
    core.calls.live.insert(
        ch,
        Call {
            started_by: user,
            started_at: t,
            joined: BTreeSet::new(),
            ringing,
        },
    );
    core.calls.wake.notify_one();
    reveal(core, c, &channel).await?;
    for v in offline {
        crate::push::call(app, v, ch, user);
    }
    publish(core, c, ch).await
}

/// That session's media connected.
pub(crate) fn connected(core: &mut Core, ch: Id, user: Id) {
    if let Some(call) = core.calls.live.get_mut(&ch) {
        call.joined.insert(user);
        call.ringing.remove(&user);
    }
}

/// A voice session of this conversation is gone. `key` is the connection it
/// belonged to: the call is over for that connection, whatever happens next.
/// When nobody is left the call ends and leaves its system message.
pub(crate) async fn left(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    ch: Id,
) -> Result<()> {
    let _ = crate::voice::send(core, key, 0, &VoiceEvent::CallEnded { channel_id: ch });
    if core.voice.values().any(|v| v.state.channel_id == ch) {
        return publish(core, c, ch).await;
    }
    let Some(call) = core.calls.live.remove(&ch) else {
        return Ok(());
    };
    let members = db::channel(c, ch).await?.summary.recipient_ids;
    deliver(core, &members, &VoiceEvent::CallEnded { channel_id: ch });
    // A call whose media never connected (a failed join) leaves no record.
    if !call.joined.is_empty() {
        record(app, core, c, ch, &call).await?;
    }
    Ok(())
}

/// The call record on message `m`, if it is a call's system message.
pub(crate) async fn summary(c: &mut SqliteConnection, m: Id) -> Result<Option<CallSummary>> {
    let row = sqlx::query_as::<_, (i64, i64, bool, i64)>(
        "SELECT started_at,ended_at,missed,participant_count FROM call_records WHERE message_id=?",
    )
    .bind(db::id(m)?)
    .fetch_optional(&mut *c)
    .await?;
    Ok(
        row.map(|(started_at, ended_at, missed, count)| CallSummary {
            started_at: started_at as u64,
            ended_at: ended_at as u64,
            missed,
            participant_count: count.clamp(0, u16::MAX as i64) as u16,
        }),
    )
}

/// The human-readable text of a call's system message, for clients that do not
/// read `Message::call`.
pub(crate) fn describe(summary: &CallSummary) -> String {
    if summary.missed {
        return "Missed call".into();
    }
    let minutes = summary.duration_ms() / 60_000;
    match minutes {
        0 => "Started a call that lasted less than a minute.".into(),
        1 => "Started a call that lasted a minute.".into(),
        n if n < 60 => format!("Started a call that lasted {n} minutes."),
        n => format!(
            "Started a call that lasted {} hour{} {} minute{}.",
            n / 60,
            if n / 60 == 1 { "" } else { "s" },
            n % 60,
            if n % 60 == 1 { "" } else { "s" }
        ),
    }
}

async fn record(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    ch: Id,
    call: &Call,
) -> Result<()> {
    let summary = CallSummary {
        started_at: call.started_at,
        ended_at: now().max(call.started_at),
        missed: !call.joined.iter().any(|u| *u != call.started_by),
        participant_count: call.joined.len().min(u16::MAX as usize) as u16,
    };
    let m = core.id().await?;
    {
        let mut tx = c.begin().await?;
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)",
        )
        .bind(db::id(m)?)
        .bind(db::id(ch)?)
        .bind(db::id(call.started_by)?)
        .bind(describe(&summary))
        .bind(summary.ended_at as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO call_records VALUES(?,?,?,?,?)")
            .bind(db::id(m)?)
            .bind(summary.started_at as i64)
            .bind(summary.ended_at as i64)
            .bind(summary.missed)
            .bind(i64::from(summary.participant_count))
            .execute(&mut *tx)
            .await?;
        core.persist_id(&mut tx).await?;
        db::recount(&mut tx, ch).await?;
        tx.commit().await?;
    }
    let message = db::message(c, call.started_by, ch, m, false).await?;
    let recipients = db::audience(c, ch).await?;
    let author = db::user(c, call.started_by).await?;
    core.emit(
        &recipients,
        Event::MessageCreated {
            message: message.clone(),
            author,
        },
    )?;
    for u in &recipients {
        let visible: bool =
            sqlx::query_scalar("SELECT visible FROM dm_members WHERE channel_id=? AND user_id=?")
                .bind(db::id(ch)?)
                .bind(db::id(*u)?)
                .fetch_optional(&mut *c)
                .await?
                .unwrap_or(false);
        if visible {
            core.emit(&[*u], Event::ReadStateUpdated(db::read(c, *u, ch).await?))?;
        }
    }
    if summary.missed {
        crate::push::message(app, c, &recipients, &message).await;
    }
    Ok(())
}

/// The call is shown to the person being called even if they had closed that DM.
async fn reveal(core: &mut Core, c: &mut SqliteConnection, ch: &Channel) -> Result<()> {
    if ch.summary.kind != ChannelKind::Direct {
        return Ok(());
    }
    let hidden: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM dm_members WHERE channel_id=? AND visible=0")
            .bind(db::id(ch.summary.id)?)
            .fetch_all(&mut *c)
            .await?;
    for v in hidden.into_iter().map(|v| Id(v as u64)) {
        if db::dm_quota(c, v, Some(ch.summary.id)).await.is_err() {
            continue;
        }
        sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=? AND user_id=?")
            .bind(db::id(ch.summary.id)?)
            .bind(db::id(v)?)
            .execute(&mut *c)
            .await?;
        for r in &ch.summary.recipient_ids {
            core.emit(&[v], Event::UserUpdated(db::user(c, *r).await?))?;
        }
        core.emit(&[v], Event::ChannelUpsert(ch.clone()))?;
        core.emit(
            &[v],
            Event::ReadStateUpdated(db::read(c, v, ch.summary.id).await?),
        )?;
    }
    Ok(())
}

fn in_call(core: &Core, ch: Id) -> BTreeSet<Id> {
    core.voice
        .values()
        .filter(|v| v.state.channel_id == ch)
        .map(|v| v.state.participant.session.user_id)
        .collect()
}

/// `VoiceSignal::Ring`: someone in the call rings members again.
pub(crate) async fn ring(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    user: Id,
    ch: Id,
    targets: &[Id],
) -> Result<()> {
    core.rate(format!("call-ring:{}", user.0), 12., 4.)?;
    require(
        !targets.is_empty() && targets.len() <= MAX_CALL_RING_TARGETS,
        ErrorCode::InvalidRequest,
    )?;
    let unique: BTreeSet<_> = targets.iter().collect();
    require(unique.len() == targets.len(), ErrorCode::InvalidRequest)?;
    let channel = db::access(c, user, ch, Permissions::VIEW_CHANNEL).await?;
    require(
        matches!(
            channel.summary.kind,
            ChannelKind::Direct | ChannelKind::GroupDm
        ),
        ErrorCode::InvalidRequest,
    )?;
    require(core.calls.live.contains_key(&ch), ErrorCode::NotFound)?;
    let present = in_call(core, ch);
    require(present.contains(&user), ErrorCode::Forbidden)?;
    for v in targets {
        require(
            *v != user && channel.summary.recipient_ids.contains(v),
            ErrorCode::InvalidRequest,
        )?;
    }
    let deadline = now() + CALL_RING_TIMEOUT_MS;
    let mut rung = vec![];
    for v in targets {
        if !present.contains(v) && !db::blocked(c, user, *v).await? {
            rung.push(*v);
        }
    }
    if let Some(call) = core.calls.live.get_mut(&ch) {
        for v in &rung {
            call.ringing.insert(*v, deadline);
        }
    }
    core.calls.wake.notify_one();
    for v in rung {
        if !online(core, v) {
            crate::push::call(app, v, ch, user);
        }
    }
    publish(core, c, ch).await
}

/// `VoiceSignal::DeclineCall`: stop ringing this user, on every device.
pub(crate) async fn decline(
    core: &mut Core,
    c: &mut SqliteConnection,
    user: Id,
    ch: Id,
) -> Result<()> {
    db::access(c, user, ch, Permissions::VIEW_CHANNEL).await?;
    let changed = core
        .calls
        .live
        .get_mut(&ch)
        .is_some_and(|call| call.ringing.remove(&user).is_some());
    if changed {
        publish(core, c, ch).await?;
    }
    Ok(())
}

/// Stops every ring whose time is up.
pub(crate) async fn expire(core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let t = now();
    let mut changed = vec![];
    for (ch, call) in &mut core.calls.live {
        let before = call.ringing.len();
        call.ringing.retain(|_, deadline| *deadline > t);
        if call.ringing.len() != before {
            changed.push(*ch);
        }
    }
    for ch in changed {
        publish(core, c, ch).await?;
    }
    Ok(())
}

/// Sent after Ready and Resumed.
pub(crate) async fn snapshot(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    u: Id,
) -> Result<()> {
    let mut calls = vec![];
    let channels: Vec<Id> = core.calls.live.keys().copied().collect();
    for ch in channels {
        if db::access(c, u, ch, Permissions::VIEW_CHANNEL)
            .await
            .is_ok()
        {
            calls.extend(state(core, ch));
        }
    }
    crate::voice::send(core, key, 0, &VoiceEvent::CallSnapshot { calls })
}

/// Commands that can take someone out of a conversation's call.
pub(crate) fn acl_changed(command: &Command) -> bool {
    matches!(
        command,
        Command::BlockUser { .. }
            | Command::RemoveGroupRecipient { .. }
            | Command::LeaveGroupDm { .. }
    )
}

/// The ring timer: one task, armed for the earliest ring deadline. With no
/// ring it waits on its `Notify` and does nothing.
pub(crate) fn spawn(app: &App) {
    let weak = Arc::downgrade(&app.0);
    let stop = app.0.shutdown.clone();
    tokio::spawn(async move {
        let wake = match weak.upgrade() {
            Some(inner) => inner.core.lock().await.calls.wake.clone(),
            None => return,
        };
        loop {
            let next = match weak.upgrade() {
                Some(inner) => inner.core.lock().await.calls.next_deadline(),
                None => return,
            };
            let sleep = async {
                match next {
                    Some(t) => {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            t.saturating_sub(now()),
                        ))
                        .await
                    }
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = wake.notified() => continue,
                _ = sleep => {}
            }
            let Some(inner) = weak.upgrade() else { return };
            let app = App(inner);
            let mut core = app.0.core.lock().await;
            if let Ok(mut c) = app.0.pool.acquire().await {
                if let Err(error) = expire(&mut core, &mut c).await {
                    tracing::warn!(?error, "call ring expiry failed");
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
