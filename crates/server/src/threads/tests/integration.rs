//! Cross-feature rules (`docs/features/integration.md`): moderation, timeouts
//! and the audit log on the thread, forum and announcement paths.
use super::*;

fn blocks(guild: u64, word: &str) -> Command {
    Command::Launch(LaunchCommand::Moderation(
        ModerationCommand::CreateAutoModRule(AutoModRule {
            id: Id(0),
            guild_id: Id(guild),
            name: "No swearing".into(),
            enabled: true,
            trigger: AutoModTrigger::Keyword {
                keywords: vec![word.into()],
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
        }),
    ))
}
#[track_caller]
fn refused(outcome: &CommandOutcome, field: &str) {
    match outcome {
        CommandOutcome::Err(e) => assert_eq!(e.field.as_deref(), Some(field), "{e:?}"),
        CommandOutcome::Ok(r) => panic!("expected a {field} refusal, got {r:?}"),
    }
}
async fn time_out(app: &App, user: u64) {
    sqlx::query("INSERT INTO member_timeouts(guild_id,user_id,until,reason,actor_id) VALUES(10,?,?,'cool off',1)")
        .bind(user as i64)
        .bind((now() + 600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn audit(app: &App, guild: u64, action: AuditAction) -> Vec<(i64, String)> {
    sqlx::query("SELECT actor_id,changes FROM audit_log WHERE guild_id=? AND action=? ORDER BY id")
        .bind(guild as i64)
        .bind(action.name())
        .fetch_all(&app.0.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

#[tokio::test]
async fn thread_names_forum_posts_and_votes_are_moderated_like_messages() {
    let (_dir, app) = world().await;
    sql(&app, "UPDATE users SET created_at=0").await;
    ok(run(&app, 1, 1, blocks(10, "heck")).await);
    // AutoMod covers thread names, forum post titles and first messages.
    refused(
        &thread_cmd(&app, 3, 2, ThreadCommand::Create(new_thread(11, "oh heck"))).await,
        "automod",
    );
    let forum = make_forum(&app, 3).await;
    refused(
        &thread_cmd(
            &app,
            3,
            4,
            ThreadCommand::CreatePost(post(forum, "heck news", vec![])),
        )
        .await,
        "automod",
    );
    let mut clean_title = post(forum, "News", vec![]);
    clean_title.content = "what the heck".into();
    refused(
        &thread_cmd(&app, 3, 5, ThreadCommand::CreatePost(clean_title)).await,
        "automod",
    );
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM threads").await, 0);
    let t = thread_of(ok(thread_cmd(
        &app,
        3,
        6,
        ThreadCommand::Create(new_thread(11, "plans")),
    )
    .await));
    let rename = |name: &str| ThreadCommand::Update {
        thread_id: t.id,
        patch: ThreadPatch {
            name: Some(name.into()),
            ..Default::default()
        },
    };
    refused(
        &thread_cmd(&app, 3, 7, rename("heck plans")).await,
        "automod",
    );
    ok(thread_cmd(&app, 3, 8, rename("better plans")).await);
    // Forum guidelines and tag names, set by a channel manager without Manage Server.
    moderator(
        &app,
        2,
        Permissions::MANAGE_CHANNELS | Permissions::VIEW_CHANNEL,
    )
    .await;
    refused(
        &thread_cmd(
            &app,
            2,
            9,
            ThreadCommand::UpdateForum {
                forum_id: forum,
                patch: ForumSettingsPatch {
                    tags: Some(vec![tag("heck", false)]),
                    ..Default::default()
                },
            },
        )
        .await,
        "automod",
    );

    // A timed-out member keeps reading but cannot create, rename, invite,
    // vote, react or publish.
    let message_id = poll_by(&app, 1, 10).await;
    time_out(&app, 3).await;
    refused(
        &thread_cmd(&app, 3, 11, ThreadCommand::Create(new_thread(11, "fine"))).await,
        "timeout",
    );
    refused(
        &thread_cmd(
            &app,
            3,
            12,
            ThreadCommand::CreatePost(post(forum, "Fine", vec![])),
        )
        .await,
        "timeout",
    );
    refused(
        &thread_cmd(&app, 3, 13, rename("my plans")).await,
        "timeout",
    );
    refused(
        &thread_cmd(
            &app,
            3,
            14,
            ThreadCommand::AddMember {
                thread_id: t.id,
                user_id: Id(2),
            },
        )
        .await,
        "timeout",
    );
    refused(
        &run(
            &app,
            3,
            15,
            Command::Content(ContentCommand::VotePoll {
                channel_id: Id(11),
                message_id,
                answers: vec![0],
            }),
        )
        .await,
        "timeout",
    );
    assert!(code(
        &run(
            &app,
            3,
            16,
            Command::AddReaction {
                channel_id: Id(11),
                message_id,
                emoji: "👍".into(),
            },
        )
        .await
    )
    .is_some());
    // Leaving a thread and reading stay allowed.
    ok(thread_cmd(&app, 3, 17, ThreadCommand::Leave { thread_id: t.id }).await);
}

async fn poll_by(app: &App, u: u64, nonce: u64) -> Id {
    match ok(run(
        app,
        u,
        nonce,
        Command::Content(ContentCommand::CreatePoll(PollDraft {
            channel_id: Id(11),
            question: "Lunch?".into(),
            answers: vec![
                PollAnswerDraft {
                    text: "Yes".into(),
                    emoji: None,
                },
                PollAnswerDraft {
                    text: "No".into(),
                    emoji: None,
                },
            ],
            multiple: false,
            duration_hours: 1,
        })),
    )
    .await)
    {
        CommandResult::Message(m) => m.id,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn moderator_actions_on_threads_forums_announcements_and_polls_are_audited() {
    let (_dir, app) = world().await;
    second_server(&app).await;
    moderator(
        &app,
        2,
        Permissions::MANAGE_THREADS | Permissions::MANAGE_MESSAGES | Permissions::VIEW_CHANNEL,
    )
    .await;
    let t = thread_of(ok(thread_cmd(
        &app,
        3,
        1,
        ThreadCommand::Create(new_thread(11, "plans")),
    )
    .await));
    let patch = |patch: ThreadPatch| ThreadCommand::Update {
        thread_id: t.id,
        patch,
    };
    // The owner managing their own thread is not a moderation action.
    ok(thread_cmd(
        &app,
        3,
        2,
        patch(ThreadPatch {
            name: Some("our plans".into()),
            archived: Some(true),
            ..Default::default()
        }),
    )
    .await);
    assert!(audit(&app, 10, AuditAction::ThreadUpdate).await.is_empty());
    // A moderator's changes are.
    ok(thread_cmd(
        &app,
        2,
        3,
        patch(ThreadPatch {
            archived: Some(false),
            locked: Some(true),
            name: Some("closed plans".into()),
            ..Default::default()
        }),
    )
    .await);
    let e = audit(&app, 10, AuditAction::ThreadUpdate).await;
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].0, 2);
    assert!(
        e[0].1.contains("locked") && e[0].1.contains("closed plans"),
        "{}",
        e[0].1
    );
    ok(thread_cmd(
        &app,
        2,
        4,
        ThreadCommand::RemoveMember {
            thread_id: t.id,
            user_id: Id(3),
        },
    )
    .await);
    assert_eq!(
        audit(&app, 10, AuditAction::ThreadMemberRemove).await.len(),
        1
    );
    ok(thread_cmd(&app, 2, 5, ThreadCommand::Delete { thread_id: t.id }).await);
    assert_eq!(audit(&app, 10, AuditAction::ThreadDelete).await.len(), 1);

    // Forum settings and tags.
    let forum = make_forum(&app, 6).await;
    ok(thread_cmd(
        &app,
        1,
        7,
        ThreadCommand::UpdateForum {
            forum_id: forum,
            patch: ForumSettingsPatch {
                guidelines: Some("Be kind".into()),
                tags: Some(vec![tag("Bug", true)]),
                ..Default::default()
            },
        },
    )
    .await);
    let e = audit(&app, 10, AuditAction::ForumUpdate).await;
    assert_eq!(e.len(), 1);
    assert!(e[0].1.contains("Be kind") && e[0].1.contains("Bug (moderators only)"));

    // Publishing, following and unfollowing (in the follower's server).
    let news = make_announcements(&app, 8).await;
    let m = message_of(ok(send(&app, 1, 9, news, "Launch day").await));
    ok(thread_cmd(
        &app,
        1,
        10,
        ThreadCommand::Publish {
            channel_id: news,
            message_id: m.id,
        },
    )
    .await);
    assert_eq!(audit(&app, 10, AuditAction::MessagePublish).await.len(), 1);
    let f = match ok(thread_cmd(
        &app,
        3,
        11,
        ThreadCommand::Follow {
            source_channel_id: news,
            target_channel_id: Id(21),
        },
    )
    .await)
    {
        CommandResult::Threads(ThreadResult::Follow(f)) => f,
        other => panic!("{other:?}"),
    };
    assert_eq!(audit(&app, 20, AuditAction::ChannelFollow).await.len(), 1);
    // Removed from the announcement side: both servers' logs.
    ok(thread_cmd(&app, 1, 12, ThreadCommand::Unfollow { follow_id: f.id }).await);
    assert_eq!(audit(&app, 20, AuditAction::ChannelUnfollow).await.len(), 1);
    assert_eq!(audit(&app, 10, AuditAction::ChannelUnfollow).await.len(), 1);

    // A moderator ending someone else's poll; the author ending their own is not logged.
    let theirs = poll_by(&app, 3, 13).await;
    let own = poll_by(&app, 3, 14).await;
    let end = |message_id| {
        Command::Content(ContentCommand::EndPoll {
            channel_id: Id(11),
            message_id,
        })
    };
    ok(run(&app, 2, 15, end(theirs)).await);
    ok(run(&app, 3, 16, end(own)).await);
    let e = audit(&app, 10, AuditAction::PollEnd).await;
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].0, 2);
}

#[tokio::test]
async fn crossposts_are_screened_by_the_receiving_servers_automod() {
    let (_dir, app) = world().await;
    second_server(&app).await;
    let news = make_announcements(&app, 1).await;
    ok(thread_cmd(
        &app,
        3,
        2,
        ThreadCommand::Follow {
            source_channel_id: news,
            target_channel_id: Id(21),
        },
    )
    .await);
    // Guild 20 (user 3's) blocks a word guild 10 allows.
    ok(run(&app, 3, 3, blocks(20, "heck")).await);
    let publish = |app: &App, nonce: u64, text: &'static str| {
        let app = app.clone();
        async move {
            let m = message_of(ok(send(&app, 1, nonce, news, text).await));
            ok(thread_cmd(
                &app,
                1,
                nonce + 1,
                ThreadCommand::Publish {
                    channel_id: news,
                    message_id: m.id,
                },
            )
            .await);
            announce::deliver_queue(&app).await.unwrap();
            m
        }
    };
    publish(&app, 4, "what the heck").await;
    // Not delivered, nothing left queued, and guild 20's log says why.
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM crossposts").await, 0);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM crosspost_queue").await,
        0
    );
    let e = audit(&app, 20, AuditAction::AutomodBlock).await;
    assert_eq!(e.len(), 1);
    assert!(e[0].1.contains("guild › #news"), "{}", e[0].1);
    assert!(audit(&app, 10, AuditAction::AutomodBlock).await.is_empty());
    // A clean one arrives; editing it into a blocked one removes the copy.
    let m = publish(&app, 6, "launch day").await;
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM crossposts").await, 1);
    ok(run(
        &app,
        1,
        8,
        Command::EditMessage(MessageEdit {
            channel_id: news,
            message_id: m.id,
            content: "launch heck".into(),
            mention_ids: vec![],
        }),
    )
    .await);
    announce::deliver_queue(&app).await.unwrap();
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM messages WHERE channel_id=21 AND deleted_at IS NULL"
        )
        .await,
        0
    );
    assert_eq!(audit(&app, 20, AuditAction::AutomodBlock).await.len(), 2);
}
