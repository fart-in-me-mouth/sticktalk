//! Stage channels: who may start, join, speak and moderate; what each person
//! is told; the record; and that the audience never publishes.
use super::*;
use crate::{
    audit_tests::fixture,
    state::{Connection, Packet},
};
use sticktalk_sfu::Authorizer as _;
use tokio::sync::mpsc;

const STAGE: Id = Id(30);
const VOICE: Id = Id(31);

/// Users 1 (owner), 2 and 3 (members) in guild 10, with Stage 30 and voice 31.
async fn setup(app: &App) {
    sqlx::raw_sql(
        "INSERT INTO guild_members VALUES(10,3,'member',0);\
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(30,'stage',10,'town-hall',1,0),(31,'voice',10,'lounge',2,0);\
         INSERT INTO read_states(user_id,channel_id) VALUES(1,30),(2,30),(3,30),(3,11);",
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
fn events(rx: &mut mpsc::Receiver<Packet>) -> (Vec<VoiceEvent>, Vec<Event>) {
    let (mut voice, mut chat) = (vec![], vec![]);
    while let Ok(packet) = rx.try_recv() {
        match rmp_serde::from_slice::<ServerMsg>(&packet.bytes).unwrap() {
            ServerMsg::Voice(v) => voice.push(rmp_serde::from_slice(&v.payload).unwrap()),
            ServerMsg::Event(e) => chat.push(e.event),
            _ => {}
        }
    }
    (voice, chat)
}
/// Sends a signal as `user`; its answer arrives on their connection.
async fn signal(app: &App, core: &mut Core, user: u64, s: VoiceSignal) {
    core.limits.clear();
    voice::signal(
        app,
        core,
        &key(user),
        Id(user),
        &format!("ws{user}"),
        sticktalk_sfu::encode_signal(7, &s).unwrap(),
    )
    .await
    .unwrap();
}
/// The answer to the last signal from `rx` (Ack or Error).
fn answer(rx: &mut mpsc::Receiver<Packet>) -> std::result::Result<(), ErrorCode> {
    let (voice, _) = events(rx);
    match voice
        .iter()
        .rev()
        .find(|e| matches!(e, VoiceEvent::Ack | VoiceEvent::Error { .. }))
    {
        Some(VoiceEvent::Error { code, .. }) => Err(*code),
        _ => Ok(()),
    }
}
fn join(ch: Id) -> VoiceSignal {
    VoiceSignal::Join {
        channel_id: ch,
        capabilities: VoiceCapabilities {
            video: vec![VideoCodec::H264],
            hardware_decode: vec![],
        },
    }
}
fn stage(s: StageSignal) -> VoiceSignal {
    VoiceSignal::Stage(s)
}
fn start() -> VoiceSignal {
    stage(StageSignal::Start {
        channel_id: STAGE,
        topic: "Launch Q&A".into(),
        privacy_note: Some("Members only".into()),
        event_id: None,
    })
}
fn session_of(core: &Core, user: u64) -> VoiceSession {
    *core
        .voice
        .keys()
        .find(|s| s.user_id == Id(user))
        .expect("on the Stage")
}
fn acl(app: &App) -> voice_acl::Acl {
    voice_acl::Acl(app.clone())
}
mod voice_acl {
    use super::*;
    pub struct Acl(pub App);
    impl Acl {
        pub fn listener(&self, s: VoiceSession) -> bool {
            crate::voice::acl_for_test(&self.0).listener(s)
        }
        pub fn speaks(&self, s: VoiceSession) -> bool {
            crate::voice::acl_for_test(&self.0)
                .permissions(s, STAGE)
                .contains(Permissions::SPEAK)
        }
    }
}

#[tokio::test]
async fn only_moderators_start_a_stage_and_nobody_else_joins_before_it_is_live() {
    let (_dir, app) = fixture().await;
    setup(&app).await;
    let mut core = app.0.core.lock().await;
    let mut owner = attach(&mut core, 1);
    let mut member = attach(&mut core, 2);
    signal(&app, &mut core, 2, start()).await;
    assert_eq!(answer(&mut member), Err(ErrorCode::Forbidden));
    signal(&app, &mut core, 2, join(STAGE)).await;
    assert_eq!(answer(&mut member), Err(ErrorCode::Forbidden));
    assert!(core.voice.is_empty());
    // A moderator may wait on the Stage before starting it, as audience.
    signal(&app, &mut core, 1, join(STAGE)).await;
    assert!(core.voice[&session_of(&core, 1)].listener);
    signal(&app, &mut core, 1, start()).await;
    assert_eq!(answer(&mut owner), Ok(()));
    let s1 = session_of(&core, 1);
    assert!(!core.voice[&s1].listener && !acl(&app).listener(s1) && acl(&app).speaks(s1));
    // Everyone who can see the channel is told it is live; a second start conflicts.
    let (voice, _) = events(&mut member);
    assert!(voice.iter().any(|e| matches!(e, VoiceEvent::Stage(StageEvent::Live(i)) if i.topic == "Launch Q&A" && i.privacy_note.as_deref() == Some("Members only"))));
    signal(&app, &mut core, 1, start()).await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::Conflict));
    // Topic validation.
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Update {
            channel_id: STAGE,
            topic: "  ".into(),
            privacy_note: None,
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::InvalidRequest));
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Update {
            channel_id: STAGE,
            topic: "x".repeat(MAX_STAGE_TOPIC_CHARS + 1),
            privacy_note: None,
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::InvalidRequest));
    // A text channel is not a Stage.
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Start {
            channel_id: Id(11),
            topic: "t".into(),
            privacy_note: None,
            event_id: None,
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::NotFound));
}

#[tokio::test]
async fn the_audience_listens_only_and_is_not_in_the_voice_state() {
    let (_dir, app) = fixture().await;
    setup(&app).await;
    let mut core = app.0.core.lock().await;
    let mut owner = attach(&mut core, 1);
    let mut member = attach(&mut core, 2);
    let mut other = attach(&mut core, 3);
    signal(&app, &mut core, 1, join(STAGE)).await;
    signal(&app, &mut core, 1, start()).await;
    for user in [1, 2, 3] {
        let s = core.voice.keys().find(|s| s.user_id == Id(user)).copied();
        if let Some(s) = s {
            core.voice.get_mut(&s).unwrap().joined = true;
        }
    }
    events(&mut other);
    signal(&app, &mut core, 2, join(STAGE)).await;
    let s2 = session_of(&core, 2);
    core.voice.get_mut(&s2).unwrap().joined = true;
    assert!(core.voice[&s2].listener && acl(&app).listener(s2) && !acl(&app).speaks(s2));
    // The member got the roster: one speaker, themselves in the audience.
    let (voice, _) = events(&mut member);
    let roster = voice
        .iter()
        .find_map(|e| match e {
            VoiceEvent::Stage(StageEvent::Roster(r)) => Some(r.clone()),
            _ => None,
        })
        .expect("roster on join");
    assert_eq!(roster.members.len(), 2);
    assert_eq!(roster.members[0].role, StageRole::Speaker);
    assert_eq!(roster.members[1].user_id, Id(2));
    assert_eq!(roster.members[1].role, StageRole::Audience);
    // Listeners are never in Ready's or the snapshot's voice states.
    let mut c = app.0.pool.acquire().await.unwrap();
    let ready = db::ready(&mut c, &mut core, &key(3), Id(3)).await.unwrap();
    assert!(ready
        .voice_states
        .iter()
        .all(|v| v.participant.session != s2));
    voice::snapshot(&mut core, &mut c, &key(3), Id(3))
        .await
        .unwrap();
    let (voice, _) = events(&mut other);
    for e in &voice {
        if let VoiceEvent::GuildSnapshot { states } = e {
            assert!(states.iter().all(|s| s.participant.session != s2));
        }
        assert!(!matches!(e, VoiceEvent::GuildState(s) if s.participant.session == s2));
    }
    assert_eq!(counts(&core, STAGE), Some((1, 1)));
    // The soundboard is for speakers (refused before the sound is looked up).
    signal(
        &app,
        &mut core,
        2,
        VoiceSignal::PlaySound { sound_id: Id(999) },
    )
    .await;
    assert_eq!(answer(&mut member), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::PlaySound { sound_id: Id(999) },
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::NotFound));
    // Moves into or out of a Stage are refused; the Stage has its own controls.
    signal(
        &app,
        &mut core,
        1,
        VoiceSignal::Moderate {
            session: s2,
            action: VoiceModeration::Move { channel_id: VOICE },
        },
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::InvalidRequest));
}

#[tokio::test]
async fn hands_invitations_and_moving_between_audience_and_speakers() {
    let (_dir, app) = fixture().await;
    setup(&app).await;
    let mut core = app.0.core.lock().await;
    let mut owner = attach(&mut core, 1);
    let mut member = attach(&mut core, 2);
    let mut other = attach(&mut core, 3);
    signal(&app, &mut core, 1, join(STAGE)).await;
    signal(&app, &mut core, 1, start()).await;
    signal(&app, &mut core, 2, join(STAGE)).await;
    signal(&app, &mut core, 3, join(STAGE)).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    // Request to Speak needs its permission (on for @everyone by default).
    sqlx::query("INSERT INTO permission_overwrites VALUES(30,'member',3,0,?)")
        .bind(Permissions::REQUEST_TO_SPEAK.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::RaiseHand {
            channel_id: STAGE,
            raised: true,
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        2,
        stage(StageSignal::RaiseHand {
            channel_id: STAGE,
            raised: true,
        }),
    )
    .await;
    assert_eq!(answer(&mut member), Ok(()));
    flush(&mut core, &mut c).await.unwrap();
    // Moderators see the raised hand; other listeners do not; the member sees their own.
    let hand = |rx: &mut mpsc::Receiver<Packet>| {
        events(rx).0.into_iter().find_map(|e| match e {
            VoiceEvent::Stage(StageEvent::Members { upserts, .. }) => upserts
                .into_iter()
                .find(|m| m.user_id == Id(2))
                .map(|m| m.hand_raised_at),
            _ => None,
        })
    };
    assert!(hand(&mut owner).flatten().is_some());
    assert!(hand(&mut member).flatten().is_some());
    assert_eq!(hand(&mut other), Some(None));
    // Only moderators accept; accepting makes them a speaker with every right back.
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::Accept {
            channel_id: STAGE,
            user_id: Id(2),
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Accept {
            channel_id: STAGE,
            user_id: Id(2),
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Ok(()));
    let s2 = session_of(&core, 2);
    assert!(!acl(&app).listener(s2) && acl(&app).speaks(s2));
    assert_eq!(counts(&core, STAGE), Some((2, 1)));
    // Accepting someone whose hand is not raised is not found.
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Accept {
            channel_id: STAGE,
            user_id: Id(3),
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Err(ErrorCode::NotFound));
    // An invitation must be accepted before anything changes.
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Invite {
            channel_id: STAGE,
            user_id: Id(3),
        }),
    )
    .await;
    assert_eq!(answer(&mut owner), Ok(()));
    let s3 = session_of(&core, 3);
    assert!(acl(&app).listener(s3));
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::AnswerInvite {
            channel_id: STAGE,
            accept: false,
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Ok(()));
    assert!(acl(&app).listener(s3));
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::AnswerInvite {
            channel_id: STAGE,
            accept: true,
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Err(ErrorCode::NotFound));
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Invite {
            channel_id: STAGE,
            user_id: Id(3),
        }),
    )
    .await;
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::AnswerInvite {
            channel_id: STAGE,
            accept: true,
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Ok(()));
    assert!(!acl(&app).listener(s3));
    // A member cannot move someone else; they can move themselves.
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::ToAudience {
            channel_id: STAGE,
            user_id: Id(2),
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        3,
        stage(StageSignal::ToAudience {
            channel_id: STAGE,
            user_id: Id(3),
        }),
    )
    .await;
    assert_eq!(answer(&mut other), Ok(()));
    assert!(acl(&app).listener(s3));
    // A moderator moves a speaker back; only moderators become speakers by themselves.
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::ToAudience {
            channel_id: STAGE,
            user_id: Id(2),
        }),
    )
    .await;
    assert!(acl(&app).listener(s2));
    signal(
        &app,
        &mut core,
        2,
        stage(StageSignal::BecomeSpeaker { channel_id: STAGE }),
    )
    .await;
    assert_eq!(answer(&mut member), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::ToAudience {
            channel_id: STAGE,
            user_id: Id(1),
        }),
    )
    .await;
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::BecomeSpeaker { channel_id: STAGE }),
    )
    .await;
    assert_eq!(answer(&mut owner), Ok(()));
    assert!(!acl(&app).listener(session_of(&core, 1)));
}

#[tokio::test]
async fn ending_records_the_session_removes_everyone_and_is_audited() {
    let (_dir, app) = fixture().await;
    setup(&app).await;
    let mut core = app.0.core.lock().await;
    let mut owner = attach(&mut core, 1);
    let mut member = attach(&mut core, 2);
    signal(&app, &mut core, 1, join(STAGE)).await;
    signal(&app, &mut core, 1, start()).await;
    signal(&app, &mut core, 2, join(STAGE)).await;
    signal(
        &app,
        &mut core,
        2,
        stage(StageSignal::End { channel_id: STAGE }),
    )
    .await;
    assert_eq!(answer(&mut member), Err(ErrorCode::Forbidden));
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::End { channel_id: STAGE }),
    )
    .await;
    assert!(core.voice.is_empty());
    assert!(!is_live(&core, STAGE));
    let (voice, chat) = events(&mut member);
    assert!(voice
        .iter()
        .any(|e| matches!(e, VoiceEvent::Stage(StageEvent::Ended { channel_id, .. }) if *channel_id == STAGE)));
    let record = chat
        .iter()
        .find_map(|e| match e {
            Event::MessageCreated { message, .. } => message.extra.stage.clone(),
            _ => None,
        })
        .and_then(|s| s.record)
        .expect("the Stage's record in its chat");
    assert_eq!(record.topic, "Launch Q&A");
    assert_eq!(record.peak_listeners, 1);
    assert_eq!(record.speakers, 1);
    let _ = events(&mut owner);
    let mut c = app.0.pool.acquire().await.unwrap();
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE guild_id=10 ORDER BY id")
            .fetch_all(&mut *c)
            .await
            .unwrap();
    assert_eq!(actions, vec!["stage_start", "stage_end"]);
    // The record loads with history too.
    let id: i64 = sqlx::query_scalar("SELECT message_id FROM stage_messages")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let m = db::message(&mut c, Id(2), STAGE, Id(id as u64), false)
        .await
        .unwrap();
    assert!(m.extra.stage.unwrap().record.is_some());
}

#[tokio::test]
async fn the_last_person_leaving_ends_the_stage_and_a_speaker_may_reconnect() {
    let (_dir, app) = fixture().await;
    setup(&app).await;
    let mut core = app.0.core.lock().await;
    let _owner = attach(&mut core, 1);
    let mut member = attach(&mut core, 2);
    signal(&app, &mut core, 1, join(STAGE)).await;
    signal(&app, &mut core, 1, start()).await;
    signal(&app, &mut core, 2, join(STAGE)).await;
    signal(
        &app,
        &mut core,
        1,
        stage(StageSignal::Invite {
            channel_id: STAGE,
            user_id: Id(2),
        }),
    )
    .await;
    signal(
        &app,
        &mut core,
        2,
        stage(StageSignal::AnswerInvite {
            channel_id: STAGE,
            accept: true,
        }),
    )
    .await;
    // The speaker drops and comes back: still a speaker.
    signal(&app, &mut core, 2, VoiceSignal::Leave).await;
    signal(&app, &mut core, 2, join(STAGE)).await;
    assert!(!acl(&app).listener(session_of(&core, 2)));
    events(&mut member);
    signal(&app, &mut core, 2, VoiceSignal::Leave).await;
    signal(&app, &mut core, 1, VoiceSignal::Leave).await;
    assert!(!is_live(&core, STAGE));
    let mut c = app.0.pool.acquire().await.unwrap();
    let records: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_messages WHERE kind='record'")
            .fetch_one(&mut *c)
            .await
            .unwrap();
    assert_eq!(records, 1);
    assert!(core.stage.rooms.is_empty());
}

#[test]
fn the_record_reads_well() {
    let s = StageSummary {
        topic: "Q&A".into(),
        started_at: 0,
        ended_at: 3_660_000,
        peak_listeners: 1,
        speakers: 2,
    };
    assert_eq!(
        describe(&s),
        "The Stage “Q&A” ended after 1 hour 1 minute. 1 person was listening at most."
    );
    assert_eq!(
        scope(Permissions::all(), true) & PUBLISH,
        Permissions::empty()
    );
    assert_eq!(scope(Permissions::all(), false), Permissions::all());
}
