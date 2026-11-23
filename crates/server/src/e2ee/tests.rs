//! End-to-end encrypted calls: admission and policy, the session-bound device
//! key, the relay's checks, settings permissions, and the frame format checked
//! against the JavaScript implementation's vectors with an independent one.
use super::*;
use crate::{
    audit_tests::fixture,
    state::{hash, Connection, Packet},
};
use axum::{body::to_bytes, http::StatusCode, response::Response};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::ServiceExt;

const VOICE: Id = Id(12);
const DM: Id = Id(20);
const GROUP: Id = Id(21);

/// Guild 10 (owner 1, members 2 and 3) with voice channel 12; users 1 and 2
/// share DM 20; group 21 (owner 1) has 1, 2 and 3.
async fn world(app: &App) {
    sqlx::raw_sql(
        "INSERT INTO guild_members VALUES(10,3,'member',0);\
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'voice',10,'Voice',1,0);\
         INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0);\
         INSERT INTO dm_pairs VALUES(1,2,20);\
         INSERT INTO channels(id,kind,name,position,owner_id,created_at) VALUES(21,'group_dm','group',0,1,0);\
         INSERT INTO dm_members VALUES(20,1,0,1),(20,2,0,1),(21,1,0,1),(21,2,0,1),(21,3,0,1);\
         INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(2,20),(1,21),(2,21),(3,21);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
}

fn key(user: u64) -> Vec<u8> {
    vec![user as u8; 32]
}
fn attach(core: &mut Core, user: u64) -> mpsc::Receiver<Packet> {
    let key = key(user);
    core.cursor(&key, Id(user));
    let (tx, rx) = mpsc::channel(512);
    let (close, _) = tokio::sync::watch::channel(None);
    core.streams.get_mut(&key).unwrap().connection = Some(Connection {
        focused: false,
        id: format!("ws{user}"),
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: Arc::new(tokio::sync::Semaphore::new(4 * 1024 * 1024)),
        close,
    });
    rx
}
fn events(rx: &mut mpsc::Receiver<Packet>) -> Vec<VoiceEvent> {
    let mut out = vec![];
    while let Ok(packet) = rx.try_recv() {
        if let ServerMsg::Voice(v) = rmp_serde::from_slice::<ServerMsg>(&packet.bytes).unwrap() {
            out.push(rmp_serde::from_slice(&v.payload).unwrap());
        }
    }
    out
}
fn error(events: &[VoiceEvent]) -> Option<ErrorCode> {
    events.iter().find_map(|e| match e {
        VoiceEvent::Error { code, .. } => Some(*code),
        _ => None,
    })
}
fn rosters(events: &[VoiceEvent]) -> Vec<(bool, Vec<E2eeMember>)> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::E2ee(E2eeEvent::Roster {
                allow_unencrypted,
                members,
                ..
            }) => Some((*allow_unencrypted, members.clone())),
            _ => None,
        })
        .collect()
}
async fn signal(app: &App, core: &mut Core, user: u64, s: VoiceSignal) {
    core.limits.clear();
    crate::voice::signal(
        app,
        core,
        &key(user),
        Id(user),
        &format!("ws{user}"),
        sticktalk_sfu::encode_signal(1, &s).unwrap(),
    )
    .await
    .unwrap();
}
async fn join(
    app: &App,
    core: &mut Core,
    user: u64,
    channel: Id,
    hello: bool,
) -> Option<VoiceSession> {
    if hello {
        signal(
            app,
            core,
            user,
            VoiceSignal::E2ee(E2eeSignal::Hello { version: 1 }),
        )
        .await;
    }
    signal(
        app,
        core,
        user,
        VoiceSignal::Join {
            channel_id: channel,
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264],
                hardware_decode: vec![],
            },
        },
    )
    .await;
    core.voice.keys().find(|s| s.user_id == Id(user)).copied()
}

struct Device {
    pair: EcdsaKeyPair,
    rng: ring::rand::SystemRandom,
}
fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}
impl Device {
    fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        Self { pair, rng }
    }
    fn public(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }
    /// A correctly signed announcement (the call keys are any valid points).
    fn announce(&self, channel: Id, session: VoiceSession, device_id: &str) -> E2eeAnnounce {
        let (identity, dh, sig) = (
            self.public(),
            Device::new().public(),
            Device::new().public(),
        );
        let msg = announce_bytes(channel, session, device_id, &identity, &dh, &sig);
        let signature = self.pair.sign(&self.rng, &msg).unwrap();
        E2eeAnnounce {
            identity: b64(&identity),
            dh: b64(&dh),
            sig: b64(&sig),
            signature: b64(signature.as_ref()),
        }
    }
}
async fn device_id(app: &App, user: u64) -> String {
    sqlx::query_scalar("SELECT id FROM session_devices WHERE token_hash=?")
        .bind(key(user))
        .fetch_one(&app.0.pool)
        .await
        .unwrap()
}

async fn bearer(app: &App, u: u8) -> String {
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u + 100; 32]);
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(hash(token.as_bytes()))
        .bind(i64::from(u))
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
async fn request(app: &App, token: &str, method: &str, path: &str, body: Option<&str>) -> Response {
    app.0.core.lock().await.limits.clear();
    let r = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.unwrap_or_default().to_owned()))
        .unwrap();
    http::router(app.clone()).oneshot(r).await.unwrap()
}
async fn json(response: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[tokio::test]
async fn encrypted_calls_refuse_clients_that_cannot_encrypt_unless_allowed() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut rx1 = attach(&mut core, 1);
    let mut rx2 = attach(&mut core, 2);
    let mut rx3 = attach(&mut core, 3);
    // No Hello: refused before any media session exists, with a clear message.
    assert_eq!(join(&app, &mut core, 1, VOICE, false).await, None);
    let refused = events(&mut rx1);
    assert_eq!(error(&refused), Some(ErrorCode::UnsupportedFeature));
    assert!(refused.iter().any(|e| matches!(e, VoiceEvent::Error { message, .. } if message.contains("end-to-end encrypted"))));
    assert!(core.e2ee.members.is_empty());
    // With Hello: Welcome with this device's ID, then the roster.
    let s1 = join(&app, &mut core, 1, VOICE, true).await.unwrap();
    let got = events(&mut rx1);
    let welcome = got.iter().find_map(|e| match e {
        VoiceEvent::E2ee(E2eeEvent::Welcome {
            session,
            device_id,
            allow_unencrypted,
            ..
        }) => Some((*session, device_id.clone(), *allow_unencrypted)),
        _ => None,
    });
    assert_eq!(welcome, Some((s1, device_id(&app, 1).await, false)));
    let r = rosters(&got);
    assert_eq!(r.last().unwrap().1.len(), 1);
    assert!(r.last().unwrap().1[0].supported);
    // Encrypted sessions get the frame overhead on top of the policed bitrate.
    assert_eq!(
        crate::voice::acl_policed_bitrate_for_test(&app, s1),
        Some(u64::from(DEFAULT_VOICE_BITRATE) + E2EE_AUDIO_OVERHEAD_BPS)
    );
    drop(core);
    // The server's owner allows unencrypted calls: user 2 may join without support.
    let owner = bearer(&app, 1).await;
    let r = request(
        &app,
        &owner,
        "PUT",
        "/api/v1/guilds/10/e2ee",
        Some(r#"{"allow_unencrypted":true}"#),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let mut core = app.0.core.lock().await;
    let roster = rosters(&events(&mut rx1));
    assert!(roster.last().unwrap().0, "live calls hear the change");
    let s2 = join(&app, &mut core, 2, VOICE, false).await.unwrap();
    assert_eq!(
        crate::voice::acl_policed_bitrate_for_test(&app, s2),
        Some(u64::from(DEFAULT_VOICE_BITRATE))
    );
    let seen = rosters(&events(&mut rx1));
    let members = &seen.last().unwrap().1;
    assert_eq!(members.len(), 2);
    assert!(members.iter().any(|m| m.session == s2 && !m.supported));
    let _ = events(&mut rx2);
    drop(core);
    // Turned off again: the unsupported session leaves the call, told why.
    let r = request(
        &app,
        &owner,
        "PUT",
        "/api/v1/guilds/10/e2ee",
        Some(r#"{"allow_unencrypted":false}"#),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let mut core = app.0.core.lock().await;
    assert!(!core.voice.contains_key(&s2));
    assert_eq!(
        error(&events(&mut rx2)),
        Some(ErrorCode::UnsupportedFeature)
    );
    assert_eq!(rosters(&events(&mut rx1)).last().unwrap().1.len(), 1);
    // DMs use their own setting.
    assert_eq!(join(&app, &mut core, 3, GROUP, false).await, None);
    assert_eq!(
        error(&events(&mut rx3)),
        Some(ErrorCode::UnsupportedFeature)
    );
}

#[tokio::test]
async fn announcements_are_verified_bound_to_the_session_and_relayed_only_within_the_call() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut rx1 = attach(&mut core, 1);
    let mut rx2 = attach(&mut core, 2);
    let mut rx3 = attach(&mut core, 3);
    let s1 = join(&app, &mut core, 1, DM, true).await.unwrap();
    let s2 = join(&app, &mut core, 2, DM, true).await.unwrap();
    let (d1, d2) = (Device::new(), Device::new());
    let id1 = device_id(&app, 1).await;
    // A relay before announcing is refused.
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Relay {
            to: s2,
            data: "A".repeat(40),
        }),
    )
    .await;
    assert_eq!(error(&events(&mut rx1)), Some(ErrorCode::Forbidden));
    // Signed for another session, or with another key: refused, nothing stored.
    for bad in [
        d1.announce(DM, s2, &id1),
        d1.announce(VOICE, s1, &id1),
        E2eeAnnounce {
            identity: b64(&d2.public()),
            ..d1.announce(DM, s1, &id1)
        },
        E2eeAnnounce {
            signature: "A".repeat(86),
            ..d1.announce(DM, s1, &id1)
        },
        E2eeAnnounce {
            dh: "A".repeat(87),
            ..d1.announce(DM, s1, &id1)
        },
    ] {
        signal(
            &app,
            &mut core,
            1,
            VoiceSignal::E2ee(E2eeSignal::Announce(bad)),
        )
        .await;
        assert_eq!(error(&events(&mut rx1)), Some(ErrorCode::InvalidRequest));
    }
    let stored = || async {
        sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT public_key FROM e2ee_device_keys WHERE token_hash=?",
        )
        .bind(key(1))
        .fetch_optional(&app.0.pool)
        .await
        .unwrap()
    };
    assert_eq!(stored().await, None);
    let a1 = d1.announce(DM, s1, &id1);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Announce(a1.clone())),
    )
    .await;
    assert_eq!(stored().await, Some(d1.public()));
    // Everyone in the call sees it; the second announcement for a session is refused.
    let seen = rosters(&events(&mut rx2));
    assert!(seen
        .last()
        .unwrap()
        .1
        .iter()
        .any(|m| m.session == s1 && m.announce.as_ref() == Some(&a1)));
    let _ = events(&mut rx1);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Announce(a1.clone())),
    )
    .await;
    assert_eq!(error(&events(&mut rx1)), Some(ErrorCode::Conflict));
    let a2 = d2.announce(DM, s2, &device_id(&app, 2).await);
    signal(
        &app,
        &mut core,
        2,
        VoiceSignal::E2ee(E2eeSignal::Announce(a2)),
    )
    .await;
    let _ = (events(&mut rx1), events(&mut rx2));
    // A relay reaches exactly its target, named by the server as the sender.
    let data = "ABCDEFGHIJKLMNOPQRSTUVWXYZ_-0123456789".to_owned();
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Relay {
            to: s2,
            data: data.clone(),
        }),
    )
    .await;
    let got = events(&mut rx2);
    assert!(got.contains(&VoiceEvent::E2ee(E2eeEvent::Relay {
        from: s1,
        data: data.clone()
    })));
    assert!(events(&mut rx3).is_empty());
    // To nobody in the call, to itself, too large, not base64url: refused.
    let stranger = VoiceSession {
        user_id: Id(3),
        connection_id: Id(77),
    };
    for (to, data, code) in [
        (stranger, data.clone(), ErrorCode::NotFound),
        (s1, data.clone(), ErrorCode::NotFound),
        (
            s2,
            "A".repeat(E2EE_RELAY_TEXT + 1),
            ErrorCode::PayloadTooLarge,
        ),
        (s2, "not base64 !!!!!!!!!".into(), ErrorCode::InvalidRequest),
    ] {
        signal(
            &app,
            &mut core,
            1,
            VoiceSignal::E2ee(E2eeSignal::Relay { to, data }),
        )
        .await;
        assert_eq!(error(&events(&mut rx1)), Some(code));
    }
    // A session outside the call cannot relay into it.
    let _s3 = join(&app, &mut core, 3, VOICE, true).await.unwrap();
    let _ = events(&mut rx3);
    signal(
        &app,
        &mut core,
        3,
        VoiceSignal::E2ee(E2eeSignal::Relay {
            to: s2,
            data: data.clone(),
        }),
    )
    .await;
    assert_eq!(error(&events(&mut rx3)), Some(ErrorCode::Forbidden));
    // Leaving: the others get the roster without it, at once.
    signal(&app, &mut core, 2, VoiceSignal::Leave).await;
    let after = rosters(&events(&mut rx1));
    assert_eq!(
        after
            .last()
            .unwrap()
            .1
            .iter()
            .map(|m| m.session)
            .collect::<Vec<_>>(),
        vec![s1]
    );
    drop(core);
    // Revoking the session (sign-out) deletes its device key.
    sqlx::query("DELETE FROM sessions WHERE token_hash=?")
        .bind(key(1))
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(stored().await, None);
}

#[tokio::test]
async fn hello_must_be_the_supported_version_and_stages_are_never_encrypted() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut rx1 = attach(&mut core, 1);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Hello { version: 2 }),
    )
    .await;
    assert_eq!(error(&events(&mut rx1)), Some(ErrorCode::VersionMismatch));
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(
        !admit(&core, &mut c, &key(1), "ws1", Id(10), Id(13), true)
            .await
            .unwrap(),
        "a Stage admits anyone, unencrypted"
    );
    assert!(admit(&core, &mut c, &key(1), "ws1", Id(10), VOICE, false)
        .await
        .is_err());
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::E2ee(E2eeSignal::Hello { version: 1 }),
    )
    .await;
    assert!(admit(&core, &mut c, &key(1), "ws1", Id(10), VOICE, false)
        .await
        .unwrap());
    assert!(
        admit(&core, &mut c, &key(1), "ws9", Id(10), VOICE, false)
            .await
            .is_err(),
        "Hello is per connection"
    );
}

#[tokio::test]
async fn the_setting_needs_manage_server_or_the_conversation() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let (owner, member, outsider) = (
        bearer(&app, 1).await,
        bearer(&app, 2).await,
        bearer(&app, 4).await,
    );
    let on = Some(r#"{"allow_unencrypted":true}"#);
    let get = json(request(&app, &member, "GET", "/api/v1/guilds/10/e2ee", None).await).await;
    assert_eq!(
        get,
        serde_json::json!({"allow_unencrypted": false, "can_manage": false})
    );
    assert_eq!(
        request(&app, &member, "PUT", "/api/v1/guilds/10/e2ee", on)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &outsider, "GET", "/api/v1/guilds/10/e2ee", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &outsider, "PUT", "/api/v1/guilds/10/e2ee", on)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &owner, "PUT", "/api/v1/guilds/10/e2ee", Some("{}"))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, &owner, "PUT", "/api/v1/guilds/10/e2ee", on)
            .await
            .status(),
        StatusCode::OK
    );
    let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE guild_id=10")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(audit, 1, "the change is in the server's audit log");
    // A direct conversation: either person. A group: its owner. Others: not found.
    assert_eq!(
        request(&app, &member, "PUT", "/api/v1/channels/20/e2ee", on)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        json(request(&app, &owner, "GET", "/api/v1/channels/20/e2ee", None).await).await
            ["allow_unencrypted"],
        true
    );
    assert_eq!(
        request(&app, &member, "PUT", "/api/v1/channels/21/e2ee", on)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &owner, "PUT", "/api/v1/channels/21/e2ee", on)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(&app, &outsider, "GET", "/api/v1/channels/20/e2ee", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &owner, "PUT", "/api/v1/channels/12/e2ee", on)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

// ---- an independent implementation of the frame format (docs/E2EE.md 5.4) ----

struct Len(usize);
impl ring::hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}
fn hkdf(ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, &[]).extract(ikm);
    let mut out = vec![0; len];
    prk.expand(&[info], Len(len))
        .unwrap()
        .fill(&mut out)
        .unwrap();
    out
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
/// NAL units as (payload start, end); a 4-byte start code absorbs one zero.
fn nal_units(b: &[u8]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i + 2] > 1 {
            i += 3;
        } else if b[i + 2] == 1 {
            if b[i] == 0 && b[i + 1] == 0 {
                let start = if i > 0 && b[i - 1] == 0 { i - 1 } else { i };
                if let Some(last) = out.last_mut() {
                    last.1 = start;
                }
                out.push((i + 3, b.len()));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}
fn slice_prefix(b: &[u8]) -> usize {
    // Three ue(v) values from the first bit; the test frames have no 00 00 03.
    let (mut bit, mut fields) = (0usize, 0);
    while fields < 3 {
        let mut zeros = 0;
        while (b[bit / 8] >> (7 - bit % 8)) & 1 == 0 {
            zeros += 1;
            bit += 1;
        }
        bit += 1 + zeros;
        fields += 1;
    }
    bit.div_ceil(8)
}
fn open(key: &[u8], salt: &[u8], data: &[u8], video: bool) -> Vec<u8> {
    let n = data.len();
    assert_eq!(data[n - 1], 0xe2);
    let len = usize::from(data[n - 3]) << 8 | usize::from(data[n - 2]);
    let trailer = n - 3 - len;
    let config = data[trailer];
    let kid_len = if config & 0x80 != 0 {
        usize::from((config >> 4) & 7) + 1
    } else {
        0
    };
    let ctr_len = if config & 0x08 != 0 {
        usize::from(config & 7) + 1
    } else {
        0
    };
    let header = &data[trailer..trailer + 1 + kid_len + ctr_len];
    let ctr = if ctr_len == 0 {
        u64::from(config & 7)
    } else {
        header[1 + kid_len..]
            .iter()
            .fold(0u64, |a, b| a << 8 | u64::from(*b))
    };
    let body_end = trailer - 16;
    let mut ranges = vec![];
    let mut metadata = vec![];
    let mut cipher = vec![];
    if video {
        for (payload, end) in nal_units(&data[..body_end]) {
            let t = data[payload] & 31;
            let clear = match t {
                7..=12 => end,
                1..=5 => payload + 1 + slice_prefix(&data[payload + 1..end]),
                _ => payload + 1,
            };
            metadata.extend_from_slice(&((end - payload) as u32).to_be_bytes());
            metadata.extend_from_slice(&data[payload..clear]);
            cipher.extend_from_slice(&data[clear..end]);
            ranges.push((clear, end));
        }
    } else {
        cipher.extend_from_slice(&data[..body_end]);
    }
    cipher.extend_from_slice(&data[body_end..trailer]);
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(salt);
    for (i, b) in ctr.to_be_bytes().iter().enumerate() {
        nonce[4 + i] ^= b;
    }
    let aead = ring::aead::LessSafeKey::new(
        ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, key).unwrap(),
    );
    let mut aad = header.to_vec();
    aad.extend_from_slice(&metadata);
    let plain = aead
        .open_in_place(
            ring::aead::Nonce::assume_unique_for_key(nonce),
            ring::aead::Aad::from(aad),
            &mut cipher,
        )
        .expect("decrypts")
        .to_vec();
    if !video {
        return plain;
    }
    let mut out = data[..body_end].to_vec();
    let mut at = 0;
    for (clear, end) in ranges {
        out[clear..end].copy_from_slice(&plain[at..at + end - clear]);
        at += end - clear;
    }
    out
}

#[test]
fn frame_vectors_from_the_web_client_decrypt_with_an_independent_implementation() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../../web/tests/fixtures/e2ee-vectors.json"
    ))
    .unwrap();
    let secret = unhex(doc["secret"].as_str().unwrap());
    let vectors = doc["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 4);
    for v in vectors {
        let kind = v["kind"].as_str().unwrap();
        let kid = v["kid"].as_u64().unwrap();
        // The track base key, then RFC 9605's key schedule (suite 0x0004).
        let base = hkdf(
            &secret,
            format!("StickTalk-E2EE-v1 track {kind}").as_bytes(),
            16,
        );
        let label = |what: &str| [what.as_bytes(), &kid.to_be_bytes(), &[0, 4]].concat();
        let key = hkdf(&base, &label("SFrame 1.0 Secret key "), 16);
        let salt = hkdf(&base, &label("SFrame 1.0 Secret salt "), 12);
        assert_eq!(key, unhex(v["key"].as_str().unwrap()), "{kind} key");
        assert_eq!(salt, unhex(v["salt"].as_str().unwrap()), "{kind} salt");
        let video = matches!(kind, "camera" | "screen");
        let frame = open(&key, &salt, &unhex(v["sealed"].as_str().unwrap()), video);
        assert_eq!(frame, unhex(v["frame"].as_str().unwrap()), "{kind} frame");
    }
}
