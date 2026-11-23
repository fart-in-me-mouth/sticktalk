use super::*;
use crate::{audit_tests::fixture, commands, db};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

async fn run(app: &App, u: u64, nonce: u64, command: Command) -> CommandOutcome {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest { nonce, command },
    )
    .await
    .unwrap()
    .outcome
}
async fn content(app: &App, u: u64, nonce: u64, cmd: ContentCommand) -> CommandOutcome {
    run(app, u, nonce, Command::Content(cmd)).await
}
fn code(outcome: &CommandOutcome) -> Option<ErrorCode> {
    match outcome {
        CommandOutcome::Err(e) => Some(e.code),
        CommandOutcome::Ok(_) => None,
    }
}
fn ok(outcome: CommandOutcome) -> CommandResult {
    match outcome {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("command failed: {e:?}"),
    }
}
fn events(core: &Core, u: u8) -> Vec<Event> {
    core.streams[&vec![u; 32]]
        .ring
        .iter()
        .filter_map(
            |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                ServerMsg::Event(e) => Some(e.event),
                _ => None,
            },
        )
        .collect()
}
async fn sql(app: &App, q: &'static str) {
    sqlx::query(q).execute(&app.0.pool).await.unwrap();
}
/// Guild 10 (#11) has users 1 (owner), 2 and 4. Guild 20 (#21) has 1 and 3.
async fn world() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,4,'member',0)").await;
    sql(&app, "INSERT INTO guilds VALUES(20,1,'other',NULL,0)").await;
    sql(
        &app,
        "INSERT INTO guild_members VALUES(20,1,'owner',0),(20,3,'member',0)",
    )
    .await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'elsewhere',0,0)").await;
    sql(
        &app,
        "INSERT INTO read_states(user_id,channel_id) VALUES(4,11),(1,21),(3,21)",
    )
    .await;
    (dir, app)
}
fn draft(ch: u64, content: &str, attachments: Vec<Id>) -> MessageDraft {
    MessageDraft {
        channel_id: Id(ch),
        content: content.into(),
        reply_to: None,
        attachment_ids: attachments,
        mention_ids: vec![],
    }
}
fn poll(ch: u64, answers: usize, multiple: bool, hours: u32) -> ContentCommand {
    ContentCommand::CreatePoll(PollDraft {
        channel_id: Id(ch),
        question: "Where should we eat?".into(),
        answers: (0..answers)
            .map(|i| PollAnswerDraft {
                text: format!("Place {i}"),
                emoji: (i == 0).then(|| "🍕".into()),
            })
            .collect(),
        multiple,
        duration_hours: hours,
    })
}
fn message_of(r: CommandResult) -> Message {
    match r {
        CommandResult::Message(m) => m,
        other => panic!("not a message: {other:?}"),
    }
}
fn poll_of(r: CommandResult) -> Poll {
    match r {
        CommandResult::Content(ContentResult::Poll { poll, .. }) => poll,
        other => panic!("not a poll: {other:?}"),
    }
}
async fn upload(app: &App, id: i64, owner: i64, ch: i64, kind: &str) {
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at,expires_at) VALUES(?,?,'message',?,?,?,4,?,0,?)")
        .bind(id)
        .bind(owner)
        .bind(ch)
        .bind(format!("file{id}"))
        .bind(kind)
        .bind(format!("key{id}"))
        .bind((now() + 3_600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn polls_validate_votes_and_permissions() {
    let (_dir, app) = world().await;
    // Opt user 1 in to content events; user 4 stays an older client.
    ok(content(&app, 1, 1, ContentCommand::Subscribe).await);
    // A second session of user 4 is an older client that never subscribes.
    app.0.core.lock().await.cursor(&[44; 32], Id(4));
    for (n, bad) in [
        poll(11, 1, false, 24),
        poll(11, 11, false, 24),
        poll(11, 3, false, 5),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            code(&content(&app, 2, 10 + n as u64, bad).await),
            Some(ErrorCode::InvalidRequest)
        );
    }
    let mut blank = poll(11, 2, false, 1);
    if let ContentCommand::CreatePoll(d) = &mut blank {
        d.question = "  ".into();
    }
    assert_eq!(
        code(&content(&app, 2, 20, blank).await),
        Some(ErrorCode::InvalidRequest)
    );
    // Someone outside the server cannot post one there.
    assert!(code(&content(&app, 3, 21, poll(11, 2, false, 1)).await).is_some());
    let m = message_of(ok(content(&app, 2, 22, poll(11, 4, false, 24)).await));
    let p = m.extra.poll.clone().unwrap();
    assert_eq!((p.answers.len(), p.multiple, p.closed), (4, false, false));
    assert_eq!(p.answers[0].emoji.as_deref(), Some("🍕"));
    assert_eq!(m.content, "Where should we eat?");
    let vote = |answers: Vec<u8>| ContentCommand::VotePoll {
        channel_id: Id(11),
        message_id: m.id,
        answers,
    };
    assert_eq!(
        poll_of(ok(content(&app, 1, 30, vote(vec![0])).await)).my_votes,
        vec![0]
    );
    // One answer only, valid indexes only, readers only.
    assert_eq!(
        code(&content(&app, 1, 31, vote(vec![0, 1])).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(&content(&app, 1, 32, vote(vec![4])).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(&content(&app, 3, 33, vote(vec![1])).await),
        Some(ErrorCode::NotFound)
    );
    let changed = poll_of(ok(content(&app, 1, 34, vote(vec![2])).await));
    assert_eq!((changed.answers[0].votes, changed.answers[2].votes), (0, 1));
    ok(content(&app, 4, 35, vote(vec![2])).await);
    let removed = poll_of(ok(content(&app, 1, 36, vote(vec![])).await));
    assert_eq!(
        (
            removed.voters,
            removed.answers[2].votes,
            removed.my_votes.len()
        ),
        (1, 1, 0)
    );
    // Live tallies reach subscribed sessions only.
    {
        let core = app.0.core.lock().await;
        let updates = events(&core, 1)
            .into_iter()
            .filter(|e| matches!(e, Event::Content(ContentEvent::PollUpdated { .. })))
            .count();
        assert_eq!(updates, 4);
        assert!(!events(&core, 44)
            .iter()
            .any(|e| matches!(e, Event::Content(_))));
        assert!(events(&core, 44)
            .iter()
            .any(|e| matches!(e, Event::MessageCreated { .. })));
    }
    // Viewers see their own votes in history.
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = db::message(&mut c, Id(4), Id(11), m.id, false)
        .await
        .unwrap();
    assert_eq!(seen.extra.poll.unwrap().my_votes, vec![2]);
    let voters = polls::voters(&mut c, Id(2), Id(11), m.id, 2, None)
        .await
        .unwrap();
    assert_eq!(
        voters.users.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![Id(4)]
    );
    assert_eq!(
        polls::voters(&mut c, Id(3), Id(11), m.id, 2, None)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    drop(c);
    // Only the author or a moderator ends it; then it takes no votes.
    let end = ContentCommand::EndPoll {
        channel_id: Id(11),
        message_id: m.id,
    };
    assert_eq!(
        code(&content(&app, 4, 40, end.clone()).await),
        Some(ErrorCode::Forbidden)
    );
    let ended = poll_of(ok(content(&app, 1, 41, end.clone()).await));
    assert!(ended.closed);
    assert_eq!(ended.winners(), vec![2]);
    assert_eq!(
        code(&content(&app, 4, 42, vote(vec![1])).await),
        Some(ErrorCode::Conflict)
    );
    assert_eq!(
        code(&content(&app, 2, 43, end).await),
        Some(ErrorCode::Conflict)
    );
    // Multiple choice accepts several answers.
    let multi = message_of(ok(content(&app, 2, 50, poll(11, 3, true, 1)).await));
    let picked = poll_of(ok(content(
        &app,
        4,
        51,
        ContentCommand::VotePoll {
            channel_id: Id(11),
            message_id: multi.id,
            answers: vec![2, 0, 2],
        },
    )
    .await));
    assert_eq!(picked.my_votes, vec![0, 2]);
}

#[tokio::test]
async fn polls_close_at_their_deadline_through_the_timer() {
    let (_dir, app) = world().await;
    ok(content(&app, 2, 1, ContentCommand::Subscribe).await);
    let m = message_of(ok(content(&app, 2, 2, poll(11, 2, false, 1)).await));
    assert_eq!(
        next_deadline(&app).await.unwrap().map(|d| d / 60_000),
        Some((now() + 3_600_000) / 60_000)
    );
    spawn(&app);
    sqlx::query("UPDATE polls SET expires_at=? WHERE message_id=?")
        .bind((now() + 150) as i64)
        .bind(m.id.0 as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    app.0.content.wake.notify_one();
    let mut closed = None;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        closed =
            sqlx::query_scalar::<_, Option<i64>>("SELECT closed_at FROM polls WHERE message_id=?")
                .bind(m.id.0 as i64)
                .fetch_one(&app.0.pool)
                .await
                .unwrap();
        if closed.is_some() {
            break;
        }
    }
    assert!(closed.is_some(), "the timer closed the poll");
    let core = app.0.core.lock().await;
    assert!(events(&core, 2).iter().any(
        |e| matches!(e, Event::Content(ContentEvent::PollUpdated { poll, .. }) if poll.closed)
    ));
    drop(core);
    assert_eq!(next_deadline(&app).await.unwrap(), None);
    app.0.shutdown.cancel();
}

#[tokio::test]
async fn forwarding_checks_both_ends_and_snapshots_without_leaking_the_source() {
    let (_dir, app) = world().await;
    for u in [1, 3] {
        ok(content(&app, u, 1, ContentCommand::Subscribe).await);
    }
    upload(&app, 500, 2, 11, "image/png").await;
    let original = message_of(ok(run(
        &app,
        2,
        2,
        Command::SendMessage(MessageDraft {
            mention_ids: vec![Id(1)],
            ..draft(11, "look <@1>", vec![Id(500)])
        }),
    )
    .await));
    let forward = |_by: u64, to: Vec<u64>, comment: &str| ContentCommand::Forward {
        channel_id: Id(11),
        message_id: original.id,
        destinations: to.into_iter().map(Id).collect(),
        comment: comment.into(),
    };
    // User 2 cannot send in #21; user 3 cannot read #11.
    assert_eq!(
        code(&content(&app, 2, 3, forward(2, vec![21], "")).await),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        code(&content(&app, 3, 4, forward(3, vec![21], "")).await),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        code(&content(&app, 1, 5, forward(1, vec![], "")).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(&content(&app, 1, 6, forward(1, vec![21, 21], "")).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(&content(&app, 1, 7, forward(1, vec![21, 11, 21, 11, 21, 11], "")).await),
        Some(ErrorCode::InvalidRequest)
    );
    let CommandResult::Content(ContentResult::Forwarded(sent)) =
        ok(content(&app, 1, 8, forward(1, vec![21, 11], "for you")).await)
    else {
        panic!("forwarded")
    };
    assert_eq!(sent.len(), 2);
    let copy = sent[0].extra.forwarded.as_ref().unwrap();
    assert_eq!(sent[0].content, "for you");
    assert_eq!(copy.content, "look @user1");
    assert_eq!(copy.attachments[0].id, Id(500));
    assert_eq!(copy.created_at, original.created_at);
    assert!(copy.source.is_some());
    // User 3 sees the snapshot in #21 but not where it came from.
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = db::message(&mut c, Id(3), Id(21), sent[0].id, false)
        .await
        .unwrap();
    assert!(seen.extra.forwarded.as_ref().unwrap().source.is_none());
    {
        let core = app.0.core.lock().await;
        let created = events(&core, 3)
            .into_iter()
            .find_map(|e| match e {
                Event::MessageCreated { message, .. } => Some(message),
                _ => None,
            })
            .unwrap();
        assert!(created.extra.forwarded.unwrap().source.is_none());
    }
    // The same stored file serves the copy, for readers of the copy only.
    assert!(forward::file_access(&mut c, Id(3), Id(500)).await.unwrap());
    sql(&app, "INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(5,'user5','x','online',0)").await;
    assert!(!forward::file_access(&mut c, Id(5), Id(500)).await.unwrap());
    // Deleting the original keeps the snapshot and its file.
    ok(run(
        &app,
        2,
        9,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: original.id,
        },
    )
    .await);
    assert!(db::gc_uploads(&mut c).await.unwrap().is_empty());
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE id=500")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(kept, 1, "a forwarded file is still referenced");
    let seen = db::message(&mut c, Id(1), Id(21), sent[0].id, false)
        .await
        .unwrap();
    let f = seen.extra.forwarded.unwrap();
    assert!(f.source.is_none(), "a deleted source has no link back");
    assert_eq!(f.attachments.len(), 1);
    drop(c);
    // Deleting the copy releases the file reference too.
    ok(run(
        &app,
        1,
        10,
        Command::DeleteMessage {
            channel_id: Id(21),
            message_id: sent[0].id,
        },
    )
    .await);
    ok(run(
        &app,
        1,
        11,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: sent[1].id,
        },
    )
    .await);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE id=500")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        left, 0,
        "the last reference is gone, so the file is collected"
    );
}

#[tokio::test]
async fn attachment_details_and_voice_messages_are_validated() {
    let (_dir, app) = world().await;
    upload(&app, 600, 2, 11, "image/png").await;
    upload(&app, 601, 2, 11, "audio/ogg").await;
    upload(&app, 602, 1, 11, "image/png").await;
    let send = |ids: Vec<i64>, content: &str, patches: Vec<(i64, AttachmentMeta)>| {
        ContentCommand::SendMessage {
            draft: draft(11, content, ids.into_iter().map(|v| Id(v as u64)).collect()),
            attachments: patches
                .into_iter()
                .map(|(id, meta)| AttachmentMetaPatch {
                    attachment_id: Id(id as u64),
                    meta,
                })
                .collect(),
        }
    };
    let alt = |d: &str, spoiler: bool| AttachmentMeta {
        description: Some(d.into()),
        spoiler,
        ..Default::default()
    };
    let voice = |ms: u32, wave: usize| AttachmentMeta {
        duration_ms: Some(ms),
        waveform: vec![128; wave],
        ..Default::default()
    };
    let long = "x".repeat(MAX_ATTACHMENT_DESCRIPTION_CHARS + 1);
    for (n, bad) in [
        send(vec![600], "", vec![(600, alt(&long, false))]),
        send(vec![600], "", vec![(601, alt("not in draft", false))]),
        send(vec![600], "", vec![(600, voice(1000, 10))]),
        send(vec![601], "hi", vec![(601, voice(1000, 10))]),
        send(
            vec![601],
            "",
            vec![(601, voice(MAX_VOICE_MESSAGE_MS + 1, 10))],
        ),
        send(
            vec![601],
            "",
            vec![(601, voice(1000, MAX_VOICE_WAVEFORM_SAMPLES + 1))],
        ),
        send(
            vec![601],
            "",
            vec![(
                601,
                AttachmentMeta {
                    waveform: vec![1],
                    ..Default::default()
                },
            )],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            code(&content(&app, 2, 10 + n as u64, bad).await),
            Some(ErrorCode::InvalidRequest),
            "case {n}"
        );
    }
    // Another person's upload cannot be described.
    assert_eq!(
        code(
            &content(
                &app,
                2,
                30,
                send(vec![602], "", vec![(602, alt("mine?", true))])
            )
            .await
        ),
        Some(ErrorCode::InvalidAttachment)
    );
    let m = message_of(ok(content(
        &app,
        2,
        31,
        send(
            vec![600],
            "a picture",
            vec![(600, alt("  A red kite  ", true))],
        ),
    )
    .await));
    assert_eq!(
        m.attachments[0].meta.description.as_deref(),
        Some("A red kite")
    );
    assert!(m.attachments[0].meta.spoiler);
    let v = message_of(ok(content(
        &app,
        2,
        32,
        send(vec![601], "", vec![(601, voice(4_200, 64))]),
    )
    .await));
    assert_eq!(v.attachments[0].meta.duration_ms, Some(4_200));
    assert_eq!(v.attachments[0].meta.waveform.len(), 64);
    assert_eq!(
        audio_mime(b"OggS\0\x02\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0OpusHead\x01\x01"),
        Some("audio/ogg")
    );
    assert_eq!(audio_mime(b"OggS but not opus at all, really no"), None);
}

#[tokio::test]
async fn ten_attachments_per_message() {
    let (_dir, app) = world().await;
    for i in 0..11 {
        upload(&app, 700 + i, 2, 11, "image/png").await;
    }
    let ids = |n: i64| (0..n).map(|i| Id(700 + i as u64)).collect::<Vec<_>>();
    assert_eq!(
        code(&run(&app, 2, 1, Command::SendMessage(draft(11, "", ids(11)))).await),
        Some(ErrorCode::InvalidRequest)
    );
    let m = message_of(ok(run(
        &app,
        2,
        2,
        Command::SendMessage(draft(11, "", ids(10))),
    )
    .await));
    assert_eq!(m.attachments.len(), MAX_MESSAGE_ATTACHMENTS);
}

fn schedule(ch: u64, text: &str, at: u64) -> ContentCommand {
    ContentCommand::ScheduleMessage {
        channel_id: Id(ch),
        content: text.into(),
        mention_ids: vec![],
        send_at: at,
        attachment_ids: vec![],
    }
}
fn scheduled_of(r: CommandResult) -> ScheduledMessage {
    match r {
        CommandResult::Content(ContentResult::Scheduled(s)) => s,
        other => panic!("not scheduled: {other:?}"),
    }
}

#[tokio::test]
async fn scheduled_messages_are_bounded_owned_and_rechecked_at_delivery() {
    let (_dir, app) = world().await;
    ok(content(&app, 2, 1, ContentCommand::Subscribe).await);
    let later = now() + 3_600_000;
    for (n, bad) in [
        schedule(11, "past", now().saturating_sub(1)),
        schedule(11, "too far", now() + MAX_SCHEDULE_AHEAD_MS + 60_000),
        schedule(11, "   ", later),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            code(&content(&app, 2, 10 + n as u64, bad).await),
            Some(ErrorCode::InvalidRequest)
        );
    }
    assert_eq!(
        code(&content(&app, 3, 20, schedule(11, "not mine", later)).await),
        Some(ErrorCode::NotFound)
    );
    let s = scheduled_of(ok(
        content(&app, 2, 21, schedule(11, "hello later", later)).await
    ));
    // Only the author can see or change it.
    let update = |id: Id, text: &str, at: u64| ContentCommand::UpdateScheduled {
        id,
        content: text.into(),
        mention_ids: vec![],
        send_at: at,
        attachment_ids: None,
    };
    assert_eq!(
        code(&content(&app, 1, 22, update(s.id, "hijack", later)).await),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        code(&content(&app, 1, 23, ContentCommand::CancelScheduled { id: s.id }).await),
        Some(ErrorCode::NotFound)
    );
    let edited = scheduled_of(ok(
        content(&app, 2, 24, update(s.id, "edited", later + 1)).await
    ));
    assert_eq!(
        (edited.content.as_str(), edited.send_at),
        ("edited", later + 1)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        scheduled::list(&mut c, Id(2), Some(Id(11)))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(scheduled::list(&mut c, Id(1), None)
        .await
        .unwrap()
        .items
        .is_empty());
    drop(c);
    // Send now delivers it as a normal message.
    let sent = message_of(ok(content(
        &app,
        2,
        25,
        ContentCommand::SendScheduledNow { id: s.id },
    )
    .await));
    assert_eq!(sent.content, "edited");
    // Bounded per person.
    for i in 0..MAX_SCHEDULED_PER_USER as u64 {
        ok(content(&app, 2, 100 + i, schedule(11, "queued", later)).await);
    }
    assert_eq!(
        code(&content(&app, 2, 99, schedule(11, "one too many", later)).await),
        Some(ErrorCode::LimitExceeded)
    );
    sql(&app, "DELETE FROM scheduled_messages").await;
    // Due ones are sent by the timer path; one that can no longer be sent fails visibly.
    let due = scheduled_of(ok(content(
        &app,
        2,
        200,
        schedule(11, "due now", now() + 60_000),
    )
    .await));
    let blocked = scheduled_of(ok(content(
        &app,
        2,
        201,
        schedule(11, "after leaving", now() + 60_000),
    )
    .await));
    sql(&app, "UPDATE scheduled_messages SET send_at=0").await;
    // User 2 loses access to the channel before the second one is sent.
    sqlx::query("UPDATE scheduled_messages SET channel_id=21 WHERE id=?")
        .bind(blocked.id.0 as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    scheduled::deliver_due(&app).await.unwrap();
    let delivered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE content='due now' AND channel_id=11",
    )
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    assert_eq!(delivered, 1);
    let failure: Option<String> =
        sqlx::query_scalar("SELECT failure FROM scheduled_messages WHERE id=?")
            .bind(blocked.id.0 as i64)
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(
        failure.as_deref(),
        Some("You can no longer send messages there.")
    );
    let core = app.0.core.lock().await;
    let seen = events(&core, 2);
    assert!(seen.iter().any(|e| matches!(e, Event::Content(ContentEvent::ScheduledRemoved { id, sent: Some(_), .. }) if *id == due.id)));
    assert!(seen.iter().any(|e| matches!(e, Event::Content(ContentEvent::ScheduledUpserted(s)) if s.id == blocked.id && s.failure.is_some())));
    drop(core);
    // A failed one is not retried by itself.
    assert_eq!(next_deadline(&app).await.unwrap(), None);
}

#[tokio::test]
async fn scheduled_delivery_respects_slowmode() {
    let (_dir, app) = world().await;
    ok(run(
        &app,
        1,
        1,
        Command::Launch(LaunchCommand::SetSlowmode {
            channel_id: Id(11),
            seconds: 60,
        }),
    )
    .await);
    let s = scheduled_of(ok(content(
        &app,
        2,
        2,
        schedule(11, "first", now() + 60_000),
    )
    .await));
    ok(run(
        &app,
        2,
        3,
        Command::SendMessage(draft(11, "sent by hand", vec![])),
    )
    .await);
    sql(&app, "UPDATE scheduled_messages SET send_at=0").await;
    scheduled::deliver_due(&app).await.unwrap();
    let failure: Option<String> =
        sqlx::query_scalar("SELECT failure FROM scheduled_messages WHERE id=?")
            .bind(s.id.0 as i64)
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    assert!(failure.unwrap().starts_with("Slowmode"));
}

#[test]
fn only_public_addresses_and_names_are_fetched() {
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "192.0.2.1",
        "198.18.0.1",
        "240.0.0.1",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd12::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::a00:1",
        "2002:a00:1::",
        "2001:db8::1",
        "2001::1",
        "fec0::1",
    ] {
        assert!(
            !previews::public_ip(ip.parse().unwrap()),
            "{ip} must be refused"
        );
    }
    for ip in [
        "93.184.216.34",
        "1.1.1.1",
        "2606:4700:4700::1111",
        "::ffff:8.8.8.8",
    ] {
        assert!(previews::public_ip(ip.parse().unwrap()), "{ip} is public");
    }
    for name in [
        "localhost",
        "intranet",
        "printer.local",
        "a.test",
        "x.example",
        "foo.invalid",
        "router.home.arpa",
        "db.internal",
    ] {
        assert!(!previews::public_name(name), "{name}");
    }
    assert!(previews::public_name("news.example.com"));
}

#[test]
fn page_metadata_is_read_from_the_head_as_plain_text() {
    let html = r#"<!doctype html><html><head><title>Fallback &amp; title</title>
        <meta property="og:title" content="A &quot;great&quot; day">
        <META NAME='description' CONTENT='Plain &lt;b&gt;text&lt;/b&gt;   with
        spaces'>
        <meta property=og:site_name content=Example>
        <meta property="og:image" content="/img.png" />
        <meta property="og:video:url" content="https://v.example.com/1">
        </head><body><meta property="og:title" content="not in head"></body></html>"#;
    let meta = previews::parse_html(html);
    assert_eq!(meta["og:title"], "A &quot;great&quot; day");
    assert_eq!(meta["description"].split_whitespace().count(), 4);
    assert_eq!(meta["og:site_name"], "Example");
    assert_eq!(meta["og:image"], "/img.png");
    assert_eq!(meta["<title>"], "Fallback &amp; title");
    assert!(meta.contains_key("og:video:url"));
}

/// A tiny local website for the previewer. Never the real internet.
async fn mock_site() -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut png = vec![];
    image::DynamicImage::new_rgb8(900, 600)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            let png = png.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, kind, body): (&str, &str, Vec<u8>) = match path.as_str() {
                    "/page" => ("200 OK", "text/html; charset=utf-8", format!(r#"<html><head><meta property="og:title" content="Kite festival"><meta property="og:description" content="Hundreds of kites"><meta property="og:site_name" content="News"><meta property="og:image" content="http://news.example.com:{port}/kite.png"></head></html>"#).into_bytes()),
                    "/kite.png" => ("200 OK", "image/png", png),
                    "/hop" => ("302 Found", "text/html", vec![]),
                    "/to-private" => ("302 Found", "text/html", vec![]),
                    "/text" => ("200 OK", "text/plain", b"<meta property=og:title content=x>".to_vec()),
                    "/huge" => ("200 OK", "text/html", {
                        let mut b = b"<html><head><title>Big</title></head>".to_vec();
                        b.resize(3 * 1024 * 1024, b' ');
                        b
                    }),
                    _ => ("404 Not Found", "text/html", vec![]),
                };
                let location = match path.as_str() {
                    "/hop" => format!("Location: http://news.example.com:{port}/page\r\n"),
                    "/to-private" => {
                        format!("Location: http://internal.example.net:{port}/page\r\n")
                    }
                    _ => String::new(),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(&body).await;
            });
        }
    });
    port
}
struct MapResolver;
impl previews::HostResolver for MapResolver {
    fn lookup(&self, host: String) -> previews::Lookup {
        Box::pin(async move {
            Ok(match host.as_str() {
                "news.example.com" => vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                "internal.example.net" => vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))],
                // A rebinding-style answer mixing an allowed and a private address.
                "mixed.example.org" => vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                ],
                _ => return Err(std::io::Error::other("no such host")),
            })
        })
    }
}
fn test_net() -> previews::Net {
    previews::Net {
        resolver: std::sync::Arc::new(MapResolver),
        // The mock site lives on loopback; nothing else is allowed.
        allow_ip: |ip| ip == IpAddr::V4(Ipv4Addr::LOCALHOST),
        allow_name: previews::public_name,
    }
}

#[tokio::test]
async fn previews_are_fetched_safely_from_a_local_mock_site() {
    let (_dir, app) = world().await;
    let port = mock_site().await;
    let p = &app.0.content.previews;
    p.set_net(test_net());
    let url = |path: &str| format!("http://news.example.com:{port}{path}");
    let page = p
        .page(&url("/hop"))
        .await
        .expect("followed a checked redirect");
    assert_eq!(page.title.as_deref(), Some("Kite festival"));
    let image = page.image.as_ref().unwrap();
    assert!(image.width <= 480 && image.height <= 480 && image.bytes.len() <= 256 * 1024);
    assert_eq!(image.content_type, "image/jpeg");
    // Refused: a redirect to a private address, a private or mixed DNS answer,
    // IP literals, non-HTML, and other schemes.
    assert!(p.page(&url("/to-private")).await.is_none());
    assert!(p
        .page(&format!("http://internal.example.net:{port}/page"))
        .await
        .is_none());
    assert!(p
        .page(&format!("http://mixed.example.org:{port}/page"))
        .await
        .is_none());
    assert!(
        p.page(&format!("http://127.0.0.1:{port}/page"))
            .await
            .is_some(),
        "the test policy allows only the mock site"
    );
    assert!(p.page(&url("/text")).await.is_none());
    assert!(p
        .page(&format!("ftp://news.example.com:{port}/page"))
        .await
        .is_none());
    assert!(p
        .page(&format!("http://user:pw@news.example.com:{port}/page"))
        .await
        .is_none());
    // A big page is read only up to the cap; its head still works.
    assert_eq!(
        p.page(&url("/huge")).await.unwrap().title.as_deref(),
        Some("Big")
    );
    // The real policy never allows loopback, even if DNS says so.
    p.set_net(previews::Net {
        allow_ip: previews::public_ip,
        ..test_net()
    });
    assert!(p.page(&url("/page")).await.is_none());
    assert!(p
        .page(&format!("http://127.0.0.1:{port}/page"))
        .await
        .is_none());
}

#[tokio::test]
async fn sent_links_get_cards_that_authors_and_moderators_can_remove() {
    let (_dir, app) = world().await;
    let port = mock_site().await;
    app.0.content.previews.set_net(test_net());
    ok(content(&app, 4, 1, ContentCommand::Subscribe).await);
    let text = format!(
        "party! http://news.example.com:{port}/page and <http://news.example.com:{port}/hop>"
    );
    let m = message_of(ok(run(
        &app,
        2,
        2,
        Command::SendMessage(draft(11, &text, vec![])),
    )
    .await));
    let mut embeds = vec![];
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut c = app.0.pool.acquire().await.unwrap();
        embeds = db::message(&mut c, Id(4), Id(11), m.id, false)
            .await
            .unwrap()
            .extra
            .embeds;
        if !embeds.is_empty() {
            break;
        }
    }
    assert_eq!(embeds.len(), 1, "the <...> link has no preview");
    assert_eq!(embeds[0].site_name.as_deref(), Some("News"));
    let image = embeds[0].image.clone().unwrap();
    assert_eq!(
        image.url,
        format!("/api/v1/channels/11/messages/{}/embeds/0/image", m.id.0)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let (kind, bytes) = previews::image(&mut c, Id(4), Id(11), m.id, 0)
        .await
        .unwrap();
    assert!(kind == "image/jpeg" && !bytes.is_empty());
    assert_eq!(
        previews::image(&mut c, Id(3), Id(11), m.id, 0)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    drop(c);
    assert!(app.0.core.lock().await.streams[&vec![4u8; 32]]
        .ring
        .iter()
        .any(|(_, _, b)| matches!(rmp_serde::from_slice::<ServerMsg>(b).unwrap(), ServerMsg::Event(e) if matches!(e.event, Event::Content(ContentEvent::EmbedsUpdated { .. })))));
    let remove = ContentCommand::RemoveEmbeds {
        channel_id: Id(11),
        message_id: m.id,
    };
    assert_eq!(
        code(&content(&app, 4, 3, remove.clone()).await),
        Some(ErrorCode::Forbidden)
    );
    ok(content(&app, 1, 4, remove).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(db::message(&mut c, Id(4), Id(11), m.id, false)
        .await
        .unwrap()
        .extra
        .embeds
        .is_empty());
}

#[tokio::test]
async fn previews_can_be_turned_off_and_are_rate_limited() {
    let (_dir, app) = world().await;
    let p = &app.0.content.previews;
    let allowed = (0..8).filter(|_| p.allow(Id(9))).count();
    assert_eq!(allowed, 5);
    assert!(p.allow(Id(10)), "limits are per person");
    // The switch is a real server option.
    let options = <ContentOptions as clap::FromArgMatches>::from_arg_matches(
        &<ContentOptions as clap::Args>::augment_args(clap::Command::new("t"))
            .try_get_matches_from(["t", "--link-previews", "false"])
            .unwrap(),
    )
    .unwrap();
    assert!(!options.link_previews);
    assert!(!Content::new(&options).previews.enabled());
    // The test-only mock host needs a loopback-only server.
    let mock = ContentOptions {
        mock_link_preview_host: Some("preview.example.com".into()),
        ..Default::default()
    };
    assert!(Content::configured(&mock, "0.0.0.0:1".parse().unwrap()).is_err());
    assert!(Content::configured(&mock, "127.0.0.1:1".parse().unwrap()).is_ok());
}

#[tokio::test]
async fn gifs_from_the_picker_keep_their_id_and_only_provider_media() {
    let (_dir, app) = world().await;
    let gif = |id: &str, url: &str| ContentCommand::SendGif {
        channel_id: Id(11),
        gif_id: id.into(),
        url: url.into(),
    };
    for (n, bad) in [
        gif("klipy:1", "https://evil.example.com/a.gif"),
        gif("klipy:1", "http://static.klipy.com/a.gif"),
        gif("klipy:1", "https://static.klipy.com.evil.com/a.gif"),
        gif("klipy:1", "https://user@static.klipy.com/a.gif"),
        gif("tenor:1", "https://static.klipy.com/a.gif"),
        gif("klipy:../x", "https://static.klipy.com/a.gif"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            code(&content(&app, 2, 1 + n as u64, bad).await),
            Some(ErrorCode::InvalidRequest),
            "case {n}"
        );
    }
    assert!(code(
        &content(
            &app,
            3,
            20,
            gif("klipy:42", "https://static.klipy.com/a.gif")
        )
        .await
    )
    .is_some());
    let m = message_of(ok(content(
        &app,
        2,
        21,
        gif("klipy:42", "https://static2.klipy.com/ii/a/b.gif"),
    )
    .await));
    assert_eq!(
        (m.content.as_str(), m.extra.gif.as_deref()),
        ("https://static2.klipy.com/ii/a/b.gif", Some("klipy:42"))
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = db::message(&mut c, Id(1), Id(11), m.id, false)
        .await
        .unwrap();
    assert_eq!(seen.extra.gif.as_deref(), Some("klipy:42"));
}

/// Files on scheduled messages.
mod integration;
