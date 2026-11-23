//! Server tests for threads, forums and announcements: one per command,
//! permission and limit.
use super::*;
use crate::{audit_tests::fixture, commands};

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
async fn thread_cmd(app: &App, u: u64, nonce: u64, cmd: ThreadCommand) -> CommandOutcome {
    run(app, u, nonce, Command::Threads(cmd)).await
}
fn code(outcome: &CommandOutcome) -> Option<ErrorCode> {
    match outcome {
        CommandOutcome::Err(e) => Some(e.code),
        CommandOutcome::Ok(_) => None,
    }
}
#[track_caller]
fn ok(outcome: CommandOutcome) -> CommandResult {
    match outcome {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("command failed: {e:?}"),
    }
}
fn thread_of(r: CommandResult) -> ThreadInfo {
    match r {
        CommandResult::Threads(ThreadResult::Thread(t)) => *t,
        CommandResult::Threads(ThreadResult::Post { thread, .. }) => *thread,
        other => panic!("not a thread: {other:?}"),
    }
}
fn message_of(r: CommandResult) -> Message {
    match r {
        CommandResult::Message(m) => m,
        other => panic!("not a message: {other:?}"),
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
async fn sql(app: &App, q: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(q.to_owned()))
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn scalar(app: &App, q: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(q.to_owned()))
        .fetch_one(&app.0.pool)
        .await
        .unwrap()
}
fn draft(ch: Id, content: &str) -> MessageDraft {
    MessageDraft {
        channel_id: ch,
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    }
}
async fn send(app: &App, u: u64, nonce: u64, ch: Id, content: &str) -> CommandOutcome {
    run(app, u, nonce, Command::SendMessage(draft(ch, content))).await
}
fn new_thread(parent: u64, name: &str) -> NewThread {
    NewThread {
        parent_id: Id(parent),
        name: name.into(),
        message_id: None,
        private: false,
        invitable: true,
        auto_archive_minutes: 1440,
    }
}
/// Guild 10: user 1 owns it, users 2 and 3 are members. #11 is text.
async fn world() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO read_states(user_id,channel_id) VALUES(3,11)").await;
    (dir, app)
}
async fn subscribe(app: &App, users: &[u64]) {
    for (i, u) in users.iter().enumerate() {
        ok(thread_cmd(app, *u, 900 + i as u64, ThreadCommand::Subscribe).await);
    }
}
/// Grants or denies bits for @everyone (role 10) on the guild.
async fn everyone(app: &App, add: Permissions, remove: Permissions) {
    let p = scalar(app, "SELECT permissions FROM roles WHERE id=10").await as u64;
    let p = (p | add.bits()) & !remove.bits();
    sqlx::query("UPDATE roles SET permissions=? WHERE id=10")
        .bind(p as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn moderator(app: &App, user: u64, bits: Permissions) {
    sqlx::query(
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,70,'mods',2,?)",
    )
    .bind(bits.bits() as i64)
    .execute(&app.0.pool)
    .await
    .unwrap();
    sql(
        app,
        &format!("INSERT INTO member_roles VALUES(10,{user},70)"),
    )
    .await;
}

#[tokio::test]
async fn new_servers_and_migrated_ones_grant_thread_rights_and_keep_read_only_channels() {
    let (_dir, app) = world().await;
    let p = Permissions::from_bits_retain(
        scalar(&app, "SELECT permissions FROM roles WHERE id=10").await as u64,
    );
    assert!(p.contains(
        Permissions::CREATE_PUBLIC_THREADS
            | Permissions::CREATE_PRIVATE_THREADS
            | Permissions::SEND_MESSAGES_IN_THREADS
    ));
    assert!(!p.intersects(Permissions::MANAGE_THREADS | Permissions::PUBLISH_MESSAGES));
    // The migration's overwrite rule, replayed on a read-only channel.
    let mut c = <sqlx::SqliteConnection as sqlx::Connection>::connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE permission_overwrites(channel_id,kind,target_id,allow,deny);INSERT INTO permission_overwrites VALUES(1,'role',1,0,2),(2,'role',1,0,4);")
        .execute(&mut c)
        .await
        .unwrap();
    sqlx::raw_sql("UPDATE permission_overwrites SET deny=deny|7696581394432 WHERE (deny & 2)<>0;")
        .execute(&mut c)
        .await
        .unwrap();
    let denies: Vec<i64> =
        sqlx::query_scalar("SELECT deny FROM permission_overwrites ORDER BY channel_id")
            .fetch_all(&mut c)
            .await
            .unwrap();
    let read_only = Permissions::from_bits_retain(denies[0] as u64);
    assert!(read_only
        .contains(Permissions::SEND_MESSAGES_IN_THREADS | Permissions::CREATE_PUBLIC_THREADS));
    assert_eq!(denies[1], 4);
}

#[tokio::test]
async fn a_thread_from_a_message_previews_on_it_and_stays_out_of_channel_lists() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1, 2, 3]).await;
    let starter = message_of(ok(send(&app, 2, 1, Id(11), "plans for friday").await));
    let mut new = new_thread(11, "Friday");
    new.message_id = Some(starter.id);
    let t = thread_of(ok(thread_cmd(
        &app,
        3,
        2,
        ThreadCommand::Create(new.clone()),
    )
    .await));
    assert_eq!(t.parent_id, Id(11));
    assert_eq!(t.starter_message_id, Some(starter.id));
    // The creator and the starter's author are members.
    assert_eq!(t.member_count, 2);
    // A message gets one thread.
    assert_eq!(
        code(&thread_cmd(&app, 1, 3, ThreadCommand::Create(new)).await),
        Some(ErrorCode::Conflict)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let m = message(&mut c, Id(1), Id(11), starter.id, false)
        .await
        .unwrap();
    let preview = m.extra.thread().cloned().expect("preview");
    assert_eq!((preview.thread_id, preview.message_count), (t.id, 0));
    // Never an ordinary channel: not in the sidebar, Ready or the 32-channel count.
    assert!(channels(&mut c, Id(1), Some(Id(10)))
        .await
        .unwrap()
        .iter()
        .all(|ch| ch.id != t.id));
    assert!(channels(&mut c, Id(1), None)
        .await
        .unwrap()
        .iter()
        .all(|ch| ch.id != t.id));
    drop(c);
    let core = app.0.core.lock().await;
    assert!(events(&core, 1)
        .iter()
        .any(|e| matches!(e, Event::Threads(ThreadEvent::ThreadUpsert(x)) if x.id == t.id)));
    // Not for a connection that never subscribed.
    assert!(core
        .streams
        .values()
        .filter(|s| !s.thread_features)
        .all(|s| s.ring.is_empty()
            || !String::from_utf8_lossy(&s.ring.back().unwrap().2).contains("thread_upsert")));
}

#[tokio::test]
async fn thread_messages_track_unreads_only_for_members_and_mentions_join() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1, 2, 3]).await;
    let t = thread_of(ok(thread_cmd(
        &app,
        2,
        1,
        ThreadCommand::Create(new_thread(11, "Plans")),
    )
    .await))
    .id;
    let mut d = draft(t, "hi <@3>");
    d.mention_ids = vec![Id(3)];
    let m = message_of(ok(run(&app, 1, 2, Command::SendMessage(d)).await));
    assert_eq!(m.channel_id, t);
    // The author and the person they mentioned joined.
    let info = load(&mut app.0.pool.acquire().await.unwrap(), t)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((info.member_count, info.message_count), (3, 1));
    assert_eq!(info.last_message_id, Some(m.id));
    let unread = scalar(
        &app,
        &format!(
            "SELECT unread_count FROM read_states WHERE user_id=2 AND channel_id={}",
            t.0
        ),
    )
    .await;
    assert_eq!(unread, 1);
    // A non-member reading the thread has nothing tracked, and no error.
    ok(thread_cmd(&app, 3, 3, ThreadCommand::Leave { thread_id: t }).await);
    assert_eq!(
        scalar(
            &app,
            &format!(
                "SELECT COUNT(*) FROM read_states WHERE user_id=3 AND channel_id={}",
                t.0
            )
        )
        .await,
        0
    );
    let r = ok(run(
        &app,
        3,
        4,
        Command::MarkRead {
            channel_id: t,
            message_id: m.id,
        },
    )
    .await);
    assert!(matches!(r, CommandResult::ReadState(r) if r.unread_count == 0));
    // A member's notifications follow their thread setting; others only hear mentions.
    let mut c = app.0.pool.acquire().await.unwrap();
    let ch = channel(&mut c, t).await.unwrap();
    let (mut level, mut muted) = (NotificationLevel::All, false);
    notify_level(&mut c, Id(3), &ch, &[], &mut level, &mut muted)
        .await
        .unwrap();
    assert_eq!(level, NotificationLevel::Mentions);
    drop(c);
    ok(thread_cmd(
        &app,
        2,
        5,
        ThreadCommand::SetNotify {
            thread_id: t,
            notify: ThreadNotify::Nothing,
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    notify_level(&mut c, Id(2), &ch, &[], &mut level, &mut muted)
        .await
        .unwrap();
    assert_eq!(level, NotificationLevel::Nothing);
    drop(c);
    // Only members choose notifications.
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                3,
                6,
                ThreadCommand::SetNotify {
                    thread_id: t,
                    notify: ThreadNotify::All
                }
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    // Reactions, replies and pins work in a thread like anywhere else.
    ok(run(
        &app,
        2,
        7,
        Command::AddReaction {
            channel_id: t,
            message_id: m.id,
            emoji: "👍".into(),
        },
    )
    .await);
    let mut reply = draft(t, "agreed");
    reply.reply_to = Some(m.id);
    ok(run(&app, 2, 8, Command::SendMessage(reply)).await);
}

#[tokio::test]
async fn private_threads_are_for_members_and_thread_managers_only() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1, 2, 3]).await;
    let mut new = new_thread(11, "Secret");
    new.private = true;
    new.invitable = false;
    let t = thread_of(ok(thread_cmd(
        &app,
        2,
        1,
        ThreadCommand::Create(new.clone()),
    )
    .await))
    .id;
    // From a message is never private.
    let starter = message_of(ok(send(&app, 2, 2, Id(11), "x").await));
    new.message_id = Some(starter.id);
    assert_eq!(
        code(&thread_cmd(&app, 2, 3, ThreadCommand::Create(new)).await),
        Some(ErrorCode::InvalidRequest)
    );
    ok(send(&app, 2, 4, t, "hidden words").await);
    // User 3 cannot see it at all.
    assert_eq!(
        code(&send(&app, 3, 5, t, "let me in").await),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        code(&thread_cmd(&app, 3, 6, ThreadCommand::Join { thread_id: t }).await),
        Some(ErrorCode::NotFound)
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(!audience(&mut c, t).await.unwrap().contains(&Id(3)));
    let q = MessageSearchQuery {
        q: "hidden".into(),
        before: None,
        limit: Some(10),
    };
    assert!(crate::search::search(&mut c, Id(3), Id(10), &q)
        .await
        .unwrap()
        .0
        .is_empty());
    assert_eq!(
        crate::search::search(&mut c, Id(2), Id(10), &q)
            .await
            .unwrap()
            .0
            .len(),
        1
    );
    drop(c);
    // Not invitable: a plain member cannot add people; the owner can.
    ok(thread_cmd(
        &app,
        2,
        7,
        ThreadCommand::AddMember {
            thread_id: t,
            user_id: Id(3),
        },
    )
    .await);
    ok(send(&app, 3, 8, t, "thanks").await);
    // Members without Manage Threads cannot remove others; a manager can.
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                3,
                9,
                ThreadCommand::RemoveMember {
                    thread_id: t,
                    user_id: Id(2)
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    ok(thread_cmd(
        &app,
        1,
        10,
        ThreadCommand::RemoveMember {
            thread_id: t,
            user_id: Id(3),
        },
    )
    .await);
    assert_eq!(
        code(&send(&app, 3, 11, t, "again").await),
        Some(ErrorCode::NotFound)
    );
    let core = app.0.core.lock().await;
    assert!(events(&core, 3).iter().any(|e| matches!(e, Event::Threads(ThreadEvent::ThreadRemoved { thread_id, .. }) if *thread_id == t)));
    drop(core);
    // Someone with Manage Threads sees every private thread.
    moderator(&app, 3, Permissions::MANAGE_THREADS).await;
    ok(send(&app, 3, 12, t, "moderating").await);
    // A member's mention adds a person who can see the channel, when invitable.
    ok(thread_cmd(
        &app,
        2,
        13,
        ThreadCommand::Update {
            thread_id: t,
            patch: ThreadPatch {
                invitable: Some(true),
                ..Default::default()
            },
        },
    )
    .await);
}

#[tokio::test]
async fn thread_rights_are_checked_on_create_send_lock_archive_and_delete() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1, 2, 3]).await;
    everyone(
        &app,
        Permissions::empty(),
        Permissions::CREATE_PUBLIC_THREADS,
    )
    .await;
    assert_eq!(
        code(&thread_cmd(&app, 2, 1, ThreadCommand::Create(new_thread(11, "Nope"))).await),
        Some(ErrorCode::Forbidden)
    );
    let mut private = new_thread(11, "Private ok");
    private.private = true;
    ok(thread_cmd(&app, 2, 2, ThreadCommand::Create(private)).await);
    everyone(
        &app,
        Permissions::CREATE_PUBLIC_THREADS,
        Permissions::empty(),
    )
    .await;
    // Names, archive choices and thread parents are validated.
    assert_eq!(
        code(&thread_cmd(&app, 2, 3, ThreadCommand::Create(new_thread(11, " "))).await),
        Some(ErrorCode::InvalidRequest)
    );
    let mut odd = new_thread(11, "Odd");
    odd.auto_archive_minutes = 30;
    assert_eq!(
        code(&thread_cmd(&app, 2, 4, ThreadCommand::Create(odd)).await),
        Some(ErrorCode::InvalidRequest)
    );
    let t = thread_of(ok(thread_cmd(
        &app,
        2,
        5,
        ThreadCommand::Create(new_thread(11, "Open")),
    )
    .await))
    .id;
    assert_eq!(
        code(&thread_cmd(&app, 2, 6, ThreadCommand::Create(new_thread(t.0, "Nested"))).await),
        Some(ErrorCode::InvalidRequest)
    );
    // Send Messages in Threads, not Send Messages, decides posting in a thread.
    everyone(
        &app,
        Permissions::empty(),
        Permissions::SEND_MESSAGES_IN_THREADS,
    )
    .await;
    assert_eq!(
        code(&send(&app, 3, 7, t, "no").await),
        Some(ErrorCode::Forbidden)
    );
    everyone(
        &app,
        Permissions::SEND_MESSAGES_IN_THREADS,
        Permissions::empty(),
    )
    .await;
    ok(send(&app, 3, 8, t, "yes").await);
    // Only the owner or a manager archives; only a manager locks.
    let archive = ThreadPatch {
        archived: Some(true),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                3,
                9,
                ThreadCommand::Update {
                    thread_id: t,
                    patch: archive.clone()
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    ok(thread_cmd(
        &app,
        2,
        10,
        ThreadCommand::Update {
            thread_id: t,
            patch: archive,
        },
    )
    .await);
    // Sending reopens an archived (not locked) thread.
    ok(send(&app, 3, 11, t, "back").await);
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT archived FROM threads WHERE channel_id={}", t.0)
        )
        .await,
        0
    );
    let lock = ThreadPatch {
        locked: Some(true),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                12,
                ThreadCommand::Update {
                    thread_id: t,
                    patch: lock.clone()
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    ok(thread_cmd(
        &app,
        1,
        13,
        ThreadCommand::Update {
            thread_id: t,
            patch: lock,
        },
    )
    .await);
    assert_eq!(
        code(&send(&app, 3, 14, t, "locked out").await),
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                15,
                ThreadCommand::Update {
                    thread_id: t,
                    patch: ThreadPatch {
                        name: Some("x".into()),
                        ..Default::default()
                    }
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    ok(send(&app, 1, 16, t, "moderator note").await);
    // Deleting needs Manage Threads.
    assert_eq!(
        code(&thread_cmd(&app, 2, 17, ThreadCommand::Delete { thread_id: t }).await),
        Some(ErrorCode::Forbidden)
    );
    ok(thread_cmd(&app, 1, 18, ThreadCommand::Delete { thread_id: t }).await);
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT COUNT(*) FROM messages WHERE channel_id={}", t.0)
        )
        .await,
        0
    );
}

#[tokio::test]
async fn threads_fit_the_channel_tree_and_go_with_their_parent() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1]).await;
    let t = thread_of(ok(thread_cmd(
        &app,
        1,
        1,
        ThreadCommand::Create(new_thread(11, "Tree")),
    )
    .await))
    .id;
    // Nothing nests in a thread; channel commands do not apply to one.
    let inside = |parent: Id| Command::CreateChannel {
        guild_id: Id(10),
        kind: ChannelKind::Text,
        name: "x".into(),
        parent_id: Some(parent),
        topic: None,
    };
    assert_eq!(
        code(&run(&app, 1, 2, inside(t)).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(&run(&app, 1, 3, Command::DeleteChannel { channel_id: t }).await),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(
            &run(
                &app,
                1,
                4,
                Command::UpdateChannel {
                    channel_id: t,
                    patch: ChannelPatch {
                        name: Patch::Set("y".into()),
                        ..Default::default()
                    }
                }
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(
            &run(
                &app,
                1,
                5,
                Command::Launch(LaunchCommand::SetOverwrites {
                    channel_id: t,
                    overwrites: vec![]
                })
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    // A forum holds posts, not channels.
    let forum = match ok(run(
        &app,
        1,
        6,
        Command::CreateChannel {
            guild_id: Id(10),
            kind: ChannelKind::Forum,
            name: "ideas".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await)
    {
        CommandResult::Channel(c) => c.summary.id,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        code(&run(&app, 1, 7, inside(forum)).await),
        Some(ErrorCode::InvalidRequest)
    );
    // A channel's threads are deleted with it, not moved up like nested channels.
    let child = match ok(run(&app, 1, 8, inside(Id(11))).await) {
        CommandResult::Channel(c) => c.summary.id,
        other => panic!("{other:?}"),
    };
    ok(send(&app, 1, 9, t, "in the thread").await);
    ok(run(&app, 1, 10, Command::DeleteChannel { channel_id: Id(11) }).await);
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT COUNT(*) FROM channels WHERE id={}", t.0)
        )
        .await,
        0
    );
    assert_eq!(
        scalar(
            &app,
            &format!(
                "SELECT COUNT(*) FROM channels WHERE id={} AND parent_id IS NULL",
                child.0
            )
        )
        .await,
        1
    );
    let core = app.0.core.lock().await;
    assert!(events(&core, 1).iter().any(|e| matches!(e, Event::Threads(ThreadEvent::ThreadRemoved { thread_id, .. }) if *thread_id == t)));
}

#[tokio::test]
async fn the_archive_timer_archives_due_threads_and_the_active_limit_holds() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1]).await;
    let t = thread_of(ok(thread_cmd(
        &app,
        1,
        1,
        ThreadCommand::Create(new_thread(11, "Old")),
    )
    .await))
    .id;
    sql(
        &app,
        &format!("UPDATE threads SET archive_at=1 WHERE channel_id={}", t.0),
    )
    .await;
    assert_eq!(archive_due(&app).await.unwrap(), 1);
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT archived FROM threads WHERE channel_id={}", t.0)
        )
        .await,
        1
    );
    assert_eq!(archive_due(&app).await.unwrap(), 0);
    assert!(next_deadline(&app).await.unwrap().is_none());
    // At the limit, no new thread and no reopening.
    sql(&app, "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) SELECT 100000+i,'thread',10,11,'t',0,0 FROM n").await;
    sql(&app, "INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,auto_archive_minutes,created_at,last_activity_at,archive_at) SELECT id,11,10,1,60,0,0,9999999999999 FROM channels WHERE id BETWEEN 100001 AND 101000").await;
    assert_eq!(
        code(&thread_cmd(&app, 1, 2, ThreadCommand::Create(new_thread(11, "Full"))).await),
        Some(ErrorCode::LimitExceeded)
    );
    assert_eq!(
        code(&send(&app, 1, 3, t, "reopen").await),
        Some(ErrorCode::LimitExceeded)
    );
}

async fn make_forum(app: &App, nonce: u64) -> Id {
    match ok(run(
        app,
        1,
        nonce,
        Command::CreateChannel {
            guild_id: Id(10),
            kind: ChannelKind::Forum,
            name: "ideas".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await)
    {
        CommandResult::Channel(c) => c.summary.id,
        other => panic!("{other:?}"),
    }
}
fn post(forum: Id, title: &str, tags: Vec<u32>) -> NewPost {
    NewPost {
        forum_id: forum,
        title: title.into(),
        content: format!("{title} body"),
        attachment_ids: vec![],
        attachments: vec![],
        mention_ids: vec![],
        tag_ids: tags,
        auto_archive_minutes: None,
    }
}
fn tag(name: &str, moderated: bool) -> ForumTag {
    ForumTag {
        id: 0,
        name: name.into(),
        emoji: None,
        moderated,
    }
}

#[tokio::test]
async fn forum_settings_tags_posts_and_their_rights() {
    let (_dir, app) = world().await;
    subscribe(&app, &[1, 2, 3]).await;
    let forum = make_forum(&app, 1).await;
    // Settings are the channel managers'.
    let patch = ForumSettingsPatch {
        tags: Some(vec![tag("bug", false), tag("staff", true)]),
        guidelines: Some("Be kind.".into()),
        require_tags: Some(true),
        default_reaction: Some("👍".into()),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                2,
                ThreadCommand::UpdateForum {
                    forum_id: forum,
                    patch: patch.clone()
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    let s = match ok(thread_cmd(
        &app,
        1,
        3,
        ThreadCommand::UpdateForum {
            forum_id: forum,
            patch,
        },
    )
    .await)
    {
        CommandResult::Threads(ThreadResult::Forum(s)) => s,
        other => panic!("{other:?}"),
    };
    let (bug, staff) = (s.tags[0].id, s.tags[1].id);
    assert_eq!(s.guidelines, "Be kind.");
    // At most 20 tags, unique names, valid emoji.
    let many = ForumSettingsPatch {
        tags: Some((0..21).map(|i| tag(&format!("t{i}"), false)).collect()),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                1,
                4,
                ThreadCommand::UpdateForum {
                    forum_id: forum,
                    patch: many
                }
            )
            .await
        ),
        Some(ErrorCode::LimitExceeded)
    );
    let dup = ForumSettingsPatch {
        tags: Some(vec![tag("a", false), tag("A", false)]),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                1,
                5,
                ThreadCommand::UpdateForum {
                    forum_id: forum,
                    patch: dup
                }
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    let bad = ForumSettingsPatch {
        default_reaction: Some("not an emoji".into()),
        ..Default::default()
    };
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                1,
                6,
                ThreadCommand::UpdateForum {
                    forum_id: forum,
                    patch: bad
                }
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    // Tags are required here; a moderated tag is the moderators'.
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                7,
                ThreadCommand::CreatePost(post(forum, "Untagged", vec![]))
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                8,
                ThreadCommand::CreatePost(post(forum, "Staff", vec![staff]))
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    // Creating a post is Send Messages in the forum, even where replies are off.
    everyone(
        &app,
        Permissions::empty(),
        Permissions::SEND_MESSAGES_IN_THREADS,
    )
    .await;
    let created = ok(thread_cmd(
        &app,
        2,
        9,
        ThreadCommand::CreatePost(post(forum, "Crash on start", vec![bug])),
    )
    .await);
    let CommandResult::Threads(ThreadResult::Post { thread, message }) = created else {
        panic!()
    };
    assert!(thread.forum_post);
    assert_eq!(thread.tag_ids, vec![bug]);
    assert_eq!(message.channel_id, thread.id);
    let first = scalar(
        &app,
        &format!(
            "SELECT first_message_id FROM threads WHERE channel_id={}",
            thread.id.0
        ),
    )
    .await;
    assert_eq!(first as u64, message.id.0);
    assert_eq!(
        code(&send(&app, 2, 10, thread.id, "a reply").await),
        Some(ErrorCode::Forbidden)
    );
    everyone(
        &app,
        Permissions::SEND_MESSAGES_IN_THREADS,
        Permissions::empty(),
    )
    .await;
    ok(send(&app, 3, 11, thread.id, "me too").await);
    // No direct messages in a forum.
    assert_eq!(
        code(&send(&app, 2, 12, forum, "hello forum").await),
        Some(ErrorCode::InvalidRequest)
    );
    // The author edits title and tags; pinning is for moderators.
    ok(thread_cmd(
        &app,
        2,
        13,
        ThreadCommand::Update {
            thread_id: thread.id,
            patch: ThreadPatch {
                name: Some("Crash on startup".into()),
                tag_ids: Some(vec![bug]),
                ..Default::default()
            },
        },
    )
    .await);
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                3,
                14,
                ThreadCommand::Update {
                    thread_id: thread.id,
                    patch: ThreadPatch {
                        name: Some("mine now".into()),
                        ..Default::default()
                    }
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                15,
                ThreadCommand::Update {
                    thread_id: thread.id,
                    patch: ThreadPatch {
                        pinned: Some(true),
                        ..Default::default()
                    }
                }
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    let second = thread_of(ok(thread_cmd(
        &app,
        3,
        16,
        ThreadCommand::CreatePost(post(forum, "Dark mode", vec![bug])),
    )
    .await));
    ok(thread_cmd(
        &app,
        1,
        17,
        ThreadCommand::Update {
            thread_id: thread.id,
            patch: ThreadPatch {
                pinned: Some(true),
                ..Default::default()
            },
        },
    )
    .await);
    // Removing a tag from the forum removes it from posts.
    ok(thread_cmd(
        &app,
        1,
        18,
        ThreadCommand::UpdateForum {
            forum_id: forum,
            patch: ForumSettingsPatch {
                tags: Some(vec![ForumTag {
                    id: bug,
                    name: "bug".into(),
                    emoji: Some("🐛".into()),
                    moderated: false,
                }]),
                ..Default::default()
            },
        },
    )
    .await);
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT COUNT(*) FROM thread_tags WHERE tag_id={staff}")
        )
        .await,
        0
    );
    // The list: pinned first, then by activity; tag filter and search.
    let mut c = app.0.pool.acquire().await.unwrap();
    let q = |sort: Option<ForumSort>, tags: Option<&str>, q: Option<&str>| forum::PostQuery {
        sort,
        tags: tags.map(Into::into),
        q: q.map(Into::into),
        before: None,
        limit: Some(10),
    };
    let page = forum::posts(&mut c, Id(3), forum, q(None, None, None))
        .await
        .unwrap();
    assert_eq!(
        page.posts.iter().map(|p| p.thread.id).collect::<Vec<_>>(),
        [thread.id, second.id]
    );
    assert!(page.posts[0].first.is_some());
    assert_eq!(page.users.len(), 2);
    let page = forum::posts(
        &mut c,
        Id(3),
        forum,
        q(Some(ForumSort::Created), None, Some("dark")),
    )
    .await
    .unwrap();
    assert_eq!(page.posts.len(), 1);
    let page = forum::posts(&mut c, Id(3), forum, q(None, None, Some("body startup")))
        .await
        .unwrap();
    assert_eq!(page.posts.len(), 1);
    assert!(
        forum::posts(&mut c, Id(3), forum, q(None, Some("999"), None))
            .await
            .unwrap()
            .posts
            .is_empty()
    );
    assert!(forum::posts(&mut c, Id(3), forum, q(None, Some("x"), None))
        .await
        .is_err());
    // Search finds a post by its title as well as its body.
    let search = |q: &str| MessageSearchQuery {
        q: q.into(),
        before: None,
        limit: Some(10),
    };
    let found = crate::search::search(&mut c, Id(3), Id(10), &search("startup"))
        .await
        .unwrap()
        .0;
    assert_eq!(found.iter().map(|m| m.id).collect::<Vec<_>>(), [message.id]);
    let found = crate::search::search(&mut c, Id(3), Id(10), &search("in:\"Dark mode\" too"))
        .await
        .unwrap()
        .0;
    assert!(found.is_empty());
    let found = crate::search::search(
        &mut c,
        Id(3),
        Id(10),
        &search("in:\"Crash on startup\" too"),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn forum_slowmode_applies_to_posts_and_replies_but_not_moderators() {
    let (_dir, app) = world().await;
    let forum = make_forum(&app, 1).await;
    ok(thread_cmd(
        &app,
        1,
        2,
        ThreadCommand::UpdateForum {
            forum_id: forum,
            patch: ForumSettingsPatch {
                post_slowmode: Some(600),
                reply_slowmode: Some(600),
                ..Default::default()
            },
        },
    )
    .await);
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                1,
                3,
                ThreadCommand::UpdateForum {
                    forum_id: forum,
                    patch: ForumSettingsPatch {
                        post_slowmode: Some(21_601),
                        ..Default::default()
                    }
                }
            )
            .await
        ),
        Some(ErrorCode::InvalidRequest)
    );
    let p = thread_of(ok(thread_cmd(
        &app,
        2,
        4,
        ThreadCommand::CreatePost(post(forum, "One", vec![])),
    )
    .await));
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                2,
                5,
                ThreadCommand::CreatePost(post(forum, "Two", vec![]))
            )
            .await
        ),
        Some(ErrorCode::RateLimited)
    );
    ok(send(&app, 3, 6, p.id, "first reply").await);
    assert_eq!(
        code(&send(&app, 3, 7, p.id, "second reply").await),
        Some(ErrorCode::RateLimited)
    );
    ok(send(&app, 1, 8, p.id, "owner one").await);
    ok(send(&app, 1, 9, p.id, "owner two").await);
    ok(thread_cmd(
        &app,
        1,
        10,
        ThreadCommand::CreatePost(post(forum, "Mod", vec![])),
    )
    .await);
}

#[tokio::test]
async fn forum_uploads_move_into_the_new_post() {
    let (_dir, app) = world().await;
    let forum = make_forum(&app, 1).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    // The upload gate accepts a forum for people who may post there.
    upload_target(&mut c, Id(2), forum, Permissions::ATTACH_FILES)
        .await
        .unwrap();
    assert!(
        upload_target(&mut c, Id(4), forum, Permissions::ATTACH_FILES)
            .await
            .is_err()
    );
    drop(c);
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,width,height,created_at,expires_at) VALUES(500,2,'message',?,'cover.png','image/png',10,'k500',64,64,0,?)")
        .bind(forum.0 as i64)
        .bind((now() + 600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut new = post(forum, "With a picture", vec![]);
    new.attachment_ids = vec![Id(500)];
    let created = ok(thread_cmd(&app, 2, 2, ThreadCommand::CreatePost(new.clone())).await);
    let CommandResult::Threads(ThreadResult::Post { message, .. }) = created else {
        panic!()
    };
    assert_eq!(message.attachments.len(), 1);
    // Someone else's upload, or one already used, is refused.
    assert_eq!(
        code(&thread_cmd(&app, 3, 3, ThreadCommand::CreatePost(new)).await),
        Some(ErrorCode::InvalidAttachment)
    );
}

async fn make_announcements(app: &App, nonce: u64) -> Id {
    match ok(run(
        app,
        1,
        nonce,
        Command::CreateChannel {
            guild_id: Id(10),
            kind: ChannelKind::Announcement,
            name: "news".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await)
    {
        CommandResult::Channel(c) => c.summary.id,
        other => panic!("{other:?}"),
    }
}
/// Guild 20 (#21) is user 3's, with user 4 as a member who cannot see guild 10.
async fn second_server(app: &App) {
    sql(app, "INSERT INTO guilds VALUES(20,3,'Followers',NULL,0);INSERT INTO guild_members VALUES(20,3,'owner',0),(20,4,'member',0);INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'feed',0,0);INSERT INTO read_states(user_id,channel_id) VALUES(3,21),(4,21)").await;
}

#[tokio::test]
async fn publishing_and_following_deliver_bounded_copies_that_follow_the_source() {
    let (_dir, app) = world().await;
    second_server(&app).await;
    subscribe(&app, &[1, 2, 3, 4]).await;
    let news = make_announcements(&app, 1).await;
    // Announcement channels have read states like text channels.
    assert_eq!(
        scalar(
            &app,
            &format!(
                "SELECT COUNT(*) FROM read_states WHERE channel_id={}",
                news.0
            )
        )
        .await,
        3
    );
    // Following needs Manage Server in the target server (user 4 lacks it)...
    let follow = |source: Id, target: u64| ThreadCommand::Follow {
        source_channel_id: source,
        target_channel_id: Id(target),
    };
    assert_eq!(
        code(&thread_cmd(&app, 4, 2, follow(news, 21)).await),
        Some(ErrorCode::NotFound)
    );
    sql(&app, "INSERT INTO guild_members VALUES(20,2,'member',0);INSERT INTO read_states(user_id,channel_id) VALUES(2,21)").await;
    assert_eq!(
        code(&thread_cmd(&app, 2, 3, follow(news, 21)).await),
        Some(ErrorCode::Forbidden)
    );
    // ...and only announcement channels can be followed, into text channels.
    assert_eq!(
        code(&thread_cmd(&app, 3, 4, follow(Id(11), 21)).await),
        Some(ErrorCode::InvalidRequest)
    );
    let f = match ok(thread_cmd(&app, 3, 5, follow(news, 21)).await) {
        CommandResult::Threads(ThreadResult::Follow(f)) => f,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (f.source_guild_name.as_str(), f.target_channel_name.as_str()),
        ("guild", "feed")
    );
    assert_eq!(
        code(&thread_cmd(&app, 3, 6, follow(news, 21)).await),
        Some(ErrorCode::Conflict)
    );
    // Publishing: your own with Publish Messages, anyone's with Manage Messages.
    let mut d = draft(news, "Launch day <@1>");
    d.mention_ids = vec![Id(1)];
    let m = message_of(ok(run(&app, 2, 7, Command::SendMessage(d)).await));
    let publish = ThreadCommand::Publish {
        channel_id: news,
        message_id: m.id,
    };
    assert_eq!(
        code(&thread_cmd(&app, 2, 8, publish.clone()).await),
        Some(ErrorCode::Forbidden)
    );
    everyone(&app, Permissions::PUBLISH_MESSAGES, Permissions::empty()).await;
    ok(thread_cmd(&app, 2, 9, publish.clone()).await);
    assert_eq!(
        code(&thread_cmd(&app, 2, 10, publish).await),
        Some(ErrorCode::Conflict)
    );
    assert!(
        QUEUED.load(Ordering::Acquire)
            || scalar(&app, "SELECT COUNT(*) FROM crosspost_queue").await == 1
    );
    announce::deliver_queue(&app).await.unwrap();
    let copy: i64 = scalar(&app, "SELECT message_id FROM crossposts").await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = message(&mut c, Id(4), Id(21), Id(copy as u64), false)
        .await
        .unwrap();
    // Only the published message: plain names, the source's names, no link
    // back for someone who cannot read the source.
    assert_eq!(seen.content, "Launch day @user1");
    assert!(seen.mention_ids.is_empty());
    let x = seen.extra.crosspost().cloned().unwrap();
    assert_eq!(
        (x.source_guild_name.as_str(), x.source_channel_name.as_str()),
        ("guild", "news")
    );
    assert!(seen.extra.forwarded.as_ref().unwrap().source.is_none());
    drop(c);
    {
        let core = app.0.core.lock().await;
        assert!(events(&core, 4).iter().any(|e| matches!(e, Event::MessageCreated { message, .. } if message.id.0 == copy as u64 && message.extra.crosspost().is_some())));
    }
    // A copy cannot be edited by hand; it follows its source.
    assert_eq!(
        code(
            &run(
                &app,
                3,
                11,
                Command::EditMessage(MessageEdit {
                    channel_id: Id(21),
                    message_id: Id(copy as u64),
                    content: "forged".into(),
                    mention_ids: vec![]
                })
            )
            .await
        ),
        Some(ErrorCode::Forbidden)
    );
    ok(run(
        &app,
        2,
        12,
        Command::EditMessage(MessageEdit {
            channel_id: news,
            message_id: m.id,
            content: "Launch moved".into(),
            mention_ids: vec![],
        }),
    )
    .await);
    announce::deliver_queue(&app).await.unwrap();
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT COUNT(*) FROM messages WHERE id={copy} AND content='Launch moved'")
        )
        .await,
        1
    );
    ok(run(
        &app,
        2,
        13,
        Command::DeleteMessage {
            channel_id: news,
            message_id: m.id,
        },
    )
    .await);
    announce::deliver_queue(&app).await.unwrap();
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT COUNT(*) FROM messages WHERE id={copy} AND deleted_at IS NOT NULL")
        )
        .await,
        1
    );
    // Ten publishes an hour per channel.
    for i in 0..9 {
        let m = message_of(ok(send(&app, 1, 20 + i, news, "update").await));
        ok(thread_cmd(
            &app,
            1,
            40 + i,
            ThreadCommand::Publish {
                channel_id: news,
                message_id: m.id,
            },
        )
        .await);
    }
    let m = message_of(ok(send(&app, 1, 60, news, "one too many").await));
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                1,
                61,
                ThreadCommand::Publish {
                    channel_id: news,
                    message_id: m.id
                }
            )
            .await
        ),
        Some(ErrorCode::RateLimited)
    );
    // Either side can see and remove the following; nobody else can.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        announce::follows(&mut c, Id(1), news)
            .await
            .unwrap()
            .follows
            .len(),
        1
    );
    assert_eq!(
        announce::follows(&mut c, Id(3), Id(21))
            .await
            .unwrap()
            .follows
            .len(),
        1
    );
    assert!(announce::follows(&mut c, Id(4), Id(21)).await.is_err());
    assert!(announce::follows(&mut c, Id(2), news).await.is_err());
    drop(c);
    assert_eq!(
        code(&thread_cmd(&app, 4, 62, ThreadCommand::Unfollow { follow_id: f.id }).await),
        Some(ErrorCode::NotFound)
    );
    ok(thread_cmd(&app, 1, 63, ThreadCommand::Unfollow { follow_id: f.id }).await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM crosspost_queue").await,
        0
    );
}

#[tokio::test]
async fn following_limits_hold() {
    let (_dir, app) = world().await;
    second_server(&app).await;
    let mut sources = vec![];
    for i in 0..11 {
        sources.push(make_announcements(&app, 1 + i).await);
    }
    sql(
        &app,
        "INSERT INTO guild_members VALUES(10,3,'member',0) ON CONFLICT DO NOTHING",
    )
    .await;
    for (i, s) in sources.iter().take(10).enumerate() {
        ok(thread_cmd(
            &app,
            3,
            20 + i as u64,
            ThreadCommand::Follow {
                source_channel_id: *s,
                target_channel_id: Id(21),
            },
        )
        .await);
    }
    assert_eq!(
        code(
            &thread_cmd(
                &app,
                3,
                40,
                ThreadCommand::Follow {
                    source_channel_id: sources[10],
                    target_channel_id: Id(21)
                }
            )
            .await
        ),
        Some(ErrorCode::LimitExceeded)
    );
}

#[tokio::test]
async fn search_covers_voice_chat_announcements_and_threads() {
    let (_dir, app) = world().await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'voice',10,'lounge',1,0);INSERT INTO read_states(user_id,channel_id) VALUES(1,12),(2,12),(3,12)").await;
    let news = make_announcements(&app, 1).await;
    ok(send(&app, 2, 2, Id(12), "zebra in voice").await);
    ok(send(&app, 2, 3, news, "zebra news").await);
    let t = thread_of(ok(thread_cmd(
        &app,
        2,
        4,
        ThreadCommand::Create(new_thread(11, "Zoo talk")),
    )
    .await))
    .id;
    ok(send(&app, 2, 5, t, "zebra thread").await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let q = |q: &str| MessageSearchQuery {
        q: q.into(),
        before: None,
        limit: Some(10),
    };
    assert_eq!(
        crate::search::search(&mut c, Id(3), Id(10), &q("zebra"))
            .await
            .unwrap()
            .0
            .len(),
        3
    );
    let only = crate::search::search(&mut c, Id(3), Id(10), &q("in:\"Zoo talk\" zebra"))
        .await
        .unwrap()
        .0;
    assert_eq!(only.iter().map(|m| m.channel_id).collect::<Vec<_>>(), [t]);
    assert_eq!(
        crate::search::search(&mut c, Id(3), Id(10), &q("in:lounge zebra"))
            .await
            .unwrap()
            .0
            .len(),
        1
    );
    // Hiding the parent hides its threads from search.
    sql(
        &app,
        &format!(
            "INSERT INTO permission_overwrites VALUES(11,'member',3,0,{})",
            Permissions::VIEW_CHANNEL.bits()
        ),
    )
    .await;
    assert_eq!(
        crate::search::search(&mut c, Id(3), Id(10), &q("zebra"))
            .await
            .unwrap()
            .0
            .len(),
        2
    );
}

#[tokio::test]
async fn rest_reads_list_threads_members_and_details_only_for_people_who_can_see_them() {
    let (_dir, app) = world().await;
    let open = thread_of(ok(thread_cmd(
        &app,
        2,
        1,
        ThreadCommand::Create(new_thread(11, "Open")),
    )
    .await))
    .id;
    let mut private = new_thread(11, "Private");
    private.private = true;
    let hidden = thread_of(ok(
        thread_cmd(&app, 2, 2, ThreadCommand::Create(private)).await
    ))
    .id;
    ok(thread_cmd(
        &app,
        2,
        3,
        ThreadCommand::Update {
            thread_id: open,
            patch: ThreadPatch {
                archived: Some(true),
                ..Default::default()
            },
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let list = |archived: bool| routes_test::ListQuery {
        archived,
        before: None,
        limit: Some(10),
    };
    let active = routes_test::page(&mut c, Id(3), Id(11), list(false))
        .await
        .unwrap();
    assert!(active.threads.is_empty());
    let active = routes_test::page(&mut c, Id(2), Id(11), list(false))
        .await
        .unwrap();
    assert_eq!(
        active.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
        [hidden]
    );
    assert_eq!(active.joined, vec![hidden]);
    let archived = routes_test::page(&mut c, Id(3), Id(11), list(true))
        .await
        .unwrap();
    assert_eq!(
        archived.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
        [open]
    );
    drop(c);
    // Over HTTP: detail and members, NotFound for the hidden one.
    use tower::ServiceExt;
    let token = |u: u8| {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u + 100; 32])
    };
    for u in [2u8, 3] {
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
    let get = |u: u8, path: String| {
        let app = app.clone();
        async move {
            let r = crate::http::router(app)
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .header("authorization", format!("Bearer {}", token(u)))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = r.status();
            (
                status,
                axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap(),
            )
        }
    };
    let (status, body) = get(2, format!("/api/v1/threads/{}", hidden.0)).await;
    assert!(status.is_success());
    let detail: ThreadDetail = serde_json::from_slice(&body).unwrap();
    assert!(detail.me.is_some() && detail.read_state.is_some());
    assert_eq!(
        get(3, format!("/api/v1/threads/{}", hidden.0)).await.0,
        axum::http::StatusCode::NOT_FOUND
    );
    let (_, body) = get(3, format!("/api/v1/threads/{}/members", open.0)).await;
    let members: ThreadMembersPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(members.members.len(), 1);
    assert_eq!(
        get(3, format!("/api/v1/threads/{}/members", hidden.0))
            .await
            .0,
        axum::http::StatusCode::NOT_FOUND
    );
    assert!(get(3, "/api/v1/channels/11/threads?archived=true".into())
        .await
        .0
        .is_success());
}

/// Access to the private REST helpers for the tests above.
mod routes_test {
    pub(super) use super::super::routes::{threads_page as page, ListQuery};
}
