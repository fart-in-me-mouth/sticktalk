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
use sticktalk_sfu::{Command as MediaCommand, Sfu, SignalOut};
use tokio::sync::mpsc;

#[derive(Args, Clone, Debug)]
pub struct VoiceOptions {
    #[arg(long, env = "STICKTALK_VOICE_BIND_IP", default_value = "0.0.0.0")]
    pub voice_bind_ip: IpAddr,
    /// Default: an assigned Tailscale 100.64/10 address, else the HTTP bind IP.
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
            voice_bind_ip: "0.0.0.0".parse().unwrap(),
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
    expires: u64,
    alive: Arc<std::sync::atomic::AtomicBool>,
}
type Projection = Arc<RwLock<HashMap<VoiceSession, Lease>>>;
pub(crate) struct Voice {
    commands: mpsc::Sender<MediaCommand>,
    acl: Projection,
    pub address: SocketAddr,
    driver: std::sync::Mutex<Option<(Sfu, mpsc::Receiver<MediaCommand>)>>,
}
pub(crate) struct Entry {
    key: Vec<u8>,
    connection: String,
    pub state: GuildVoiceState,
    pub joined: bool,
}
impl Voice {
    pub fn new(options: &VoiceOptions, http: SocketAddr) -> Result<Self> {
        let advertised = options
            .voice_advertised_ip
            .or_else(|| {
                if_addrs::get_if_addrs()
                    .ok()?
                    .into_iter()
                    .map(|a| a.ip())
                    .find(|ip| tailscale(*ip))
            })
            .unwrap_or(http.ip());
        let acl: Projection = Default::default();
        let projection = acl.clone();
        let sfu = Sfu::new(
            sticktalk_sfu::Config {
                bind_ip: options.voice_bind_ip,
                advertised_ip: advertised,
                ports: options.voice_udp_port_start..=options.voice_udp_port_end,
                video_egress_bps: options.voice_egress_bps,
                max_datagram_bytes: options.voice_max_datagram_bytes,
                ..Default::default()
            },
            Arc::new(move |session: VoiceSession, channel: Id| {
                projection.read().ok().is_some_and(|p| {
                    p.get(&session).is_some_and(|lease| {
                        lease.channel == channel
                            && now() < lease.expires
                            && lease.alive.load(std::sync::atomic::Ordering::Acquire)
                    })
                })
            }),
        )
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let address = sfu.local_addr();
        let (commands, receiver) = mpsc::channel(128);
        Ok(Self {
            commands,
            acl,
            address,
            driver: std::sync::Mutex::new(Some((sfu, receiver))),
        })
    }
    fn command(&self, command: MediaCommand) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| Error::new(ErrorCode::Unavailable))
    }
    pub fn clear(&self) {
        self.acl.write().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
pub(crate) fn spawn(app: &App) {
    let Some((sfu, commands)) = app
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
    let stop = app.0.shutdown.clone();
    let weak = Arc::downgrade(&app.0);
    tokio::spawn(async move {
        tokio::select! {
            _ = stop.cancelled() => {},
            result = sfu.run(commands, tx) => {
                if let Err(error) = result { tracing::error!(%error, "SFU stopped; shutting down fail closed"); }
                if let Some(inner) = weak.upgrade() { App(inner).shutdown(); }
            }
        }
    });
    let stop = app.0.shutdown.clone();
    let weak = Arc::downgrade(&app.0);
    tokio::spawn(async move {
        loop {
            let signal =
                tokio::select! { _ = stop.cancelled() => break, signal = rx.recv() => signal };
            let (Some(signal), Some(inner)) = (signal, weak.upgrade()) else {
                break;
            };
            let app = App(inner);
            let mut core = app.0.core.lock().await;
            let result = async {
                let mut c = app.0.pool.acquire().await?;
                output(&app, &mut core, &mut c, signal).await
            }
            .await;
            if result.is_err() {
                app.shutdown();
                break;
            }
        }
    });
}
fn send(core: &mut Core, key: &[u8], nonce: u64, event: &VoiceEvent) -> Result<()> {
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
        send(core, &key, 0, event)?;
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
    let denied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM voice_connect_denials WHERE channel_id=? AND user_id=?)",
    )
    .bind(db::id(channel)?)
    .bind(db::id(user)?)
    .fetch_one(&mut *c)
    .await?;
    let guild_id = ch
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    require(
        !denied || db::role(c, user, guild_id).await? == MemberRole::Owner,
        ErrorCode::Forbidden,
    )?;
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
    app.0
        .voice
        .acl
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session);
    if let Some(entry) = core.voice.remove(&session) {
        // A full command queue cannot keep media access alive: ACL is already gone.
        if app
            .0
            .voice
            .command(MediaCommand::Disconnect(session))
            .is_err()
        {
            app.shutdown();
        }
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
        }
    }
    Ok(())
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
        app.0
            .voice
            .acl
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                session,
                Lease {
                    channel: channel_id,
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
                state: GuildVoiceState {
                    guild_id,
                    channel_id,
                    participant: VoiceParticipant {
                        session,
                        muted: false,
                        deafened: false,
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
                let state = entry.state.clone();
                broadcast(core, c, state.guild_id, &VoiceEvent::GuildState(state)).await?;
            }
        }
        VoiceEvent::Left { session } if *session == signal.session => {
            return remove(app, core, c, *session).await;
        }
        VoiceEvent::Error { .. } if !entry.joined => {
            remove(app, core, c, signal.session).await?;
        }
        _ => {}
    }
    core.send(&key, &ServerMsg::Voice(signal.envelope))?;
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
    let guilds: Vec<i64> = sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=?")
        .bind(db::id(user)?)
        .fetch_all(c)
        .await?;
    let states = core
        .voice
        .values()
        .filter(|v| v.joined && guilds.contains(&(v.state.guild_id.0 as i64)))
        .map(|v| v.state.clone())
        .collect();
    send(core, key, 0, &VoiceEvent::GuildSnapshot { states })
}

#[cfg(test)]
mod tests {
    use super::*;
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
