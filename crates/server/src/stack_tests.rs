//! Stack depth of the deepest command paths (docs/features/perf.md).
//!
//! A stack overflow aborts the whole server, so a path that needs more stack
//! than its threads have must fail a test instead. Each path runs in a child
//! test process, in a spawned task on a runtime whose worker threads have a
//! deliberately small stack (the budget): an overflow kills only the child,
//! and this test reports which path it was.
//!
//! `cargo test -p sticktalk-server stack_tests::measure -- --ignored --nocapture`
//! prints the smallest stack each path runs in (a binary search).
use crate::{audit_tests::fixture, commands, App};
use std::process::Command as Process;
use sticktalk_proto::*;

/// What a path may use: 1 MiB in optimized builds, 2 MiB in debug builds
/// (whose frames are several times larger). The server's threads have 8 MiB.
const BUDGET: usize = if cfg!(debug_assertions) {
    2 << 20
} else {
    1 << 20
};
// The integration branch' features: a `/tts` message, sharing an event (a
// message through the send path), an event starting (the schedule timer) and
// a server built from a template. Task `lazy2`: a bot answering a command
// with a message with embeds and components, an activity launched and its
// invite card posted, a server tag edit told to hundreds of wearers.
const PATHS: &[&str] = &[
    "forum_post",
    "publish",
    "send",
    "scheduled",
    "open",
    "tts",
    "event_share",
    "event_start",
    "template",
    "bot_reply",
    "activity_card",
    "tag_fanout",
];
/// A bot's session key and the interaction it answers (`bot_reply`), set by
/// `prepare` in the child process.
static BOT: std::sync::Mutex<Option<(Vec<u8>, Id, InteractionAnswer)>> =
    std::sync::Mutex::new(None);
const WEARERS: std::ops::Range<u64> = 1000..1400;
const PROBE: &str = "STICKTALK_STACK_PROBE";

async fn run(app: &App, u: u64, nonce: u64, command: Command) -> CommandResult {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    match commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest { nonce, command },
    )
    .await
    .unwrap()
    .outcome
    {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("{e:?}"),
    }
}
async fn run_key(app: &App, key: &[u8], u: Id, nonce: u64, command: Command) -> CommandResult {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    match commands::execute(app, &mut core, key, u, CommandRequest { nonce, command })
        .await
        .unwrap()
        .outcome
    {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("{e:?}"),
    }
}
/// A live connection for `key`, so its stream counts as online.
fn connect(core: &mut crate::state::Core, key: &[u8]) {
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    let (close, _) = tokio::sync::watch::channel(None);
    core.streams.get_mut(key).unwrap().connection = Some(crate::state::Connection {
        id: crate::state::stream_id(),
        focused: false,
        alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: std::sync::Arc::new(tokio::sync::Semaphore::new(64 << 20)),
        close,
    });
    // The child process ends after this path: keep the receiver open.
    std::mem::forget(rx);
}
async fn sql(app: &App, q: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(q.to_owned()))
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn channel(app: &App, nonce: u64, kind: ChannelKind) -> Id {
    match run(
        app,
        1,
        nonce,
        Command::CreateChannel {
            guild_id: Id(10),
            kind,
            name: "probe".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await
    {
        CommandResult::Channel(c) => c.summary.id,
        other => panic!("{other:?}"),
    }
}
fn draft(ch: Id, content: &str) -> MessageDraft {
    MessageDraft {
        channel_id: ch,
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![Id(2)],
    }
}

/// Everything a path needs before the measured part, on the test thread.
async fn prepare(app: &App, path: &str) -> Option<(Id, Id)> {
    sql(app, "INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO read_states(user_id,channel_id) VALUES(3,11)").await;
    // Every feature's subscribers, so each fan-out runs.
    for u in [1u64, 2, 3] {
        let n = 900 + u * 10;
        run(app, u, n, Command::Threads(ThreadCommand::Subscribe)).await;
        run(app, u, n + 1, Command::Content(ContentCommand::Subscribe)).await;
        run(app, u, n + 2, Command::Privacy(PrivacyCommand::Subscribe)).await;
        run(
            app,
            u,
            n + 3,
            Command::Expression(ExpressionCommand::Subscribe),
        )
        .await;
        run(
            app,
            u,
            n + 4,
            Command::Community(CommunityCommand::Subscribe),
        )
        .await;
        run(app, u, n + 5, Command::Schedule(ScheduleCommand::Subscribe)).await;
        run(
            app,
            u,
            n + 6,
            Command::Appearance(AppearanceCommand::Subscribe),
        )
        .await;
    }
    match path {
        "forum_post" => Some((channel(app, 1, ChannelKind::Forum).await, Id(0))),
        "publish" => {
            let news = channel(app, 1, ChannelKind::Announcement).await;
            sql(app, "INSERT INTO guilds VALUES(20,3,'Followers',NULL,0);INSERT INTO guild_members VALUES(20,3,'owner',0);INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'feed',0,0);INSERT INTO read_states(user_id,channel_id) VALUES(3,21)").await;
            run(
                app,
                3,
                2,
                Command::Threads(ThreadCommand::Follow {
                    source_channel_id: news,
                    target_channel_id: Id(21),
                }),
            )
            .await;
            let m = match run(app, 1, 3, Command::SendMessage(draft(news, "Launch day"))).await {
                CommandResult::Message(m) => m.id,
                other => panic!("{other:?}"),
            };
            Some((news, m))
        }
        "scheduled" => {
            let at = crate::state::now() + 60_000;
            run(
                app,
                1,
                4,
                Command::Content(ContentCommand::ScheduleMessage {
                    channel_id: Id(11),
                    content: "later <@2>".into(),
                    mention_ids: vec![Id(2)],
                    send_at: at,
                    attachment_ids: vec![],
                    attachments: vec![],
                }),
            )
            .await;
            sql(app, "UPDATE scheduled_messages SET send_at=0").await;
            None
        }
        "event_share" | "event_start" => {
            let at = crate::state::now() + 3_600_000;
            let e = match run(
                app,
                1,
                5,
                Command::Schedule(ScheduleCommand::Create(NewEvent {
                    guild_id: Id(10),
                    name: "Launch party".into(),
                    description: String::new(),
                    start_at: at,
                    end_at: Some(at + 3_600_000),
                    location: EventLocation::External {
                        place: "The park".into(),
                    },
                    recurrence: EventRecurrence::None,
                    time_zone: "UTC".into(),
                })),
            )
            .await
            {
                CommandResult::Schedule(ScheduleResult::Event(e)) => e.id,
                other => panic!("{other:?}"),
            };
            for u in [2u64, 3] {
                run(
                    app,
                    u,
                    60 + u,
                    Command::Schedule(ScheduleCommand::SetInterested {
                        event_id: e,
                        interested: true,
                    }),
                )
                .await;
            }
            if path == "event_start" {
                // Due now: the timer starts it and tells the interested members.
                sql(app, "UPDATE scheduled_events SET next_at=0").await;
            }
            Some((e, Id(0)))
        }
        "bot_reply" => {
            let token = match run(
                app,
                1,
                70,
                Command::Apps(AppCommand::CreateApp(AppDraft {
                    name: "Probe Bot".into(),
                    permissions: Permissions::VIEW_CHANNEL
                        | Permissions::SEND_MESSAGES
                        | Permissions::READ_HISTORY
                        | Permissions::EMBED_LINKS,
                    ..Default::default()
                })),
            )
            .await
            {
                CommandResult::Apps(AppResult::Token { token, .. }) => token,
                other => panic!("{other:?}"),
            };
            let app_id: i64 = sqlx::query_scalar("SELECT id FROM apps")
                .fetch_one(&app.0.pool)
                .await
                .unwrap();
            let app_id = Id(app_id as u64);
            let basic = Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::READ_HISTORY
                | Permissions::EMBED_LINKS;
            run(
                app,
                1,
                72,
                Command::Apps(AppCommand::Install(InstallRequest {
                    app_id,
                    guild_id: Id(10),
                    permissions: basic,
                    message_content: false,
                    confirm_administrator: false,
                })),
            )
            .await;
            let (key, bot) = {
                let mut core = app.0.core.lock().await;
                let mut c = app.0.pool.acquire().await.unwrap();
                let (key, bot) = crate::apps::authenticate(&mut core, &mut c, &token)
                    .await
                    .unwrap();
                connect(&mut core, &key);
                (key, bot)
            };
            let command = match run_key(
                app,
                &key,
                bot,
                73,
                Command::Apps(AppCommand::SetCommands {
                    guild_id: None,
                    commands: vec![CommandDraft {
                        kind: CommandKind::ChatInput,
                        name: "forecast".into(),
                        description: "Weather".into(),
                        options: vec![],
                        default_member_permissions: None,
                    }],
                }),
            )
            .await
            {
                CommandResult::Apps(AppResult::Commands(c)) => c[0].id,
                other => panic!("{other:?}"),
            };
            run(
                app,
                2,
                74,
                Command::Apps(AppCommand::Invoke(Invocation {
                    channel_id: Id(11),
                    command_id: command,
                    options: vec![],
                    target_id: None,
                })),
            )
            .await;
            let answer = {
                let core = app.0.core.lock().await;
                core.streams[&key]
                    .ring
                    .iter()
                    .find_map(
                        |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                            ServerMsg::Event(e) => match e.event {
                                Event::Apps(AppEvent::Interaction(i)) => Some(InteractionAnswer {
                                    interaction_id: i.id,
                                    token: i.token,
                                }),
                                _ => None,
                            },
                            _ => None,
                        },
                    )
                    .unwrap()
            };
            *BOT.lock().unwrap() = Some((key, bot, answer));
            None
        }
        "activity_card" => {
            crate::activities::tests::world(app).await;
            let mut core = app.0.core.lock().await;
            for u in [1, 2] {
                std::mem::forget(crate::activities::tests::attach(&mut core, u));
                crate::activities::tests::join(app, &mut core, u, crate::activities::tests::VOICE)
                    .await;
            }
            None
        }
        "tag_fanout" => {
            run(
                app,
                1,
                80,
                Command::Identity(IdentityCommand::SetTag {
                    guild_id: Id(10),
                    tag: Some(TagEdit {
                        text: "GG".into(),
                        glyph: "leaf".into(),
                        color: 0x1C3A2E,
                        color2: 0x3DDC97,
                        invite_code: None,
                    }),
                }),
            )
            .await;
            run(
                app,
                2,
                81,
                Command::Identity(IdentityCommand::WearTag {
                    guild_id: Some(Id(10)),
                }),
            )
            .await;
            let mut q = String::new();
            for u in WEARERS {
                q.push_str(&format!("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES({u},'wearer{u}','x','online',0); INSERT INTO guild_members VALUES(10,{u},'member',0);"));
            }
            sql(app, &q).await;
            sql(app, "UPDATE users SET tag=(SELECT tag FROM users WHERE id=2),tag_guild=10 WHERE id>=1000").await;
            let mut core = app.0.core.lock().await;
            for u in WEARERS {
                core.cursor(&u.to_be_bytes().repeat(4), Id(u));
            }
            None
        }
        _ => None,
    }
}

/// The measured part: what a worker thread polls in production.
async fn exercise(app: App, path: &'static str, prepared: Option<(Id, Id)>) {
    match path {
        "forum_post" => {
            run(
                &app,
                2,
                50,
                Command::Threads(ThreadCommand::CreatePost(NewPost {
                    forum_id: prepared.unwrap().0,
                    title: "Probe".into(),
                    content: "A post <@1>".into(),
                    attachment_ids: vec![],
                    attachments: vec![],
                    mention_ids: vec![Id(1)],
                    tag_ids: vec![],
                    auto_archive_minutes: None,
                })),
            )
            .await;
        }
        "publish" => {
            let (news, message_id) = prepared.unwrap();
            run(
                &app,
                1,
                51,
                Command::Threads(ThreadCommand::Publish {
                    channel_id: news,
                    message_id,
                }),
            )
            .await;
            crate::threads::deliver_crossposts(&app).await.unwrap();
        }
        "send" => {
            run(
                &app,
                2,
                52,
                Command::SendMessage(draft(Id(11), "hello <@1> :)")),
            )
            .await;
        }
        "scheduled" => crate::content::deliver_scheduled(&app).await.unwrap(),
        "tts" => {
            run(
                &app,
                2,
                53,
                Command::Appearance(AppearanceCommand::SendTts(draft(Id(11), "hello <@1>"))),
            )
            .await;
        }
        "event_share" => {
            run(
                &app,
                1,
                54,
                Command::Schedule(ScheduleCommand::Share {
                    event_id: prepared.unwrap().0,
                    channel_id: Id(11),
                }),
            )
            .await;
        }
        "event_start" => crate::schedule::tick(&app).await.unwrap(),
        "template" => {
            run(
                &app,
                2,
                55,
                Command::Community(CommunityCommand::CreateFromTemplate {
                    template: TemplateRef::Builtin("gaming".into()),
                    name: "Probe".into(),
                }),
            )
            .await;
        }
        "bot_reply" => {
            let (key, bot, answer) = BOT.lock().unwrap().take().unwrap();
            let button = |id: &str| {
                Component::Button(Button {
                    style: ButtonStyle::Primary,
                    label: Some(id.into()),
                    emoji: None,
                    custom_id: Some(id.into()),
                    url: None,
                    disabled: false,
                })
            };
            run_key(
                &app,
                &key,
                bot,
                90,
                Command::Apps(AppCommand::Respond {
                    answer,
                    response: InteractionResponse::Message(AppMessageDraft {
                        content: "Sunny <@2>".into(),
                        embeds: vec![EmbedDraft {
                            title: Some("Oslo".into()),
                            description: Some("**21°** and clear".into()),
                            ..Default::default()
                        }],
                        components: vec![ActionRow {
                            components: vec![button("more"), button("less")],
                        }],
                        ..Default::default()
                    }),
                }),
            )
            .await;
        }
        "activity_card" => {
            use crate::activities::tests::*;
            let mut core = app.0.core.lock().await;
            act(&app, &mut core, 1, launch(VOICE, "whiteboard")).await;
            let session_id = sid(&core, VOICE);
            act(
                &app,
                &mut core,
                1,
                ActivitySignal::Invite {
                    session_id,
                    channel_id: Id(11),
                },
            )
            .await;
        }
        "tag_fanout" => {
            run(
                &app,
                1,
                91,
                Command::Identity(IdentityCommand::SetTag {
                    guild_id: Id(10),
                    tag: Some(TagEdit {
                        text: "GGWP".into(),
                        glyph: "star".into(),
                        color: 0x1C3A2E,
                        color2: 0x3DDC97,
                        invite_code: None,
                    }),
                }),
            )
            .await;
            crate::identity::fanout(&app).await.unwrap();
        }
        // Opening the server (migrations, timers) runs on a runtime thread too.
        "open" => drop(fixture().await),
        other => panic!("unknown path {other}"),
    }
}

/// In a child process: run one path on a thread of `stack` bytes.
fn probe(path: &'static str, stack: usize) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (dir, app) = runtime.block_on(fixture());
    let prepared = runtime.block_on(prepare(&app, path));
    let handle = runtime.handle().clone();
    // The future is on the heap, as a spawned task's is; only its polls use
    // this thread's stack, as they use a worker's in the server.
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || handle.block_on(Box::pin(exercise(app, path, prepared))))
        .unwrap()
        .join()
        .unwrap();
    drop(dir);
}

/// Whether `path` runs in `stack` bytes, in a child process.
fn fits(path: &str, stack: usize) -> bool {
    Process::new(std::env::current_exe().unwrap())
        .args([
            "stack_tests::child",
            "--exact",
            "--ignored",
            "--test-threads=1",
        ])
        .env(PROBE, format!("{path}:{stack}"))
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
#[ignore = "run by deepest_paths_fit_the_stack_budget in a child process"]
fn child() {
    let Ok(spec) = std::env::var(PROBE) else {
        return;
    };
    let (path, stack) = spec.split_once(':').unwrap();
    let path = PATHS.iter().find(|p| **p == path).unwrap();
    probe(path, stack.parse().unwrap());
}

#[test]
fn deepest_paths_fit_the_stack_budget() {
    let failed: Vec<_> = PATHS.iter().filter(|p| !fits(p, BUDGET)).collect();
    assert!(
        failed.is_empty(),
        "these paths overflow a {} KiB stack: {failed:?} (measure with stack_tests::measure)",
        BUDGET >> 10
    );
}

#[test]
#[ignore = "measurement: prints the smallest stack each path needs"]
fn measure() {
    for path in PATHS {
        let (mut lo, mut hi) = (16usize << 10, 16usize << 20);
        assert!(fits(path, hi), "{path} does not fit even {} KiB", hi >> 10);
        while hi - lo > 8 << 10 {
            let mid = (lo + hi) / 2;
            if fits(path, mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        println!("stack {path}: {} KiB", hi >> 10);
    }
}
