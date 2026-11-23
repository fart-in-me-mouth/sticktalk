//! Activity sessions: every signal, permission check and limit, the REST
//! routes, the frames' policies, invite cards, moderation and reports.
use super::*;
use crate::{
    audit_tests::fixture,
    http,
    state::{hash, Connection, Packet},
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::Response,
};
use base64::Engine as _;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::ServiceExt;

pub(crate) const VOICE: Id = Id(12);
const STAGE: Id = Id(13);
const VOICE2: Id = Id(14);
const DM: Id = Id(20);

/// Guild 10 (owner 1, member 2, member 4) with text 11, voice 12 and 14 and
/// stage 13; users 1 and 3 share DM 20.
pub(crate) async fn world(app: &App) {
    sqlx::raw_sql(
        "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'voice',10,'Voice',1,0),(13,'stage',10,'Stage',2,0),(14,'voice',10,'Voice 2',3,0);\
         INSERT INTO guild_members VALUES(10,4,'member',0);\
         INSERT INTO read_states(user_id,channel_id) VALUES(1,12),(2,12),(4,12),(4,11);\
         INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0);\
         INSERT INTO dm_pairs VALUES(1,3,20);\
         INSERT INTO dm_members VALUES(20,1,0,1),(20,3,0,1);\
         INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(3,20);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
}

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

async fn request(app: &App, token: &str, method: &str, path: &str, body: Option<&str>) -> Response {
    app.0.core.lock().await.limits.clear();
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        r = r.header("content-type", "application/json");
    }
    http::router(app.clone())
        .oneshot(
            r.body(Body::from(body.unwrap_or_default().to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn key(user: u64) -> Vec<u8> {
    vec![user as u8; 32]
}

pub(crate) fn attach(core: &mut Core, user: u64) -> mpsc::Receiver<Packet> {
    let key = key(user);
    core.cursor(&key, Id(user));
    let (tx, rx) = mpsc::channel(512);
    let (close, _) = tokio::sync::watch::channel(None);
    core.streams.get_mut(&key).unwrap().connection = Some(Connection {
        focused: false,
        id: format!("ws{user}"),
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: Arc::new(tokio::sync::Semaphore::new(8 * 1024 * 1024)),
        close,
    });
    rx
}

/// Everything a connection received: voice events and ordinary events.
fn drain(rx: &mut mpsc::Receiver<Packet>) -> (Vec<VoiceEvent>, Vec<Event>) {
    let (mut voice, mut events) = (vec![], vec![]);
    while let Ok(packet) = rx.try_recv() {
        match rmp_serde::from_slice::<ServerMsg>(&packet.bytes).unwrap() {
            ServerMsg::Voice(v) => voice.push(rmp_serde::from_slice(&v.payload).unwrap()),
            ServerMsg::Event(e) => events.push(e.event),
            _ => {}
        }
    }
    (voice, events)
}

fn activity(events: &[VoiceEvent]) -> Vec<ActivityEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::Activities(a) => Some(a.clone()),
            _ => None,
        })
        .collect()
}

fn error(events: &[VoiceEvent]) -> Option<ErrorCode> {
    events.iter().find_map(|e| match e {
        VoiceEvent::Error { code, .. } => Some(*code),
        _ => None,
    })
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

pub(crate) async fn act(app: &App, core: &mut Core, user: u64, s: ActivitySignal) {
    signal(app, core, user, VoiceSignal::Activities(s)).await;
}

/// `act`, returning the error code the caller got (None: accepted).
async fn try_act(
    app: &App,
    core: &mut Core,
    rx: &mut mpsc::Receiver<Packet>,
    user: u64,
    s: ActivitySignal,
) -> Option<ErrorCode> {
    drain(rx);
    act(app, core, user, s).await;
    error(&drain(rx).0)
}

/// Join `channel` and act as if its media connected (the SFU's `Joined`).
pub(crate) async fn join(app: &App, core: &mut Core, user: u64, channel: Id) -> VoiceSession {
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
    let session = *core
        .voice
        .keys()
        .find(|s| s.user_id == Id(user))
        .expect("in the call");
    core.voice.get_mut(&session).unwrap().joined = true;
    super::joined(core, session);
    session
}

pub(crate) fn launch(channel: Id, id: &str) -> ActivitySignal {
    ActivitySignal::Launch {
        channel_id: channel,
        activity_id: id.into(),
    }
}

pub(crate) fn sid(core: &Core, channel: Id) -> Id {
    session_for_test(core, channel)
        .expect("a session")
        .session_id
}

fn write(session_id: Id, key: &str, value: Option<&str>) -> ActivitySignal {
    ActivitySignal::Write {
        session_id,
        ops: vec![ActivityOp {
            key: key.into(),
            value: value.map(Into::into),
            expect: None,
        }],
    }
}

fn entries(events: &[ActivityEvent]) -> Vec<ActivityEntry> {
    events
        .iter()
        .flat_map(|e| match e {
            ActivityEvent::State { entries, .. } => entries.clone(),
            _ => vec![],
        })
        .collect()
}

#[tokio::test]
async fn everyone_may_use_activities_by_default_in_old_and_new_servers() {
    let (_dir, app) = fixture().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let p = crate::permissions::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap();
    assert!(p.contains(Permissions::USE_ACTIVITIES));
    assert_eq!(Permissions::USE_ACTIVITIES.bits(), 1 << 48);
}

#[tokio::test]
async fn launching_needs_the_call_the_permission_and_one_session_per_call() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r4 = attach(&mut core, 4);
    // Not in the call: refused.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(VOICE, "whiteboard")).await,
        Some(ErrorCode::Forbidden)
    );
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    join(&app, &mut core, 4, VOICE2).await;
    // In another call than the one named: refused.
    assert_eq!(
        try_act(&app, &mut core, &mut r4, 4, launch(VOICE, "whiteboard")).await,
        Some(ErrorCode::Forbidden)
    );
    // Unknown activities do not exist.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(VOICE, "nope")).await,
        Some(ErrorCode::NotFound)
    );
    // A role without Use Activities cannot launch.
    sqlx::query("UPDATE roles SET permissions=permissions&~? WHERE guild_id=10 AND id=10")
        .bind(Permissions::USE_ACTIVITIES.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, launch(VOICE, "whiteboard")).await,
        Some(ErrorCode::Forbidden)
    );
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE guild_id=10 AND id=10")
        .bind(Permissions::USE_ACTIVITIES.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, launch(VOICE, "whiteboard")).await,
        None
    );
    // Everyone in the call heard; the launcher is participant 1 and the host.
    let (v, _) = drain(&mut r1);
    let seen = activity(&v);
    assert!(seen.iter().any(|e| matches!(e, ActivityEvent::Session(s) if s.you.is_none() && s.participants.len() == 1 && s.host == 1)));
    // One session per call.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(VOICE, "checkers")).await,
        Some(ErrorCode::Conflict)
    );
    // People in other calls hear nothing.
    assert!(activity(&drain(&mut r4).0).is_empty());
    // Starting is in the audit log.
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE guild_id=10 AND action='activity_start' AND actor_id=2",
    )
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn stages_have_no_activities_and_direct_calls_do() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r3 = attach(&mut core, 3);
    // A DM call: always allowed, and nobody can moderate it.
    join(&app, &mut core, 1, DM).await;
    join(&app, &mut core, 3, DM).await;
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(DM, "four-in-a-row")).await,
        None
    );
    let s = sid(&core, DM);
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r3,
            3,
            ActivitySignal::Join { session_id: s }
        )
        .await,
        None
    );
    assert_eq!(session_for_test(&core, DM).unwrap().participants.len(), 2);
    drop(core);
    // A Stage: a moderator starts it and joins, but activities are refused.
    let mut core = app.0.core.lock().await;
    crate::voice::signal(
        &app,
        &mut core,
        &key(1),
        Id(1),
        "ws1",
        sticktalk_sfu::encode_signal(1, &VoiceSignal::Leave).unwrap(),
    )
    .await
    .unwrap();
    join(&app, &mut core, 1, STAGE).await;
    // (Backstage, a moderator is a listener; on the Stage, a speaker: both refused.)
    assert!(matches!(
        try_act(&app, &mut core, &mut r1, 1, launch(STAGE, "whiteboard")).await,
        Some(ErrorCode::Forbidden | ErrorCode::InvalidRequest)
    ));
    let v = core
        .voice
        .keys()
        .copied()
        .find(|v| v.user_id == Id(1))
        .unwrap();
    core.voice.get_mut(&v).unwrap().listener = false;
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(STAGE, "whiteboard")).await,
        Some(ErrorCode::InvalidRequest)
    );
}

#[tokio::test]
async fn joining_leaving_hosting_and_the_end_of_the_call() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    join(&app, &mut core, 1, VOICE).await;
    let v2 = join(&app, &mut core, 2, VOICE).await;
    act(&app, &mut core, 1, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 1, write(s, "note", Some(r#"{"x":1}"#))).await;
    drain(&mut r2);
    // A late joiner gets the whole document and its own number.
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    let seen = activity(&drain(&mut r2).0);
    assert!(seen
        .iter()
        .any(|e| matches!(e, ActivityEvent::Session(x) if x.you == Some(2))));
    assert!(seen.iter().any(|e| matches!(e, ActivityEvent::State { reset: true, done: true, entries, .. } if entries.iter().any(|e| e.key == "note"))));
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            ActivitySignal::Join { session_id: s }
        )
        .await,
        Some(ErrorCode::Conflict)
    );
    // Only the host passes hosting; the host leaving passes it on.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            ActivitySignal::SetHost {
                session_id: s,
                pid: 2
            }
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    act(&app, &mut core, 1, ActivitySignal::Leave { session_id: s }).await;
    let now = session_for_test(&core, VOICE).unwrap();
    assert_eq!(now.host, 2);
    assert_eq!(now.participants.len(), 1);
    // The session ends with the call: the last call session leaving ends it.
    crate::voice::signal(
        &app,
        &mut core,
        &key(1),
        Id(1),
        "ws1",
        sticktalk_sfu::encode_signal(1, &VoiceSignal::Leave).unwrap(),
    )
    .await
    .unwrap();
    assert!(
        session_for_test(&core, VOICE).is_some(),
        "user 2 is still in the call"
    );
    drain(&mut r2);
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::voice::remove(&app, &mut core, &mut c, v2)
        .await
        .unwrap();
    assert!(session_for_test(&core, VOICE).is_none());
    assert!(activity(&drain(&mut r2).0)
        .iter()
        .any(|e| matches!(e, ActivityEvent::Ended { .. })));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action='activity_end'")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let _ = &mut r1;
}

#[tokio::test]
async fn writes_follow_the_key_modes_and_the_limits() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r4 = attach(&mut core, 4);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    join(&app, &mut core, 4, VOICE).await;
    act(&app, &mut core, 1, launch(VOICE, "watch-together")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    // Not a participant (user 4 is in the call but not in the activity).
    assert_eq!(
        try_act(&app, &mut core, &mut r4, 4, write(s, "a", Some("1"))).await,
        Some(ErrorCode::Forbidden)
    );
    // Shared keys: anyone, and everyone else hears it.
    drain(&mut r1);
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, write(s, "q/1", Some("1"))).await,
        None
    );
    assert!(entries(&activity(&drain(&mut r1).0))
        .iter()
        .any(|e| e.key == "q/1" && e.by == 2));
    // Host keys: the host only, until control is shared.
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, write(s, "h/media", Some("{}"))).await,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "h/media", Some("{}"))).await,
        None
    );
    act(
        &app,
        &mut core,
        1,
        ActivitySignal::SetShared {
            session_id: s,
            shared: true,
        },
    )
    .await;
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, write(s, "h/media", Some("{}"))).await,
        None
    );
    // Server keys: nobody. Own keys: their owner only.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            write(s, "s/media/1", Some("{}"))
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, write(s, "p/2/pen", Some("1"))).await,
        None
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "p/2/pen", Some("1"))).await,
        Some(ErrorCode::Forbidden)
    );
    // Values are JSON; keys are checked.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "x", Some("not json"))).await,
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "bad key", Some("1"))).await,
        Some(ErrorCode::InvalidRequest)
    );
    // Compare-and-set.
    let current = core.activities.sessions[&VOICE].doc["q/1"].version;
    let cas = |expect| ActivitySignal::Write {
        session_id: s,
        ops: vec![ActivityOp {
            key: "q/1".into(),
            value: Some("2".into()),
            expect: Some(expect),
        }],
    };
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, cas(current + 100)).await,
        Some(ErrorCode::Conflict)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, cas(current)).await,
        None
    );
    // Sizes: one value, one write, the document.
    let big = format!("\"{}\"", "x".repeat(MAX_ACTIVITY_VALUE_BYTES));
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "big", Some(&big))).await,
        Some(ErrorCode::PayloadTooLarge)
    );
    let many = ActivitySignal::Write {
        session_id: s,
        ops: (0..=MAX_ACTIVITY_OPS)
            .map(|i| ActivityOp {
                key: format!("k{i}"),
                value: Some("1".into()),
                expect: None,
            })
            .collect(),
    };
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, many).await,
        Some(ErrorCode::InvalidRequest)
    );
    let chunk = format!("\"{}\"", "y".repeat(MAX_ACTIVITY_VALUE_BYTES - 64));
    let mut refused = None;
    for i in 0..(MAX_ACTIVITY_STATE_BYTES / MAX_ACTIVITY_VALUE_BYTES + 2) {
        refused = try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            write(s, &format!("fill/{i}"), Some(&chunk)),
        )
        .await;
        if refused.is_some() {
            break;
        }
    }
    assert_eq!(refused, Some(ErrorCode::LimitExceeded));
    assert!(core.activities.sessions[&VOICE].bytes <= MAX_ACTIVITY_STATE_BYTES);
    // A late joiner gets the full document in several parts.
    drain(&mut r4);
    act(&app, &mut core, 4, ActivitySignal::Join { session_id: s }).await;
    let parts: Vec<_> = activity(&drain(&mut r4).0)
        .into_iter()
        .filter(|e| matches!(e, ActivityEvent::State { .. }))
        .collect();
    assert!(parts.len() > 1, "{} parts", parts.len());
    assert!(matches!(
        parts.first(),
        Some(ActivityEvent::State {
            reset: true,
            done: false,
            ..
        })
    ));
    assert!(matches!(
        parts.last(),
        Some(ActivityEvent::State { done: true, .. })
    ));
    let total: usize = entries(&parts).len();
    assert_eq!(total, core.activities.sessions[&VOICE].doc.len());
}

#[tokio::test]
async fn ephemeral_messages_reach_the_named_participants_only() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r4 = attach(&mut core, 4);
    for u in [1, 2, 4] {
        join(&app, &mut core, u, VOICE).await;
    }
    act(&app, &mut core, 1, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    act(&app, &mut core, 4, ActivitySignal::Join { session_id: s }).await;
    for rx in [&mut r1, &mut r2, &mut r4] {
        drain(rx);
    }
    let send = |to: Vec<u16>, data: &str| ActivitySignal::Send {
        session_id: s,
        to,
        data: data.into(),
    };
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, send(vec![], r#"{"x":1}"#)).await,
        None
    );
    let got = |rx: &mut mpsc::Receiver<Packet>| {
        activity(&drain(rx).0)
            .into_iter()
            .filter(|e| matches!(e, ActivityEvent::Message { from: 1, .. }))
            .count()
    };
    assert_eq!((got(&mut r2), got(&mut r4)), (1, 1));
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, send(vec![3], r#"{"x":2}"#)).await,
        None
    );
    assert_eq!((got(&mut r2), got(&mut r4)), (0, 1));
    // Not to people outside the session, not too large, JSON only.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, send(vec![9], "{}")).await,
        Some(ErrorCode::NotFound)
    );
    let big = format!("\"{}\"", "z".repeat(MAX_ACTIVITY_MESSAGE_BYTES));
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, send(vec![], &big)).await,
        Some(ErrorCode::PayloadTooLarge)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, send(vec![], "<b>")).await,
        Some(ErrorCode::InvalidRequest)
    );
    // Rate limited per device (the limit is not cleared within one burst here).
    drain(&mut r1);
    let mut limited = false;
    for _ in 0..80 {
        crate::voice::signal(
            &app,
            &mut core,
            &key(1),
            Id(1),
            "ws1",
            sticktalk_sfu::encode_signal(1, &VoiceSignal::Activities(send(vec![], "{}"))).unwrap(),
        )
        .await
        .unwrap();
        if error(&drain(&mut r1).0) == Some(ErrorCode::RateLimited) {
            limited = true;
            break;
        }
    }
    assert!(limited);
}

#[tokio::test]
async fn games_are_decided_by_the_server_and_keep_their_result() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r4 = attach(&mut core, 4);
    for u in [1, 2, 4] {
        join(&app, &mut core, u, VOICE).await;
    }
    act(&app, &mut core, 1, launch(VOICE, "four-in-a-row")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    act(&app, &mut core, 4, ActivitySignal::Join { session_id: s }).await;
    let a = |data: &str| ActivitySignal::Action {
        session_id: s,
        data: data.into(),
    };
    // Nobody writes the game directly, not even the host.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, write(s, "s/game", Some("{}"))).await,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            write(s, "anything", Some("{}"))
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, a(r#"{"type":"sit","seat":0}"#)).await,
        None
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, a(r#"{"type":"sit","seat":1}"#)).await,
        None
    );
    // Out of turn, and spectators, are refused.
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, a(r#"{"type":"move","col":0}"#)).await,
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r4, 4, a(r#"{"type":"move","col":0}"#)).await,
        Some(ErrorCode::InvalidRequest)
    );
    for _ in 0..3 {
        act(&app, &mut core, 1, a(r#"{"type":"move","col":0}"#)).await;
        act(&app, &mut core, 2, a(r#"{"type":"move","col":1}"#)).await;
    }
    drain(&mut r4);
    act(&app, &mut core, 1, a(r#"{"type":"move","col":0}"#)).await;
    // The spectator sees the server's game state.
    let game = entries(&activity(&drain(&mut r4).0))
        .into_iter()
        .find(|e| e.key == "s/game" && e.by == 0)
        .and_then(|e| e.value)
        .unwrap();
    assert!(
        game.contains("\"status\":\"over\"") && game.contains("\"winner\":0"),
        "{game}"
    );
    // The result is kept with the conversation when the session ends.
    act(&app, &mut core, 1, ActivitySignal::End { session_id: s }).await;
    let results: Vec<String> =
        sqlx::query_scalar("SELECT text FROM activity_results WHERE channel_id=12")
            .fetch_all(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(
        results,
        vec!["user1 beat user2 at Four in a Row".to_owned()]
    );
}

#[tokio::test]
async fn the_host_or_a_moderator_ends_a_session() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r4 = attach(&mut core, 4);
    join(&app, &mut core, 2, VOICE).await;
    join(&app, &mut core, 4, VOICE).await;
    act(&app, &mut core, 2, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 4, ActivitySignal::Join { session_id: s }).await;
    // A participant who is not the host cannot end it.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r4,
            4,
            ActivitySignal::End { session_id: s }
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    // The owner (a moderator) can, from outside the call.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            ActivitySignal::End { session_id: s }
        )
        .await,
        None
    );
    assert!(session_for_test(&core, VOICE).is_none());
    let actor: i64 =
        sqlx::query_scalar("SELECT actor_id FROM audit_log WHERE action='activity_end'")
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(actor, 1);
    // The host can too.
    act(&app, &mut core, 2, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            ActivitySignal::End { session_id: s }
        )
        .await,
        None
    );
}

#[tokio::test]
async fn automod_screens_activity_text_and_lost_rights_leave_the_session() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r2 = attach(&mut core, 2);
    let _r1 = attach(&mut core, 1);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    act(&app, &mut core, 2, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    let rule = AutoModRule {
        id: Id(0),
        guild_id: Id(10),
        name: "No swearing".into(),
        enabled: true,
        trigger: AutoModTrigger::Keyword {
            keywords: vec!["heck*".into()],
            allow: vec![],
        },
        actions: AutoModActions {
            block: true,
            message: None,
            alert_channel_id: None,
            timeout_seconds: 0,
        },
        exempt_roles: vec![],
        exempt_channels: vec![],
    };
    let r = crate::commands::execute(
        &app,
        &mut core,
        &key(1),
        Id(1),
        CommandRequest {
            nonce: 77,
            command: Command::Launch(LaunchCommand::Moderation(
                ModerationCommand::CreateAutoModRule(rule),
            )),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(r.outcome, CommandOutcome::Ok(_)),
        "{:?}",
        r.outcome
    );
    // Visible text (a `text` property) is screened; other data is not.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            write(s, "t/1", Some(r#"{"text":"what the heck"}"#))
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            write(s, "t/2", Some(r#"{"points":"heck"}"#))
        )
        .await,
        None
    );
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r2,
            2,
            ActivitySignal::Send {
                session_id: s,
                to: vec![],
                data: r#"{"text":"heckin"}"#.into()
            }
        )
        .await,
        Some(ErrorCode::Forbidden)
    );
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action='automod_block'")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
    // Losing Use Activities (a role change runs the voice reconcile) leaves the session.
    act(&app, &mut core, 1, ActivitySignal::Join { session_id: s }).await;
    sqlx::query("UPDATE roles SET permissions=permissions&~? WHERE guild_id=10 AND id=10")
        .bind(Permissions::USE_ACTIVITIES.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::voice::reconcile(&app, &mut core, &mut c)
        .await
        .unwrap();
    let now = session_for_test(&core, VOICE).unwrap();
    assert_eq!(now.participants.len(), 1, "the owner keeps every right");
    assert_eq!(now.participants[0].session.user_id, Id(1));
}

#[tokio::test]
async fn timed_out_members_leave_the_call_and_the_activity() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let _r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    act(&app, &mut core, 1, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    let r = crate::commands::execute(
        &app,
        &mut core,
        &key(1),
        Id(1),
        CommandRequest {
            nonce: 78,
            command: Command::Launch(LaunchCommand::Moderation(ModerationCommand::Timeout {
                guild_id: Id(10),
                user_id: Id(2),
                seconds: 600,
                reason: None,
            })),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(r.outcome, CommandOutcome::Ok(_)),
        "{:?}",
        r.outcome
    );
    assert_eq!(
        session_for_test(&core, VOICE).unwrap().participants.len(),
        1
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, write(s, "a", Some("1"))).await,
        Some(ErrorCode::Forbidden)
    );
}

#[tokio::test]
async fn shared_media_must_be_readable_and_is_streamable_by_participants_only() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    sqlx::raw_sql(
        "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(500,11,1,'',0),(501,11,1,'',0);\
         INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(600,1,'message',11,'clip.mp4','video/mp4',9,'clip',0),(601,1,'message',11,'notes.txt','text/plain',9,'notes',0);\
         INSERT INTO message_attachments VALUES(500,600,0),(501,601,0);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r4 = attach(&mut core, 4);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 4, VOICE).await;
    act(&app, &mut core, 1, launch(VOICE, "watch-together")).await;
    let s = sid(&core, VOICE);
    let share = |a| ActivitySignal::ShareMedia {
        session_id: s,
        attachment_id: Id(a),
        confirmed: true,
    };
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, share(601)).await,
        Some(ErrorCode::UnsupportedMediaType)
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, share(999)).await,
        Some(ErrorCode::NotFound)
    );
    assert!(!super::file_access(&core, Id(4), Id(600)));
    // Everyone in the session will be able to watch it: the sharer confirms.
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            ActivitySignal::ShareMedia {
                session_id: s,
                attachment_id: Id(600),
                confirmed: false,
            }
        )
        .await,
        Some(ErrorCode::InvalidRequest)
    );
    assert!(!super::file_access(&core, Id(4), Id(600)));
    assert_eq!(try_act(&app, &mut core, &mut r1, 1, share(600)).await, None);
    // A guild call's share is in the audit log, with the file and its source.
    let changes: String = sqlx::query_scalar(
        "SELECT changes FROM audit_log WHERE guild_id=10 AND action='activity_media_share' AND actor_id=1",
    )
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    assert!(
        changes.contains("clip.mp4") && changes.contains("\"11\""),
        "{changes}"
    );
    let doc = &core.activities.sessions[&VOICE].doc;
    assert!(doc["s/media/600"].text.contains("clip.mp4"));
    // Streamable by participants while the session lasts, not by others.
    assert!(super::file_access(&core, Id(1), Id(600)));
    assert!(!super::file_access(&core, Id(4), Id(600)));
    act(&app, &mut core, 4, ActivitySignal::Join { session_id: s }).await;
    assert!(super::file_access(&core, Id(4), Id(600)));
    // Someone who cannot read the conversation cannot share from it.
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',4,0,?)")
        .bind(Permissions::READ_HISTORY.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        try_act(&app, &mut core, &mut r4, 4, share(600)).await,
        None,
        "already shared: a no-op"
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(catalog::shareable(&mut c, Id(4), Id(600)).await.is_err());
    assert!(catalog::shareable(&mut c, Id(1), Id(600)).await.is_ok());
    act(&app, &mut core, 1, ActivitySignal::End { session_id: s }).await;
    assert!(!super::file_access(&core, Id(1), Id(600)));
}

#[tokio::test]
async fn invite_cards_follow_the_session_and_respect_permissions() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    let mut r2 = attach(&mut core, 2);
    let mut r3 = attach(&mut core, 3);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    act(&app, &mut core, 1, launch(VOICE, "sketch-guess")).await;
    let s = sid(&core, VOICE);
    let invite = |ch| ActivitySignal::Invite {
        session_id: s,
        channel_id: ch,
    };
    drain(&mut r2);
    // Into the voice channel's chat and a text channel of the same server.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, invite(VOICE)).await,
        None
    );
    let (_, events) = drain(&mut r2);
    let card = events
        .iter()
        .find_map(|e| match e {
            Event::MessageCreated { message, .. } => message.extra.activity.clone(),
            _ => None,
        })
        .expect("a card");
    assert!(
        card.live
            && card.count == 1
            && card.call_channel_id == VOICE
            && card.name == "Sketch & Guess"
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, invite(Id(11))).await,
        None
    );
    // Not into a Stage.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, invite(STAGE)).await,
        Some(ErrorCode::InvalidRequest)
    );
    // A DM of the inviter works (a game invite to a friend).
    assert_eq!(try_act(&app, &mut core, &mut r1, 1, invite(DM)).await, None);
    assert!(drain(&mut r3).1.iter().any(
        |e| matches!(e, Event::MessageCreated { message, .. } if message.extra.activity.is_some())
    ));
    // A member without Send Messages there cannot post one.
    act(&app, &mut core, 2, ActivitySignal::Join { session_id: s }).await;
    sqlx::query("UPDATE roles SET permissions=permissions&~? WHERE guild_id=10 AND id=10")
        .bind(Permissions::SEND_MESSAGES.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        try_act(&app, &mut core, &mut r2, 2, invite(Id(11))).await,
        Some(ErrorCode::Forbidden)
    );
    // At most four cards per session.
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, invite(Id(11))).await,
        None
    );
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, invite(Id(11))).await,
        Some(ErrorCode::LimitExceeded)
    );
    // Joining rewrote the cards; ending shows the summary.
    let mut c = app.0.pool.acquire().await.unwrap();
    let (m,): (i64,) =
        sqlx::query_as("SELECT message_id FROM activity_cards ORDER BY message_id LIMIT 1")
            .fetch_one(&mut *c)
            .await
            .unwrap();
    let live = crate::db::message(&mut c, Id(1), VOICE, Id(m as u64), false)
        .await
        .unwrap();
    assert_eq!(live.extra.activity.as_ref().unwrap().count, 2);
    drain(&mut r1);
    act(&app, &mut core, 1, ActivitySignal::End { session_id: s }).await;
    let ended = crate::db::message(&mut c, Id(1), VOICE, Id(m as u64), false)
        .await
        .unwrap();
    let card = ended.extra.activity.unwrap();
    assert!(!card.live && card.summary.as_ref().is_some_and(|s| s.people == 2));
    assert!(drain(&mut r1)
        .1
        .iter()
        .any(|e| matches!(e, Event::MessageUpdated(_))));
}

#[tokio::test]
async fn a_direct_call_card_goes_only_into_its_own_conversation() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    join(&app, &mut core, 1, DM).await;
    act(&app, &mut core, 1, launch(DM, "checkers")).await;
    let s = sid(&core, DM);
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            ActivitySignal::Invite {
                session_id: s,
                channel_id: Id(11)
            }
        )
        .await,
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        try_act(
            &app,
            &mut core,
            &mut r1,
            1,
            ActivitySignal::Invite {
                session_id: s,
                channel_id: DM
            }
        )
        .await,
        None
    );
}

#[tokio::test]
async fn reports_can_name_a_participant_of_a_running_session() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut core = app.0.core.lock().await;
    let _r1 = attach(&mut core, 1);
    let _r2 = attach(&mut core, 2);
    join(&app, &mut core, 1, VOICE).await;
    join(&app, &mut core, 2, VOICE).await;
    act(&app, &mut core, 2, launch(VOICE, "whiteboard")).await;
    let s = sid(&core, VOICE);
    act(
        &app,
        &mut core,
        2,
        write(s, "t/1", Some(r#"{"text":"something rude"}"#)),
    )
    .await;
    let report = |user| {
        Command::Privacy(PrivacyCommand::Report(ReportDraft {
            target: ReportTarget::Activity {
                channel_id: VOICE,
                session_id: s,
                user_id: Id(user),
            },
            category: ReportCategory::Harassment,
            note: None,
        }))
    };
    let r = crate::commands::execute(
        &app,
        &mut core,
        &key(1),
        Id(1),
        CommandRequest {
            nonce: 90,
            command: report(2),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(r.outcome, CommandOutcome::Ok(_)),
        "{:?}",
        r.outcome
    );
    let evidence: String =
        sqlx::query_scalar("SELECT evidence FROM reports WHERE queue='guild' AND guild_id=10")
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    assert!(
        evidence.contains("something rude") && evidence.contains("Whiteboard"),
        "{evidence}"
    );
    // Not about yourself; not about someone who never took part.
    for (nonce, user) in [(91, 1), (92, 3)] {
        let r = crate::commands::execute(
            &app,
            &mut core,
            &key(1),
            Id(1),
            CommandRequest {
                nonce,
                command: report(user),
            },
        )
        .await
        .unwrap();
        assert!(matches!(r.outcome, CommandOutcome::Err(_)));
    }
}

#[tokio::test]
async fn the_catalog_and_server_settings_routes() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    let outsider = token(&app, 3).await;
    let r = request(
        &app,
        &member,
        "GET",
        "/api/v1/activities?channel_id=12",
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json(r).await;
    assert_eq!(v["can_launch"], true);
    assert_eq!(v["activities"].as_array().unwrap().len(), 5);
    // Someone who cannot see the channel learns nothing.
    assert_eq!(
        request(
            &app,
            &outsider,
            "GET",
            "/api/v1/activities?channel_id=12",
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    // A Stage offers none.
    let v = json(
        request(
            &app,
            &member,
            "GET",
            "/api/v1/activities?channel_id=13",
            None,
        )
        .await,
    )
    .await;
    assert_eq!(v["activities"].as_array().unwrap().len(), 0);
    // Server settings: Manage Server only.
    assert_eq!(
        request(&app, &member, "GET", "/api/v1/guilds/10/activities", None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            &member,
            "PUT",
            "/api/v1/guilds/10/activities/checkers",
            Some(r#"{"enabled":false}"#)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // Register an activity by URL (the administrator's CLI).
    let mut c = app.0.pool.acquire().await.unwrap();
    for bad in [
        "ftp://games.example/",
        "https://user:pw@games.example/",
        "http://games.example/",
    ] {
        assert!(
            catalog::run_admin(
                &mut c,
                &ActivityAdmin::Add {
                    name: "Bad".into(),
                    url: bad.into(),
                    icon: "🎲".into(),
                    description: String::new(),
                    min_players: 1,
                    max_players: 4,
                    in_dms: false,
                }
            )
            .await
            .is_err(),
            "{bad}"
        );
    }
    catalog::run_admin(
        &mut c,
        &ActivityAdmin::Add {
            name: "Trivia".into(),
            url: "https://games.example/trivia/".into(),
            icon: "🎲".into(),
            description: "Questions".into(),
            min_players: 1,
            max_players: 8,
            in_dms: false,
        },
    )
    .await
    .unwrap();
    let list = json(request(&app, &owner, "GET", "/api/v1/guilds/10/activities", None).await).await;
    let custom = list["activities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["activity"]["name"] == "Trivia")
        .unwrap()
        .clone();
    assert_eq!(custom["enabled"], false, "registered activities start off");
    assert_eq!(custom["activity"]["origin"], "https://games.example");
    let id = custom["activity"]["id"].as_str().unwrap().to_owned();
    // Off: not launchable. On: listed.
    let mut core = app.0.core.lock().await;
    let mut r1 = attach(&mut core, 1);
    join(&app, &mut core, 1, VOICE).await;
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(VOICE, &id)).await,
        Some(ErrorCode::NotFound)
    );
    drop(core);
    assert_eq!(
        request(
            &app,
            &owner,
            "PUT",
            &format!("/api/v1/guilds/10/activities/{id}"),
            Some(r#"{"enabled":true}"#)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let mut core = app.0.core.lock().await;
    assert_eq!(
        try_act(&app, &mut core, &mut r1, 1, launch(VOICE, &id)).await,
        None
    );
    drop(core);
    // Turning it off ends its running sessions.
    assert_eq!(
        request(
            &app,
            &owner,
            "PUT",
            &format!("/api/v1/guilds/10/activities/{id}"),
            Some(r#"{"enabled":false}"#)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(session_for_test(&*app.0.core.lock().await, VOICE).is_none());
    // Which activities members can start is a server setting: each change is
    // audited (on, then off); setting the same value again is not a change.
    let toggles: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE guild_id=10 AND action='guild_update' AND changes LIKE '%activity %'",
    )
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    assert_eq!(toggles, 2);
    assert_eq!(
        request(
            &app,
            &owner,
            "PUT",
            "/api/v1/guilds/10/activities/nope",
            Some(r#"{"enabled":true}"#)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    // Not offered in direct calls unless registered for them.
    let v = json(
        request(
            &app,
            &owner,
            "GET",
            "/api/v1/activities?channel_id=20",
            None,
        )
        .await,
    )
    .await;
    assert!(!v["activities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["name"] == "Trivia"));
    // Removing it.
    catalog::run_admin(&mut c, &ActivityAdmin::Remove { id: id.clone() })
        .await
        .unwrap();
    assert!(catalog::run_admin(&mut c, &ActivityAdmin::Remove { id })
        .await
        .is_err());
}

#[tokio::test]
async fn media_lists_show_only_readable_audio_and_video() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    sqlx::raw_sql(
        "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(500,11,1,'',5),(501,11,1,'',6);\
         INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(600,1,'message',11,'song.ogg','audio/ogg',9,'song',0),(601,1,'message',11,'notes.txt','text/plain',9,'notes',0);\
         INSERT INTO message_attachments VALUES(500,600,0),(501,601,0);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
    let member = token(&app, 2).await;
    let outsider = token(&app, 3).await;
    let owner = token(&app, 1).await;
    let v = json(
        request(
            &app,
            &member,
            "GET",
            "/api/v1/activities/media?channel_id=11",
            None,
        )
        .await,
    )
    .await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["filename"], "song.ogg");
    assert_eq!(items[0]["attachment_id"], "600");
    assert_eq!(
        request(
            &app,
            &outsider,
            "GET",
            "/api/v1/activities/media?channel_id=11",
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let v = json(request(&app, &owner, "GET", "/api/v1/activities/media", None).await).await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn frames_have_their_own_policies_and_the_app_lists_no_third_party() {
    let (_dir, app) = fixture().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    catalog::run_admin(
        &mut c,
        &ActivityAdmin::Add {
            name: "Trivia".into(),
            url: "https://games.example/trivia/?v=1".into(),
            icon: "🎲".into(),
            description: String::new(),
            min_players: 1,
            max_players: 8,
            in_dms: true,
        },
    )
    .await
    .unwrap();
    let id: i64 = sqlx::query_scalar("SELECT id FROM custom_activities")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let get = |path: String| {
        let app = app.clone();
        async move {
            http::router(app)
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let r = get(format!("/activities/frame/custom-{id}")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let csp = r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("frame-src https://games.example;"), "{csp}");
    assert!(csp.contains("frame-ancestors http://localhost"), "{csp}");
    assert!(csp.contains("connect-src 'none'"), "{csp}");
    assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
    assert!(!r.headers().contains_key("x-frame-options"));
    let body = String::from_utf8(to_bytes(r.into_body(), 1 << 16).await.unwrap().to_vec()).unwrap();
    assert!(body.contains(r#"sandbox="allow-scripts""#));
    assert!(body.contains("https://games.example/trivia/?v=1"));
    assert_eq!(
        get("/activities/frame/custom-999".into()).await.status(),
        StatusCode::NOT_FOUND
    );
    // The app's own pages may frame only this server, and are never framed.
    let r = get("/api/v1/health".into()).await;
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(
        csp.contains("frame-src 'self'") && !csp.contains("games.example"),
        "{csp}"
    );
    assert_eq!(r.headers()["x-frame-options"], "DENY");
    // The built-in policy allows nothing over the network.
    let p = frames::builtin_policy("https://chat.example");
    assert!(
        p.contains("connect-src 'none'")
            && p.contains("frame-ancestors https://chat.example")
            && p.contains("media-src 'none'")
    );
}

#[tokio::test]
async fn built_in_frames_are_served_from_the_web_root_with_their_policy() {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(web.join("public/activities/whiteboard")).unwrap();
    std::fs::write(
        web.join("public/activities/whiteboard/index.html"),
        "<!doctype html><p>board",
    )
    .unwrap();
    std::fs::write(web.join("public/activities/.secret"), "x").unwrap();
    let (_d, app) = fixture().await;
    let mut config = app.0.config.clone();
    config.data_dir = dir.path().join("data");
    config.browser.web_root = Some(web);
    let app = App::open(config).await.unwrap();
    let get = |path: &str| {
        let app = app.clone();
        let path = path.to_owned();
        async move {
            http::router(app)
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let r = get("/activities/whiteboard/").await;
    assert_eq!(r.status(), StatusCode::OK);
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(
        csp.contains("connect-src 'none'") && csp.contains("frame-ancestors http://localhost"),
        "{csp}"
    );
    // Sandboxed however it is loaded: opened top-level, it is not the app's origin.
    assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
    assert!(!r.headers().contains_key("x-frame-options"));
    assert_eq!(
        get("/activities/.secret").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/activities/../index.html").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/activities/missing/app.js").await.status(),
        StatusCode::NOT_FOUND
    );
}
