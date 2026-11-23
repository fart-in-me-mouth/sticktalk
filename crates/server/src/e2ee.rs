//! End-to-end encrypted calls on the server (`docs/E2EE.md`,
//! `docs/features/e2ee.md`). The server's whole part: the call's policy, each
//! device's public identity key bound to its session, and a relay for the
//! clients' signed announcements and pairwise key messages. It never holds a
//! media key and cannot decrypt a call; clients verify everything it relays.
//!
//! Stage channels are not end-to-end encrypted and never reach this module.
#[cfg(test)]
mod tests;

use crate::{
    db::{self, id},
    error::{require, Error, Result},
    http,
    state::{now, App, Core},
    voice,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request},
    routing::get,
};
use base64::Engine;
use sqlx::{Row, SqliteConnection};
use std::collections::HashMap;
use sticktalk_proto::*;

const LABEL: &[u8] = b"StickTalk-E2EE-v1 ";
/// Hello records kept for connections that have not joined yet.
const MAX_HELLO: usize = 1024;

#[derive(Default)]
pub(crate) struct Runtime {
    /// Connections that declared support, by session token: the connection ID.
    /// One entry per stream (a session has at most one live connection).
    hello: HashMap<Vec<u8>, String>,
    /// Every non-Stage voice session, supported or not.
    members: HashMap<VoiceSession, Member>,
}
struct Member {
    channel: Id,
    device_id: String,
    device_name: String,
    supported: bool,
    announce: Option<E2eeAnnounce>,
}

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route(
            "/api/v1/guilds/{id}/e2ee",
            get(guild_settings).put(set_guild),
        )
        .route("/api/v1/channels/{id}/e2ee", get(dm_settings).put(set_dm))
}

fn refusal() -> Error {
    let mut e = Error::new(ErrorCode::UnsupportedFeature);
    e.0.message = "This call is end-to-end encrypted, and this app can’t encrypt calls. Use the StickTalk Windows app or a current Chrome, Edge or Firefox, or ask whoever manages this server or conversation to allow unencrypted calls.".into();
    e
}

/// The policy for a call: per server (guild voice channels), per conversation (DMs).
pub(crate) async fn allow_unencrypted(
    c: &mut SqliteConnection,
    guild: Id,
    channel: Id,
) -> Result<bool> {
    let v: Option<bool> = if guild.0 == 0 {
        sqlx::query_scalar("SELECT allow_unencrypted FROM e2ee_dm_settings WHERE channel_id=?")
            .bind(id(channel)?)
            .fetch_optional(c)
            .await?
    } else {
        sqlx::query_scalar("SELECT allow_unencrypted FROM e2ee_guild_settings WHERE guild_id=?")
            .bind(id(guild)?)
            .fetch_optional(c)
            .await?
    };
    Ok(v.unwrap_or(false))
}

fn own_session(core: &Core, key: &[u8], connection: &str) -> Option<VoiceSession> {
    core.voice
        .iter()
        .find(|(_, v)| v.key() == key && v.connection_id() == connection)
        .map(|(s, _)| *s)
}

/// `Join` admission (before the session exists). Ok(true): this connection
/// will run end-to-end encryption; Ok(false): it will not (a Stage, or an
/// unsupported client in a call that allows that). Refuses an unsupported
/// client in a call that requires encryption.
pub(crate) async fn admit(
    core: &Core,
    c: &mut SqliteConnection,
    key: &[u8],
    connection: &str,
    guild: Id,
    channel: Id,
    stage: bool,
) -> Result<bool> {
    if stage {
        return Ok(false);
    }
    let supported = core.e2ee.hello.get(key).is_some_and(|c| c == connection);
    if !supported && !allow_unencrypted(c, guild, channel).await? {
        return Err(refusal());
    }
    Ok(supported)
}

async fn device(c: &mut SqliteConnection, key: &[u8]) -> Result<(String, String)> {
    let row = sqlx::query("SELECT id,device_name,platform FROM session_devices WHERE token_hash=?")
        .bind(key)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let name: String = row.get("device_name");
    let platform: String = row.get("platform");
    let label: String = format!("{name} · {platform}").chars().take(64).collect();
    Ok((row.get("id"), label))
}

/// A non-Stage voice session now exists: tell it who it is, then everyone.
pub(crate) async fn joined(
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
    stage: bool,
    supported: bool,
) -> Result<()> {
    if stage {
        return Ok(());
    }
    let entry = &core.voice[&session];
    let (key, channel, guild) = (
        entry.key().to_vec(),
        entry.state.channel_id,
        entry.state.guild_id,
    );
    let (device_id, device_name) = device(c, &key).await?;
    let allow = allow_unencrypted(c, guild, channel).await?;
    core.e2ee.members.insert(
        session,
        Member {
            channel,
            device_id: device_id.clone(),
            device_name,
            supported,
            announce: None,
        },
    );
    voice::send(
        core,
        &key,
        0,
        &VoiceEvent::E2ee(E2eeEvent::Welcome {
            channel_id: channel,
            guild_id: guild,
            session,
            device_id,
            allow_unencrypted: allow,
        }),
    )?;
    roster(core, c, channel).await
}

/// A voice session ended (any reason): the rest of its call rekeys.
pub(crate) async fn left(core: &mut Core, c: &mut SqliteConnection, session: VoiceSession) {
    if let Some(m) = core.e2ee.members.remove(&session) {
        let _ = roster(core, c, m.channel).await;
    }
}

/// A moderator moved a session to another voice channel of the same server:
/// it leaves one call and joins the other, with new keys for the new one.
pub(crate) async fn moved(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
    from: Id,
) -> Result<()> {
    let Some(m) = core.e2ee.members.get_mut(&session) else {
        return Ok(());
    };
    let entry = &core.voice[&session];
    let (to, guild) = (entry.state.channel_id, entry.state.guild_id);
    m.channel = to;
    m.announce = None;
    let supported = m.supported;
    let device_id = m.device_id.clone();
    let allow = allow_unencrypted(c, guild, to).await?;
    roster(core, c, from).await?;
    if !supported && !allow {
        return voice::remove(app, core, c, session).await;
    }
    let key = core.voice[&session].key().to_vec();
    voice::send(
        core,
        &key,
        0,
        &VoiceEvent::E2ee(E2eeEvent::Welcome {
            channel_id: to,
            guild_id: guild,
            session,
            device_id,
            allow_unencrypted: allow,
        }),
    )?;
    roster(core, c, to).await
}

/// The full member list of one call, to every session in it.
pub(crate) async fn roster(core: &mut Core, c: &mut SqliteConnection, channel: Id) -> Result<()> {
    let mut sessions: Vec<VoiceSession> = core
        .e2ee
        .members
        .iter()
        .filter(|(s, m)| m.channel == channel && core.voice.contains_key(s))
        .map(|(s, _)| *s)
        .collect();
    sessions.sort();
    let Some(first) = sessions.first() else {
        return Ok(());
    };
    let guild = core.voice[first].state.guild_id;
    let allow = allow_unencrypted(c, guild, channel).await?;
    let members = sessions
        .iter()
        .map(|s| {
            let m = &core.e2ee.members[s];
            E2eeMember {
                session: *s,
                device_id: m.device_id.clone(),
                device_name: m.device_name.clone(),
                supported: m.supported,
                announce: m.announce.clone(),
            }
        })
        .collect();
    let event = VoiceEvent::E2ee(E2eeEvent::Roster {
        channel_id: channel,
        allow_unencrypted: allow,
        members,
    });
    let envelope =
        sticktalk_sfu::encode_event(0, &event).map_err(|_| Error::new(ErrorCode::Internal))?;
    for s in sessions {
        let key = core.voice[&s].key().to_vec();
        // Membership is what clients rekey on: never skipped for a busy socket.
        let _ = voice::send_payload(core, &key, envelope.clone(), true);
    }
    Ok(())
}

fn decode(text: &str, len: usize) -> Result<Vec<u8>> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(bytes.len() == len, ErrorCode::InvalidRequest)?;
    Ok(bytes)
}
fn point(text: &str) -> Result<Vec<u8>> {
    require(text.len() == E2EE_KEY_TEXT, ErrorCode::InvalidRequest)?;
    let p = decode(text, 65)?;
    require(p[0] == 4, ErrorCode::InvalidRequest)?;
    Ok(p)
}
/// The bytes a device signs to announce itself (docs/E2EE.md 5.1).
pub(crate) fn announce_bytes(
    channel: Id,
    session: VoiceSession,
    device_id: &str,
    identity: &[u8],
    dh: &[u8],
    sig: &[u8],
) -> Vec<u8> {
    let mut out = LABEL.to_vec();
    out.extend_from_slice(b"announce");
    out.extend_from_slice(&channel.0.to_be_bytes());
    out.extend_from_slice(&session.user_id.0.to_be_bytes());
    out.extend_from_slice(&session.connection_id.0.to_be_bytes());
    out.extend_from_slice(device_id.as_bytes());
    out.extend_from_slice(identity);
    out.extend_from_slice(dh);
    out.extend_from_slice(sig);
    out
}

/// `VoiceSignal::E2ee`.
pub(crate) async fn signal(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    connection: &str,
    user: Id,
    signal: &E2eeSignal,
) -> Result<()> {
    match signal {
        E2eeSignal::Hello { version } => {
            require(*version == E2EE_VERSION, ErrorCode::VersionMismatch)?;
            core.rate(format!("e2ee-hello:{}", user.0), 60., 10.)?;
            if core.e2ee.hello.len() >= MAX_HELLO {
                let live: Vec<Vec<u8>> = core
                    .streams
                    .iter()
                    .filter(|(_, s)| s.connection.is_some())
                    .map(|(k, _)| k.clone())
                    .collect();
                core.e2ee.hello.retain(|k, _| live.contains(k));
                require(core.e2ee.hello.len() < MAX_HELLO, ErrorCode::LimitExceeded)?;
            }
            core.e2ee.hello.insert(key.to_vec(), connection.to_owned());
            Ok(())
        }
        E2eeSignal::Announce(a) => {
            core.rate(format!("e2ee-announce:{}", user.0), 30., 10.)?;
            let session = own_session(core, key, connection)
                .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
            let m = core
                .e2ee
                .members
                .get(&session)
                .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
            require(m.supported, ErrorCode::Forbidden)?;
            require(m.announce.is_none(), ErrorCode::Conflict)?;
            let (identity, dh, sig) = (point(&a.identity)?, point(&a.dh)?, point(&a.sig)?);
            require(
                a.signature.len() == E2EE_SIGNATURE_TEXT,
                ErrorCode::InvalidRequest,
            )?;
            let signature = decode(&a.signature, 64)?;
            let message = announce_bytes(m.channel, session, &m.device_id, &identity, &dh, &sig);
            ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P256_SHA256_FIXED,
                &identity,
            )
            .verify(&message, &signature)
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
            let channel = m.channel;
            // The device's key belongs to its session: deleting the session
            // (sign-out, revocation, expiry) deletes the key.
            let previous: Option<Vec<u8>> =
                sqlx::query_scalar("SELECT public_key FROM e2ee_device_keys WHERE token_hash=?")
                    .bind(key)
                    .fetch_optional(&mut *c)
                    .await?;
            if previous.as_deref() != Some(identity.as_slice()) {
                if previous.is_some() {
                    tracing::info!(user = user.0, "e2ee device key replaced");
                }
                sqlx::query("INSERT INTO e2ee_device_keys(token_hash,user_id,public_key,created_at) VALUES(?,?,?,?) ON CONFLICT(token_hash) DO UPDATE SET public_key=excluded.public_key,created_at=excluded.created_at")
                    .bind(key)
                    .bind(id(user)?)
                    .bind(&identity)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
            }
            core.e2ee.members.get_mut(&session).unwrap().announce = Some(a.clone());
            roster(core, c, channel).await
        }
        E2eeSignal::Relay { to, data } => {
            let from = own_session(core, key, connection)
                .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
            core.rate(
                format!("e2ee-relay:{}:{}", from.user_id.0, from.connection_id.0),
                1200.,
                256.,
            )?;
            require(data.len() <= E2EE_RELAY_TEXT, ErrorCode::PayloadTooLarge)?;
            require(
                data.len() >= 16
                    && data
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                ErrorCode::InvalidRequest,
            )?;
            let sender = core
                .e2ee
                .members
                .get(&from)
                .filter(|m| m.announce.is_some())
                .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
            core.e2ee
                .members
                .get(to)
                .filter(|m| m.channel == sender.channel && m.announce.is_some() && *to != from)
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            let key = core
                .voice
                .get(to)
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?
                .key()
                .to_vec();
            let envelope = sticktalk_sfu::encode_event(
                0,
                &VoiceEvent::E2ee(E2eeEvent::Relay {
                    from,
                    data: data.clone(),
                }),
            )
            .map_err(|_| Error::new(ErrorCode::Internal))?;
            voice::send_payload(core, &key, envelope, true)
        }
    }
}

/// The policy changed: every live call it covers hears at once, and a client
/// that cannot encrypt leaves a call that now requires it.
async fn changed(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    guild: Id,
    channel: Option<Id>,
) -> Result<()> {
    let mut rooms: Vec<Id> = core
        .voice
        .iter()
        .filter(|(s, v)| {
            core.e2ee.members.contains_key(s)
                && v.state.guild_id == guild
                && channel.is_none_or(|ch| v.state.channel_id == ch)
        })
        .map(|(_, v)| v.state.channel_id)
        .collect();
    rooms.sort();
    rooms.dedup();
    for room in rooms {
        if !allow_unencrypted(c, guild, room).await? {
            let out: Vec<VoiceSession> = core
                .e2ee
                .members
                .iter()
                .filter(|(_, m)| m.channel == room && !m.supported)
                .map(|(s, _)| *s)
                .collect();
            for s in out {
                if let Some(key) = core.voice.get(&s).map(|v| v.key().to_vec()) {
                    let e = refusal();
                    let _ = voice::send(
                        core,
                        &key,
                        0,
                        &VoiceEvent::Error {
                            code: e.0.code,
                            message: e.0.message,
                        },
                    );
                }
                voice::remove(app, core, c, s).await?;
            }
        }
        roster(core, c, room).await?;
    }
    Ok(())
}

// ---- REST: the "Allow unencrypted calls" setting ------------------------------

fn token(h: &HeaderMap) -> Result<String> {
    Ok(http::bearer(h)?.to_owned())
}
async fn read_body(r: Request<Body>) -> Result<bool> {
    #[derive(serde::Deserialize)]
    struct Patch {
        allow_unencrypted: bool,
    }
    let p: Patch = http::parse(r).await?;
    Ok(p.allow_unencrypted)
}
async fn guild_rights(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Permissions> {
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM guild_members WHERE guild_id=? AND user_id=?)",
    )
    .bind(id(g)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?;
    require(member, ErrorCode::NotFound)?;
    crate::permissions::evaluate(c, u, g, None).await
}
async fn guild_settings(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let rights = guild_rights(&mut c, u, Id(g)).await?;
    Ok(http::json(E2eeSettings {
        allow_unencrypted: allow_unencrypted(&mut c, Id(g), Id(0)).await?,
        can_manage: rights.contains(Permissions::MANAGE_GUILD),
    }))
}
async fn set_guild(
    State(app): State<App>,
    Path(g): Path<u64>,
    r: Request<Body>,
) -> Result<http::ApiResponse> {
    let token = token(r.headers())?;
    let allow = read_body(r).await?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("e2ee-settings:{}", u.0), 30., 10.)?;
    let g = Id(g);
    let rights = guild_rights(&mut c, u, g).await?;
    require(
        rights.contains(Permissions::MANAGE_GUILD),
        ErrorCode::Forbidden,
    )?;
    let before = allow_unencrypted(&mut c, g, Id(0)).await?;
    if before != allow {
        let mut tx = sqlx::Connection::begin(&mut *c).await?;
        sqlx::query("INSERT INTO e2ee_guild_settings(guild_id,allow_unencrypted) VALUES(?,?) ON CONFLICT(guild_id) DO UPDATE SET allow_unencrypted=excluded.allow_unencrypted")
            .bind(id(g)?)
            .bind(allow)
            .execute(&mut *tx)
            .await?;
        let word = |v: bool| Some(if v { "on" } else { "off" }.to_owned());
        crate::moderation::audit::write(
            &mut tx,
            u,
            crate::moderation::audit::Entry::new(
                g,
                AuditAction::GuildUpdate,
                AuditTarget::Guild,
                Some(g),
            )
            .change("allow_unencrypted_calls", word(before), word(allow)),
        )
        .await?;
        tx.commit().await?;
        changed(&app, &mut core, &mut c, g, None).await?;
    }
    Ok(http::json(E2eeSettings {
        allow_unencrypted: allow,
        can_manage: true,
    }))
}
/// A direct conversation: either person. A group: its owner.
async fn dm(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<bool> {
    let channel = db::access(c, u, ch, Permissions::empty()).await?;
    require(
        channel.summary.guild_id.is_none(),
        ErrorCode::InvalidRequest,
    )?;
    Ok(match channel.summary.kind {
        ChannelKind::Direct => true,
        ChannelKind::GroupDm => channel.summary.owner_id == Some(u),
        _ => return Err(Error::new(ErrorCode::InvalidRequest)),
    })
}
async fn dm_settings(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let can_manage = dm(&mut c, u, Id(ch)).await?;
    Ok(http::json(E2eeSettings {
        allow_unencrypted: allow_unencrypted(&mut c, Id(0), Id(ch)).await?,
        can_manage,
    }))
}
async fn set_dm(
    State(app): State<App>,
    Path(ch): Path<u64>,
    r: Request<Body>,
) -> Result<http::ApiResponse> {
    let token = token(r.headers())?;
    let allow = read_body(r).await?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("e2ee-settings:{}", u.0), 30., 10.)?;
    let ch = Id(ch);
    require(dm(&mut c, u, ch).await?, ErrorCode::Forbidden)?;
    if allow_unencrypted(&mut c, Id(0), ch).await? != allow {
        sqlx::query("INSERT INTO e2ee_dm_settings(channel_id,allow_unencrypted) VALUES(?,?) ON CONFLICT(channel_id) DO UPDATE SET allow_unencrypted=excluded.allow_unencrypted")
            .bind(id(ch)?)
            .bind(allow)
            .execute(&mut *c)
            .await?;
        changed(&app, &mut core, &mut c, Id(0), Some(ch)).await?;
    }
    Ok(http::json(E2eeSettings {
        allow_unencrypted: allow,
        can_manage: true,
    }))
}
