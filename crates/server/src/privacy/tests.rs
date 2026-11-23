//! Privacy: every command, route, permission check and limit, and the
//! notification decisions that badges, the Inbox and push share.
use super::*;
use crate::{audit_tests::fixture, commands, state};
use axum::http::StatusCode;
use sqlx::Row;
use tower::ServiceExt;

async fn run(app: &App, u: u64, command: Command) -> CommandOutcome {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
/// Like `run`, without clearing the rate limiters.
async fn run_limited(app: &App, u: u64, command: Command) -> CommandOutcome {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000_000);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut core = app.0.core.lock().await;
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
fn p(cmd: PrivacyCommand) -> Command {
    Command::Privacy(cmd)
}
fn ok(o: &CommandOutcome) -> CommandResult {
    match o {
        CommandOutcome::Ok(r) => r.clone(),
        o => panic!("expected success, got {o:?}"),
    }
}
fn err(o: &CommandOutcome) -> ApiError {
    match o {
        CommandOutcome::Err(e) => e.clone(),
        o => panic!("expected an error, got {o:?}"),
    }
}
async fn sql(app: &App, q: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(q.to_owned()))
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn scalar(app: &App, q: &'static str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}
fn send(ch: u64, content: &str, mentions: Vec<u64>) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: Id(ch),
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: mentions.into_iter().map(Id).collect(),
    })
}
fn message_id(r: &CommandResult) -> Id {
    match r {
        CommandResult::Message(m) => m.id,
        r => panic!("not a message: {r:?}"),
    }
}
/// Events a user's command stream received (its replay ring).
async fn events(app: &App, u: u64) -> Vec<Event> {
    let core = app.0.core.lock().await;
    core.streams.get(&vec![u as u8; 32]).map_or(vec![], |s| {
        s.ring
            .iter()
            .filter_map(
                |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).ok()? {
                    ServerMsg::Event(e) => Some(e.event),
                    _ => None,
                },
            )
            .collect()
    })
}
async fn clear_events(app: &App) {
    let mut core = app.0.core.lock().await;
    for s in core.streams.values_mut() {
        s.clear_replay();
    }
}
async fn subscribe(app: &App, u: u64) {
    ok(&run(app, u, p(PrivacyCommand::Subscribe)).await);
}
async fn read_state(app: &App, u: u64, ch: u64) -> (i64, i64) {
    let r = sqlx::query(
        "SELECT unread_count,mention_count FROM read_states WHERE user_id=? AND channel_id=?",
    )
    .bind(u as i64)
    .bind(ch as i64)
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    (r.get(0), r.get(1))
}
async fn token(app: &App, u: i64) -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u as u8 + 100; 32]);
    sqlx::query("INSERT OR IGNORE INTO sessions VALUES(?,?,?,?,?)")
        .bind(state::hash(token.as_bytes()))
        .bind(u)
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
async fn call(
    app: &App,
    u: i64,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let token = token(app, u).await;
    app.0.core.lock().await.limits.clear();
    let mut r = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        r = r.header("content-type", "application/json");
    }
    let r = r
        .body(axum::body::Body::from(
            body.map_or(vec![], |b| serde_json::to_vec(&b).unwrap()),
        ))
        .unwrap();
    let response = crate::http::router(app.clone()).oneshot(r).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
async fn settings_of(app: &App, u: u64) -> PrivacySettings {
    let core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    settings(&mut c, &core, Id(u)).await.unwrap()
}
async fn decide_for(app: &App, u: u64, ch: u64, m: Id) -> Option<notify::Decision> {
    let mut c = app.0.pool.acquire().await.unwrap();
    let channel = crate::db::channel(&mut c, Id(ch)).await.unwrap();
    let message = crate::db::message(&mut c, Id(u), Id(ch), m, false)
        .await
        .unwrap();
    notify::decide(&mut c, Id(u), &channel, &message)
        .await
        .unwrap()
}
async fn tick_now(app: &App) {
    tick(app).await.unwrap();
}
fn friends_sql(a: u64, b: u64) -> String {
    format!("INSERT INTO relationships(user_id,other_id,kind,since) VALUES({a},{b},'friend',0),({b},{a},'friend',0)")
}
fn note_settings(
    guild: Option<u64>,
    channel: Option<u64>,
    level: NotificationLevel,
) -> NotificationSettings {
    NotificationSettings {
        guild_id: guild.map(Id),
        channel_id: channel.map(Id),
        level,
        muted: false,
        mute_until: None,
        suppress_everyone: false,
        hide_muted_channels: false,
    }
}
fn set_notifications(s: NotificationSettings, suppress_roles: bool, mobile_push: bool) -> Command {
    p(PrivacyCommand::SetNotifications(NotificationOptions {
        settings: s,
        suppress_roles,
        mobile_push,
    }))
}

// ---- Status ----

#[tokio::test]
async fn status_choice_is_validated_and_a_timed_status_returns_to_online() {
    let (_dir, app) = fixture().await;
    let set = |status, until| p(PrivacyCommand::SetStatus { status, until });
    assert_eq!(
        err(&run(&app, 1, set(PresenceStatus::Offline, None)).await).code,
        ErrorCode::InvalidRequest
    );
    // Online has no end; an end must be ahead and within a day.
    for (status, until) in [
        (PresenceStatus::Online, Some(now() + 60_000)),
        (PresenceStatus::Dnd, Some(now().saturating_sub(1))),
        (PresenceStatus::Dnd, Some(now() + 2 * 86_400_000)),
    ] {
        assert_eq!(
            err(&run(&app, 1, set(status, until)).await).code,
            ErrorCode::InvalidRequest
        );
    }
    ok(&run(&app, 1, set(PresenceStatus::Dnd, Some(now() + 3_600_000))).await);
    let s = settings_of(&app, 1).await;
    assert_eq!(s.status.status, PresenceStatus::Dnd);
    assert!(s.status.until.is_some());
    // The deadline passes: the timer returns it to Online.
    sql(&app, "UPDATE users SET presence_until=1 WHERE id=1").await;
    tick_now(&app).await;
    let s = settings_of(&app, 1).await;
    assert_eq!(s.status.status, PresenceStatus::Online);
    assert_eq!(s.status.until, None);
    assert!(events(&app, 1).await.iter().any(|e| matches!(e,
        Event::SelfUpdated(me) if me.desired_presence == PresenceStatus::Online)));
    // "Until I change it", and the older SetPresence also clears a deadline.
    ok(&run(&app, 1, set(PresenceStatus::Idle, Some(now() + 60_000))).await);
    ok(&run(
        &app,
        1,
        Command::SetPresence {
            status: PresenceStatus::Invisible,
        },
    )
    .await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM users WHERE id=1 AND presence_until IS NULL AND desired_presence='invisible'").await,
        1
    );
}

#[tokio::test]
async fn automatic_idle_follows_window_focus_and_never_overrides_a_choice() {
    let (_dir, app) = fixture().await;
    sql(&app, &friends_sql(1, 2)).await;
    let attach = |core: &mut Core, u: u64| {
        let key = vec![u as u8; 32];
        core.cursor(&key, Id(u));
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let (close, _) = tokio::sync::watch::channel(None);
        core.streams.get_mut(&key).unwrap().connection = Some(crate::state::Connection {
            focused: false,
            id: format!("ws{u}"),
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(1 << 22)),
            close,
        });
        rx
    };
    let mut core = app.0.core.lock().await;
    let _rx1 = attach(&mut core, 1);
    let _rx2 = attach(&mut core, 2);
    status::connected(&mut core, Id(1));
    assert!(core.privacy.unfocused.contains_key(&Id(1)));
    // Ten minutes without a focused window.
    core.privacy
        .unfocused
        .insert(Id(1), now() - AUTO_IDLE_MS - 1);
    let mut changed = vec![];
    status::expire_idle(&mut core, &mut changed);
    assert_eq!(changed, vec![Id(1)]);
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = crate::db::presence(&mut c, &core, Id(2), Id(1))
        .await
        .unwrap();
    assert_eq!(seen.status, PresenceStatus::Idle);
    // Focusing a window ends it.
    core.streams
        .get_mut(&vec![1; 32])
        .unwrap()
        .connection
        .as_mut()
        .unwrap()
        .focused = true;
    status::focus(&app, &mut core, Id(1)).await;
    assert!(!core.privacy.idle.contains(&Id(1)));
    let seen = crate::db::presence(&mut c, &core, Id(2), Id(1))
        .await
        .unwrap();
    assert_eq!(seen.status, PresenceStatus::Online);
    // A manual Do Not Disturb is never turned into Idle.
    sqlx::query("UPDATE users SET desired_presence='dnd' WHERE id=1")
        .execute(&mut *c)
        .await
        .unwrap();
    core.privacy.idle.insert(Id(1));
    let seen = crate::db::presence(&mut c, &core, Id(2), Id(1))
        .await
        .unwrap();
    assert_eq!(seen.status, PresenceStatus::Dnd);
    // Invisible looks offline to others but not to yourself.
    sqlx::query("UPDATE users SET desired_presence='invisible' WHERE id=1")
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        crate::db::presence(&mut c, &core, Id(2), Id(1))
            .await
            .unwrap()
            .status,
        PresenceStatus::Offline
    );
    assert_eq!(
        crate::db::presence(&mut c, &core, Id(1), Id(1))
            .await
            .unwrap()
            .status,
        PresenceStatus::Invisible
    );
}

#[tokio::test]
async fn custom_status_validates_emoji_and_clears_itself() {
    let (_dir, app) = fixture().await;
    let custom = |text: Option<&str>, emoji: Option<&str>, clear_at| {
        p(PrivacyCommand::SetCustomStatus(CustomStatus {
            text: text.map(Into::into),
            emoji: emoji.map(Into::into),
            clear_at,
        }))
    };
    for e in ["a", "ab", "🎉🎉", "<:x:1>", "<:party:12a>", "🎉 "] {
        if e.trim() == "🎉" {
            continue;
        }
        assert_eq!(
            err(&run(&app, 1, custom(Some("hi"), Some(e), None)).await).code,
            ErrorCode::InvalidRequest,
            "{e}"
        );
    }
    assert_eq!(
        err(&run(&app, 1, custom(Some(&"x".repeat(129)), None, None)).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(
            &app,
            1,
            custom(Some("hi"), None, Some(now() + 400 * 86_400_000))
        )
        .await)
        .code,
        ErrorCode::InvalidRequest
    );
    ok(&run(&app, 1, custom(Some("custom"), Some("<:party:123>"), None)).await);
    ok(&run(
        &app,
        1,
        custom(Some("Out to lunch"), Some("🍜"), Some(now() + 3_600_000)),
    )
    .await);
    // Friends see the emoji with the text.
    sql(&app, &friends_sql(1, 2)).await;
    let _ = run(&app, 2, p(PrivacyCommand::Subscribe)).await;
    clear_events(&app).await;
    ok(&run(
        &app,
        1,
        custom(Some("Back soon"), Some("☕"), Some(now() + 3_600_000)),
    )
    .await);
    assert!(events(&app, 2).await.iter().any(|e| matches!(e,
        Event::UserUpdated(u) if u.status_emoji.as_deref() == Some("☕") && u.status_text.as_deref() == Some("Back soon"))));
    sql(&app, "UPDATE users SET status_clear_at=1 WHERE id=1").await;
    tick_now(&app).await;
    let s = settings_of(&app, 1).await;
    assert_eq!(s.custom, CustomStatus::default());
    let mut c = app.0.pool.acquire().await.unwrap();
    let u = crate::db::user(&mut c, Id(1)).await.unwrap();
    assert_eq!((u.status_text, u.status_emoji), (None, None));
}

// ---- Friend nicknames, notes, ignore ----

#[tokio::test]
async fn friend_nicknames_and_notes_are_private_and_bounded() {
    let (_dir, app) = fixture().await;
    let nick = |v, n: Option<&str>| {
        p(PrivacyCommand::SetFriendNickname {
            user_id: Id(v),
            nickname: n.map(Into::into),
        })
    };
    // Friends only.
    assert_eq!(
        err(&run(&app, 1, nick(2, Some("Sis"))).await).code,
        ErrorCode::Forbidden
    );
    sql(&app, &friends_sql(1, 2)).await;
    assert_eq!(
        err(&run(&app, 1, nick(2, Some(&"n".repeat(33)))).await).code,
        ErrorCode::InvalidRequest
    );
    ok(&run(&app, 1, nick(2, Some("  Sis  "))).await);
    assert_eq!(
        settings_of(&app, 1).await.nicknames,
        vec![FriendNickname {
            user_id: Id(2),
            nickname: "Sis".into()
        }]
    );
    // Never visible to the other person.
    assert!(settings_of(&app, 2).await.nicknames.is_empty());
    let (_, me2) = call(&app, 2, "GET", "/api/v1/users/@me/privacy", None).await;
    assert!(!me2.to_string().contains("Sis"));
    // A former friend's nickname is not shown.
    sql(&app, "DELETE FROM relationships").await;
    assert!(settings_of(&app, 1).await.nicknames.is_empty());

    // Notes: anyone whose profile you can see; private to you.
    let note = |v, n: Option<&str>| {
        p(PrivacyCommand::SetNote {
            user_id: Id(v),
            note: n.map(Into::into),
        })
    };
    assert_eq!(
        err(&run(&app, 1, note(3, Some("who?"))).await).code,
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&run(&app, 1, note(2, Some(&"x".repeat(257)))).await).code,
        ErrorCode::InvalidRequest
    );
    ok(&run(&app, 1, note(2, Some("Met at the\nmeetup"))).await);
    let (s, v) = call(&app, 1, "GET", "/api/v1/users/2/note", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["data"]["note"], "Met at the\nmeetup");
    // User 2 reading "their note about user 1" sees their own (empty) one.
    let (_, v) = call(&app, 2, "GET", "/api/v1/users/1/note", None).await;
    assert_eq!(v["data"]["note"], "");
    // At most MAX_NOTES.
    sql(&app, "WITH RECURSIVE s(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM s WHERE value<500) INSERT INTO users(id,username,password_hash,desired_presence,created_at) SELECT 1000+value,'n'||value,'','online',0 FROM s").await;
    sql(&app, "WITH RECURSIVE s(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM s WHERE value<499) INSERT INTO user_notes SELECT 1,1000+value,'x',0 FROM s").await;
    ok(&run(&app, 1, note(2, Some("still fits"))).await);
    sql(&app, "INSERT INTO user_notes VALUES(1,1500,'x',0)").await;
    sql(&app, "INSERT INTO guild_members VALUES(10,1499,'member',0)").await;
    assert_eq!(
        err(&run(&app, 1, note(1499, Some("full"))).await).code,
        ErrorCode::LimitExceeded
    );
}

#[tokio::test]
async fn ignoring_hides_badges_alerts_and_typing_without_telling_them() {
    let (_dir, app) = fixture().await;
    subscribe(&app, 1).await;
    subscribe(&app, 2).await;
    ok(&run(&app, 1, p(PrivacyCommand::Ignore { user_id: Id(2) })).await);
    assert_eq!(settings_of(&app, 1).await.ignored[0].id, Id(2));
    // Nothing reached the ignored person.
    assert!(!events(&app, 2)
        .await
        .iter()
        .any(|e| matches!(e, Event::Privacy(_))));
    let m = message_id(&ok(&run(&app, 2, send(11, "hello <@1>", vec![1])).await));
    assert_eq!(read_state(&app, 1, 11).await, (0, 0));
    assert_eq!(decide_for(&app, 1, 11, m).await, None);
    assert!(!events(&app, 1)
        .await
        .iter()
        .any(|e| matches!(e, Event::Privacy(PrivacyEvent::Alert(_)))));
    clear_events(&app).await;
    ok(&run(&app, 2, Command::Typing { channel_id: Id(11) }).await);
    assert!(!events(&app, 1)
        .await
        .iter()
        .any(|e| matches!(e, Event::Typing { .. })));
    // Mentions from them stay out of the Inbox.
    let (_, v) = call(&app, 1, "GET", "/api/v1/users/@me/mentions", None).await;
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 0);
    // Reversible: the counts come back.
    ok(&run(&app, 1, p(PrivacyCommand::Unignore { user_id: Id(2) })).await);
    assert_eq!(read_state(&app, 1, 11).await.0, 1);
    assert!(settings_of(&app, 1).await.ignored.is_empty());
    // Limits.
    assert_eq!(
        err(&run(&app, 1, p(PrivacyCommand::Ignore { user_id: Id(1) })).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 1, p(PrivacyCommand::Ignore { user_id: Id(3) })).await).code,
        ErrorCode::NotFound
    );
}

// ---- Notifications ----

#[tokio::test]
async fn saving_notifications_keeps_the_mute_deadline_and_hiding_and_mutes_end_by_themselves() {
    let (_dir, app) = fixture().await;
    let mut s = note_settings(Some(10), None, NotificationLevel::All);
    s.muted = true;
    s.mute_until = Some(now() + 900_000);
    s.hide_muted_channels = true;
    ok(&run(&app, 1, set_notifications(s.clone(), true, false)).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let saved = crate::launch::notifications(&mut c, Id(1)).await.unwrap();
    assert_eq!(saved, vec![s.clone()]);
    let extras = settings_of(&app, 1).await.notification_extras;
    assert_eq!(
        extras,
        vec![NotificationExtra {
            guild_id: Some(Id(10)),
            channel_id: None,
            suppress_roles: true,
            mobile_push: false
        }]
    );
    // Validation: scope, membership, a deadline within a year.
    let mut bad = s.clone();
    bad.mute_until = Some(now() + 400 * 86_400_000);
    assert_eq!(
        err(&run(&app, 1, set_notifications(bad, false, true)).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 3, set_notifications(s.clone(), false, true)).await).code,
        ErrorCode::NotFound
    );
    // The mute's time comes: the timer turns it off and says so.
    clear_events(&app).await;
    app.0
        .core
        .lock()
        .await
        .streams
        .get_mut(&vec![1; 32])
        .unwrap()
        .launch_features = true;
    sql(&app, "UPDATE notification_extras SET mute_until=1").await;
    let mut stored = s.clone();
    stored.mute_until = Some(1);
    sqlx::query("UPDATE notification_settings SET settings=? WHERE user_id=1")
        .bind(serde_json::to_string(&stored).unwrap())
        .execute(&mut *c)
        .await
        .unwrap();
    drop(c);
    tick_now(&app).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let saved = crate::launch::notifications(&mut c, Id(1)).await.unwrap();
    assert!(!saved[0].muted && saved[0].mute_until.is_none() && saved[0].hide_muted_channels);
    assert!(events(&app, 1).await.iter().any(|e| matches!(e,
        Event::NotificationSettingsUpdated(n) if !n.muted)));
    assert_eq!(notify::next_mute(&mut c).await.unwrap(), None);
}

#[tokio::test]
async fn levels_mutes_and_nesting_decide_notifications_badges_and_push_alike() {
    let (_dir, app) = fixture().await;
    // 11 general; 12 is inside 11; 13 is inside 12.
    sql(&app, "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(12,'text',10,11,'inner',1,0),(13,'text',10,12,'deepest',2,0); INSERT INTO read_states(user_id,channel_id) VALUES(1,12),(2,12),(1,13),(2,13)").await;
    let plain = message_id(&ok(&run(&app, 2, send(13, "plain", vec![])).await));
    let mention = message_id(&ok(&run(&app, 2, send(13, "hey <@1>", vec![1])).await));
    // Server default: only mentions.
    assert_eq!(decide_for(&app, 1, 13, plain).await, None);
    assert!(decide_for(&app, 1, 13, mention).await.unwrap().mentioned);
    // Server: all messages.
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::All),
            false,
            true,
        ),
    )
    .await);
    assert!(decide_for(&app, 1, 13, plain).await.is_some());
    // The parent channel 11 says nothing: it applies to 12 and 13 inside it.
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), Some(11), NotificationLevel::Nothing),
            false,
            true,
        ),
    )
    .await);
    assert_eq!(decide_for(&app, 1, 13, mention).await, None);
    // The nearer channel 12 wins over 11.
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), Some(12), NotificationLevel::Mentions),
            false,
            true,
        ),
    )
    .await);
    assert_eq!(decide_for(&app, 1, 13, plain).await, None);
    assert!(decide_for(&app, 1, 13, mention).await.is_some());
    // A mute anywhere above mutes it.
    let mut muted = note_settings(Some(10), Some(11), NotificationLevel::All);
    muted.muted = true;
    ok(&run(&app, 1, set_notifications(muted.clone(), false, true)).await);
    assert_eq!(decide_for(&app, 1, 13, mention).await, None);
    // ... but a mention still badges (muting hides unread, not mentions).
    assert_eq!(read_state(&app, 1, 13).await, (2, 1));
    // An expired mute is no mute (lazily, before the timer runs).
    muted.mute_until = Some(now() + 60_000);
    ok(&run(&app, 1, set_notifications(muted.clone(), false, true)).await);
    muted.mute_until = Some(1);
    sqlx::query("UPDATE notification_settings SET settings=? WHERE user_id=1 AND target_id=11")
        .bind(serde_json::to_string(&muted).unwrap())
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert!(decide_for(&app, 1, 13, mention).await.is_some());
    // Mobile push is the server's switch.
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::All),
            false,
            false,
        ),
    )
    .await);
    assert!(!decide_for(&app, 1, 13, mention).await.unwrap().push);
    // Do Not Disturb holds everything back; Invisible does not.
    sql(&app, "UPDATE users SET desired_presence='dnd' WHERE id=1").await;
    assert_eq!(decide_for(&app, 1, 13, mention).await, None);
    sql(
        &app,
        "UPDATE users SET desired_presence='invisible' WHERE id=1",
    )
    .await;
    assert!(decide_for(&app, 1, 13, mention).await.is_some());
    // Direct messages default to every message.
    sql(&app, "INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0); INSERT INTO dm_members VALUES(20,1,0,1),(20,2,0,1); INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(2,20)").await;
    sql(&app, &friends_sql(1, 2)).await;
    let dm = message_id(&ok(&run(&app, 2, send(20, "hi", vec![])).await));
    assert!(decide_for(&app, 1, 20, dm).await.is_some());
    let mut dm_muted = note_settings(None, Some(20), NotificationLevel::All);
    dm_muted.muted = true;
    ok(&run(&app, 1, set_notifications(dm_muted, false, true)).await);
    assert_eq!(decide_for(&app, 1, 20, dm).await, None);
}

#[tokio::test]
async fn suppressing_everyone_and_role_mentions_reaches_badges_inbox_and_push() {
    let (_dir, app) = fixture().await;
    sqlx::query("UPDATE roles SET permissions=? WHERE id=10")
        .bind((Permissions::MEMBER | Permissions::MENTION_EVERYONE).bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    sql(&app, "INSERT INTO roles(guild_id,id,name,position,permissions,mentionable) VALUES(10,30,'crew',1,0,1); INSERT INTO member_roles VALUES(10,1,30)").await;
    let mut s = note_settings(Some(10), None, NotificationLevel::Mentions);
    s.suppress_everyone = true;
    ok(&run(&app, 1, set_notifications(s, true, true)).await);
    let everyone = message_id(&ok(&run(&app, 2, send(11, "@everyone hi", vec![])).await));
    let role = message_id(&ok(&run(&app, 2, send(11, "<@&30> hi", vec![])).await));
    let direct = message_id(&ok(&run(&app, 2, send(11, "<@1> hi", vec![1])).await));
    // Both broadcasts are suppressed everywhere; the explicit mention is not.
    assert_eq!(read_state(&app, 1, 11).await, (3, 1));
    assert_eq!(decide_for(&app, 1, 11, everyone).await, None);
    assert_eq!(decide_for(&app, 1, 11, role).await, None);
    assert!(decide_for(&app, 1, 11, direct).await.unwrap().mentioned);
    let (_, v) = call(&app, 1, "GET", "/api/v1/users/@me/mentions", None).await;
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["content"], "<@1> hi");
    // Only role suppression: @everyone counts again.
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::Mentions),
            true,
            true,
        ),
    )
    .await);
    let everyone2 = message_id(&ok(&run(&app, 2, send(11, "@everyone again", vec![])).await));
    let role2 = message_id(&ok(&run(&app, 2, send(11, "<@&30> again", vec![])).await));
    assert!(decide_for(&app, 1, 11, everyone2).await.is_some());
    assert_eq!(decide_for(&app, 1, 11, role2).await, None);
    assert_eq!(read_state(&app, 1, 11).await.1, 2);
}

#[tokio::test]
async fn alerts_follow_the_decision_and_reaction_preferences() {
    let (_dir, app) = fixture().await;
    let attach = |core: &mut Core, u: u64| {
        let key = vec![u as u8; 32];
        core.cursor(&key, Id(u));
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let (close, _) = tokio::sync::watch::channel(None);
        core.streams.get_mut(&key).unwrap().connection = Some(crate::state::Connection {
            focused: false,
            id: format!("ws{u}"),
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(1 << 22)),
            close,
        });
        rx
    };
    let (_rx1, _rx2) = {
        let mut core = app.0.core.lock().await;
        (attach(&mut core, 1), attach(&mut core, 2))
    };
    subscribe(&app, 1).await;
    subscribe(&app, 2).await;
    let alerts = |e: Vec<Event>| {
        e.into_iter()
            .filter_map(|e| match e {
                Event::Privacy(PrivacyEvent::Alert(a)) => Some(a),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    ok(&run(&app, 2, send(11, "no mention", vec![])).await);
    let m = message_id(&ok(&run(&app, 2, send(11, "<@1> look", vec![1])).await));
    let got = alerts(events(&app, 1).await);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].kind, got[0].message_id), (AlertKind::Mention, m));
    // Reactions to your messages: direct messages by default, so none here.
    let mine = message_id(&ok(&run(&app, 1, send(11, "mine", vec![])).await));
    clear_events(&app).await;
    let react = Command::AddReaction {
        channel_id: Id(11),
        message_id: mine,
        emoji: "👍".into(),
    };
    ok(&run(&app, 2, react).await);
    assert!(alerts(events(&app, 1).await).is_empty());
    let g = GlobalNotifications {
        reactions: ReactionAlerts::All,
        ..Default::default()
    };
    ok(&run(&app, 1, p(PrivacyCommand::SetGlobalNotifications(g))).await);
    assert_eq!(
        settings_of(&app, 1).await.global.reactions,
        ReactionAlerts::All
    );
    ok(&run(
        &app,
        2,
        Command::AddReaction {
            channel_id: Id(11),
            message_id: mine,
            emoji: "🎉".into(),
        },
    )
    .await);
    let got = alerts(events(&app, 1).await);
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].kind, got[0].emoji.as_deref()),
        (AlertKind::Reaction, Some("🎉"))
    );
    // Do Not Disturb: no alerts at all.
    clear_events(&app).await;
    ok(&run(
        &app,
        1,
        p(PrivacyCommand::SetStatus {
            status: PresenceStatus::Dnd,
            until: None,
        }),
    )
    .await);
    ok(&run(&app, 2, send(11, "<@1> again", vec![1])).await);
    assert!(alerts(events(&app, 1).await).is_empty());
}

// ---- Message requests ----

/// Users 1 and 3 share server 10 and are not friends.
async fn strangers() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(
        &app,
        "INSERT INTO guild_members VALUES(10,3,'member',0); UPDATE users SET created_at=0",
    )
    .await;
    (dir, app)
}
fn allow(app_user: u64) -> Command {
    let _ = app_user;
    p(PrivacyCommand::SetDmPrivacy(DmPrivacy {
        allow: DmAllow::ServerMembers,
        servers: vec![],
    }))
}
fn channel_of(r: &CommandResult) -> Id {
    match r {
        CommandResult::Channel(c) => c.summary.id,
        r => panic!("not a channel: {r:?}"),
    }
}

#[tokio::test]
async fn first_contact_dms_become_quiet_requests_the_recipient_controls() {
    let (_dir, app) = strangers().await;
    subscribe(&app, 1).await;
    subscribe(&app, 3).await;
    // Friends only by default.
    assert_eq!(
        err(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await).code,
        ErrorCode::Forbidden
    );
    ok(&run(&app, 1, allow(1)).await);
    let ch = channel_of(&ok(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await));
    let hello = message_id(&ok(&run(
        &app,
        3,
        send(ch.0, "hi, saw you in guild", vec![]),
    )
    .await));
    // The recipient sees only a count; never the conversation or its messages.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(!crate::db::channels(&mut c, Id(1), None)
        .await
        .unwrap()
        .iter()
        .any(|x| x.id == ch));
    drop(c);
    let seen = events(&app, 1).await;
    assert!(!seen.iter().any(|e| matches!(
        e,
        Event::MessageCreated { .. } | Event::ChannelUpsert(_) | Event::ReadStateUpdated(_)
    )));
    assert!(seen
        .iter()
        .any(|e| matches!(e, Event::Privacy(PrivacyEvent::Requests(r)) if r.pending == 1)));
    assert!(!seen
        .iter()
        .any(|e| matches!(e, Event::Privacy(PrivacyEvent::Alert(_)))));
    assert_eq!(decide_for(&app, 1, ch.0, hello).await, None);
    // The sender sees their DM as usual.
    assert!(events(&app, 3)
        .await
        .iter()
        .any(|e| matches!(e, Event::MessageCreated { message, .. } if message.id == hello)));
    let (s, v) = call(&app, 1, "GET", "/api/v1/users/@me/message-requests", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v["data"]["requests"][0]["messages"][0]["content"],
        "hi, saw you in guild"
    );
    // Accept: a normal DM.
    clear_events(&app).await;
    ok(&run(&app, 1, p(PrivacyCommand::AcceptRequest { channel_id: ch })).await);
    assert!(events(&app, 1)
        .await
        .iter()
        .any(|e| matches!(e, Event::ChannelUpsert(c) if c.summary.id == ch)));
    let next = message_id(&ok(&run(&app, 3, send(ch.0, "thanks", vec![])).await));
    assert!(decide_for(&app, 1, ch.0, next).await.is_some());
    assert_eq!(
        settings_of(&app, 1).await.requests,
        RequestCounts::default()
    );
    // Not found for someone else's request.
    assert_eq!(
        err(&run(&app, 3, p(PrivacyCommand::AcceptRequest { channel_id: ch })).await).code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn declined_requests_stay_hidden_and_the_sender_is_not_told() {
    let (_dir, app) = strangers().await;
    ok(&run(&app, 1, allow(1)).await);
    subscribe(&app, 3).await;
    let ch = channel_of(&ok(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await));
    ok(&run(&app, 3, send(ch.0, "hello?", vec![])).await);
    clear_events(&app).await;
    ok(&run(
        &app,
        1,
        p(PrivacyCommand::DeclineRequest { channel_id: ch }),
    )
    .await);
    assert!(events(&app, 3).await.is_empty());
    let later = message_id(&ok(&run(&app, 3, send(ch.0, "hello??", vec![])).await));
    assert_eq!(decide_for(&app, 1, ch.0, later).await, None);
    let (_, v) = call(&app, 1, "GET", "/api/v1/users/@me/message-requests", None).await;
    assert!(v["data"]["requests"].as_array().unwrap().is_empty());
    assert_eq!(
        scalar(&app, "SELECT visible FROM dm_members WHERE user_id=1").await,
        0
    );
}

#[tokio::test]
async fn request_settings_per_server_pauses_rate_limits_and_spam() {
    let (_dir, app) = strangers().await;
    // Allowed by default, but not from server 10.
    ok(&run(
        &app,
        1,
        p(PrivacyCommand::SetDmPrivacy(DmPrivacy {
            allow: DmAllow::ServerMembers,
            servers: vec![DmServerOverride {
                guild_id: Id(10),
                allow: false,
            }],
        })),
    )
    .await);
    assert_eq!(
        err(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await).code,
        ErrorCode::Forbidden
    );
    // Overrides must be servers you are in.
    assert_eq!(
        err(&run(
            &app,
            1,
            p(PrivacyCommand::SetDmPrivacy(DmPrivacy {
                allow: DmAllow::Friends,
                servers: vec![DmServerOverride {
                    guild_id: Id(99),
                    allow: true,
                }],
            })),
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    // Friends only, but server 10 allowed.
    ok(&run(
        &app,
        1,
        p(PrivacyCommand::SetDmPrivacy(DmPrivacy {
            allow: DmAllow::Friends,
            servers: vec![DmServerOverride {
                guild_id: Id(10),
                allow: true,
            }],
        })),
    )
    .await);
    // A server that paused DMs between non-friends still wins.
    sqlx::query("INSERT INTO guild_safety(guild_id,dms_paused_until) VALUES(10,?)")
        .bind((now() + 3_600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let e = err(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await);
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::Forbidden, Some("dms_paused"))
    );
    sql(&app, "DELETE FROM guild_safety").await;
    let ch = channel_of(&ok(&run(&app, 3, Command::OpenDm { user_id: Id(1) }).await));
    // A links-only message moves the request to spam.
    ok(&run(&app, 3, send(ch.0, "https://free.example/prize", vec![])).await);
    assert_eq!(
        settings_of(&app, 1).await.requests,
        RequestCounts {
            pending: 0,
            spam: 1
        }
    );
    let (_, v) = call(&app, 1, "GET", "/api/v1/users/@me/message-requests", None).await;
    assert_eq!(v["data"]["spam"].as_array().unwrap().len(), 1);
    // A burst of first contacts is spam, and the rate limit stops it.
    for u in 200..215_i64 {
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'','online',0)")
            .bind(u)
            .bind(format!("burst{u}"))
            .execute(&app.0.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO guild_members VALUES(10,?,'member',0); INSERT INTO dm_privacy VALUES(?,'server_members')")
            .bind(u)
            .bind(u)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    let mut outcomes = vec![];
    for u in 200..215_u64 {
        outcomes.push(run(&app, 3, Command::OpenDm { user_id: Id(u) }).await);
    }
    // One request already exists (to user 1): nine more fit in the hour.
    assert!(outcomes[..9]
        .iter()
        .all(|o| matches!(o, CommandOutcome::Ok(_))));
    assert_eq!(err(&outcomes[9]).code, ErrorCode::RateLimited);
    assert!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM message_requests WHERE sender_id=3 AND state='spam'"
        )
        .await
            >= 5
    );
}

// ---- Reports ----

/// Server 10: owner 1, member 2 (moderator role with Manage Messages and Ban), 3 and 4.
async fn moderated() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0),(10,4,'member',0); INSERT INTO read_states(user_id,channel_id) VALUES(3,11),(4,11); UPDATE users SET created_at=0").await;
    sqlx::query(
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,30,'mods',2,?)",
    )
    .bind((Permissions::MANAGE_MESSAGES | Permissions::BAN_MEMBERS).bits() as i64)
    .execute(&app.0.pool)
    .await
    .unwrap();
    sql(&app, "INSERT INTO member_roles VALUES(10,2,30)").await;
    (dir, app)
}
fn report(target: ReportTarget, note: Option<&str>) -> Command {
    p(PrivacyCommand::Report(ReportDraft {
        target,
        category: ReportCategory::Harassment,
        note: note.map(Into::into),
    }))
}
fn filed(r: &CommandResult) -> MyReport {
    match r {
        CommandResult::Privacy(PrivacyResult::Filed(m)) => m.clone(),
        r => panic!("not a report: {r:?}"),
    }
}

#[tokio::test]
async fn reports_keep_evidence_reach_moderators_and_never_name_the_reporter() {
    let (_dir, app) = moderated().await;
    let m = message_id(&ok(&run(&app, 4, send(11, "something nasty", vec![])).await));
    let target = ReportTarget::Message {
        channel_id: Id(11),
        message_id: m,
    };
    // Not your own message; at most 1,000 characters of note.
    let own = message_id(&ok(&run(&app, 3, send(11, "mine", vec![])).await));
    assert_eq!(
        err(&run(
            &app,
            3,
            report(
                ReportTarget::Message {
                    channel_id: Id(11),
                    message_id: own
                },
                None
            )
        )
        .await)
        .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 3, report(target, Some(&"n".repeat(1001)))).await).code,
        ErrorCode::InvalidRequest
    );
    subscribe(&app, 2).await;
    let mine = filed(&ok(&run(&app, 3, report(target, Some("please look"))).await));
    assert_eq!(mine.status, ReportStatus::Open);
    // Once per target while it is open.
    assert_eq!(
        err(&run(&app, 3, report(target, None)).await).code,
        ErrorCode::AlreadyExists
    );
    assert!(events(&app, 2).await.iter().any(|e| matches!(e,
        Event::Privacy(PrivacyEvent::GuildReports { guild_id, open: 1 }) if *guild_id == Id(10))));
    // The author edits and then deletes it: the evidence is kept.
    ok(&run(
        &app,
        4,
        Command::EditMessage(MessageEdit {
            channel_id: Id(11),
            message_id: m,
            content: "nothing to see".into(),
            mention_ids: vec![],
        }),
    )
    .await);
    ok(&run(
        &app,
        4,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: m,
        },
    )
    .await);
    let (s, v) = call(&app, 2, "GET", "/api/v1/guilds/10/reports", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v["data"]["reports"][0];
    assert_eq!(r["evidence"]["content"], "something nasty");
    assert_eq!(r["evidence"]["user"]["username"], "user4");
    assert_eq!(r["note"], "please look");
    // The reporter appears nowhere in what moderators get.
    assert!(!v.to_string().contains("user3") && !v.to_string().contains("reporter"));
    // Members without a moderation permission cannot see the queue.
    assert_eq!(
        call(&app, 4, "GET", "/api/v1/guilds/10/reports", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, 1, "GET", "/api/v1/guilds/10/reports", None)
            .await
            .0,
        StatusCode::OK
    );
    // The reporter sees their report's status.
    let (_, v) = call(&app, 3, "GET", "/api/v1/users/@me/reports", None).await;
    assert_eq!(v["data"][0]["status"], "open");
    assert_eq!(v["data"][0]["summary"], "A message from @user4 in #general");
}

#[tokio::test]
async fn moderators_act_on_reports_through_the_normal_commands_and_the_audit_log() {
    let (_dir, app) = moderated().await;
    let m = message_id(&ok(&run(&app, 4, send(11, "spam spam", vec![])).await));
    let r1 = filed(&ok(&run(
        &app,
        3,
        report(
            ReportTarget::Message {
                channel_id: Id(11),
                message_id: m,
            },
            None,
        ),
    )
    .await));
    let resolve = |id: Id, action: serde_json::Value| {
        (
            format!("/api/v1/guilds/10/reports/{}/resolve", id.0),
            serde_json::json!({ "action": action, "reason": "rule 2" }),
        )
    };
    // Someone who is not a moderator: not found.
    let (uri, body) = resolve(r1.id, serde_json::json!({"type":"dismiss"}));
    assert_eq!(
        call(&app, 4, "POST", &uri, Some(body)).await.0,
        StatusCode::NOT_FOUND
    );
    // A moderator without Time Out Members: the command's own check refuses.
    let (uri, body) = resolve(
        r1.id,
        serde_json::json!({"type":"timeout","data":{"seconds":60}}),
    );
    assert_eq!(
        call(&app, 2, "POST", &uri, Some(body)).await.0,
        StatusCode::FORBIDDEN
    );
    // Delete the message: deleted, audited twice (the deletion and the report).
    subscribe(&app, 3).await;
    let (uri, body) = resolve(r1.id, serde_json::json!({"type":"delete_message"}));
    let (s, v) = call(&app, 2, "POST", &uri, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "actioned");
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM messages WHERE content='' AND deleted_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='message_delete' AND actor_id=2"
        )
        .await,
        1
    );
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='report_resolve' AND actor_id=2"
        )
        .await,
        1
    );
    assert!(events(&app, 3).await.iter().any(|e| matches!(e,
        Event::Privacy(PrivacyEvent::ReportUpdated(r)) if r.status == ReportStatus::Actioned)));
    // Already handled.
    let (uri, body) = resolve(r1.id, serde_json::json!({"type":"dismiss"}));
    assert_eq!(
        call(&app, 2, "POST", &uri, Some(body)).await.0,
        StatusCode::CONFLICT
    );
    // Ban the person behind a user... reports about people go to the administrator.
    let r2 = filed(&ok(&run(
        &app,
        3,
        report(ReportTarget::User { user_id: Id(4) }, None),
    )
    .await));
    let (uri, body) = resolve(r2.id, serde_json::json!({"type":"ban"}));
    assert_eq!(
        call(&app, 2, "POST", &uri, Some(body)).await.0,
        StatusCode::NOT_FOUND
    );
    let m2 = message_id(&ok(&run(&app, 4, send(11, "more", vec![])).await));
    let r3 = filed(&ok(&run(
        &app,
        3,
        report(
            ReportTarget::Message {
                channel_id: Id(11),
                message_id: m2,
            },
            None,
        ),
    )
    .await));
    let (uri, body) = resolve(r3.id, serde_json::json!({"type":"ban"}));
    let (s, v) = call(&app, 2, "POST", &uri, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM guild_bans WHERE user_id=4").await,
        1
    );
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='member_ban'"
        )
        .await,
        1
    );
    // Dismiss.
    sql(&app, "INSERT INTO guild_members VALUES(10,4,'member',0); INSERT INTO read_states(user_id,channel_id) VALUES(4,11)").await;
    sql(&app, "DELETE FROM guild_bans").await;
    let m3 = message_id(&ok(&run(&app, 4, send(11, "fine actually", vec![])).await));
    let r4 = filed(&ok(&run(
        &app,
        3,
        report(
            ReportTarget::Message {
                channel_id: Id(11),
                message_id: m3,
            },
            None,
        ),
    )
    .await));
    let (uri, body) = resolve(r4.id, serde_json::json!({"type":"dismiss"}));
    let (_, v) = call(&app, 2, "POST", &uri, Some(body)).await;
    assert_eq!(v["status"], "dismissed");
}

#[tokio::test]
async fn reports_about_dms_people_and_servers_go_to_the_administrator() {
    let (_dir, app) = moderated().await;
    sql(&app, "INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0); INSERT INTO dm_members VALUES(20,3,0,1),(20,4,0,1); INSERT INTO read_states(user_id,channel_id) VALUES(3,20),(4,20)").await;
    sql(&app, &friends_sql(3, 4)).await;
    let dm = message_id(&ok(&run(&app, 4, send(20, "dm threat", vec![])).await));
    filed(&ok(&run(
        &app,
        3,
        report(
            ReportTarget::Message {
                channel_id: Id(20),
                message_id: dm,
            },
            None,
        ),
    )
    .await));
    filed(&ok(&run(
        &app,
        3,
        report(ReportTarget::User { user_id: Id(4) }, Some("profile")),
    )
    .await));
    filed(&ok(&run(
        &app,
        3,
        report(ReportTarget::Guild { guild_id: Id(10) }, None),
    )
    .await));
    // Not in the server's queue.
    let (_, v) = call(&app, 1, "GET", "/api/v1/guilds/10/reports", None).await;
    assert!(v["data"]["reports"].as_array().unwrap().is_empty());
    // Only what you can see can be reported.
    assert_eq!(
        err(&run(
            &app,
            1,
            report(
                ReportTarget::Message {
                    channel_id: Id(20),
                    message_id: dm
                },
                None
            )
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    // The administrator's CLI: list, dismiss, and disable the reported account.
    let mut c = app.0.pool.acquire().await.unwrap();
    reports::admin(&mut c, &crate::admin::ReportAdmin::List { all: false })
        .await
        .unwrap();
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM reports WHERE queue='admin' ORDER BY id")
            .fetch_all(&mut *c)
            .await
            .unwrap();
    assert_eq!(ids.len(), 3);
    reports::admin(
        &mut c,
        &crate::admin::ReportAdmin::Show { id: ids[0] as u64 },
    )
    .await
    .unwrap();
    reports::admin(
        &mut c,
        &crate::admin::ReportAdmin::Dismiss { id: ids[2] as u64 },
    )
    .await
    .unwrap();
    reports::admin(
        &mut c,
        &crate::admin::ReportAdmin::DisableUser { id: ids[1] as u64 },
    )
    .await
    .unwrap();
    drop(c);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM disabled_users WHERE user_id=4").await,
        1
    );
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM sessions WHERE user_id=4").await,
        0
    );
    let (_, v) = call(&app, 3, "GET", "/api/v1/users/@me/reports", None).await;
    let statuses: Vec<_> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["status"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(statuses, vec!["dismissed", "actioned", "open"]);
}

#[tokio::test]
async fn reporting_is_rate_limited_and_bounded() {
    let (_dir, app) = moderated().await;
    let mut ids = vec![];
    for n in 0..5 {
        ids.push(message_id(&ok(&run(
            &app,
            4,
            send(11, &format!("m{n}"), vec![]),
        )
        .await)));
    }
    let mut outcomes = vec![];
    for m in &ids {
        outcomes.push(
            run_limited(
                &app,
                3,
                report(
                    ReportTarget::Message {
                        channel_id: Id(11),
                        message_id: *m,
                    },
                    None,
                ),
            )
            .await,
        );
    }
    assert!(outcomes[..3]
        .iter()
        .all(|o| matches!(o, CommandOutcome::Ok(_))));
    assert_eq!(err(&outcomes[3]).code, ErrorCode::RateLimited);
    // At most 50 open reports per person.
    sql(&app, "WITH RECURSIVE s(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM s WHERE value<50) INSERT INTO reports(id,reporter_id,queue,target,target_key,category,evidence,created_at) SELECT 900000+value,3,'admin','{}','k'||value,'spam','{}',0 FROM s").await;
    assert_eq!(
        err(&run(
            &app,
            3,
            report(
                ReportTarget::Message {
                    channel_id: Id(11),
                    message_id: ids[4]
                },
                None
            )
        )
        .await)
        .code,
        ErrorCode::LimitExceeded
    );
}

// ---- Age-restricted channels, on the server ----

#[tokio::test]
async fn unconfirmed_members_get_no_content_from_age_restricted_channels() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0); INSERT INTO read_states(user_id,channel_id) VALUES(3,11); INSERT INTO channel_age_restricted VALUES(11); INSERT INTO age_confirmations VALUES(3,11,0)").await;
    ok(&run(
        &app,
        1,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::All),
            false,
            true,
        ),
    )
    .await);
    ok(&run(
        &app,
        3,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::All),
            false,
            true,
        ),
    )
    .await);
    subscribe(&app, 1).await;
    subscribe(&app, 3).await;
    clear_events(&app).await;
    let m = message_id(&ok(&run(
        &app,
        2,
        send(11, "adults only <@1> <@3>", vec![1, 3]),
    )
    .await));
    // New messages: only to the member who confirmed.
    let content = |e: &Event| {
        matches!(
            e,
            Event::MessageCreated { .. }
                | Event::MessageUpdated(_)
                | Event::ReactionUpdated(_)
                | Event::Typing { .. }
        )
    };
    assert!(!events(&app, 1).await.iter().any(content));
    assert!(events(&app, 3).await.iter().any(content));
    // The count still arrives (no content in it).
    assert!(events(&app, 1)
        .await
        .iter()
        .any(|e| matches!(e, Event::ReadStateUpdated(r) if r.channel_id == Id(11))));
    // Edits, reactions and typing too.
    ok(&run(
        &app,
        2,
        Command::EditMessage(MessageEdit {
            channel_id: Id(11),
            message_id: m,
            content: "edited <@1> <@3>".into(),
            mention_ids: vec![Id(1), Id(3)],
        }),
    )
    .await);
    ok(&run(
        &app,
        3,
        Command::AddReaction {
            channel_id: Id(11),
            message_id: m,
            emoji: "👀".into(),
        },
    )
    .await);
    ok(&run(&app, 2, Command::Typing { channel_id: Id(11) }).await);
    assert!(!events(&app, 1).await.iter().any(content));
    // Push and alerts.
    let mut c = app.0.pool.acquire().await.unwrap();
    let channel = crate::db::channel(&mut c, Id(11)).await.unwrap();
    let message = crate::db::message(&mut c, Id(3), Id(11), m, false)
        .await
        .unwrap();
    assert_eq!(
        notify::decide(&mut c, Id(1), &channel, &message)
            .await
            .unwrap(),
        None
    );
    assert!(notify::decide(&mut c, Id(3), &channel, &message)
        .await
        .unwrap()
        .is_some());
    drop(c);
    assert!(!events(&app, 1)
        .await
        .iter()
        .any(|e| matches!(e, Event::Privacy(PrivacyEvent::Alert(_)))));
    // History, pins, the Inbox (mentions and unreads) and search.
    sqlx::query("INSERT INTO pins VALUES(11,?,2,0)")
        .bind(m.0 as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    for (u, visible) in [(1, false), (3, true)] {
        let (s, _) = call(&app, u, "GET", "/api/v1/channels/11/messages", None).await;
        assert_eq!(s == StatusCode::OK, visible, "history {u}");
        let (s, _) = call(&app, u, "GET", "/api/v1/channels/11/pins", None).await;
        assert_eq!(s == StatusCode::OK, visible, "pins {u}");
        let (_, v) = call(&app, u, "GET", "/api/v1/users/@me/mentions", None).await;
        assert_eq!(
            !v["data"]["items"].as_array().unwrap().is_empty(),
            visible,
            "mentions {u}"
        );
        let (_, v) = call(&app, u, "GET", "/api/v1/users/@me/unreads", None).await;
        assert_eq!(v.to_string().contains("edited"), visible, "unreads {u}");
        let (_, v) = call(
            &app,
            u,
            "GET",
            "/api/v1/guilds/10/messages/search?q=edited",
            None,
        )
        .await;
        assert_eq!(
            !v["data"]["items"].as_array().unwrap().is_empty(),
            visible,
            "search {u}"
        );
    }
    // Saved messages too.
    let (s, v) = call(
        &app,
        1,
        "PUT",
        &format!("/api/v1/users/@me/saved/{}", m.0),
        Some(serde_json::json!({"channel_id": Id(11)})),
    )
    .await;
    assert_ne!(s, StatusCode::OK, "{v}");
}

/// Threads and forum posts inside an age-restricted channel are restricted
/// with it on every path (task `qa2`): before, each check looked at the
/// conversation's own row only, so a thread of an age-restricted channel was
/// readable, searchable and alerting without confirming. Confirming from inside
/// the thread confirms the channel above it.
#[tokio::test]
async fn threads_inside_an_age_restricted_channel_are_restricted_with_it() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0); INSERT INTO read_states(user_id,channel_id) VALUES(3,11); INSERT INTO channel_age_restricted VALUES(11); INSERT INTO age_confirmations VALUES(3,11,0);
         INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(40,'thread',10,11,'after dark',0,0);
         INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(40,11,10,2,1440,0,0,9999999999999);
         INSERT INTO read_states(user_id,channel_id) VALUES(1,40),(2,40),(3,40);
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'lobby',1,0);
         INSERT INTO read_states(user_id,channel_id) VALUES(1,12),(2,12),(3,12);").await;
    for u in [1, 3] {
        ok(&run(
            &app,
            u,
            set_notifications(
                note_settings(Some(10), None, NotificationLevel::All),
                false,
                true,
            ),
        )
        .await);
        subscribe(&app, u).await;
    }
    clear_events(&app).await;
    let m = message_id(&ok(&run(
        &app,
        2,
        send(40, "in the thread <@1> <@3>", vec![1, 3]),
    )
    .await));
    let created = |e: &Event| matches!(e, Event::MessageCreated { .. });
    assert!(!events(&app, 1).await.iter().any(created), "live message");
    assert!(events(&app, 3).await.iter().any(created));
    let mut c = app.0.pool.acquire().await.unwrap();
    let thread = crate::db::channel(&mut c, Id(40)).await.unwrap();
    let message = crate::db::message(&mut c, Id(3), Id(40), m, false)
        .await
        .unwrap();
    assert_eq!(
        notify::decide(&mut c, Id(1), &thread, &message)
            .await
            .unwrap(),
        None,
        "push and alerts"
    );
    assert!(notify::decide(&mut c, Id(3), &thread, &message)
        .await
        .unwrap()
        .is_some());
    drop(c);
    sqlx::query("INSERT INTO pins VALUES(40,?,2,0)")
        .bind(m.0 as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    for (u, visible) in [(1, false), (3, true)] {
        let (s, _) = call(&app, u, "GET", "/api/v1/channels/40/messages", None).await;
        assert_eq!(s == StatusCode::OK, visible, "history {u}");
        let (s, _) = call(&app, u, "GET", "/api/v1/channels/40/pins", None).await;
        assert_eq!(s == StatusCode::OK, visible, "pins {u}");
        let (_, v) = call(&app, u, "GET", "/api/v1/users/@me/mentions", None).await;
        assert_eq!(
            !v["data"]["items"].as_array().unwrap().is_empty(),
            visible,
            "mentions {u}"
        );
        let (_, v) = call(
            &app,
            u,
            "GET",
            "/api/v1/guilds/10/messages/search?q=thread",
            None,
        )
        .await;
        assert_eq!(
            !v["data"]["items"].as_array().unwrap().is_empty(),
            visible,
            "search {u}"
        );
    }
    let (s, v) = call(
        &app,
        1,
        "PUT",
        &format!("/api/v1/users/@me/saved/{}", m.0),
        Some(serde_json::json!({"channel_id": Id(40)})),
    )
    .await;
    assert_ne!(s, StatusCode::OK, "saved {v}");
    // Forwarding out of it: only into another age-restricted channel (its
    // snapshot is shown to everyone who can read the destination).
    let forward = |to: u64| {
        Command::Content(ContentCommand::Forward {
            channel_id: Id(40),
            message_id: m,
            destinations: vec![Id(to)],
            comment: String::new(),
        })
    };
    assert_eq!(
        err(&run(&app, 3, forward(12)).await).code,
        ErrorCode::Forbidden
    );
    ok(&run(&app, 3, forward(11)).await);
    // Confirming from inside the thread confirms the channel it is in.
    ok(&run(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::ConfirmAge {
            channel_id: Id(40),
        })),
    )
    .await);
    let (s, _) = call(&app, 1, "GET", "/api/v1/channels/11/messages", None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, 1, "GET", "/api/v1/channels/40/messages", None).await;
    assert_eq!(s, StatusCode::OK);
}

/// Notifications that are not messages (an event starting, a reminder, a
/// call ringing) go through the same decision as messages (`notify::notice`):
/// ignore and block, age restriction, Do Not Disturb, mutes and level, and
/// the per-server mobile push switch (task `qa2`). AutoMod alerts are
/// messages and use `decide`; Stage invitations, application decisions and
/// report status are never an alert or a push.
#[tokio::test]
async fn events_reminders_and_rings_go_through_the_one_notification_decision() {
    use notify::Notice;
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0); INSERT INTO dm_members VALUES(20,1,0,1),(20,2,0,1); INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(2,20)").await;
    let event = Notice::EventStarting {
        guild: Id(10),
        channel: None,
    };
    let in_channel = Notice::EventStarting {
        guild: Id(10),
        channel: Some(Id(11)),
    };
    let reminder = Notice::Reminder { channel: Id(11) };
    let ring = Notice::Ring {
        channel: Id(20),
        caller: Id(1),
    };
    let decide = |n: Notice| {
        let app = app.clone();
        async move {
            let mut c = app.0.pool.acquire().await.unwrap();
            notify::notice(&mut c, Id(2), n).await.unwrap()
        }
    };
    let pushes = |d: Option<notify::Decision>| d.map(|d| d.push);
    for n in [event, in_channel, reminder, ring] {
        assert_eq!(pushes(decide(n).await), Some(true), "{n:?}");
    }
    // The server's mobile push switch: still told in the app, never pushed.
    ok(&run(
        &app,
        2,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::Mentions),
            false,
            false,
        ),
    )
    .await);
    for n in [event, in_channel, reminder] {
        assert_eq!(pushes(decide(n).await), Some(false), "{n:?}");
    }
    // A muted server: no event notice; a reminder still comes (you asked).
    let mut muted = note_settings(Some(10), None, NotificationLevel::Mentions);
    muted.muted = true;
    ok(&run(&app, 2, set_notifications(muted, false, true)).await);
    assert_eq!(decide(event).await, None);
    assert_eq!(decide(in_channel).await, None);
    assert!(decide(reminder).await.is_some());
    // Nothing at the channel level silences an event held there.
    ok(&run(
        &app,
        2,
        set_notifications(
            note_settings(Some(10), None, NotificationLevel::Mentions),
            false,
            true,
        ),
    )
    .await);
    ok(&run(
        &app,
        2,
        set_notifications(
            note_settings(Some(10), Some(11), NotificationLevel::Nothing),
            false,
            true,
        ),
    )
    .await);
    assert_eq!(decide(in_channel).await, None);
    assert!(decide(event).await.is_some());
    // An age-restricted channel not confirmed: no reminder from it.
    sql(&app, "INSERT INTO channel_age_restricted VALUES(11)").await;
    assert_eq!(decide(reminder).await, None);
    sql(&app, "INSERT INTO age_confirmations VALUES(2,11,0)").await;
    assert!(decide(reminder).await.is_some());
    // A muted DM does not ring; an ignored or blocked caller never does.
    let mut dm = note_settings(None, Some(20), NotificationLevel::All);
    dm.muted = true;
    ok(&run(&app, 2, set_notifications(dm, false, true)).await);
    assert_eq!(decide(ring).await, None);
    ok(&run(
        &app,
        2,
        set_notifications(
            note_settings(None, Some(20), NotificationLevel::All),
            false,
            true,
        ),
    )
    .await);
    assert!(decide(ring).await.is_some());
    ok(&run(&app, 2, p(PrivacyCommand::Ignore { user_id: Id(1) })).await);
    assert_eq!(decide(ring).await, None);
    ok(&run(&app, 2, p(PrivacyCommand::Unignore { user_id: Id(1) })).await);
    assert!(decide(ring).await.is_some());
    // Do Not Disturb: none of them.
    sql(&app, "UPDATE users SET desired_presence='dnd' WHERE id=2").await;
    for n in [event, reminder, ring] {
        assert_eq!(decide(n).await, None, "{n:?}");
    }
}

/// Security review F2: a message request the recipient has not accepted, or a
/// caller they ignore, never rings them, pushes "Incoming call" or shows them
/// the hidden conversation.
#[tokio::test]
async fn requests_and_ignored_callers_never_ring_or_unhide_the_conversation() {
    let (_dir, app) = fixture().await;
    // 20: user 1's first-contact DM to user 2, still a request (hidden for 2).
    // 21: an accepted DM between 3 and 2, where 2 ignores 3. 22: a group DM
    // of 1, 2 and 4 (the group-DM equivalent: 2 ignores 1 there too).
    sql(&app, "INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0),(21,'direct',0,0);
        INSERT INTO channels(id,kind,name,position,owner_id,created_at) VALUES(22,'group_dm','group',0,1,0);
        INSERT INTO dm_members VALUES(20,1,0,1),(20,2,0,0),(21,3,0,1),(21,2,0,1),(22,1,0,1),(22,2,0,1),(22,4,0,1);
        INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(2,20),(2,21),(3,21),(1,22),(2,22),(4,22);
        INSERT INTO message_requests VALUES(20,2,1,'pending',0);").await;
    let visible = |ch: u64| {
        let app = app.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT visible FROM dm_members WHERE channel_id=? AND user_id=2",
            )
            .bind(ch as i64)
            .fetch_one(&app.0.pool)
            .await
            .unwrap()
        }
    };
    let mut c = app.0.pool.acquire().await.unwrap();
    for state in ["pending", "spam", "ignored"] {
        sqlx::query("UPDATE message_requests SET state=? WHERE channel_id=20")
            .bind(state)
            .execute(&mut *c)
            .await
            .unwrap();
        let ch = crate::db::channel(&mut c, Id(20)).await.unwrap();
        let refused = crate::calls::admit(&mut c, Id(1), &ch).await.unwrap_err();
        assert_eq!(refused.0.code, ErrorCode::Forbidden, "{state}");
        // The recipient may still call the sender back.
        crate::calls::admit(&mut c, Id(2), &ch).await.unwrap();
    }
    {
        // Even a call that got started never rings or un-hides a request.
        let mut core = app.0.core.lock().await;
        crate::calls::joining(&app, &mut core, &mut c, Id(1), Id(20))
            .await
            .unwrap();
        assert!(!core.calls.ringing(Id(20)).contains(&Id(2)));
    }
    assert_eq!(visible(20).await, 0, "the request stays hidden");
    // The push decision agrees.
    let ring = |channel, caller| notify::Notice::Ring { channel, caller };
    assert_eq!(
        notify::notice(&mut c, Id(2), ring(Id(20), Id(1)))
            .await
            .unwrap(),
        None
    );
    // Accepted: calls work and ring.
    sqlx::query("UPDATE message_requests SET state='accepted' WHERE channel_id=20")
        .execute(&mut *c)
        .await
        .unwrap();
    let ch = crate::db::channel(&mut c, Id(20)).await.unwrap();
    crate::calls::admit(&mut c, Id(1), &ch).await.unwrap();
    // An ignored caller in an ordinary DM and in a group DM: not rung.
    drop(c);
    ok(&run(&app, 2, p(PrivacyCommand::Ignore { user_id: Id(3) })).await);
    ok(&run(&app, 2, p(PrivacyCommand::Ignore { user_id: Id(1) })).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    {
        let mut core = app.0.core.lock().await;
        crate::calls::joining(&app, &mut core, &mut c, Id(3), Id(21))
            .await
            .unwrap();
        assert!(!core.calls.ringing(Id(21)).contains(&Id(2)));
        crate::calls::joining(&app, &mut core, &mut c, Id(1), Id(22))
            .await
            .unwrap();
        let rung = core.calls.ringing(Id(22));
        assert!(rung.contains(&Id(4)) && !rung.contains(&Id(2)), "{rung:?}");
    }
    assert_eq!(
        notify::notice(&mut c, Id(2), ring(Id(21), Id(3)))
            .await
            .unwrap(),
        None
    );
}

/// Security review F4: a moderator who cannot read the channel a reported
/// message is in sees the report, its category and the reported person, not
/// what was said.
#[tokio::test]
async fn reported_text_is_shown_only_to_moderators_who_can_read_the_channel() {
    let (_dir, app) = moderated().await;
    // #staff (13): moderator 2 may not view it; 3 and 4 may.
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(13,'text',10,'staff',3,0);
        INSERT INTO permission_overwrites VALUES(13,'member',2,0,1);
        INSERT INTO read_states(user_id,channel_id) VALUES(1,13),(3,13),(4,13);").await;
    let m = message_id(&ok(
        &run(&app, 4, send(13, "staff-only words", vec![])).await
    ));
    let target = ReportTarget::Message {
        channel_id: Id(13),
        message_id: m,
    };
    filed(&ok(&run(&app, 3, report(target, None)).await));
    let (s, v) = call(&app, 2, "GET", "/api/v1/guilds/10/reports", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v["data"]["reports"][0];
    assert_eq!(r["evidence_hidden"], true);
    assert_eq!(r["evidence"]["user"]["username"], "user4");
    assert!(!v.to_string().contains("staff-only words") && !v.to_string().contains("\"staff\""));
    // The owner can read #staff and sees the text.
    let (_, v) = call(&app, 1, "GET", "/api/v1/guilds/10/reports", None).await;
    assert_eq!(
        v["data"]["reports"][0]["evidence"]["content"],
        "staff-only words"
    );
    assert!(v["data"]["reports"][0].get("evidence_hidden").is_none());
}
