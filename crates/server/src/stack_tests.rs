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
const PATHS: &[&str] = &["forum_post", "publish", "send", "scheduled", "open"];
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
                }),
            )
            .await;
            sql(app, "UPDATE scheduled_messages SET send_at=0").await;
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
