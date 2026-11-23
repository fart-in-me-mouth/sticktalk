//! Direct and group calls, and text chat in voice channels: admission, ringing,
//! the end-of-call record and every permission check.
use super::*;
use crate::{
    audit_tests::fixture,
    commands,
    state::{Connection, Packet},
};
use tokio::sync::mpsc;

const DM: Id = Id(20);
const GROUP: Id = Id(21);

/// Users 1 and 2 share DM 20; users 1, 2 and 3 share group 21.
async fn conversations(app: &App) {
    sqlx::raw_sql(
        "INSERT INTO guild_members VALUES(10,3,'member',0);\
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

fn key(user: u64, device: u8) -> Vec<u8> {
    vec![user as u8 + device * 16; 32]
}

/// A live connection for `user` on `device`; device 0 is the fixture session.
async fn attach(app: &App, core: &mut Core, user: u64, device: u8) -> mpsc::Receiver<Packet> {
    let key = key(user, device);
    if device > 0 {
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(&key)
            .bind(user as i64)
            .bind(now() as i64)
            .bind((now() + 86_400_000) as i64)
            .bind(now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    core.cursor(&key, Id(user));
    let (tx, rx) = mpsc::channel(256);
    let (close, _) = tokio::sync::watch::channel(None);
    core.streams.get_mut(&key).unwrap().connection = Some(Connection {
        focused: false,
        id: format!("ws{user}-{device}"),
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
        close,
    });
    rx
}

fn voice_events(rx: &mut mpsc::Receiver<Packet>) -> Vec<VoiceEvent> {
    let mut out = vec![];
    while let Ok(packet) = rx.try_recv() {
        if let ServerMsg::Voice(v) = rmp_serde::from_slice::<ServerMsg>(&packet.bytes).unwrap() {
            out.push(rmp_serde::from_slice(&v.payload).unwrap());
        }
    }
    out
}

fn last_call(events: &[VoiceEvent]) -> Option<DmCall> {
    events.iter().rev().find_map(|e| match e {
        VoiceEvent::CallState(s) => Some(s.clone()),
        _ => None,
    })
}

fn error(events: &[VoiceEvent]) -> Option<ErrorCode> {
    events.iter().find_map(|e| match e {
        VoiceEvent::Error { code, .. } => Some(*code),
        _ => None,
    })
}

async fn signal(app: &App, core: &mut Core, user: u64, device: u8, nonce: u64, s: VoiceSignal) {
    core.limits.clear();
    if matches!(s, VoiceSignal::Join { .. }) {
        // The web client says it can encrypt before it joins (`e2ee.rs`).
        let hello = VoiceSignal::E2ee(E2eeSignal::Hello { version: 1 });
        crate::voice::signal(
            app,
            core,
            &key(user, device),
            Id(user),
            &format!("ws{user}-{device}"),
            sticktalk_sfu::encode_signal(nonce, &hello).unwrap(),
        )
        .await
        .unwrap();
    }
    crate::voice::signal(
        app,
        core,
        &key(user, device),
        Id(user),
        &format!("ws{user}-{device}"),
        sticktalk_sfu::encode_signal(nonce, &s).unwrap(),
    )
    .await
    .unwrap();
}

fn join(channel_id: Id) -> VoiceSignal {
    VoiceSignal::Join {
        channel_id,
        capabilities: VoiceCapabilities {
            video: vec![VideoCodec::H264],
            hardware_decode: vec![],
        },
    }
}

fn session_of(core: &Core, user: u64) -> VoiceSession {
    *core
        .voice
        .keys()
        .find(|s| s.user_id == Id(user))
        .expect("in the call")
}

/// What the SFU's `Joined` would do: the session's media is connected.
fn connect(core: &mut Core, user: u64, ch: Id) {
    let session = session_of(core, user);
    core.voice.get_mut(&session).unwrap().joined = true;
    connected(core, ch, Id(user));
}

async fn command(app: &App, core: &mut Core, u: u64, nonce: u64, cmd: Command) -> CommandOutcome {
    core.limits.clear();
    commands::execute(
        app,
        core,
        &key(u, 0),
        Id(u),
        CommandRequest {
            nonce,
            command: cmd,
        },
    )
    .await
    .unwrap()
    .outcome
}

#[tokio::test]
async fn starting_a_call_rings_the_others_until_they_answer_decline_or_time_runs_out() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let mut a = attach(&app, &mut core, 1, 0).await;
    let mut b_desk = attach(&app, &mut core, 2, 0).await;
    let mut b_phone = attach(&app, &mut core, 2, 1).await;
    let mut c_rx = attach(&app, &mut core, 3, 0).await;
    let mut outsider = attach(&app, &mut core, 4, 0).await;
    signal(&app, &mut core, 1, 0, 1, join(GROUP)).await;
    assert!(core.calls.active(GROUP));
    assert_eq!(core.calls.ringing(GROUP), vec![Id(2), Id(3)]);
    for rx in [&mut a, &mut b_desk, &mut b_phone, &mut c_rx] {
        let call = last_call(&voice_events(rx)).expect("every member sees the call");
        assert_eq!(call.started_by, Id(1));
        assert_eq!(call.ringing, vec![Id(2), Id(3)]);
    }
    // Someone outside the conversation learns nothing about it.
    assert!(last_call(&voice_events(&mut outsider)).is_none());

    // Declining on the phone stops the ring on the desktop too.
    signal(
        &app,
        &mut core,
        2,
        1,
        2,
        VoiceSignal::DeclineCall { channel_id: GROUP },
    )
    .await;
    assert_eq!(core.calls.ringing(GROUP), vec![Id(3)]);
    assert_eq!(
        last_call(&voice_events(&mut b_desk)).unwrap().ringing,
        vec![Id(3)]
    );
    assert!(voice_events(&mut b_phone).contains(&VoiceEvent::Ack));

    // Answering (joining) stops that person's ring.
    signal(&app, &mut core, 3, 0, 3, join(GROUP)).await;
    assert!(core.calls.ringing(GROUP).is_empty());
    connect(&mut core, 1, GROUP);
    connect(&mut core, 3, GROUP);
    let mut c = app.0.pool.acquire().await.unwrap();
    publish(&mut core, &mut c, GROUP).await.unwrap();
    let call = last_call(&voice_events(&mut a)).unwrap();
    assert_eq!(call.participants.len(), 2);

    // Ringing someone again, and the single timer ending that ring.
    signal(
        &app,
        &mut core,
        1,
        0,
        4,
        VoiceSignal::Ring {
            channel_id: GROUP,
            user_ids: vec![Id(2)],
        },
    )
    .await;
    assert_eq!(core.calls.ringing(GROUP), vec![Id(2)]);
    assert!(core.calls.next_deadline().is_some());
    core.calls.age_rings(GROUP);
    expire(&mut core, &mut c).await.unwrap();
    assert!(core.calls.ringing(GROUP).is_empty());
    assert!(core.calls.next_deadline().is_none());
    assert!(last_call(&voice_events(&mut b_desk))
        .unwrap()
        .ringing
        .is_empty());
}

#[tokio::test]
async fn only_members_join_and_blocked_people_cannot_call_each_other() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let _a = attach(&app, &mut core, 1, 0).await;
    let mut outsider = attach(&app, &mut core, 4, 0).await;
    signal(&app, &mut core, 4, 0, 1, join(DM)).await;
    assert_eq!(
        error(&voice_events(&mut outsider)),
        Some(ErrorCode::NotFound)
    );
    assert!(core.voice.is_empty() && !core.calls.active(DM));

    sqlx::query("INSERT INTO relationships(user_id,other_id,kind,since) VALUES(2,1,'blocked',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut b = attach(&app, &mut core, 2, 0).await;
    signal(&app, &mut core, 2, 0, 2, join(DM)).await;
    assert_eq!(error(&voice_events(&mut b)), Some(ErrorCode::Blocked));
    // Nor the other way round, and in a group the blocked person is not rung.
    let mut a = attach(&app, &mut core, 1, 1).await;
    signal(&app, &mut core, 1, 1, 3, join(DM)).await;
    assert_eq!(error(&voice_events(&mut a)), Some(ErrorCode::Blocked));
    assert!(core.voice.is_empty());
    signal(&app, &mut core, 1, 1, 4, join(GROUP)).await;
    assert_eq!(core.calls.ringing(GROUP), vec![Id(3)]);
}

#[tokio::test]
async fn ringing_requires_being_in_the_call_and_names_members_only() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let mut a0 = attach(&app, &mut core, 1, 0).await;
    let mut b = attach(&app, &mut core, 2, 0).await;
    let mut d = attach(&app, &mut core, 4, 0).await;
    let ring = |user_ids: Vec<Id>| VoiceSignal::Ring {
        channel_id: GROUP,
        user_ids,
    };
    // No call yet.
    signal(&app, &mut core, 2, 0, 1, ring(vec![Id(3)])).await;
    assert_eq!(error(&voice_events(&mut b)), Some(ErrorCode::NotFound));
    signal(&app, &mut core, 1, 0, 2, join(GROUP)).await;
    voice_events(&mut b);
    // A member who is not in the call cannot ring.
    signal(&app, &mut core, 2, 0, 3, ring(vec![Id(3)])).await;
    assert_eq!(error(&voice_events(&mut b)), Some(ErrorCode::Forbidden));
    // An outsider cannot ring, nor learn whether the call exists.
    signal(&app, &mut core, 4, 0, 4, ring(vec![Id(3)])).await;
    assert_eq!(error(&voice_events(&mut d)), Some(ErrorCode::NotFound));
    // Targets must be members other than you, not repeated, and not too many.
    voice_events(&mut a0);
    for (nonce, targets) in [
        (5, vec![Id(4)]),
        (6, vec![Id(1)]),
        (7, vec![Id(3), Id(3)]),
        (8, vec![]),
        (9, (0..17).map(|n| Id(100 + n)).collect()),
    ] {
        signal(&app, &mut core, 1, 0, nonce, ring(targets)).await;
        assert_eq!(
            error(&voice_events(&mut a0)),
            Some(ErrorCode::InvalidRequest)
        );
    }
    signal(&app, &mut core, 1, 0, 12, ring(vec![Id(2)])).await;
    assert!(voice_events(&mut a0).contains(&VoiceEvent::Ack));
}

#[tokio::test]
async fn the_last_person_leaving_ends_the_call_and_records_it() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let mut a = attach(&app, &mut core, 1, 0).await;
    let mut b = attach(&app, &mut core, 2, 0).await;
    let mut c = app.0.pool.acquire().await.unwrap();

    // Nobody answers: a missed call.
    signal(&app, &mut core, 1, 0, 1, join(DM)).await;
    connect(&mut core, 1, DM);
    let session = session_of(&core, 1);
    crate::voice::remove(&app, &mut core, &mut c, session)
        .await
        .unwrap();
    assert!(!core.calls.active(DM));
    assert!(voice_events(&mut b).contains(&VoiceEvent::CallEnded { channel_id: DM }));
    let created: Vec<_> = core.streams[&key(2, 0)]
        .ring
        .iter()
        .filter_map(
            |(_, _, bytes)| match rmp_serde::from_slice::<ServerMsg>(bytes).unwrap() {
                ServerMsg::Event(e) => match e.event {
                    Event::MessageCreated { message, .. } => Some(message),
                    _ => None,
                },
                _ => None,
            },
        )
        .collect();
    let missed = created.last().expect("a system message");
    assert_eq!(missed.author_id, Id(1));
    assert_eq!(missed.content, "Missed call");
    let summary = missed.call.clone().unwrap();
    assert!(summary.missed);
    assert_eq!(summary.participant_count, 1);
    // It counts as unread for the person who missed it.
    assert_eq!(db::read(&mut c, Id(2), DM).await.unwrap().unread_count, 1);
    // History returns the same record.
    let stored = db::message(&mut c, Id(2), DM, missed.id, false)
        .await
        .unwrap();
    assert_eq!(stored.call, Some(summary));

    // An answered call: one leaves, the call goes on; the last leaves, it ends.
    signal(&app, &mut core, 1, 0, 2, join(DM)).await;
    signal(&app, &mut core, 2, 0, 3, join(DM)).await;
    connect(&mut core, 1, DM);
    connect(&mut core, 2, DM);
    voice_events(&mut a);
    let first = session_of(&core, 1);
    crate::voice::remove(&app, &mut core, &mut c, first)
        .await
        .unwrap();
    assert!(core.calls.active(DM));
    let events = voice_events(&mut a);
    // The connection that left is told the call is over for it.
    assert!(events.contains(&VoiceEvent::CallEnded { channel_id: DM }));
    assert_eq!(last_call(&events).unwrap().participants.len(), 1);
    let second = session_of(&core, 2);
    crate::voice::remove(&app, &mut core, &mut c, second)
        .await
        .unwrap();
    assert!(!core.calls.active(DM));
    let last: i64 = sqlx::query_scalar("SELECT MAX(message_id) FROM call_records")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let answered = db::message(&mut c, Id(1), DM, Id(last as u64), false)
        .await
        .unwrap();
    let summary = answered.call.unwrap();
    assert!(!summary.missed);
    assert_eq!(summary.participant_count, 2);
    assert!(answered.content.starts_with("Started a call"));

    // The record cannot be edited into something else.
    let outcome = command(
        &app,
        &mut core,
        1,
        50,
        Command::EditMessage(MessageEdit {
            channel_id: DM,
            message_id: answered.id,
            content: "forged".into(),
            mention_ids: vec![],
        }),
    )
    .await;
    assert!(matches!(outcome, CommandOutcome::Err(e) if e.code == ErrorCode::Forbidden));

    // A call whose media never connected leaves nothing behind.
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM call_records")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    signal(&app, &mut core, 1, 0, 4, join(DM)).await;
    let session = session_of(&core, 1);
    crate::voice::remove(&app, &mut core, &mut c, session)
        .await
        .unwrap();
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM call_records")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn nobody_can_moderate_a_direct_call() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let _a = attach(&app, &mut core, 1, 0).await;
    let mut b = attach(&app, &mut core, 2, 0).await;
    signal(&app, &mut core, 1, 0, 1, join(DM)).await;
    let target = session_of(&core, 1);
    for (nonce, action) in [
        (2, VoiceModeration::Mute { muted: true }),
        (3, VoiceModeration::Deafen { deafened: true }),
        (4, VoiceModeration::Disconnect),
        (5, VoiceModeration::Move { channel_id: GROUP }),
    ] {
        signal(
            &app,
            &mut core,
            2,
            0,
            nonce,
            VoiceSignal::Moderate {
                session: target,
                action,
            },
        )
        .await;
        assert_eq!(error(&voice_events(&mut b)), Some(ErrorCode::Forbidden));
    }
    assert!(core.voice.contains_key(&target));
    assert!(!core.voice[&target].state.participant.muted);
}

#[tokio::test]
async fn blocking_or_leaving_the_group_takes_you_out_of_its_call() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let _a = attach(&app, &mut core, 1, 0).await;
    let _b = attach(&app, &mut core, 2, 0).await;
    let _c = attach(&app, &mut core, 3, 0).await;
    signal(&app, &mut core, 1, 0, 1, join(DM)).await;
    signal(&app, &mut core, 2, 0, 2, join(DM)).await;
    assert_eq!(core.voice.len(), 2);
    let outcome = command(
        &app,
        &mut core,
        2,
        10,
        Command::BlockUser { user_id: Id(1) },
    )
    .await;
    assert!(matches!(outcome, CommandOutcome::Ok(_)), "{outcome:?}");
    assert!(core.voice.is_empty(), "both sides of a 1:1 call end");
    assert!(!core.calls.active(DM));

    signal(&app, &mut core, 3, 0, 3, join(GROUP)).await;
    signal(&app, &mut core, 1, 0, 4, join(GROUP)).await;
    let outcome = command(
        &app,
        &mut core,
        3,
        11,
        Command::LeaveGroupDm { channel_id: GROUP },
    )
    .await;
    assert!(matches!(outcome, CommandOutcome::Ok(_)), "{outcome:?}");
    assert!(core.voice.keys().all(|s| s.user_id != Id(3)));
    assert!(core.calls.active(GROUP));
}

#[tokio::test]
async fn snapshots_list_calls_in_my_conversations_only() {
    let (_dir, app) = fixture().await;
    conversations(&app).await;
    let mut core = app.0.core.lock().await;
    let _a = attach(&app, &mut core, 1, 0).await;
    let mut c_rx = attach(&app, &mut core, 3, 0).await;
    let mut d = attach(&app, &mut core, 4, 0).await;
    let _a1 = attach(&app, &mut core, 1, 1).await;
    signal(&app, &mut core, 1, 0, 1, join(DM)).await;
    signal(&app, &mut core, 1, 1, 2, join(GROUP)).await;
    voice_events(&mut c_rx);
    let mut c = app.0.pool.acquire().await.unwrap();
    snapshot(&mut core, &mut c, &key(3, 0), Id(3))
        .await
        .unwrap();
    snapshot(&mut core, &mut c, &key(4, 0), Id(4))
        .await
        .unwrap();
    let calls = |events: Vec<VoiceEvent>| {
        events.into_iter().find_map(|e| match e {
            VoiceEvent::CallSnapshot { calls } => Some(calls),
            _ => None,
        })
    };
    let mine = calls(voice_events(&mut c_rx)).unwrap();
    assert!(mine.iter().all(|call| call.channel_id == GROUP));
    assert!(calls(voice_events(&mut d)).unwrap().is_empty());
    // DM sessions never appear as guild voice state.
    let ready = db::ready(&mut c, &mut core, &key(1, 0), Id(1))
        .await
        .unwrap();
    assert!(ready.voice_states.is_empty());
}

#[tokio::test]
async fn voice_channels_have_their_own_permission_checked_chat() {
    let (_dir, app) = fixture().await;
    // As CreateChannel leaves it: a read position for every member.
    sqlx::raw_sql("INSERT INTO channels VALUES(12,'voice',10,NULL,'lounge',NULL,1,NULL,0);INSERT INTO read_states(user_id,channel_id) VALUES(1,12);")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    // Ready gives everyone who can see it a read position for its chat.
    let ready = db::ready(&mut c, &mut core, &key(2, 0), Id(2))
        .await
        .unwrap();
    assert!(ready.read_states.iter().any(|r| r.channel_id == Id(12)));
    let send = |nonce: u64, content: &str| {
        (
            nonce,
            Command::SendMessage(MessageDraft {
                channel_id: Id(12),
                content: content.into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }),
        )
    };
    let (n, cmd) = send(1, "hello from the lounge");
    let outcome = command(&app, &mut core, 2, n, cmd).await;
    let CommandOutcome::Ok(CommandResult::Message(m)) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(m.channel_id, Id(12));
    // It is unread for the other member, and only for them.
    assert_eq!(
        db::read(&mut c, Id(1), Id(12)).await.unwrap().unread_count,
        1
    );
    assert_eq!(
        db::read(&mut c, Id(2), Id(12)).await.unwrap().unread_count,
        0
    );
    let outcome = command(
        &app,
        &mut core,
        1,
        2,
        Command::MarkRead {
            channel_id: Id(12),
            message_id: m.id,
        },
    )
    .await;
    assert!(matches!(outcome, CommandOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(
        db::read(&mut c, Id(1), Id(12)).await.unwrap().unread_count,
        0
    );

    // Without Send Messages on that voice channel: forbidden.
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
        .bind(Permissions::SEND_MESSAGES.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    let (n, cmd) = send(3, "no");
    let outcome = command(&app, &mut core, 2, n, cmd).await;
    assert!(matches!(outcome, CommandOutcome::Err(e) if e.code == ErrorCode::Forbidden));
    // Without View Channel it does not exist for them, and nor does its history.
    sqlx::query("UPDATE permission_overwrites SET deny=? WHERE channel_id=12")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    let (n, cmd) = send(4, "no");
    let outcome = command(&app, &mut core, 2, n, cmd).await;
    assert!(matches!(outcome, CommandOutcome::Err(e) if e.code == ErrorCode::NotFound));
    assert_eq!(
        db::text(&mut c, Id(2), Id(12), Permissions::READ_HISTORY)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    // A category still has no chat.
    sqlx::query("INSERT INTO channels VALUES(13,'category',10,NULL,'cat',NULL,2,NULL,0)")
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        db::text(&mut c, Id(1), Id(13), Permissions::VIEW_CHANNEL)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn call_records_read_naturally() {
    let at = |minutes: u64, missed: bool| CallSummary {
        started_at: 1_000,
        ended_at: 1_000 + minutes * 60_000 + 5_000,
        missed,
        participant_count: 2,
    };
    assert_eq!(describe(&at(0, true)), "Missed call");
    assert_eq!(
        describe(&at(0, false)),
        "Started a call that lasted less than a minute."
    );
    assert_eq!(
        describe(&at(1, false)),
        "Started a call that lasted a minute."
    );
    assert_eq!(
        describe(&at(12, false)),
        "Started a call that lasted 12 minutes."
    );
    assert_eq!(
        describe(&at(61, false)),
        "Started a call that lasted 1 hour 1 minute."
    );
    assert_eq!(
        describe(&at(125, false)),
        "Started a call that lasted 2 hours 5 minutes."
    );
}
