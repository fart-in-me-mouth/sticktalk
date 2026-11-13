//! Host-owned voice lifecycle. The synchronous ACL projection is checked on every
//! SFU media/control operation; SQLite and WS state stay behind the normal gate.
use crate::{
    db,
    error::{require, Error, Result},
    state::{now, App, Core},
};
use clap::Args;
use sqlx::SqliteConnection;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
};
use sticktalk_proto::*;
use sticktalk_sfu::{Command as MediaCommand, ControlOut, Sfu, SignalOut};
use tokio::sync::mpsc;

#[derive(Args, Clone, Debug)]
pub struct VoiceOptions {
    #[arg(long, env = "STICKTALK_VOICE_BIND_IP", default_value = "127.0.0.1")]
    pub voice_bind_ip: IpAddr,
    /// Loopback dev binds advertise HTTP loopback; deployment binds may auto-detect Tailscale.
    #[arg(long, env = "STICKTALK_VOICE_ADVERTISED_IP")]
    pub voice_advertised_ip: Option<IpAddr>,
    #[arg(long, env = "STICKTALK_VOICE_UDP_PORT_START", default_value_t = 50000)]
    pub voice_udp_port_start: u16,
    #[arg(long, env = "STICKTALK_VOICE_UDP_PORT_END", default_value_t = 50100)]
    pub voice_udp_port_end: u16,
    #[arg(long, env = "STICKTALK_VOICE_EGRESS_BPS", default_value_t = 32_000_000)]
    pub voice_egress_bps: u64,
    /// SRTP UDP bytes, including RTP extensions, authentication and RTX.
    #[arg(long, env = "STICKTALK_VOICE_MAX_DATAGRAM_BYTES", default_value_t = DEFAULT_VOICE_UDP_BYTES)]
    pub voice_max_datagram_bytes: u16,
}
impl Default for VoiceOptions {
    fn default() -> Self {
        Self {
            voice_bind_ip: "127.0.0.1".parse().unwrap(),
            voice_advertised_ip: None,
            voice_udp_port_start: 50000,
            voice_udp_port_end: 50100,
            voice_egress_bps: 32_000_000,
            voice_max_datagram_bytes: DEFAULT_VOICE_UDP_BYTES,
        }
    }
}
fn tailscale(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v) if v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1]))
}
struct Lease {
    channel: Id,
    permissions: Permissions,
    server_muted: bool,
    server_deafened: bool,
    self_muted: bool,
    self_deafened: bool,
    expires: u64,
    alive: Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Default)]
struct Policy {
    leases: HashMap<VoiceSession, Lease>,
    generation: u64,
}
type Projection = Arc<RwLock<Policy>>;
struct ProjectedAcl(Projection);
impl sticktalk_sfu::Authorizer for ProjectedAcl {
    fn can_join(&self, session: VoiceSession, channel: Id) -> bool {
        self.permissions(session, channel)
            .contains(Permissions::CONNECT | Permissions::VIEW_CHANNEL)
    }
    fn permissions(&self, session: VoiceSession, channel: Id) -> Permissions {
        self.0
            .read()
            .ok()
            .and_then(|p| {
                p.leases
                    .get(&session)
                    .filter(|l| {
                        l.channel == channel
                            && now() < l.expires
                            && l.alive.load(std::sync::atomic::Ordering::Acquire)
                    })
                    .map(|l| l.permissions)
            })
            .unwrap_or_default()
    }
    fn relocated_channel(&self, session: VoiceSession) -> Option<Id> {
        self.0
            .read()
            .ok()
            .and_then(|p| p.leases.get(&session).map(|l| l.channel))
    }
    fn receive_audio(&self, session: VoiceSession) -> bool {
        self.0.read().ok().is_some_and(|p| {
            p.leases
                .get(&session)
                .is_some_and(|l| !l.server_deafened && !l.self_deafened)
        })
    }
    fn send_audio(&self, session: VoiceSession) -> bool {
        self.0.read().ok().is_some_and(|p| {
            p.leases.get(&session).is_some_and(|l| {
                !l.server_muted && !l.server_deafened && !l.self_muted && !l.self_deafened
            })
        })
    }
    fn generation(&self) -> u64 {
        self.0.read().unwrap_or_else(|e| e.into_inner()).generation
    }
}
fn advertised_ip(explicit: Option<IpAddr>, http: IpAddr, interfaces: Vec<IpAddr>) -> IpAddr {
    explicit
        .or_else(|| interfaces.into_iter().find(|ip| tailscale(*ip)))
        .unwrap_or(http)
}
type Driver = (
    Sfu,
    mpsc::Receiver<MediaCommand>,
    mpsc::Receiver<VoiceSession>,
);
pub(crate) struct Voice {
    commands: mpsc::Sender<MediaCommand>,
    cleanup: mpsc::Sender<VoiceSession>,
    acl: Projection,
    pub address: SocketAddr,
    driver: std::sync::Mutex<Option<Driver>>,
}
pub(crate) struct Entry {
    key: Vec<u8>,
    connection: String,
    pub state: GuildVoiceState,
    pub joined: bool,
    server_muted: bool,
    server_deafened: bool,
    self_muted: bool,
    self_deafened: bool,
}
impl Voice {
    pub fn new(options: &VoiceOptions, http: SocketAddr) -> Result<Self> {
        let advertised = advertised_ip(
            options.voice_advertised_ip,
            http.ip(),
            if options.voice_bind_ip.is_loopback() {
                vec![]
            } else {
                if_addrs::get_if_addrs()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| a.ip())
                    .collect()
            },
        );
        let acl: Projection = Default::default();
        let sfu = Sfu::new(
            sticktalk_sfu::Config {
                bind_ip: options.voice_bind_ip,
                advertised_ip: advertised,
                ports: options.voice_udp_port_start..=options.voice_udp_port_end,
                video_egress_bps: options.voice_egress_bps,
                max_datagram_bytes: options.voice_max_datagram_bytes,
                ..Default::default()
            },
            Arc::new(ProjectedAcl(acl.clone())),
        )
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let address = sfu.local_addr();
        let (commands, receiver) = mpsc::channel(128);
        let (cleanup, cleanup_rx) = mpsc::channel(128);
        Ok(Self {
            commands,
            cleanup,
            acl,
            address,
            driver: std::sync::Mutex::new(Some((sfu, receiver, cleanup_rx))),
        })
    }
    fn command(&self, command: MediaCommand) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| Error::new(ErrorCode::Unavailable))
    }
    pub fn clear(&self) {
        let mut p = self.acl.write().unwrap_or_else(|e| e.into_inner());
        p.leases.clear();
        p.generation += 1;
    }
}
pub(crate) fn spawn(app: &App) {
    let Some((sfu, commands, cleanup)) = app
        .0
        .voice
        .driver
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    else {
        return;
    };
    let (tx, mut rx) = mpsc::channel::<SignalOut>(128);
    let (revoked_tx, mut revoked_rx) = mpsc::channel::<ControlOut>(128);
    let stop = app.0.shutdown.clone();
    let weak = Arc::downgrade(&app.0);
    tokio::spawn(async move {
        tokio::select! {
            _ = stop.cancelled() => {},
            result = sfu.run_with_control(commands, tx, cleanup, revoked_tx) => {
                if let Err(error) = result { tracing::error!(%error, "SFU stopped; chat remains available"); }
                if let Some(inner) = weak.upgrade() {
                    let app = App(inner);
                    app.0.voice.clear();
                    let mut core = app.0.core.lock().await;
                    if let Ok(mut c) = app.0.pool.acquire().await {
                        let sessions: Vec<_> = core.voice.keys().copied().collect();
                        for session in sessions { let _ = remove(&app, &mut core, &mut c, session).await; }
                    }
                }
            }
        }
    });
    let stop = app.0.shutdown.clone();
    let weak = Arc::downgrade(&app.0);
    tokio::spawn(async move {
        loop {
            let signal = tokio::select! {
                _ = stop.cancelled() => break,
                Some(control) = revoked_rx.recv() => {
                    let session=match control {ControlOut::Revoked(session)=>session,ControlOut::Signal(signal)=>{
                        if let Some(inner)=weak.upgrade() {
                            let app=App(inner);let mut core=app.0.core.lock().await;
                            if let Ok(mut c)=app.0.pool.acquire().await {let _=output(&app,&mut core,&mut c,signal).await;}
                        }
                        continue;
                    }};
                    if let Some(inner) = weak.upgrade() {
                        let app = App(inner);
                        let mut core = app.0.core.lock().await;
                        if let Ok(mut c) = app.0.pool.acquire().await { let _ = remove(&app, &mut core, &mut c, session).await; }
                    }
                    continue;
                },
                signal = rx.recv() => signal,
            };
            let (Some(signal), Some(inner)) = (signal, weak.upgrade()) else {
                break;
            };
            let app = App(inner);
            let mut core = app.0.core.lock().await;
            let affected = signal.session;
            let result = async {
                let mut c = app.0.pool.acquire().await?;
                output(&app, &mut core, &mut c, signal).await
            }
            .await;
            if result.is_err() {
                // A voice output failure revokes only its destination call.
                if let Ok(mut c) = app.0.pool.acquire().await {
                    let _ = remove(&app, &mut core, &mut c, affected).await;
                }
            }
        }
    });
}
fn send(core: &mut Core, key: &[u8], nonce: u64, event: &VoiceEvent) -> Result<()> {
    let critical = matches!(
        event,
        VoiceEvent::Left { .. } | VoiceEvent::ControlRevoked { .. }
    );
    if let Some(connection) = core.streams.get(key).and_then(|s| s.connection.as_ref()) {
        require(
            connection.tx.capacity() >= if critical { 1 } else { 32 },
            ErrorCode::Unavailable,
        )?;
        // Never let voice signaling fill the shared chat frame/byte budget.
        require(
            connection.budget.available_permits() >= if critical { 65536 } else { 262144 },
            ErrorCode::Unavailable,
        )?;
    }
    let envelope =
        sticktalk_sfu::encode_event(nonce, event).map_err(|_| Error::new(ErrorCode::Internal))?;
    core.send(key, &ServerMsg::Voice(envelope))
}
async fn broadcast(
    core: &mut Core,
    c: &mut SqliteConnection,
    guild: Id,
    event: &VoiceEvent,
) -> Result<()> {
    let users = db::guild_users(c, guild).await?;
    let keys: Vec<_> = core
        .streams
        .iter()
        .filter(|(_, s)| s.connection.is_some() && users.contains(&s.user))
        .map(|(k, _)| k.clone())
        .collect();
    for key in keys {
        if let VoiceEvent::GuildState(state) = event {
            if db::access(
                c,
                core.streams[&key].user,
                state.channel_id,
                Permissions::VIEW_CHANNEL,
            )
            .await
            .is_err()
            {
                continue;
            }
        }
        let _ = send(core, &key, 0, event);
    }
    Ok(())
}
pub(crate) async fn allowed(c: &mut SqliteConnection, user: Id, channel: Id) -> Result<Id> {
    let ch = db::access(
        c,
        user,
        channel,
        Permissions::VIEW_CHANNEL | Permissions::CONNECT,
    )
    .await?;
    require(
        ch.summary.kind == ChannelKind::Voice,
        ErrorCode::InvalidRequest,
    )?;
    let guild_id = ch
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    Ok(guild_id)
}
/// Called with the normal mutation gate held. Denial happens synchronously before
/// returning a command response, never after an asynchronously queued SFU command.
pub(crate) async fn remove(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
) -> Result<()> {
    {
        let mut p = app.0.voice.acl.write().unwrap_or_else(|e| e.into_inner());
        p.leases.remove(&session);
        p.generation += 1;
    }
    if let Some(entry) = core.voice.remove(&session) {
        // Reserved cleanup lane, sized above the SFU's maximum live sessions.
        // ACL denial above is synchronous even if the actor has stopped.
        let _ = app.0.voice.cleanup.try_send(session);
        send(core, &entry.key, 0, &VoiceEvent::Left { session })?;
        if entry.joined {
            broadcast(
                core,
                c,
                entry.state.guild_id,
                &VoiceEvent::GuildLeft {
                    guild_id: entry.state.guild_id,
                    session,
                },
            )
            .await?;
        }
    }
    Ok(())
}
pub(crate) async fn disconnect(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    connection: Option<&str>,
) -> Result<()> {
    let sessions: Vec<_> = core
        .voice
        .iter()
        .filter(|(_, v)| v.key == key && connection.is_none_or(|id| v.connection == id))
        .map(|(s, _)| *s)
        .collect();
    for session in sessions {
        remove(app, core, c, session).await?;
    }
    Ok(())
}
pub(crate) async fn reconcile(app: &App, core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let sessions: Vec<_> = core.voice.keys().copied().collect();
    for session in sessions {
        let entry = &core.voice[&session];
        let active = core
            .streams
            .get(&entry.key)
            .and_then(|s| s.connection.as_ref())
            .is_some_and(|v| v.id == entry.connection);
        let alive: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=sessions.user_id))")
            .bind(&entry.key).bind(now() as i64).fetch_one(&mut *c).await?;
        if !active
            || !alive
            || allowed(c, session.user_id, entry.state.channel_id)
                .await
                .is_err()
        {
            remove(app, core, c, session).await?;
        } else {
            let entry = &core.voice[&session];
            let rights = crate::permissions::evaluate(
                c,
                session.user_id,
                entry.state.guild_id,
                Some(entry.state.channel_id),
            )
            .await?;
            let changed = {
                let mut p = app.0.voice.acl.write().unwrap_or_else(|e| e.into_inner());
                let lease = p
                    .leases
                    .get_mut(&session)
                    .ok_or_else(|| Error::new(ErrorCode::Internal))?;
                let changed = lease.permissions != rights;
                lease.permissions = rights;
                if changed {
                    p.generation += 1;
                }
                changed
            };
            if changed {
                refresh(app, core, c, session).await?;
            }
        }
    }
    Ok(())
}
async fn refresh(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
) -> Result<()> {
    if app.0.voice.command(MediaCommand::Refresh(session)).is_err() {
        return remove(app, core, c, session).await;
    }
    let permissions = app
        .0
        .voice
        .acl
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .leases[&session]
        .permissions;
    let entry = core
        .voice
        .get_mut(&session)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    entry
        .state
        .participant
        .tracks
        .retain(|t| permissions.contains(track_permission(t.kind)));
    let state = entry.state.clone();
    broadcast(core, c, state.guild_id, &VoiceEvent::GuildState(state)).await
}
fn track_permission(kind: TrackKind) -> Permissions {
    match kind {
        TrackKind::Audio => Permissions::SPEAK,
        TrackKind::Camera => Permissions::VIDEO,
        TrackKind::Screen | TrackKind::SystemAudio => Permissions::STREAM,
    }
}
async fn moderate(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    actor: Id,
    session: VoiceSession,
    action: VoiceModeration,
) -> Result<()> {
    let target = core
        .voice
        .get(&session)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let guild = target.state.guild_id;
    let channel = target.state.channel_id;
    let required = match action {
        VoiceModeration::Mute { .. } => Permissions::MUTE_MEMBERS,
        VoiceModeration::Deafen { .. } => Permissions::DEAFEN_MEMBERS,
        _ => Permissions::MOVE_MEMBERS,
    };
    db::access(c, actor, channel, required).await?;
    crate::permissions::above_member(c, guild, actor, session.user_id).await?;
    if let VoiceModeration::Disconnect = action {
        return remove(app, core, c, session).await;
    }
    if let VoiceModeration::Move { channel_id } = action {
        require(
            allowed(c, session.user_id, channel_id).await? == guild,
            ErrorCode::InvalidRequest,
        )?;
        db::access(
            c,
            actor,
            channel_id,
            Permissions::MOVE_MEMBERS | Permissions::CONNECT,
        )
        .await?;
        require(
            core.voice
                .values()
                .filter(|v| v.state.channel_id == channel_id)
                .count()
                < 16,
            ErrorCode::LimitExceeded,
        )?;
        let rights =
            crate::permissions::evaluate(c, session.user_id, guild, Some(channel_id)).await?;
        {
            let mut p = app.0.voice.acl.write().unwrap_or_else(|e| e.into_inner());
            let lease = p
                .leases
                .get_mut(&session)
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            lease.channel = channel_id;
            lease.permissions = rights;
            p.generation += 1;
        }
        core.voice.get_mut(&session).unwrap().state.channel_id = channel_id;
        if app
            .0
            .voice
            .command(MediaCommand::Move {
                session,
                channel: channel_id,
            })
            .is_err()
        {
            return remove(app, core, c, session).await;
        }
        broadcast(
            core,
            c,
            guild,
            &VoiceEvent::GuildLeft {
                guild_id: guild,
                session,
            },
        )
        .await?;
    } else {
        let (mut muted, mut deafened) = (target.server_muted, target.server_deafened);
        match action {
            VoiceModeration::Mute { muted: value } => muted = value,
            VoiceModeration::Deafen { deafened: value } => deafened = value,
            _ => unreachable!(),
        }
        let sessions: Vec<_> = core
            .voice
            .iter()
            .filter(|(_, e)| {
                e.state.guild_id == guild && e.state.participant.session.user_id == session.user_id
            })
            .map(|(s, e)| (*s, e.state.channel_id))
            .collect();
        // A second device in a private channel is not an ACL bypass for the moderator.
        for (_, channel) in &sessions {
            db::access(c, actor, *channel, required).await?;
        }
        sqlx::query("INSERT INTO guild_voice_moderation VALUES(?,?,?,?) ON CONFLICT(guild_id,user_id) DO UPDATE SET muted=excluded.muted,deafened=excluded.deafened").bind(db::id(guild)?).bind(db::id(session.user_id)?).bind(muted).bind(deafened).execute(&mut *c).await?;
        {
            let mut p = app.0.voice.acl.write().unwrap_or_else(|e| e.into_inner());
            for (session, _) in &sessions {
                let entry = core.voice.get_mut(session).unwrap();
                entry.server_muted = muted;
                entry.server_deafened = deafened;
                entry.state.participant.muted = muted || entry.self_muted;
                entry.state.participant.deafened = deafened || entry.self_deafened;
                if let Some(lease) = p.leases.get_mut(session) {
                    lease.server_muted = muted;
                    lease.server_deafened = deafened;
                }
            }
            p.generation += 1;
        }
        for (session, _) in sessions {
            refresh(app, core, c, session).await?;
        }
        return Ok(());
    }
    refresh(app, core, c, session).await
}
pub(crate) async fn signal(
    app: &App,
    core: &mut Core,
    key: &[u8],
    user: Id,
    connection: &str,
    envelope: VoicePayload,
) -> Result<()> {
    let nonce = envelope.nonce;
    let result = signal_inner(app, core, key, user, connection, envelope).await;
    if let Err(error) = result {
        send(
            core,
            key,
            nonce,
            &VoiceEvent::Error {
                code: error.0.code,
                message: error.0.message,
            },
        )?;
    }
    Ok(())
}
async fn signal_inner(
    app: &App,
    core: &mut Core,
    key: &[u8],
    user: Id,
    connection: &str,
    envelope: VoicePayload,
) -> Result<()> {
    let nonce = envelope.nonce;
    require(
        nonce > 0 && nonce <= i64::MAX as u64,
        ErrorCode::InvalidRequest,
    )?;
    require(
        envelope.version == VOICE_VERSION && envelope.kind == "signal",
        ErrorCode::VersionMismatch,
    )?;
    require(
        envelope.payload.len() <= 72 * 1024,
        ErrorCode::PayloadTooLarge,
    )?;
    // The inner body is another untrusted MessagePack document. Bound its
    // lengths/depth and consumption before serde can reserve nested collections.
    let mut position = 0;
    crate::ws::scan(&envelope.payload, &mut position, 0)?;
    require(
        position == envelope.payload.len(),
        ErrorCode::InvalidRequest,
    )?;
    let signal: VoiceSignal = rmp_serde::from_slice(&envelope.payload)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let mut c = app.0.pool.acquire().await?;
    let expiry: Option<i64> = sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=sessions.user_id)")
        .bind(key).fetch_optional(&mut *c).await?;
    require(
        expiry.is_some_and(|t| t > now() as i64),
        ErrorCode::Unauthenticated,
    )?;
    if let VoiceSignal::Moderate { session, action } = signal {
        core.rate(format!("voice-moderate:{}", user.0), 120., 20.)?;
        moderate(app, core, &mut c, user, session, action).await?;
        return send(core, key, nonce, &VoiceEvent::Ack);
    }
    let existing = core
        .voice
        .iter()
        .find(|(_, v)| v.key == key && v.connection == connection)
        .map(|(s, _)| *s);
    if let VoiceSignal::Join { channel_id, .. } = signal {
        core.rate(format!("voice-join:{}", user.0), 30., 5.)?;
        require(existing.is_none(), ErrorCode::Conflict)?;
        let guild_id = allowed(&mut c, user, channel_id).await?;
        require(
            core.voice.len() < 64
                && core
                    .voice
                    .values()
                    .filter(|v| v.state.channel_id == channel_id)
                    .count()
                    < 16,
            ErrorCode::LimitExceeded,
        )?;
        // Fresh even on leave/rejoin in the same WS. Delayed old events and
        // grants can never bind to a new call lifetime.
        let session = VoiceSession {
            user_id: user,
            connection_id: core.id().await?,
        };
        let rights = crate::permissions::evaluate(&mut c, user, guild_id, Some(channel_id)).await?;
        let (server_muted, server_deafened) = sqlx::query_as::<_, (bool, bool)>(
            "SELECT muted,deafened FROM guild_voice_moderation WHERE guild_id=? AND user_id=?",
        )
        .bind(db::id(guild_id)?)
        .bind(db::id(user)?)
        .fetch_optional(&mut *c)
        .await?
        .unwrap_or_default();
        app.0
            .voice
            .acl
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .leases
            .insert(
                session,
                Lease {
                    channel: channel_id,
                    permissions: rights,
                    server_muted,
                    server_deafened,
                    self_muted: false,
                    self_deafened: false,
                    expires: expiry.unwrap() as u64,
                    alive: core.streams[key]
                        .connection
                        .as_ref()
                        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?
                        .alive
                        .clone(),
                },
            );
        core.voice.insert(
            session,
            Entry {
                key: key.to_vec(),
                connection: connection.into(),
                joined: false,
                server_muted,
                server_deafened,
                self_muted: false,
                self_deafened: false,
                state: GuildVoiceState {
                    guild_id,
                    channel_id,
                    participant: VoiceParticipant {
                        session,
                        muted: server_muted,
                        deafened: server_deafened,
                        tracks: vec![],
                    },
                },
            },
        );
        if let Err(e) = app
            .0
            .voice
            .command(MediaCommand::Signal { session, envelope })
        {
            remove(app, core, &mut c, session).await?;
            return Err(e);
        }
    } else {
        let session = existing.ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
        if matches!(signal, VoiceSignal::Leave) {
            remove(app, core, &mut c, session).await?;
        } else {
            if let Err(error) = allowed(&mut c, user, core.voice[&session].state.channel_id).await {
                remove(app, core, &mut c, session).await?;
                return Err(error);
            }
            if let VoiceSignal::State { muted, deafened } = signal {
                let entry = core.voice.get_mut(&session).unwrap();
                entry.self_muted = muted;
                entry.self_deafened = deafened;
                let mut p = app.0.voice.acl.write().unwrap_or_else(|e| e.into_inner());
                if let Some(lease) = p.leases.get_mut(&session) {
                    lease.self_muted = muted;
                    lease.self_deafened = deafened;
                    p.generation += 1;
                }
            }
            app.0
                .voice
                .command(MediaCommand::Signal { session, envelope })?;
        }
    }
    Ok(())
}
async fn output(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    signal: SignalOut,
) -> Result<()> {
    let Some(entry) = core.voice.get(&signal.session) else {
        return Ok(());
    };
    let key = entry.key.clone();
    let event: VoiceEvent = rmp_serde::from_slice(&signal.envelope.payload)
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    // Process authoritative own-session state exactly once, not once/recipient.
    match &event {
        VoiceEvent::Joined { .. } => {
            core.voice.get_mut(&signal.session).unwrap().joined = true;
            let state = core.voice[&signal.session].state.clone();
            broadcast(core, c, state.guild_id, &VoiceEvent::GuildState(state)).await?;
        }
        VoiceEvent::Participant(p) if p.session == signal.session => {
            let entry = core.voice.get_mut(&signal.session).unwrap();
            if entry.state.participant != *p {
                entry.state.participant = p.clone();
                entry.state.participant.muted = entry.server_muted || entry.self_muted;
                entry.state.participant.deafened = entry.server_deafened || entry.self_deafened;
                let rights = app
                    .0
                    .voice
                    .acl
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .leases
                    .get(&signal.session)
                    .map(|l| l.permissions)
                    .unwrap_or_default();
                entry
                    .state
                    .participant
                    .tracks
                    .retain(|t| rights.contains(track_permission(t.kind)));
                let state = entry.state.clone();
                broadcast(core, c, state.guild_id, &VoiceEvent::GuildState(state)).await?;
            }
        }
        // SFU fatal failures send a sanitized correlated Error BEFORE Left.
        // Keep routing errors for joined sessions; only the subsequent Left
        // revokes this call's lease/projection, never the whole SFU actor.
        VoiceEvent::Left { session } if *session == signal.session => {
            return remove(app, core, c, *session).await;
        }
        VoiceEvent::Error { .. } if !entry.joined => {
            remove(app, core, c, signal.session).await?;
        }
        _ => {}
    }
    let mut event = event;
    if matches!(&event,VoiceEvent::Participant(p) if !core.voice.contains_key(&p.session)) {
        return Ok(());
    }
    if let VoiceEvent::Joined { participants, .. } = &mut event {
        participants.retain(|p| core.voice.contains_key(&p.session));
    }
    let participants: Vec<&mut VoiceParticipant> = match &mut event {
        VoiceEvent::Participant(p) => vec![p],
        VoiceEvent::Joined { participants, .. } => participants.iter_mut().collect(),
        _ => vec![],
    };
    for p in participants {
        if let Some(entry) = core.voice.get(&p.session) {
            p.muted = entry.server_muted || entry.self_muted;
            p.deafened = entry.server_deafened || entry.self_deafened;
            let rights = crate::permissions::evaluate(
                c,
                p.session.user_id,
                entry.state.guild_id,
                Some(entry.state.channel_id),
            )
            .await?;
            p.tracks
                .retain(|t| rights.contains(track_permission(t.kind)));
        }
    }
    core.send(
        &key,
        &ServerMsg::Voice(
            sticktalk_sfu::encode_event(signal.envelope.nonce, &event)
                .map_err(|_| Error::new(ErrorCode::Internal))?,
        ),
    )?;
    // queue() can detach a slow consumer; revoke its synchronous media ACL now.
    let detached: Vec<_> = core
        .voice
        .iter()
        .filter(|(_, entry)| {
            !core
                .streams
                .get(&entry.key)
                .and_then(|s| s.connection.as_ref())
                .is_some_and(|v| v.id == entry.connection)
        })
        .map(|(session, _)| *session)
        .collect();
    for session in detached {
        remove(app, core, c, session).await?;
    }
    Ok(())
}
pub(crate) async fn snapshot(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    user: Id,
) -> Result<()> {
    let mut states = vec![];
    for entry in core.voice.values().filter(|v| v.joined) {
        if db::access(c, user, entry.state.channel_id, Permissions::VIEW_CHANNEL)
            .await
            .is_ok()
        {
            states.push(entry.state.clone());
        }
    }
    send(core, key, 0, &VoiceEvent::GuildSnapshot { states })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn s5_voice_frames_cannot_consume_reserved_chat_capacity() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut core = app.0.core.lock().await;
        let key = vec![1; 32];
        core.cursor(&key, Id(1));
        let (tx, _rx) = mpsc::channel(128);
        let (close, watch) = tokio::sync::watch::channel(None);
        core.streams.get_mut(&key).unwrap().connection = Some(crate::state::Connection {
            id: "chat".into(),
            focused: false,
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
        for nonce in 1..=1000 {
            let _ = send(&mut core, &key, nonce, &VoiceEvent::Ack);
        }
        assert_eq!(*watch.borrow(), None);
        let connection = core.streams[&key].connection.as_ref().unwrap();
        assert!(connection.tx.capacity() >= 31);
        core.send(
            &key,
            &ServerMsg::HeartbeatAck(HeartbeatAck {
                nonce: 1,
                server_time: now(),
            }),
        )
        .unwrap();
        assert_eq!(*watch.borrow(), None);
    }
    #[tokio::test]
    async fn s5_signaling_flood_does_not_shutdown_chat() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        spawn(&app);
        let gate = app.0.core.lock().await;
        let tx = app.0.voice.commands.clone();
        let flood = tokio::spawn(async move {
            for nonce in 1..=2000 {
                tx.send(MediaCommand::Signal {
                    session: VoiceSession {
                        user_id: Id(1),
                        connection_id: Id(100),
                    },
                    envelope: VoicePayload {
                        nonce,
                        version: 0,
                        kind: "invalid".into(),
                        payload: vec![],
                    },
                })
                .await
                .unwrap();
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!app.0.shutdown.is_cancelled());
        drop(gate);
        tokio::time::timeout(std::time::Duration::from_secs(10), flood)
            .await
            .unwrap()
            .unwrap();
        let mut core = app.0.core.lock().await;
        core.limits.clear();
        let response = crate::commands::execute(
            &app,
            &mut core,
            &[1; 32],
            Id(1),
            CommandRequest {
                nonce: 1,
                command: Command::SendMessage(MessageDraft {
                    channel_id: Id(11),
                    content: "chat survives signaling overload".into(),
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![],
                }),
            },
        )
        .await
        .unwrap();
        assert!(matches!(response.outcome, CommandOutcome::Ok(_)));
        assert!(!app.0.shutdown.is_cancelled());
        drop(core);
        app.shutdown();
    }
    #[test]
    fn only_tailscale_cgnat_addresses_match() {
        for ip in ["100.64.0.1", "100.127.255.254"] {
            assert!(tailscale(ip.parse().unwrap()));
        }
        for ip in ["100.63.0.1", "100.128.0.1", "10.0.0.1", "127.0.0.1", "::1"] {
            assert!(!tailscale(ip.parse().unwrap()));
        }
    }
    #[tokio::test]
    async fn upgrade_retains_channel_children_messages_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("sticktalk.sqlite3"))
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let mut legacy = sqlx::migrate!();
        legacy.migrations =
            std::borrow::Cow::Owned(legacy.iter().filter(|m| m.version < 9).cloned().collect());
        legacy.run(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(1,'owner','','online',0); INSERT INTO guilds VALUES(2,1,'guild',NULL,0); INSERT INTO guild_members VALUES(2,1,'owner',0); INSERT INTO channels VALUES(3,'category',2,NULL,'cat',NULL,0,NULL,0),(4,'text',2,3,'text',NULL,0,NULL,0); INSERT INTO messages VALUES(5,4,1,'retained',0,NULL,NULL,NULL); INSERT INTO read_states(user_id,channel_id) VALUES(1,4);").execute(&pool).await.unwrap();
        pool.close().await;
        use clap::Parser;
        let mut config = crate::Config::parse_from(["server"]);
        config.data_dir = dir.path().into();
        config.voice.voice_bind_ip = "127.0.0.1".parse().unwrap();
        config.voice.voice_advertised_ip = Some("127.0.0.1".parse().unwrap());
        crate::admin::run(&config, &crate::admin::AdminCommand::ListUsers)
            .await
            .unwrap();
        let app = App::open(config).await.unwrap();
        let message: String = sqlx::query_scalar("SELECT content FROM messages WHERE id=5")
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
        assert_eq!(message, "retained");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT message_count FROM history_extents WHERE channel_id=4"
            )
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
            1
        );
        let parent: i64 = sqlx::query_scalar("SELECT parent_id FROM channels WHERE id=4")
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
        assert_eq!(parent, 3);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&app.0.pool)
            .await
            .unwrap()
            .is_empty());
        let mut c = app.0.pool.acquire().await.unwrap();
        let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(enabled, 1);
        assert!(
            sqlx::query("INSERT INTO voice_connect_denials VALUES(999,1)")
                .execute(&mut *c)
                .await
                .is_err()
        );
        sqlx::query("INSERT INTO channels VALUES(6,'voice',2,3,'voice',NULL,1,NULL,0)")
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("INSERT INTO voice_connect_denials VALUES(6,1)")
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("DELETE FROM channels WHERE id=6")
            .execute(&mut *c)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_connect_denials")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;
    use crate::{
        audit_tests::fixture,
        commands,
        state::{Connection, Packet},
    };
    use sqlx::Connection as _;
    use sticktalk_sfu::Authorizer;
    fn attach(core: &mut Core, user: Id) -> mpsc::Receiver<Packet> {
        let key = vec![user.0 as u8; 32];
        core.cursor(&key, user);
        let (tx, rx) = mpsc::channel(128);
        let (close, _) = tokio::sync::watch::channel(None);
        core.streams.get_mut(&key).unwrap().connection = Some(Connection {
            focused: false,
            id: format!("ws{}", user.0),
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
        rx
    }
    async fn cmd(app: &App, core: &mut Core, nonce: u64, command: Command) {
        let response = commands::execute(
            app,
            core,
            &[1; 32],
            Id(1),
            CommandRequest { nonce, command },
        )
        .await
        .unwrap();
        assert!(
            matches!(response.outcome, CommandOutcome::Ok(_)),
            "{:?}",
            response.outcome
        );
    }
    async fn call(app: &App, core: &mut Core) -> VoiceSession {
        signal_inner(
            app,
            core,
            &[2; 32],
            Id(2),
            "ws2",
            sticktalk_sfu::encode_signal(
                1,
                &VoiceSignal::Join {
                    channel_id: Id(12),
                    capabilities: VoiceCapabilities {
                        video: vec![VideoCodec::H264],
                        hardware_decode: vec![VideoCodec::H264],
                    },
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let session = *core.voice.keys().next().unwrap();
        core.voice.get_mut(&session).unwrap().joined = true;
        session
    }
    #[test]
    fn injected_advertised_address_selection_and_loopback_defaults() {
        let http = "127.0.0.1".parse().unwrap();
        let tail = "100.72.1.2".parse().unwrap();
        let lan = "192.168.1.2".parse().unwrap();
        assert_eq!(advertised_ip(None, http, vec![lan, tail]), tail);
        assert_eq!(advertised_ip(Some(lan), http, vec![tail]), lan);
        assert_eq!(advertised_ip(None, http, vec![lan]), http);
        assert!(VoiceOptions::default().voice_bind_ip.is_loopback());
        assert!(sticktalk_sfu::Config::default().bind_ip.is_loopback());
    }
    #[tokio::test]
    async fn both_branch_database_histories_upgrade_without_checksum_changes_or_data_loss() {
        for voice_history in [true, false] {
            let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
                .await
                .unwrap();
            sqlx::query("PRAGMA foreign_keys=OFF")
                .execute(&mut c)
                .await
                .unwrap();
            let mut old = sqlx::migrate!();
            old.migrations = std::borrow::Cow::Owned(
                old.iter()
                    .filter(|m| {
                        if voice_history {
                            m.version <= 9
                        } else {
                            m.version != 9 && m.version <= 202611120001
                        }
                    })
                    .cloned()
                    .collect(),
            );
            old.run(&mut c).await.unwrap();
            sqlx::raw_sql("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(1,'owner','','online',0),(2,'guest','','online',0);INSERT INTO guilds VALUES(10,1,'old',NULL,0);INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'member',0);INSERT INTO channels VALUES(11,'category',10,NULL,'category',NULL,0,NULL,0),(12,'text',10,11,'retained',NULL,0,NULL,0);INSERT INTO messages VALUES(13,12,2,'retained',0,NULL,NULL,NULL);INSERT INTO sessions VALUES(zeroblob(32),2,0,9999999999999,0);").execute(&mut c).await.unwrap();
            if voice_history {
                sqlx::raw_sql("UPDATE channels SET kind='voice' WHERE id=12;INSERT INTO voice_connect_denials VALUES(12,2);").execute(&mut c).await.unwrap();
            } else {
                sqlx::raw_sql("INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'manager',2,16384);INSERT INTO member_roles VALUES(10,2,20);INSERT INTO permission_overwrites VALUES(12,'member',2,0,524288);INSERT INTO nicknames VALUES(10,2,'retained');").execute(&mut c).await.unwrap();
            }
            db::migrate(&mut c).await.unwrap();
            db::migrate(&mut c).await.unwrap();
            let p = crate::permissions::evaluate(&mut c, Id(2), Id(10), Some(Id(12)))
                .await
                .unwrap();
            assert!(!p.contains(Permissions::CONNECT));
            assert_eq!(p.contains(Permissions::MANAGE_ROLES), !voice_history);
            assert_eq!(
                sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id=13")
                    .fetch_one(&mut c)
                    .await
                    .unwrap(),
                "retained"
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT parent_id FROM channels WHERE id=12")
                    .fetch_one(&mut c)
                    .await
                    .unwrap(),
                11
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_devices")
                    .fetch_one(&mut c)
                    .await
                    .unwrap(),
                1
            );
            assert!(sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(&mut c)
                .await
                .unwrap()
                .is_empty());
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                    .fetch_one(&mut c)
                    .await
                    .unwrap(),
                1
            );
        }
    }
    #[tokio::test]
    async fn overwrite_revokes_live_speak_stream_video_then_connect_synchronously() {
        let (_dir, app) = fixture().await;
        sqlx::query("INSERT INTO channels VALUES(12,'voice',10,NULL,'voice',NULL,1,NULL,0)")
            .execute(&app.0.pool)
            .await
            .unwrap();
        let mut core = app.0.core.lock().await;
        let _a = attach(&mut core, Id(1));
        let _b = attach(&mut core, Id(2));
        let session = call(&app, &mut core).await;
        core.voice
            .get_mut(&session)
            .unwrap()
            .state
            .participant
            .tracks = [
            TrackKind::Audio,
            TrackKind::Screen,
            TrackKind::SystemAudio,
            TrackKind::Camera,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| PublishedTrack {
            mid: i.to_string(),
            kind,
            codec: None,
            layers: vec![],
            surface: None,
        })
        .collect();
        let acl = ProjectedAcl(app.0.voice.acl.clone());
        for (nonce, deny, left) in [
            (10, Permissions::SPEAK, 3),
            (11, Permissions::SPEAK | Permissions::STREAM, 1),
            (
                12,
                Permissions::SPEAK | Permissions::STREAM | Permissions::VIDEO,
                0,
            ),
        ] {
            cmd(
                &app,
                &mut core,
                nonce,
                Command::Launch(LaunchCommand::SetOverwrites {
                    channel_id: Id(12),
                    overwrites: vec![PermissionOverwrite {
                        kind: OverwriteKind::Member,
                        target_id: Id(2),
                        allow: Permissions::empty(),
                        deny,
                    }],
                }),
            )
            .await;
            assert!(!acl.permissions(session, Id(12)).intersects(deny));
            assert_eq!(core.voice[&session].state.participant.tracks.len(), left);
        }
        cmd(
            &app,
            &mut core,
            13,
            Command::SetVoicePermission {
                channel_id: Id(12),
                user_id: Id(2),
                connect: false,
            },
        )
        .await;
        assert!(!core.voice.contains_key(&session));
        assert!(!acl.can_join(session, Id(12)));
    }
    #[tokio::test]
    async fn all_role_and_membership_mutations_revoke_live_connect() {
        for case in 0..6 {
            let (_dir, app) = fixture().await;
            sqlx::raw_sql("INSERT INTO channels VALUES(12,'voice',10,NULL,'voice',NULL,1,NULL,0);UPDATE roles SET permissions=1 WHERE id=10;INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'voice',2,524288);INSERT INTO member_roles VALUES(10,2,20);").execute(&app.0.pool).await.unwrap();
            let mut core = app.0.core.lock().await;
            let _a = attach(&mut core, Id(1));
            let _b = attach(&mut core, Id(2));
            let session = call(&app, &mut core).await;
            let command = match case {
                0 => {
                    let mut c = app.0.pool.acquire().await.unwrap();
                    let mut role = crate::permissions::roles(&mut c, Id(10))
                        .await
                        .unwrap()
                        .into_iter()
                        .find(|r| r.id == Id(20))
                        .unwrap();
                    role.permissions = Permissions::empty();
                    Command::Launch(LaunchCommand::UpdateRole { role })
                }
                1 => Command::Launch(LaunchCommand::DeleteRole {
                    guild_id: Id(10),
                    role_id: Id(20),
                }),
                2 => Command::Launch(LaunchCommand::SetMemberRoles {
                    guild_id: Id(10),
                    user_id: Id(2),
                    role_ids: vec![],
                }),
                3 => Command::KickMember {
                    guild_id: Id(10),
                    user_id: Id(2),
                },
                4 => Command::BanMember {
                    guild_id: Id(10),
                    user_id: Id(2),
                    reason: None,
                },
                _ => Command::Launch(LaunchCommand::SetOverwrites {
                    channel_id: Id(12),
                    overwrites: vec![PermissionOverwrite {
                        kind: OverwriteKind::Role,
                        target_id: Id(10),
                        allow: Permissions::empty(),
                        deny: Permissions::CONNECT,
                    }],
                }),
            };
            cmd(&app, &mut core, 10, command).await;
            assert!(!core.voice.contains_key(&session), "case {case}");
            assert!(!ProjectedAcl(app.0.voice.acl.clone()).can_join(session, Id(12)));
        }
    }
    #[tokio::test]
    async fn private_voice_visibility_and_individual_moderator_rights_use_overwrites() {
        let (_dir, app) = fixture().await;
        sqlx::raw_sql("INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO channels VALUES(12,'voice',10,NULL,'private',NULL,1,NULL,0),(13,'voice',10,NULL,'other',NULL,2,NULL,0);INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'moderator',2,8388608);INSERT INTO member_roles VALUES(10,3,20);").execute(&app.0.pool).await.unwrap();
        let mut core = app.0.core.lock().await;
        let _a = attach(&mut core, Id(1));
        let mut b = attach(&mut core, Id(2));
        let _moderator = attach(&mut core, Id(3));
        let session = call(&app, &mut core).await;
        let mut c = app.0.pool.acquire().await.unwrap();
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(3),
            session,
            VoiceModeration::Mute { muted: true },
        )
        .await
        .unwrap();
        for action in [
            VoiceModeration::Deafen { deafened: true },
            VoiceModeration::Move { channel_id: Id(13) },
            VoiceModeration::Disconnect,
        ] {
            assert!(moderate(&app, &mut core, &mut c, Id(3), session, action)
                .await
                .is_err());
        }
        sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',3,0,8388608)")
            .execute(&mut *c)
            .await
            .unwrap();
        assert!(moderate(
            &app,
            &mut core,
            &mut c,
            Id(3),
            session,
            VoiceModeration::Mute { muted: false }
        )
        .await
        .is_err());
        sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,1)")
            .execute(&mut *c)
            .await
            .unwrap();
        assert!(allowed(&mut c, Id(2), Id(12)).await.is_err());
        let ready = db::ready(&mut c, &mut core, &[2; 32], Id(2)).await.unwrap();
        assert!(!ready.channels.iter().any(|ch| ch.id == Id(12)));
        assert!(ready.voice_states.is_empty());
        while b.try_recv().is_ok() {}
        broadcast(
            &mut core,
            &mut c,
            Id(10),
            &VoiceEvent::GuildState(core_state(session)),
        )
        .await
        .unwrap();
        assert!(b.try_recv().is_err());
        reconcile(&app, &mut core, &mut c).await.unwrap();
        assert!(!core.voice.contains_key(&session));
    }
    fn core_state(session: VoiceSession) -> GuildVoiceState {
        GuildVoiceState {
            guild_id: Id(10),
            channel_id: Id(12),
            participant: VoiceParticipant {
                session,
                muted: false,
                deafened: false,
                tracks: vec![],
            },
        }
    }
    #[tokio::test]
    async fn moderation_checks_channel_roles_hierarchy_and_cannot_be_self_unmuted() {
        let (_dir, app) = fixture().await;
        sqlx::raw_sql("INSERT INTO channels VALUES(12,'voice',10,NULL,'one',NULL,1,NULL,0),(13,'voice',10,NULL,'two',NULL,2,NULL,0);").execute(&app.0.pool).await.unwrap();
        let mut core = app.0.core.lock().await;
        let _a = attach(&mut core, Id(1));
        let _b = attach(&mut core, Id(2));
        let session = call(&app, &mut core).await;
        let mut c = app.0.pool.acquire().await.unwrap();
        let acl = ProjectedAcl(app.0.voice.acl.clone());
        assert!(moderate(
            &app,
            &mut core,
            &mut c,
            Id(2),
            session,
            VoiceModeration::Mute { muted: true }
        )
        .await
        .is_err());
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Mute { muted: true },
        )
        .await
        .unwrap();
        assert!(!acl.send_audio(session));
        signal_inner(
            &app,
            &mut core,
            &[2; 32],
            Id(2),
            "ws2",
            sticktalk_sfu::encode_signal(
                2,
                &VoiceSignal::State {
                    muted: false,
                    deafened: false,
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert!(!acl.send_audio(session));
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Mute { muted: false },
        )
        .await
        .unwrap();
        assert!(acl.send_audio(session));
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Deafen { deafened: true },
        )
        .await
        .unwrap();
        assert!(!acl.receive_audio(session));
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Move { channel_id: Id(13) },
        )
        .await
        .unwrap();
        assert_eq!(core.voice[&session].state.channel_id, Id(13));
        assert!(acl.can_join(session, Id(13)));
        assert!(!acl.can_join(session, Id(12)));
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Disconnect,
        )
        .await
        .unwrap();
        assert!(!core.voice.contains_key(&session));
        // Persistent membership flags survive a fresh call lifetime and cannot
        // be bypassed by reconnecting a second device.
        let session = call(&app, &mut core).await;
        moderate(
            &app,
            &mut core,
            &mut c,
            Id(1),
            session,
            VoiceModeration::Mute { muted: true },
        )
        .await
        .unwrap();
        remove(&app, &mut core, &mut c, session).await.unwrap();
        let rejoined = call(&app, &mut core).await;
        assert!(core.voice[&rejoined].server_muted);
        assert!(!acl.send_audio(rejoined));
    }
}
