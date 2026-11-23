//! Scheduled events: permissions, validation, recurrence edits, the timer,
//! Stage and voice sessions, sharing, invites and the REST reads.
use super::*;
use crate::{audit_tests::fixture, commands, state::Connection};
use base64::Engine;
use tower::ServiceExt;

const STAGE: Id = Id(30);
const VOICE: Id = Id(31);
const PRIVATE: Id = Id(32);
const DAY: u64 = 86_400_000;

/// Owner 1; members 2 (Create Events) and 3; user 4 outside. Stage 30,
/// voice 31, and voice 32 that member 3 cannot see.
async fn world(app: &App) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO guild_members VALUES(10,3,'member',0);\
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(30,'stage',10,'town-hall',1,0),(31,'voice',10,'lounge',2,0),(32,'voice',10,'staff',3,0);\
         INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'hosts',2,{});\
         INSERT INTO member_roles VALUES(10,2,20);\
         INSERT INTO permission_overwrites VALUES(32,'member',3,0,1);         INSERT INTO read_states(user_id,channel_id) VALUES(3,11);",
        Permissions::CREATE_EVENTS.bits()
    )))
    .execute(&app.0.pool)
    .await
    .unwrap();
}
async fn cmd(
    app: &App,
    u: u64,
    command: ScheduleCommand,
) -> std::result::Result<ScheduleResult, ErrorCode> {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    let r = commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest {
            nonce: crate::state::random(4)
                .iter()
                .fold(1u64, |a, b| a * 7 + u64::from(*b)),
            command: Command::Schedule(command),
        },
    )
    .await
    .unwrap();
    match r.outcome {
        CommandOutcome::Ok(CommandResult::Schedule(s)) => Ok(s),
        CommandOutcome::Ok(other) => panic!("{other:?}"),
        CommandOutcome::Err(e) => Err(e.code),
    }
}
fn event_of(r: std::result::Result<ScheduleResult, ErrorCode>) -> ScheduledEvent {
    match r {
        Ok(ScheduleResult::Event(e)) => e,
        other => panic!("{other:?}"),
    }
}
fn external(start: u64, recurrence: EventRecurrence) -> NewEvent {
    NewEvent {
        guild_id: Id(10),
        name: "Book club".into(),
        description: "Chapter 3".into(),
        start_at: start,
        end_at: Some(start + 3_600_000),
        location: EventLocation::External {
            place: "Library".into(),
        },
        recurrence,
        time_zone: "Europe/Berlin".into(),
    }
}
async fn row_count(app: &App, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&app.0.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn creating_needs_create_or_manage_events_and_valid_input() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let soon = now() + DAY;
    assert_eq!(
        cmd(
            &app,
            3,
            ScheduleCommand::Create(external(soon, EventRecurrence::None))
        )
        .await,
        Err(ErrorCode::Forbidden)
    );
    assert_eq!(
        cmd(
            &app,
            4,
            ScheduleCommand::Create(external(soon, EventRecurrence::None))
        )
        .await,
        Err(ErrorCode::NotFound)
    );
    let mine = event_of(
        cmd(
            &app,
            2,
            ScheduleCommand::Create(external(soon, EventRecurrence::None)),
        )
        .await,
    );
    assert_eq!(mine.status, EventStatus::Scheduled);
    assert_eq!(mine.start_at, soon);
    // Invalid input.
    let mut bad = external(soon, EventRecurrence::None);
    bad.name = "  ".into();
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(now() - DAY, EventRecurrence::None);
    bad.end_at = Some(now() - DAY + 1000);
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(soon, EventRecurrence::None);
    bad.end_at = None;
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(soon, EventRecurrence::None);
    bad.end_at = Some(soon - 1);
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(soon, EventRecurrence::None);
    bad.time_zone = "Nowhere/Land".into();
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(soon, EventRecurrence::None);
    bad.location = EventLocation::Stage { channel_id: VOICE };
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    let mut bad = external(soon, EventRecurrence::None);
    bad.location = EventLocation::Voice { channel_id: Id(11) };
    assert_eq!(
        cmd(&app, 1, ScheduleCommand::Create(bad)).await,
        Err(ErrorCode::InvalidRequest)
    );
    // Create Events manages only your own; Manage Events (the owner) manages all.
    let theirs = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(soon, EventRecurrence::None)),
        )
        .await,
    );
    let rename = |e: Id| ScheduleCommand::Update {
        event_id: e,
        occurrence: 0,
        scope: EditScope::This,
        patch: EventPatch {
            name: Some("Renamed".into()),
            ..Default::default()
        },
    };
    let fix = |c: ScheduleCommand, occ: u64| match c {
        ScheduleCommand::Update {
            event_id,
            scope,
            patch,
            ..
        } => ScheduleCommand::Update {
            event_id,
            occurrence: occ,
            scope,
            patch,
        },
        c => c,
    };
    assert_eq!(
        cmd(&app, 2, fix(rename(theirs.id), theirs.occurrence)).await,
        Err(ErrorCode::Forbidden)
    );
    assert_eq!(
        event_of(cmd(&app, 2, fix(rename(mine.id), mine.occurrence)).await).name,
        "Renamed"
    );
    assert_eq!(
        event_of(cmd(&app, 1, fix(rename(mine.id), mine.occurrence)).await).name,
        "Renamed"
    );
    assert_eq!(
        cmd(&app, 3, ScheduleCommand::Delete { event_id: mine.id }).await,
        Err(ErrorCode::Forbidden)
    );
    assert_eq!(
        cmd(&app, 2, ScheduleCommand::Delete { event_id: mine.id }).await,
        Ok(ScheduleResult::Ack)
    );
    // Audited: create, create, two updates, delete.
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE guild_id=10 ORDER BY id")
            .fetch_all(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(
        actions,
        vec![
            "event_create",
            "event_create",
            "event_update",
            "event_update",
            "event_delete"
        ]
    );
}

#[tokio::test]
async fn events_in_a_channel_are_only_for_people_who_can_see_it() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut e = external(now() + DAY, EventRecurrence::None);
    e.location = EventLocation::Voice {
        channel_id: PRIVATE,
    };
    e.end_at = None;
    let hidden = event_of(cmd(&app, 1, ScheduleCommand::Create(e)).await);
    assert_eq!(
        cmd(
            &app,
            3,
            ScheduleCommand::SetInterested {
                event_id: hidden.id,
                interested: true
            }
        )
        .await,
        Err(ErrorCode::NotFound)
    );
    assert_eq!(
        cmd(
            &app,
            2,
            ScheduleCommand::SetInterested {
                event_id: hidden.id,
                interested: true
            }
        )
        .await,
        Ok(ScheduleResult::Ack)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(summary(&mut c, Id(3), Id(10)).await.unwrap().count, 0);
    assert_eq!(summary(&mut c, Id(2), Id(10)).await.unwrap().count, 1);
    // Member 3's subscription knows nothing of it.
    let Ok(ScheduleResult::Summary { guilds }) = cmd(&app, 3, ScheduleCommand::Subscribe).await
    else {
        panic!()
    };
    assert!(guilds.is_empty());
    assert_eq!(
        row_count(&app, "SELECT interested_count FROM scheduled_events").await,
        1
    );
}

#[tokio::test]
async fn recurring_edits_this_or_following_and_cancellations() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let start = now() + DAY;
    let weekly = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(start, EventRecurrence::Weekly)),
        )
        .await,
    );
    cmd(
        &app,
        2,
        ScheduleCommand::SetInterested {
            event_id: weekly.id,
            interested: true,
        },
    )
    .await
    .unwrap();
    let s = load(&mut app.0.pool.acquire().await.unwrap(), weekly.id)
        .await
        .unwrap();
    let second = s.rule.nth(1).unwrap();
    let third = s.rule.nth(2).unwrap();
    // "This event": only the second occurrence is renamed.
    let edited = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Update {
                event_id: weekly.id,
                occurrence: second,
                scope: EditScope::This,
                patch: EventPatch {
                    name: Some("Special".into()),
                    ..Default::default()
                },
            },
        )
        .await,
    );
    assert_eq!(
        (edited.name.as_str(), edited.occurrence),
        ("Special", second)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let s = load(&mut c, weekly.id).await.unwrap();
    assert_eq!(s.occurrence(&mut c, third).await.unwrap().name, "Book club");
    // Cancelling one occurrence.
    assert_eq!(
        cmd(
            &app,
            1,
            ScheduleCommand::Cancel {
                event_id: weekly.id,
                occurrence: second,
                scope: EditScope::This
            }
        )
        .await,
        Ok(ScheduleResult::Ack)
    );
    assert!(s.occurrence(&mut c, second).await.unwrap().cancelled);
    // Not an occurrence of the rule.
    assert_eq!(
        cmd(
            &app,
            1,
            ScheduleCommand::Cancel {
                event_id: weekly.id,
                occurrence: second + 1,
                scope: EditScope::This
            }
        )
        .await,
        Err(ErrorCode::NotFound)
    );
    // "This and following": a new series from the third, interest carried over.
    let split = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Update {
                event_id: weekly.id,
                occurrence: third,
                scope: EditScope::Following,
                patch: EventPatch {
                    name: Some("New season".into()),
                    recurrence: Some(EventRecurrence::EveryTwoWeeks),
                    ..Default::default()
                },
            },
        )
        .await,
    );
    assert_ne!(split.id, weekly.id);
    assert_eq!(split.recurrence, EventRecurrence::EveryTwoWeeks);
    assert_eq!(split.start_at, third);
    let old = load(&mut c, weekly.id).await.unwrap();
    assert_eq!(old.rule.until, Some(third));
    assert!(old.rule.nth(2).is_none());
    assert_eq!(
        row_count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM event_interest WHERE event_id={}",
                split.id.0
            )
        )
        .await,
        1
    );
    // The calendar file carries the rule, the zone and the cancelled date.
    let ics = routes::ics(&mut c, &old, "https://chat.example")
        .await
        .unwrap();
    assert!(ics.contains("RRULE:FREQ=WEEKLY;UNTIL="));
    assert!(ics.contains("DTSTART;TZID=Europe/Berlin:"));
    assert!(ics.contains("EXDATE:"));
    assert!(ics.contains("SUMMARY:Book club"));
    assert!(ics.contains("URL:https://chat.example/?event="));
    assert!(ics.lines().all(|l| l.len() <= 76));
}

#[tokio::test]
async fn the_timer_announces_starts_runs_external_events_and_cleans_up() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let start = now() + 300;
    let e = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(start, EventRecurrence::None)),
        )
        .await,
    );
    cmd(
        &app,
        2,
        ScheduleCommand::SetInterested {
            event_id: e.id,
            interested: true,
        },
    )
    .await
    .unwrap();
    let mut rx = {
        let mut core = app.0.core.lock().await;
        let key = vec![2u8; 32];
        core.cursor(&key, Id(2));
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let (close, _) = tokio::sync::watch::channel(None);
        core.streams.get_mut(&key).unwrap().connection = Some(Connection {
            focused: true,
            id: "ws2".into(),
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
        core.streams.get_mut(&key).unwrap().schedule_features = true;
        rx
    };
    // The deadline is the start, armed once.
    let next: i64 = row_count(&app, "SELECT MIN(next_at) FROM scheduled_events").await;
    assert_eq!(next as u64, start);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    tick(&app).await.unwrap();
    let s = load(&mut app.0.pool.acquire().await.unwrap(), e.id)
        .await
        .unwrap();
    assert_eq!(s.status, EventStatus::Live);
    let mut starting = false;
    while let Ok(p) = rx.try_recv() {
        if let ServerMsg::Event(ev) = rmp_serde::from_slice::<ServerMsg>(&p.bytes).unwrap() {
            starting |= matches!(ev.event, Event::Schedule(ScheduleUpdate::Starting(ref x)) if x.id == e.id);
        }
    }
    assert!(starting, "interested members are told it is starting");
    // Its end passes: a one-off event is over.
    sqlx::query("UPDATE scheduled_events SET duration_ms=1,next_at=? WHERE id=?")
        .bind(now() as i64)
        .bind(e.id.0 as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    tick(&app).await.unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    let s = load(&mut c, e.id).await.unwrap();
    assert_eq!(s.status, EventStatus::Ended);
    // Deleted a bounded time after it ended.
    let at: i64 = row_count(&app, "SELECT next_at FROM scheduled_events").await;
    assert!(at as u64 >= now() + EVENT_RETENTION_MS - 60_000);
    sqlx::query("UPDATE scheduled_events SET next_at=?")
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    tick(&app).await.unwrap();
    assert_eq!(
        row_count(&app, "SELECT COUNT(*) FROM scheduled_events").await,
        0
    );
}

#[tokio::test]
async fn stage_and_voice_sessions_take_their_events_live_and_end_them() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let mut e = external(now() + 60_000, EventRecurrence::None);
    e.location = EventLocation::Stage { channel_id: STAGE };
    e.end_at = None;
    let staged = event_of(cmd(&app, 1, ScheduleCommand::Create(e)).await);
    let mut e = external(now() + 60_000, EventRecurrence::None);
    e.location = EventLocation::Voice { channel_id: VOICE };
    e.end_at = None;
    let voiced = event_of(cmd(&app, 1, ScheduleCommand::Create(e)).await);
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    // The Stage starts (within half an hour of the event): the event is its session.
    let live = stage_starting(&mut core, &mut c, Id(10), STAGE, None, Id(1))
        .await
        .unwrap();
    assert_eq!(live, Some(staged.id));
    assert_eq!(
        load(&mut c, staged.id).await.unwrap().status,
        EventStatus::Live
    );
    stage_ended(&mut core, &mut c, STAGE, live).await.unwrap();
    assert_eq!(
        load(&mut c, staged.id).await.unwrap().status,
        EventStatus::Ended
    );
    // Someone joins the voice channel: live. It empties with no end time: over.
    core.voice.clear();
    voice_joined(&mut core, &mut c, VOICE).await.unwrap();
    assert_eq!(
        load(&mut c, voiced.id).await.unwrap().status,
        EventStatus::Live
    );
    voice_emptied(&mut core, &mut c, VOICE).await.unwrap();
    assert_eq!(
        load(&mut c, voiced.id).await.unwrap().status,
        EventStatus::Ended
    );
    drop(core);
    // A manager can start and end by hand too; others cannot.
    let mut e = external(now() + DAY, EventRecurrence::Daily);
    e.location = EventLocation::Voice { channel_id: VOICE };
    let daily = event_of(cmd(&app, 1, ScheduleCommand::Create(e)).await);
    assert_eq!(
        cmd(&app, 3, ScheduleCommand::Start { event_id: daily.id }).await,
        Err(ErrorCode::Forbidden)
    );
    let started = event_of(cmd(&app, 1, ScheduleCommand::Start { event_id: daily.id }).await);
    assert_eq!(started.status, EventStatus::Live);
    let ended = event_of(cmd(&app, 1, ScheduleCommand::End { event_id: daily.id }).await);
    // A daily event moves on to tomorrow's occurrence.
    assert_eq!(ended.status, EventStatus::Scheduled);
    assert!(ended.occurrence > started.occurrence);
}

#[tokio::test]
async fn sharing_posts_a_card_and_event_invites_mark_new_members_interested() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let e = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(now() + DAY, EventRecurrence::None)),
        )
        .await,
    );
    let Ok(ScheduleResult::Message(m)) = cmd(
        &app,
        3,
        ScheduleCommand::Share {
            event_id: e.id,
            channel_id: Id(11),
        },
    )
    .await
    else {
        panic!("shared")
    };
    let card = m.extra.stage.and_then(|s| s.event).expect("the card");
    assert_eq!((card.event_id, card.name.as_str()), (e.id, "Book club"));
    let Ok(ScheduleResult::Invite(invite)) = cmd(
        &app,
        1,
        ScheduleCommand::CreateInvite {
            event_id: e.id,
            ttl_seconds: 3600,
            max_uses: 5,
        },
    )
    .await
    else {
        panic!("invite")
    };
    // The invite's event preview, then joining through it.
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([104u8; 32]);
    sqlx::query("INSERT INTO sessions VALUES(?,4,?,?,?)")
        .bind(crate::state::hash(token.as_bytes()))
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let r = crate::http::router(app.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/api/v1/invites/{}/event", invite.code))
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(r.status().is_success());
    let preview: InviteEvent =
        serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap())
            .unwrap();
    assert_eq!(preview.event.event_id, e.id);
    // The Join dialog's preview says there is a card, so the client asks for
    // it; a plain invite says nothing and the client asks for nothing.
    let preview_of = |code: String| {
        let app = app.clone();
        let token = token.clone();
        async move {
            let r = crate::http::router(app)
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/api/v1/invites/{code}/preview"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(r.status().is_success());
            let v: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap(),
            )
            .unwrap();
            v["data"].clone()
        }
    };
    assert_eq!(preview_of(invite.code.clone()).await["event"], true);
    let plain = {
        let mut core = app.0.core.lock().await;
        let r = commands::execute(
            &app,
            &mut core,
            &[1u8; 32],
            Id(1),
            CommandRequest {
                nonce: 98,
                command: Command::CreateInvite {
                    guild_id: Id(10),
                    ttl_seconds: 3600,
                    max_uses: 5,
                },
            },
        )
        .await
        .unwrap();
        let CommandOutcome::Ok(CommandResult::Invite(i)) = r.outcome else {
            panic!("{:?}", r.outcome)
        };
        i.code
    };
    let plain = preview_of(plain).await;
    assert_eq!(plain["name"].is_string(), true);
    assert!(plain.get("event").is_none(), "{plain}");
    {
        let mut core = app.0.core.lock().await;
        let r = commands::execute(
            &app,
            &mut core,
            &[4u8; 32],
            Id(4),
            CommandRequest {
                nonce: 99,
                command: Command::JoinGuild {
                    invite_code: invite.code.clone(),
                },
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(r.outcome, CommandOutcome::Ok(_)),
            "{:?}",
            r.outcome
        );
    }
    assert_eq!(
        row_count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM event_interest WHERE event_id={} AND user_id=4",
                e.id.0
            )
        )
        .await,
        1
    );
    // The REST reads: the list, the detail and the interested members.
    let get = |path: String| {
        let app = app.clone();
        let token = token.clone();
        async move {
            let r = crate::http::router(app)
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .header("authorization", format!("Bearer {token}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            (
                r.status(),
                axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap(),
            )
        }
    };
    let (status, body) = get("/api/v1/guilds/10/events".into()).await;
    assert!(status.is_success());
    let page: EventPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.interested, vec![e.id]);
    assert!(!page.can_create);
    let (_, body) = get(format!("/api/v1/events/{}", e.id.0)).await;
    let detail: EventDetail = serde_json::from_slice(&body).unwrap();
    assert!(detail.me_interested && !detail.can_manage);
    let (_, body) = get(format!("/api/v1/events/{}/interested", e.id.0)).await;
    let people: InterestedPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(people.users.len(), 1);
    let (status, body) = get(format!("/api/v1/events/{}/calendar.ics", e.id.0)).await;
    assert!(status.is_success() && body.starts_with(b"BEGIN:VCALENDAR"));
}

#[tokio::test]
async fn cover_images_need_the_manage_right_and_a_real_picture() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let e = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(now() + DAY, EventRecurrence::None)),
        )
        .await,
    );
    let token = |u: u8| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u + 100; 32]);
    for u in [1u8, 3] {
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(crate::state::hash(token(u).as_bytes()))
            .bind(i64::from(u))
            .bind(now() as i64)
            .bind((now() + 600_000) as i64)
            .bind(now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    let call = |u: u8, method: &'static str, body: Vec<u8>| {
        let app = app.clone();
        let path = format!("/api/v1/events/{}/cover", e.id.0);
        async move {
            crate::http::router(app)
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {}", token(u)))
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(64, 36)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    assert_eq!(
        call(3, "PUT", png.clone()).await,
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(1, "PUT", b"not a picture".to_vec()).await,
        axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(call(1, "PUT", png).await, axum::http::StatusCode::OK);
    assert_eq!(call(3, "GET", vec![]).await, axum::http::StatusCode::OK);
    assert_eq!(
        row_count(&app, "SELECT cover_version FROM scheduled_events").await,
        1
    );
    assert_eq!(
        call(3, "DELETE", vec![]).await,
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(call(1, "DELETE", vec![]).await, axum::http::StatusCode::OK);
    assert_eq!(
        call(3, "GET", vec![]).await,
        axum::http::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn start_pushes_respect_do_not_disturb() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let e = event_of(
        cmd(
            &app,
            1,
            ScheduleCommand::Create(external(now() + DAY, EventRecurrence::None)),
        )
        .await,
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(push_allowed(&mut c, Id(2), Id(10), e.id).await.unwrap());
    sqlx::query("UPDATE users SET desired_presence='dnd' WHERE id=2")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(!push_allowed(&mut c, Id(2), Id(10), e.id).await.unwrap());
}
