//! Soundboard, entrance sounds, clip notices, voice bitrate and camera
//! backgrounds: every route, permission check and limit.
use super::audio::tests::{opus_clip, wav_clip};
use super::*;
use crate::{
    audit_tests::fixture,
    state::{hash, Connection, Packet},
};
use axum::http::Request;
use base64::Engine;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::ServiceExt;

const VOICE: Id = Id(12);
const DM: Id = Id(20);

async fn token(app: &App, u: u8) -> String {
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

async fn request(
    app: &App,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    json: bool,
) -> Response {
    app.0.core.lock().await.limits.clear();
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if json {
        r = r.header("content-type", "application/json");
    }
    http::router(app.clone())
        .oneshot(r.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap()
}

async fn json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Guild 10 (owner 1, member 2) with voice channel 12; user 3 is only in guild
/// 30, which has its own sound. Users 1 and 3 share DM 20.
async fn world(app: &App) {
    sqlx::raw_sql(
        "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'voice',10,'Voice',1,0);\
         INSERT INTO guilds VALUES(30,3,'other',NULL,0);\
         INSERT INTO guild_members VALUES(30,3,'owner',0);\
         INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0);\
         INSERT INTO dm_pairs VALUES(1,3,20);\
         INSERT INTO dm_members VALUES(20,1,0,1),(20,3,0,1);\
         INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(3,20);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
}

async fn add_sound(app: &App, token: &str, guild: u64, name: &str) -> Response {
    request(
        app,
        token,
        "POST",
        &format!("/api/v1/guilds/{guild}/soundboard?name={name}&volume=80"),
        Some(wav_clip(400)),
        false,
    )
    .await
}

async fn sound_id(response: Response) -> Id {
    assert_eq!(response.status(), StatusCode::OK);
    Id(json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap())
}

fn key(user: u64) -> Vec<u8> {
    vec![user as u8; 32]
}

async fn attach(core: &mut Core, user: u64) -> mpsc::Receiver<Packet> {
    let key = key(user);
    core.cursor(&key, Id(user));
    let (tx, rx) = mpsc::channel(256);
    let (close, _) = tokio::sync::watch::channel(None);
    core.streams.get_mut(&key).unwrap().connection = Some(Connection {
        focused: false,
        id: format!("ws{user}"),
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
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

fn played(events: &[VoiceEvent]) -> Vec<(Id, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::SoundPlayed {
                sound, entrance, ..
            } => Some((sound.sound_id, *entrance)),
            _ => None,
        })
        .collect()
}

async fn signal(app: &App, core: &mut Core, user: u64, s: VoiceSignal, clear: bool) {
    if clear {
        core.limits.clear();
    }
    if matches!(s, VoiceSignal::Join { .. }) {
        // The web client says it can encrypt before it joins (`e2ee.rs`).
        let hello = VoiceSignal::E2ee(E2eeSignal::Hello { version: 1 });
        crate::voice::signal(
            app,
            core,
            &key(user),
            Id(user),
            &format!("ws{user}"),
            sticktalk_sfu::encode_signal(1, &hello).unwrap(),
        )
        .await
        .unwrap();
    }
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

/// Join `channel` and act as if its media connected (the SFU's `Joined`).
async fn join(app: &App, core: &mut Core, user: u64, channel: Id) -> VoiceSession {
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
        true,
    )
    .await;
    let session = *core
        .voice
        .keys()
        .find(|s| s.user_id == Id(user))
        .expect("in the call");
    core.voice.get_mut(&session).unwrap().joined = true;
    let mut c = app.0.pool.acquire().await.unwrap();
    joined(core, &mut c, session).await;
    session
}

#[tokio::test]
async fn everyone_may_use_the_soundboard_by_default_in_old_and_new_servers() {
    let (_dir, app) = fixture().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    // Guild 10 was created after the migration; its @everyone gets the bit too.
    let p = permissions::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap();
    assert!(p.contains(Permissions::USE_SOUNDBOARD));
    assert!(!p.contains(Permissions::MANAGE_SOUNDBOARD));
}

#[tokio::test]
async fn upload_needs_manage_soundboard_and_validates_the_clip() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    let outsider = token(&app, 3).await;
    assert_eq!(
        add_sound(&app, &member, 10, "horn").await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        add_sound(&app, &outsider, 10, "horn").await.status(),
        StatusCode::NOT_FOUND
    );
    let id = sound_id(add_sound(&app, &owner, 10, "horn").await).await;
    // A role can grant it without Manage Server.
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE guild_id=10 AND id=10")
        .bind(Permissions::MANAGE_SOUNDBOARD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    sound_id(add_sound(&app, &member, 10, "bell").await).await;
    // Too long, not audio, too large, bad names and volumes are all refused.
    let long = request(
        &app,
        &owner,
        "POST",
        "/api/v1/guilds/10/soundboard?name=long",
        Some(wav_clip(5400)),
        false,
    )
    .await;
    assert_eq!(long.status(), StatusCode::BAD_REQUEST);
    assert!(json(long).await["message"]
        .as_str()
        .unwrap()
        .contains("5.2"));
    for (path, body, status) in [
        (
            "/api/v1/guilds/10/soundboard?name=html",
            b"<html>".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "/api/v1/guilds/10/soundboard?name=big",
            vec![0; MAX_SOUND_BYTES + 1],
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "/api/v1/guilds/10/soundboard?name=",
            wav_clip(100),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/guilds/10/soundboard?name=loud&volume=101",
            wav_clip(100),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/guilds/10/soundboard?name=x&emoji=%3Cb%3E",
            wav_clip(100),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            request(&app, &owner, "POST", path, Some(body), false)
                .await
                .status(),
            status,
            "{path}"
        );
    }
    let opus = request(
        &app,
        &owner,
        "POST",
        "/api/v1/guilds/10/soundboard?name=opus&emoji=%F0%9F%8E%BA",
        Some(opus_clip(1000)),
        false,
    )
    .await;
    assert_eq!(opus.status(), StatusCode::OK);
    let opus = json(opus).await;
    assert_eq!(opus["format"], "ogg");
    assert_eq!(opus["emoji"], "🎺");
    assert_eq!(opus["duration_ms"], 1000);
    // The 48-sound limit.
    sqlx::query("WITH RECURSIVE n(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM n WHERE v<45) INSERT INTO soundboard_sounds(id,guild_id,name,volume,duration_ms,format,data,created_by,created_at) SELECT 1000+v,10,'filler',100,100,'wav',x'00',1,0 FROM n")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        add_sound(&app, &owner, 10, "late").await.status(),
        StatusCode::CONFLICT
    );
    // Rename, change emoji and volume; members cannot, outsiders see nothing.
    sqlx::query("UPDATE roles SET permissions=permissions&~? WHERE guild_id=10 AND id=10")
        .bind(Permissions::MANAGE_SOUNDBOARD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let path = format!("/api/v1/soundboard/sounds/{}", id.0);
    let patch = br#"{"name":"air horn","emoji":"","volume":40}"#.to_vec();
    assert_eq!(
        request(&app, &member, "PATCH", &path, Some(patch.clone()), true)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &outsider, "PATCH", &path, Some(patch.clone()), true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let renamed = json(request(&app, &owner, "PATCH", &path, Some(patch), true).await).await;
    assert_eq!(renamed["name"], "air horn");
    assert_eq!(renamed["volume"], 40);
    assert!(renamed["emoji"].is_null());
    assert_eq!(
        request(&app, &member, "DELETE", &path, None, false)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &owner, "DELETE", &path, None, false)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, &owner, "GET", &format!("{path}/audio"), None, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn library_lists_the_calls_server_first_and_reports_use_soundboard() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    sqlx::query("INSERT INTO guild_members VALUES(30,1,'member',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let other = token(&app, 3).await;
    let theirs = sound_id(add_sound(&app, &other, 30, "theirs").await).await;
    let ours = sound_id(add_sound(&app, &owner, 10, "ours").await).await;
    let lib = json(
        request(
            &app,
            &owner,
            "GET",
            &format!("/api/v1/soundboard?channel_id={}", VOICE.0),
            None,
            false,
        )
        .await,
    )
    .await;
    assert_eq!(lib["current_guild_id"], "10");
    assert_eq!(lib["can_play"], true);
    assert_eq!(lib["guilds"][0]["guild_id"], "10");
    assert_eq!(lib["guilds"][0]["can_manage"], true);
    assert_eq!(lib["guilds"][0]["sounds"][0]["id"], ours.0.to_string());
    assert_eq!(lib["guilds"][1]["sounds"][0]["id"], theirs.0.to_string());
    assert_eq!(lib["guilds"][1]["can_manage"], false);
    // Deny Use Soundboard on the voice channel for everyone.
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'role',10,0,?)")
        .bind(Permissions::USE_SOUNDBOARD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let lib = json(
        request(
            &app,
            &member,
            "GET",
            &format!("/api/v1/soundboard?channel_id={}", VOICE.0),
            None,
            false,
        )
        .await,
    )
    .await;
    assert_eq!(lib["can_play"], false);
    assert_eq!(lib["guilds"].as_array().unwrap().len(), 1);
    // A direct call always allows it.
    let lib = json(
        request(
            &app,
            &other,
            "GET",
            &format!("/api/v1/soundboard?channel_id={}", DM.0),
            None,
            false,
        )
        .await,
    )
    .await;
    assert_eq!(lib["can_play"], true);
    assert!(lib["current_guild_id"].is_null());
    // Favorites: only sounds of your own servers, listed back as strings.
    let fav = format!("/api/v1/soundboard/favorites/{}", theirs.0);
    assert_eq!(
        request(&app, &member, "PUT", &fav, None, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &owner, "PUT", &fav, None, false)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let lib = json(request(&app, &owner, "GET", "/api/v1/soundboard", None, false).await).await;
    assert_eq!(lib["favorites"][0], theirs.0.to_string());
    assert_eq!(
        request(&app, &owner, "DELETE", &fav, None, false)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn playing_checks_the_call_the_permission_the_server_and_the_rate() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let other = token(&app, 3).await;
    let ours = sound_id(add_sound(&app, &owner, 10, "ours").await).await;
    let theirs = sound_id(add_sound(&app, &other, 30, "theirs").await).await;
    let mut core = app.0.core.lock().await;
    let mut one = attach(&mut core, 1).await;
    let mut two = attach(&mut core, 2).await;
    // Not in a call.
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: ours },
        true,
    )
    .await;
    assert_eq!(error(&events(&mut one)), Some(ErrorCode::Forbidden));
    let s1 = join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    let joined_events = events(&mut one);
    assert!(joined_events.iter().any(|e| matches!(e,
        VoiceEvent::AudioQuality { bitrate, .. } if *bitrate == DEFAULT_VOICE_BITRATE)));
    events(&mut two);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: ours },
        true,
    )
    .await;
    let heard = events(&mut two);
    assert_eq!(played(&heard), vec![(ours, false)]);
    assert!(heard
        .iter()
        .any(|e| matches!(e, VoiceEvent::SoundPlayed { by, .. } if *by == s1)));
    assert_eq!(played(&events(&mut one)), vec![(ours, false)]);
    // A sound of a server the player is not in does not exist for them.
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: theirs },
        true,
    )
    .await;
    assert_eq!(error(&events(&mut one)), Some(ErrorCode::NotFound));
    assert!(played(&events(&mut two)).is_empty());
    // Use Soundboard denied in this channel, for user 2.
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
        .bind(Permissions::USE_SOUNDBOARD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    signal(
        &app,
        &mut core,
        2,
        VoiceSignal::PlaySound { sound_id: ours },
        true,
    )
    .await;
    assert_eq!(error(&events(&mut two)), Some(ErrorCode::Forbidden));
    // A moderator's mute is not bypassed.
    core.voice.get_mut(&s1).unwrap().server_mute_for_test(true);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: ours },
        true,
    )
    .await;
    assert_eq!(error(&events(&mut one)), Some(ErrorCode::Forbidden));
    core.voice.get_mut(&s1).unwrap().server_mute_for_test(false);
    // Rate: a burst of four, then a wait.
    core.limits.clear();
    for _ in 0..4 {
        signal(
            &app,
            &mut core,
            1,
            VoiceSignal::PlaySound { sound_id: ours },
            false,
        )
        .await;
    }
    assert_eq!(played(&events(&mut two)).len(), 4);
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: ours },
        false,
    )
    .await;
    assert_eq!(error(&events(&mut one)), Some(ErrorCode::RateLimited));
    assert!(played(&events(&mut two)).is_empty());
}

#[tokio::test]
async fn a_direct_call_plays_any_of_your_servers_sounds_and_the_listener_can_fetch_it() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let other = token(&app, 3).await;
    let ours = sound_id(add_sound(&app, &owner, 10, "ours").await).await;
    let audio = format!("/api/v1/soundboard/sounds/{}/audio", ours.0);
    // User 3 is not in guild 10 and not in a call with a member: nothing.
    assert_eq!(
        request(&app, &other, "GET", &audio, None, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    {
        let mut core = app.0.core.lock().await;
        let _one = attach(&mut core, 1).await;
        let mut three = attach(&mut core, 3).await;
        join(&app, &mut core, 1, DM).await;
        join(&app, &mut core, 3, DM).await;
        let got = events(&mut three);
        assert!(got.iter().any(|e| matches!(e,
            VoiceEvent::AudioQuality { bitrate, .. } if *bitrate == CALL_VOICE_BITRATE)));
        signal(
            &app,
            &mut core,
            1,
            VoiceSignal::PlaySound { sound_id: ours },
            true,
        )
        .await;
        assert_eq!(played(&events(&mut three)), vec![(ours, false)]);
    }
    // In a call with a member of its server, the listener can fetch the clip.
    let r = request(&app, &other, "GET", &audio, None, false).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "audio/wav");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn entrance_sounds_play_once_a_minute_and_respect_choices() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    let other = token(&app, 3).await;
    let ours = sound_id(add_sound(&app, &owner, 10, "ours").await).await;
    let theirs = sound_id(add_sound(&app, &other, 30, "theirs").await).await;
    let put = |t: String, body: String| {
        let app = app.clone();
        async move {
            request(
                &app,
                &t,
                "PUT",
                "/api/v1/soundboard/entrance",
                Some(body.into_bytes()),
                true,
            )
            .await
        }
    };
    // Only sounds of your own servers.
    assert_eq!(
        put(
            member.clone(),
            format!(r#"{{"enabled":true,"default_sound_id":"{}"}}"#, theirs.0)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put(
            member.clone(),
            r#"{"enabled":true,"overrides":[{"guild_id":"30","sound_id":null}]}"#.into()
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put(
            member.clone(),
            r#"{"enabled":true,"overrides":[{"guild_id":"10","sound_id":null},{"guild_id":"10","sound_id":null}]}"#.into()
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let saved = json(
        put(
            member.clone(),
            format!(r#"{{"enabled":true,"default_sound_id":"{}"}}"#, ours.0),
        )
        .await,
    )
    .await;
    assert_eq!(saved["default_sound_id"], ours.0.to_string());
    let mut core = app.0.core.lock().await;
    let mut one = attach(&mut core, 1).await;
    let _two = attach(&mut core, 2).await;
    join(&app, &mut core, 1, VOICE).await;
    events(&mut one);
    core.limits.clear();
    let s2 = join_keep_limits(&app, &mut core, 2, VOICE).await;
    assert_eq!(played(&events(&mut one)), vec![(ours, true)]);
    // Leave and rejoin at once: no second entrance sound.
    crate::voice::remove(
        &app,
        &mut core,
        &mut app.0.pool.acquire().await.unwrap(),
        s2,
    )
    .await
    .unwrap();
    join_keep_limits(&app, &mut core, 2, VOICE).await;
    assert!(played(&events(&mut one)).is_empty());
    drop(core);
    // "None in this server" overrides the default; turning it off stops it too.
    put(
        member.clone(),
        format!(
            r#"{{"enabled":true,"default_sound_id":"{}","overrides":[{{"guild_id":"10","sound_id":null}}]}}"#,
            ours.0
        ),
    )
    .await;
    let mut core = app.0.core.lock().await;
    let s2 = *core.voice.keys().find(|s| s.user_id == Id(2)).unwrap();
    crate::voice::remove(
        &app,
        &mut core,
        &mut app.0.pool.acquire().await.unwrap(),
        s2,
    )
    .await
    .unwrap();
    join(&app, &mut core, 2, VOICE).await;
    assert!(played(&events(&mut one)).is_empty());
}

/// Join without clearing rate limits, so the entrance cooldown is observable.
async fn join_keep_limits(app: &App, core: &mut Core, user: u64, channel: Id) -> VoiceSession {
    let saved: Vec<String> = core
        .limits
        .keys()
        .filter(|k| k.starts_with("sound-entrance-play"))
        .cloned()
        .collect();
    let kept: Vec<_> = saved
        .iter()
        .map(|k| (k.clone(), core.limits.remove(k).unwrap()))
        .collect();
    core.limits.clear();
    core.limits.extend(kept);
    crate::voice::signal(
        app,
        core,
        &key(user),
        Id(user),
        &format!("ws{user}"),
        sticktalk_sfu::encode_signal(1, &VoiceSignal::E2ee(E2eeSignal::Hello { version: 1 }))
            .unwrap(),
    )
    .await
    .unwrap();
    crate::voice::signal(
        app,
        core,
        &key(user),
        Id(user),
        &format!("ws{user}"),
        sticktalk_sfu::encode_signal(
            1,
            &VoiceSignal::Join {
                channel_id: channel,
                capabilities: VoiceCapabilities {
                    video: vec![VideoCodec::H264],
                    hardware_decode: vec![],
                },
            },
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let session = *core.voice.keys().find(|s| s.user_id == Id(user)).unwrap();
    core.voice.get_mut(&session).unwrap().joined = true;
    let mut c = app.0.pool.acquire().await.unwrap();
    joined(core, &mut c, session).await;
    session
}

#[tokio::test]
async fn clipping_is_announced_to_the_call_and_to_late_joiners() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut one = attach(&mut core, 1).await;
    let mut two = attach(&mut core, 2).await;
    let s1 = join(&app, &mut core, 1, VOICE).await;
    // A clip without clipping on is refused.
    signal(&app, &mut core, 1, VoiceSignal::ClipTaken, true).await;
    assert_eq!(error(&events(&mut one)), Some(ErrorCode::InvalidRequest));
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::SetClipping { enabled: true },
        true,
    )
    .await;
    assert!(events(&mut one).iter().any(|e| matches!(e,
        VoiceEvent::Clipping { session, enabled: true } if *session == s1)));
    join(&app, &mut core, 2, VOICE).await;
    assert!(events(&mut two).iter().any(|e| matches!(e,
        VoiceEvent::Clipping { session, enabled: true } if *session == s1)));
    signal(&app, &mut core, 1, VoiceSignal::ClipTaken, true).await;
    assert!(events(&mut two).iter().any(|e| matches!(e,
        VoiceEvent::ClipTaken { session } if *session == s1)));
}

#[tokio::test]
async fn voice_bitrate_needs_manage_channels_and_reaches_live_calls() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    let path = format!("/api/v1/channels/{}/voice-quality", VOICE.0);
    let got = json(request(&app, &member, "GET", &path, None, false).await).await;
    assert_eq!(got["bitrate"], DEFAULT_VOICE_BITRATE);
    assert_eq!(got["channel_id"], VOICE.0.to_string());
    let body = |v: u32| Some(format!(r#"{{"bitrate":{v}}}"#).into_bytes());
    assert_eq!(
        request(&app, &member, "PUT", &path, body(96_000), true)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for bad in [7_999, 384_001] {
        assert_eq!(
            request(&app, &owner, "PUT", &path, body(bad), true)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    // A text channel has no bitrate.
    assert_eq!(
        request(
            &app,
            &owner,
            "PUT",
            "/api/v1/channels/11/voice-quality",
            body(96_000),
            true
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let (s2, mut two) = {
        let mut core = app.0.core.lock().await;
        let rx = attach(&mut core, 2).await;
        (join(&app, &mut core, 2, VOICE).await, rx)
    };
    events(&mut two);
    let saved = request(&app, &owner, "PUT", &path, body(256_000), true).await;
    assert_eq!(saved.status(), StatusCode::OK);
    assert!(events(&mut two).iter().any(|e| matches!(
        e,
        VoiceEvent::AudioQuality {
            bitrate: 256_000,
            ..
        }
    )));
    assert_eq!(crate::voice::acl_bitrate_for_test(&app, s2), Some(256_000));
    let got = json(request(&app, &member, "GET", &path, None, false).await).await;
    assert_eq!(got["bitrate"], 256_000);
}

#[tokio::test]
async fn camera_backgrounds_are_private_bounded_and_typed_by_content() {
    let (_dir, app) = fixture().await;
    let one = token(&app, 1).await;
    let two = token(&app, 2).await;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(64, 36)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let base = "/api/v1/users/@me/camera-backgrounds";
    let created = request(&app, &one, "POST", base, Some(png.clone()), false).await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json(created).await;
    assert_eq!(created["kind"], "image");
    assert_eq!(created["mime"], "image/png");
    let item = format!("{base}/{}", created["id"].as_str().unwrap());
    let mut mp4 = vec![0, 0, 0, 24];
    mp4.extend_from_slice(b"ftypisom");
    mp4.extend_from_slice(&[0; 64]);
    let video = json(request(&app, &one, "POST", base, Some(mp4), false).await).await;
    assert_eq!(video["kind"], "video");
    // Someone else can neither list nor fetch nor delete it.
    assert_eq!(
        json(request(&app, &two, "GET", base, None, false).await).await,
        serde_json::json!([])
    );
    assert_eq!(
        request(&app, &two, "GET", &item, None, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &two, "DELETE", &item, None, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let r = request(&app, &one, "GET", &item, None, false).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "image/png");
    // Not a picture or video; a broken picture; too large.
    for (body, status) in [
        (
            b"<svg onload=alert(1)>".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            b"\x89PNG\r\n\x1a\nbroken".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            vec![0xff; MAX_CAMERA_BACKGROUND_BYTES + 1],
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        assert_eq!(
            request(&app, &one, "POST", base, Some(body), false)
                .await
                .status(),
            status
        );
    }
    // Ten at most.
    for _ in 2..MAX_CAMERA_BACKGROUNDS {
        assert_eq!(
            request(&app, &one, "POST", base, Some(png.clone()), false)
                .await
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        request(&app, &one, "POST", base, Some(png.clone()), false)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, &one, "DELETE", &item, None, false)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        json(request(&app, &one, "GET", base, None, false).await)
            .await
            .as_array()
            .unwrap()
            .len(),
        MAX_CAMERA_BACKGROUNDS - 1
    );
}

/// Audit log, AutoMod and timeouts on the soundboard and voice quality.
mod integration;
