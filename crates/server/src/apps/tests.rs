//! Server tests for apps, bots, commands, interactions and webhooks. Every
//! permission check and limit added in `apps/` has a test here.
use super::*;
use crate::{audit_tests::fixture, state::Connection};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use std::sync::atomic::{AtomicU64, Ordering};
use tower::ServiceExt;

static NONCE: AtomicU64 = AtomicU64::new(1);

fn key(u: u64) -> Vec<u8> {
    vec![u as u8; 32]
}
async fn run(app: &App, core: &mut Core, key: &[u8], u: Id, command: Command) -> CommandOutcome {
    core.limits.clear();
    crate::commands::execute(
        app,
        core,
        key,
        u,
        CommandRequest {
            nonce: NONCE.fetch_add(1, Ordering::Relaxed),
            command,
        },
    )
    .await
    .unwrap()
    .outcome
}
async fn as_user(app: &App, core: &mut Core, u: u64, command: Command) -> CommandOutcome {
    run(app, core, &key(u), Id(u), command).await
}
fn ok(o: CommandOutcome) -> CommandResult {
    match o {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("expected success, got {e:?}"),
    }
}
fn err(o: CommandOutcome) -> ApiError {
    match o {
        CommandOutcome::Err(e) => e,
        CommandOutcome::Ok(r) => panic!("expected an error, got {r:?}"),
    }
}
fn apps(r: CommandResult) -> AppResult {
    match r {
        CommandResult::Apps(a) => a,
        other => panic!("not an app result: {other:?}"),
    }
}
fn events(core: &Core, key: &[u8]) -> Vec<Event> {
    core.streams
        .get(key)
        .map(|s| {
            s.ring
                .iter()
                .filter_map(
                    |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                        ServerMsg::Event(e) => Some(e.event),
                        _ => None,
                    },
                )
                .collect()
        })
        .unwrap_or_default()
}
fn clear(core: &mut Core, key: &[u8]) {
    if let Some(s) = core.streams.get_mut(key) {
        s.clear_replay();
    }
}
/// A live connection, so the stream counts as online.
fn connect(
    core: &mut Core,
    key: &[u8],
    keep: &mut Vec<tokio::sync::mpsc::Receiver<crate::state::Packet>>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    let (close, _) = tokio::sync::watch::channel(None);
    let s = core.streams.get_mut(key).unwrap();
    s.connection = Some(Connection {
        id: crate::state::stream_id(),
        focused: false,
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        tx,
        budget: Arc::new(tokio::sync::Semaphore::new(64 * 1024 * 1024)),
        close,
    });
    keep.push(rx);
}

struct Bot {
    app: Id,
    user: Id,
    key: Vec<u8>,
    token: String,
}

async fn create_app(
    app: &App,
    core: &mut Core,
    owner: u64,
    intents: Intents,
    permissions: Permissions,
) -> Bot {
    let r = apps(ok(as_user(
        app,
        core,
        owner,
        Command::Apps(AppCommand::CreateApp(AppDraft {
            name: "Weather Bot".into(),
            description: "Forecasts".into(),
            public: true,
            intents,
            permissions,
            ..Default::default()
        })),
    )
    .await));
    let AppResult::Token { app: info, token } = r else {
        panic!("no token");
    };
    let raw = token.strip_prefix(BOT_TOKEN_PREFIX).unwrap();
    Bot {
        app: info.id,
        user: info.bot.id,
        key: crate::state::hash(raw.as_bytes()),
        token,
    }
}
const BASIC: Permissions = Permissions::from_bits_retain(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::READ_HISTORY.bits()
        | Permissions::EMBED_LINKS.bits(),
);
async fn install(
    app: &App,
    core: &mut Core,
    bot: &Bot,
    by: u64,
    permissions: Permissions,
    content: bool,
) -> CommandOutcome {
    as_user(
        app,
        core,
        by,
        Command::Apps(AppCommand::Install(InstallRequest {
            app_id: bot.app,
            guild_id: Id(10),
            permissions,
            message_content: content,
            confirm_administrator: false,
        })),
    )
    .await
}
async fn as_bot(app: &App, core: &mut Core, bot: &Bot, command: Command) -> CommandOutcome {
    run(app, core, &bot.key.clone(), bot.user, command).await
}
async fn online(
    app: &App,
    core: &mut Core,
    bot: &Bot,
    keep: &mut Vec<tokio::sync::mpsc::Receiver<crate::state::Packet>>,
) {
    let mut c = app.0.pool.acquire().await.unwrap();
    let (k, u) = authenticate(core, &mut c, &bot.token).await.unwrap();
    assert_eq!((k.as_slice(), u), (bot.key.as_slice(), bot.user));
    connect(core, &bot.key, keep);
}
fn say(content: &str) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: Id(11),
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    })
}
async fn send(app: &App, r: Request<Body>) -> axum::response::Response {
    app.0.core.lock().await.limits.clear();
    crate::http::router(app.clone()).oneshot(r).await.unwrap()
}
async fn json(r: axum::response::Response) -> serde_json::Value {
    let b = to_bytes(r.into_body(), 1 << 22).await.unwrap();
    serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null)
}
async fn audit_actions(app: &App) -> Vec<String> {
    sqlx::query_scalar("SELECT action FROM audit_log WHERE guild_id=10 ORDER BY id")
        .fetch_all(&app.0.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn bots_are_accounts_that_cannot_sign_in_and_tokens_work_only_where_allowed() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&key(1), Id(1));
    let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let r = sqlx::query("SELECT username,password_hash,style FROM users WHERE id=?")
        .bind(bot.user.0 as i64)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let name: String = r.get("username");
    assert!(r.get::<String, _>("password_hash").is_empty());
    assert!(
        username(&name).is_err(),
        "no person can register or sign in as {name}"
    );
    assert!(
        user(&mut c, bot.user).await.unwrap().style.bot,
        "marked APP"
    );
    // The token is never a person's session.
    let raw = bot.token.strip_prefix(BOT_TOKEN_PREFIX).unwrap();
    assert!(auth(&mut c, raw).await.is_err());
    assert!(auth(&mut c, &bot.token).await.is_err());
    assert!(authenticate(&mut core, &mut c, &bot.token).await.is_ok());
    assert!(authenticate(
        &mut core,
        &mut c,
        "stkb_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    )
    .await
    .is_err());
    drop(c);
    drop(core);
    let login = send(
        &app,
        Request::post("/api/v1/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"username": name, "password": "x"}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(login.status(), 401);
    for auth_header in [format!("Bearer {raw}"), format!("Bot {}", bot.token)] {
        let me = send(
            &app,
            Request::get("/api/v1/users/@me/server-folders")
                .header("authorization", auth_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(me.status(), 401, "the bot token works on no person's route");
    }
    let me = send(
        &app,
        Request::get("/api/v1/bot/@me")
            .header("authorization", format!("Bot {}", bot.token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(me.status(), 200);
    assert_eq!(json(me).await["user"]["id"], bot.user.0);
    // A reset token replaces the old one at once; only the owner may reset.
    let mut core = app.0.core.lock().await;
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::ResetToken { app_id: bot.app })
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    let AppResult::Token { token, .. } = apps(ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::ResetToken { app_id: bot.app }),
    )
    .await)) else {
        panic!()
    };
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(authenticate(&mut core, &mut c, &bot.token).await.is_err());
    assert!(authenticate(&mut core, &mut c, &token).await.is_ok());
    // The instance administrator can turn apps off, or allow only some people.
    admin::run(
        &mut c,
        &admin::AppsAdmin::Creation {
            policy: "allowlist".into(),
        },
    )
    .await
    .unwrap();
    let refused = err(as_user(
        &app,
        &mut core,
        2,
        Command::Apps(AppCommand::CreateApp(AppDraft {
            name: "X".into(),
            ..Default::default()
        })),
    )
    .await);
    assert_eq!(refused.field.as_deref(), Some("apps_creation"));
    admin::run(
        &mut c,
        &admin::AppsAdmin::Allow {
            username: "user2".into(),
        },
    )
    .await
    .unwrap();
    ok(as_user(
        &app,
        &mut core,
        2,
        Command::Apps(AppCommand::CreateApp(AppDraft {
            name: "X".into(),
            ..Default::default()
        })),
    )
    .await);
    admin::run(
        &mut c,
        &admin::AppsAdmin::Creation {
            policy: "disabled".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        authenticate(&mut core, &mut c, &token).await.is_err(),
        "no bot connects while apps are off"
    );
    admin::run(
        &mut c,
        &admin::AppsAdmin::Creation {
            policy: "everyone".into(),
        },
    )
    .await
    .unwrap();
    admin::run(&mut c, &admin::AppsAdmin::Disable { app_id: bot.app.0 })
        .await
        .unwrap();
    assert!(
        authenticate(&mut core, &mut c, &token).await.is_err(),
        "a disabled app's bot is refused"
    );
    admin::run(&mut c, &admin::AppsAdmin::Enable { app_id: bot.app.0 })
        .await
        .unwrap();
    assert!(authenticate(&mut core, &mut c, &token).await.is_ok());
    // Names and limits.
    for bad in ["", &"n".repeat(33)] {
        assert_eq!(
            err(as_user(
                &app,
                &mut core,
                3,
                Command::Apps(AppCommand::CreateApp(AppDraft {
                    name: bad.into(),
                    ..Default::default()
                }))
            )
            .await)
            .code,
            ErrorCode::InvalidRequest
        );
    }
}

#[tokio::test]
async fn installing_grants_exactly_what_the_installer_may_and_removal_takes_only_the_app() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let wanted = BASIC | Permissions::MANAGE_ROLES | Permissions::ADMINISTRATOR;
    let bot = create_app(&app, &mut core, 1, Intents::empty(), wanted).await;
    // A member without Manage Server can't install.
    assert_eq!(
        err(install(&app, &mut core, &bot, 2, BASIC, false).await).code,
        ErrorCode::Forbidden
    );
    // A manager can't grant what they lack, what the app didn't ask for, or Administrator.
    sqlx::query(
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,50,'Managers',2,?)",
    )
    .bind((Permissions::MANAGE_GUILD | BASIC).bits() as i64)
    .execute(&app.0.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO member_roles VALUES(10,2,50)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        err(install(
            &app,
            &mut core,
            &bot,
            2,
            BASIC | Permissions::MANAGE_ROLES,
            false
        )
        .await)
        .field
        .as_deref(),
        Some("permissions")
    );
    assert_eq!(
        err(install(
            &app,
            &mut core,
            &bot,
            1,
            BASIC | Permissions::KICK_MEMBERS,
            false
        )
        .await)
        .field
        .as_deref(),
        Some("permissions")
    );
    assert_eq!(
        err(install(
            &app,
            &mut core,
            &bot,
            1,
            BASIC | Permissions::ADMINISTRATOR,
            false
        )
        .await)
        .field
        .as_deref(),
        Some("administrator")
    );
    assert_eq!(
        err(install(&app, &mut core, &bot, 1, BASIC, true).await)
            .field
            .as_deref(),
        Some("message_content"),
        "content needs the intent"
    );
    let AppResult::Installation(inst) =
        apps(ok(install(&app, &mut core, &bot, 2, BASIC, false).await))
    else {
        panic!()
    };
    let managed = inst.role_id.unwrap();
    assert_eq!(inst.permissions, BASIC);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(crate::permissions::role_ids(&mut c, Id(10), bot.user)
        .await
        .unwrap()
        .contains(&managed));
    assert_eq!(
        err(install(&app, &mut core, &bot, 1, BASIC, false).await).code,
        ErrorCode::AlreadyExists
    );
    // The managed role stays with the app.
    let guarded = [
        Command::Launch(LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: managed,
        }),
        Command::Launch(LaunchCommand::SetMemberRoles {
            guild_id: Id(10),
            user_id: bot.user,
            role_ids: vec![],
        }),
        Command::Launch(LaunchCommand::SetMemberRoles {
            guild_id: Id(10),
            user_id: Id(2),
            role_ids: vec![Id(50), managed],
        }),
        Command::OpenDm { user_id: bot.user },
        Command::TransferGuildOwnership {
            guild_id: Id(10),
            user_id: bot.user,
        },
    ];
    for cmd in guarded {
        assert_eq!(
            err(as_user(&app, &mut core, 1, cmd).await).code,
            ErrorCode::Forbidden
        );
    }
    // The bot posts; then the app is removed: member, role and commands go, messages stay.
    let m = match ok(as_bot(&app, &mut core, &bot, say("forecast: sun")).await) {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::SetCommands {
            guild_id: Some(Id(10)),
            commands: vec![CommandDraft {
                kind: CommandKind::ChatInput,
                name: "ping".into(),
                description: "Ping".into(),
                options: vec![],
                default_member_permissions: None,
            }],
        }),
    )
    .await);
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            3,
            Command::Apps(AppCommand::Uninstall {
                guild_id: Id(10),
                app_id: bot.app
            })
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    ok(as_user(
        &app,
        &mut core,
        2,
        Command::Apps(AppCommand::Uninstall {
            guild_id: Id(10),
            app_id: bot.app,
        }),
    )
    .await);
    assert!(role(&mut c, bot.user, Id(10)).await.is_err());
    assert!(crate::permissions::roles(&mut c, Id(10))
        .await
        .unwrap()
        .iter()
        .all(|r| r.id != managed));
    assert!(registry::commands(&mut c, bot.app, Some(Id(10)))
        .await
        .unwrap()
        .iter()
        .all(|c| c.guild_id.is_none()));
    assert!(
        message(&mut c, Id(1), Id(11), m.id, false).await.is_ok(),
        "its messages stay"
    );
    let actions = audit_actions(&app).await;
    assert!(
        actions.contains(&"app_install".to_owned()) && actions.contains(&"app_remove".to_owned()),
        "{actions:?}"
    );
    // Kicking a bot removes the app too (Discord's behavior), and its
    // managed role with it, in the same command, with an audit entry.
    let AppResult::Installation(again) =
        apps(ok(install(&app, &mut core, &bot, 1, BASIC, false).await))
    else {
        panic!()
    };
    let managed = again.role_id.unwrap();
    core.cursor(&key(2), Id(2));
    clear(&mut core, &key(2));
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::KickMember {
            guild_id: Id(10),
            user_id: bot.user,
        },
    )
    .await);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_installs")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert!(
        crate::permissions::roles(&mut c, Id(10))
            .await
            .unwrap()
            .iter()
            .all(|r| r.id != managed),
        "the managed role went with the bot"
    );
    let drops: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_role_drops")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(drops, 0);
    // Members are told (the server's roles and integrations changed).
    let seen = events(&core, &key(2));
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Apps(AppEvent::IntegrationsUpdated { .. }))),
        "{seen:?}"
    );
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_log WHERE guild_id=10 AND action='app_remove' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    assert!(reason.unwrap().contains("its role with it"));
}

#[tokio::test]
async fn bots_get_the_same_checks_as_members_and_no_user_only_features() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let bot = create_app(&app, &mut core, 1, Intents::GUILD_MESSAGES, BASIC).await;
    ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
    ok(as_bot(&app, &mut core, &bot, say("hello")).await);
    for cmd in [
        Command::OpenDm { user_id: Id(1) },
        Command::UpdateProfile(ProfilePatch::default()),
        Command::CreateGuild {
            name: "mine".into(),
            icon_id: None,
        },
        Command::SendFriendRequest {
            username: "user1".into(),
        },
        Command::LeaveGuild { guild_id: Id(10) },
        Command::Apps(AppCommand::CreateApp(AppDraft {
            name: "child".into(),
            ..Default::default()
        })),
        Command::Apps(AppCommand::Invoke(Invocation {
            channel_id: Id(11),
            command_id: Id(1),
            options: vec![],
            target_id: None,
        })),
        Command::Privacy(PrivacyCommand::Subscribe),
    ] {
        assert_eq!(
            err(as_bot(&app, &mut core, &bot, cmd).await)
                .field
                .as_deref(),
            Some("bot")
        );
    }
    // A person can't send bot-only or webhook-only commands.
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::Send {
                channel_id: Id(11),
                draft: AppMessageDraft {
                    content: "x".into(),
                    ..Default::default()
                }
            })
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::ExecuteWebhook(Box::new(WebhookPost {
                channel_id: Id(11),
                content: "x".into(),
                ..Default::default()
            })))
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    // AutoMod screens a bot's embeds too.
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::CreateAutoModRule(AutoModRule {
                id: Id(0),
                guild_id: Id(10),
                name: "words".into(),
                enabled: true,
                trigger: AutoModTrigger::Keyword {
                    keywords: vec!["forbidden".into()],
                    allow: vec![],
                },
                actions: AutoModActions {
                    block: true,
                    ..Default::default()
                },
                exempt_roles: vec![],
                exempt_channels: vec![],
            }),
        )),
    )
    .await);
    let blocked = err(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Send {
            channel_id: Id(11),
            draft: AppMessageDraft {
                embeds: vec![EmbedDraft {
                    title: Some("a forbidden title".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }),
    )
    .await);
    assert_eq!(blocked.field.as_deref(), Some("automod"));
    // An embed-only message is fine.
    let m = match apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Send {
            channel_id: Id(11),
            draft: AppMessageDraft {
                embeds: vec![EmbedDraft {
                    title: Some("Sunny".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }),
    )
    .await))
    {
        AppResult::Message(m) => m,
        _ => panic!(),
    };
    assert!(m.content.is_empty() && m.extra.app.as_ref().unwrap().embeds.len() == 1);
    // A timeout applies to the bot's member.
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::Timeout {
            guild_id: Id(10),
            user_id: bot.user,
            seconds: 600,
            reason: None,
        })),
    )
    .await);
    assert_eq!(
        err(as_bot(&app, &mut core, &bot, say("still here")).await)
            .field
            .as_deref(),
        Some("timeout")
    );
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::Timeout {
            guild_id: Id(10),
            user_id: bot.user,
            seconds: 0,
            reason: None,
        })),
    )
    .await);
    // Without Send Messages in a channel, the bot can't post there.
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: bot.user,
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            }],
        }),
    )
    .await);
    assert_eq!(
        err(as_bot(&app, &mut core, &bot, say("hi")).await).code,
        ErrorCode::Forbidden
    );
    // Bots don't receive direct messages.
    assert_eq!(
        err(as_user(&app, &mut core, 1, Command::OpenDm { user_id: bot.user }).await)
            .field
            .as_deref(),
        Some("bot")
    );
}

#[tokio::test]
async fn intents_decide_what_a_bot_receives_and_content_needs_a_grant() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let mut keep = vec![];
    core.cursor(&key(2), Id(2));
    let quiet = create_app(&app, &mut core, 1, Intents::GUILD_MESSAGES, BASIC).await;
    ok(install(&app, &mut core, &quiet, 1, BASIC, false).await);
    online(&app, &mut core, &quiet, &mut keep).await;
    let reader = create_app(
        &app,
        &mut core,
        1,
        Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
        BASIC,
    )
    .await;
    ok(install(&app, &mut core, &reader, 1, BASIC, true).await);
    online(&app, &mut core, &reader, &mut keep).await;
    let deaf = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    ok(install(&app, &mut core, &deaf, 1, BASIC, false).await);
    online(&app, &mut core, &deaf, &mut keep).await;
    for b in [&quiet, &reader, &deaf] {
        clear(&mut core, &b.key);
    }
    ok(as_user(&app, &mut core, 2, say("secret plans")).await);
    ok(as_user(
        &app,
        &mut core,
        2,
        Command::SendMessage(MessageDraft {
            channel_id: Id(11),
            content: format!("<@{}> weather?", quiet.user.0),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![quiet.user],
        }),
    )
    .await);
    let texts = |core: &Core, k: &[u8]| -> Vec<String> {
        events(core, k)
            .into_iter()
            .filter_map(|e| match e {
                Event::MessageCreated { message, .. } => Some(message.content),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        texts(&core, &quiet.key),
        vec![String::new(), format!("<@{}> weather?", quiet.user.0)]
    );
    assert_eq!(
        texts(&core, &reader.key),
        vec![
            "secret plans".to_owned(),
            format!("<@{}> weather?", quiet.user.0)
        ]
    );
    assert!(
        texts(&core, &deaf.key).is_empty(),
        "no message events without the intent"
    );
    assert!(events(&core, &quiet.key)
        .iter()
        .all(|e| !matches!(e, Event::ReadStateUpdated(_) | Event::Typing { .. })));
    // History over REST withholds content the same way.
    drop(core);
    let page = json(
        send(
            &app,
            Request::get("/api/v1/bot/channels/11/messages")
                .header("authorization", format!("Bot {}", quiet.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    let contents: Vec<&str> = page
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert!(
        contents.contains(&"") && !contents.contains(&"secret plans"),
        "{contents:?}"
    );
    // Members need Server Members.
    let r = send(
        &app,
        Request::get("/api/v1/bot/guilds/10/members")
            .header("authorization", format!("Bot {}", quiet.token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 403);
    // A view of a member event without the intent.
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let m = member(&mut c, Id(10), Id(2)).await.unwrap();
    clear(&mut core, &quiet.key);
    core.emit(&[quiet.user], Event::MemberUpsert(m)).unwrap();
    assert!(events(&core, &quiet.key).is_empty());
    // Message content in other events (a link's preview, a poll's text) is
    // treated like a message: only the bot granted Message Content there gets
    // it; profile text and the features' events reach no bot (deny by default).
    let preview = Event::Content(ContentEvent::EmbedsUpdated {
        channel_id: Id(11),
        message_id: Id(1),
        embeds: vec![LinkEmbed {
            url: "https://internal.example/plan-q3".into(),
            site_name: None,
            title: Some("Q3 plan".into()),
            description: Some("Launch dates".into()),
            image: None,
            video: false,
        }],
    });
    let profile = Event::ProfileUpdated(UserProfile {
        user: user(&mut c, Id(2)).await.unwrap(),
        banner_color: 0,
        pronouns: String::new(),
        about_me: "My private bio".into(),
        created_at: 0,
    });
    for b in [&quiet, &reader, &deaf] {
        clear(&mut core, &b.key);
        // A bot that sent a content command (a poll) gets content events.
        core.streams.get_mut(&b.key).unwrap().content_features = true;
        core.emit(&[b.user], preview.clone()).unwrap();
        core.emit(&[b.user], profile.clone()).unwrap();
    }
    assert!(events(&core, &quiet.key).is_empty(), "no grant: no preview");
    assert!(events(&core, &deaf.key).is_empty());
    assert_eq!(events(&core, &reader.key), vec![preview]);
}

fn ping_commands() -> Vec<CommandDraft> {
    vec![
        CommandDraft {
            kind: CommandKind::ChatInput,
            name: "forecast".into(),
            description: "Weather".into(),
            options: vec![
                CommandOption {
                    kind: OptionKind::String,
                    name: "city".into(),
                    description: "City".into(),
                    required: true,
                    choices: vec![],
                    options: vec![],
                    min_value: None,
                    max_value: None,
                    min_length: Some(2),
                    max_length: Some(40),
                    autocomplete: true,
                },
                CommandOption {
                    kind: OptionKind::Integer,
                    name: "days".into(),
                    description: "Days".into(),
                    required: false,
                    choices: vec![],
                    options: vec![],
                    min_value: Some(Number(1.)),
                    max_value: Some(Number(7.)),
                    min_length: None,
                    max_length: None,
                    autocomplete: false,
                },
                CommandOption {
                    kind: OptionKind::User,
                    name: "for".into(),
                    description: "For".into(),
                    required: false,
                    choices: vec![],
                    options: vec![],
                    min_value: None,
                    max_value: None,
                    min_length: None,
                    max_length: None,
                    autocomplete: false,
                },
            ],
            default_member_permissions: None,
        },
        CommandDraft {
            kind: CommandKind::ChatInput,
            name: "purge".into(),
            description: "Admins".into(),
            options: vec![],
            default_member_permissions: Some(Permissions::BAN_MEMBERS),
        },
        CommandDraft {
            kind: CommandKind::User,
            name: "Wave".into(),
            description: String::new(),
            options: vec![],
            default_member_permissions: None,
        },
        CommandDraft {
            kind: CommandKind::Message,
            name: "Quote".into(),
            description: String::new(),
            options: vec![],
            default_member_permissions: None,
        },
    ]
}
fn names(cat: &CommandCatalog) -> Vec<String> {
    cat.apps
        .iter()
        .flat_map(|a| a.commands.iter().map(|c| c.name.clone()))
        .collect()
}

#[tokio::test]
async fn commands_are_validated_and_the_picker_shows_only_what_you_may_use() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
    let set = |commands| {
        Command::Apps(AppCommand::SetCommands {
            guild_id: None,
            commands,
        })
    };
    let mut bad_name = ping_commands();
    bad_name[0].name = "Has Spaces".into();
    let mut bad_choice = ping_commands();
    bad_choice[0].options[1].choices = vec![Choice {
        name: "x".into(),
        value: OptionValue::String("1".into()),
    }];
    let mut late_required = ping_commands();
    late_required[0].options[2].required = true;
    late_required[0].options.swap(1, 2);
    late_required[0].options[2].required = true;
    late_required[0].options[1].required = false;
    for bad in [
        bad_name,
        bad_choice,
        vec![ping_commands()[0].clone(); 2],
        (0..101)
            .map(|i| CommandDraft {
                kind: CommandKind::ChatInput,
                name: format!("c{i}"),
                description: "d".into(),
                options: vec![],
                default_member_permissions: None,
            })
            .collect(),
    ] {
        assert_eq!(
            err(as_bot(&app, &mut core, &bot, set(bad)).await).code,
            ErrorCode::InvalidRequest
        );
    }
    let AppResult::Commands(first) =
        apps(ok(as_bot(&app, &mut core, &bot, set(ping_commands())).await))
    else {
        panic!()
    };
    let AppResult::Commands(again) =
        apps(ok(as_bot(&app, &mut core, &bot, set(ping_commands())).await))
    else {
        panic!()
    };
    assert_eq!(
        first.iter().map(|c| c.id).collect::<Vec<_>>(),
        again.iter().map(|c| c.id).collect::<Vec<_>>(),
        "same names keep their IDs"
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let cat = registry::catalog(&mut c, Id(2), Id(11)).await.unwrap();
    assert_eq!(
        names(&cat),
        vec!["forecast", "Wave", "Quote"],
        "purge needs Ban Members"
    );
    assert!(
        names(&registry::catalog(&mut c, Id(1), Id(11)).await.unwrap())
            .contains(&"purge".to_owned()),
        "Manage Server sees everything"
    );
    // Overrides: only Manage Server sets them; @everyone denied on one command.
    let deny = vec![CommandTarget {
        kind: TargetKind::Role,
        id: Id(10),
        allow: false,
    }];
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::SetCommandPermissions {
                guild_id: Id(10),
                id: first[0].id,
                targets: deny.clone()
            })
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::SetCommandPermissions {
            guild_id: Id(10),
            id: first[0].id,
            targets: deny,
        }),
    )
    .await);
    assert_eq!(
        names(&registry::catalog(&mut c, Id(2), Id(11)).await.unwrap()),
        vec!["Wave", "Quote"]
    );
    // A channel rule on the whole app.
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::SetCommandPermissions {
            guild_id: Id(10),
            id: bot.app,
            targets: vec![CommandTarget {
                kind: TargetKind::Channel,
                id: Id(11),
                allow: false,
            }],
        }),
    )
    .await);
    assert!(names(&registry::catalog(&mut c, Id(2), Id(11)).await.unwrap()).is_empty());
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::SetCommandPermissions {
            guild_id: Id(10),
            id: bot.app,
            targets: vec![],
        }),
    )
    .await);
    // Use Application Commands denied in the channel.
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: Id(10),
                allow: Permissions::empty(),
                deny: Permissions::USE_APPLICATION_COMMANDS,
            }],
        }),
    )
    .await);
    assert!(registry::catalog(&mut c, Id(2), Id(11))
        .await
        .unwrap()
        .apps
        .is_empty());
    let inv = Invocation {
        channel_id: Id(11),
        command_id: first[2].id,
        options: vec![],
        target_id: Some(Id(1)),
    };
    assert_eq!(
        err(as_user(&app, &mut core, 2, Command::Apps(AppCommand::Invoke(inv))).await)
            .field
            .as_deref(),
        Some("command")
    );
    let actions = audit_actions(&app).await;
    assert!(actions.contains(&"command_permissions_update".to_owned()));
}

fn option(name: &str, value: OptionValue) -> InvokedOption {
    InvokedOption {
        name: name.into(),
        value: Some(value),
        options: vec![],
        focused: false,
    }
}
fn interaction_of(core: &Core, bot: &Bot) -> Interaction {
    events(core, &bot.key)
        .into_iter()
        .rev()
        .find_map(|e| match e {
            Event::Apps(AppEvent::Interaction(i)) => Some(*i),
            _ => None,
        })
        .expect("an interaction")
}
fn answer(i: &Interaction) -> InteractionAnswer {
    InteractionAnswer {
        interaction_id: i.id,
        token: i.token.clone(),
    }
}

#[tokio::test]
async fn interactions_reach_the_bot_and_every_kind_of_answer_works() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let mut keep = vec![];
    core.cursor(&key(1), Id(1));
    core.cursor(&key(2), Id(2));
    for u in [1, 2] {
        ok(as_user(&app, &mut core, u, Command::Apps(AppCommand::Subscribe)).await);
    }
    let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
    let AppResult::Commands(cmds) = apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::SetCommands {
            guild_id: None,
            commands: ping_commands(),
        }),
    )
    .await)) else {
        panic!()
    };
    let forecast = cmds[0].id;
    let invoke = |options: Vec<InvokedOption>| {
        Command::Apps(AppCommand::Invoke(Invocation {
            channel_id: Id(11),
            command_id: forecast,
            options,
            target_id: None,
        }))
    };
    // Offline bots can't be reached.
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            invoke(vec![option("city", OptionValue::String("Oslo".into()))])
        )
        .await)
        .field
        .as_deref(),
        Some("app_offline")
    );
    online(&app, &mut core, &bot, &mut keep).await;
    // Options are checked against the definition.
    for bad in [
        vec![],
        vec![option("city", OptionValue::Integer(3))],
        vec![option("city", OptionValue::String("O".into()))],
        vec![
            option("city", OptionValue::String("Oslo".into())),
            option("days", OptionValue::Integer(9)),
        ],
        vec![
            option("city", OptionValue::String("Oslo".into())),
            option("for", OptionValue::User(Id(4))),
        ],
        vec![option("town", OptionValue::String("Oslo".into()))],
    ] {
        assert_eq!(
            err(as_user(&app, &mut core, 2, invoke(bad)).await).code,
            ErrorCode::InvalidRequest
        );
    }
    let good = vec![
        option("city", OptionValue::String("Oslo".into())),
        option("for", OptionValue::User(Id(1))),
    ];
    ok(as_user(&app, &mut core, 2, invoke(good.clone())).await);
    let i = interaction_of(&core, &bot);
    let InteractionData::Command(data) = &i.data else {
        panic!()
    };
    assert_eq!(
        (i.user.id, data.name.as_str(), data.resolved.users.len()),
        (Id(2), "forecast", 1)
    );
    // Only the right token, once.
    let message = |content: &str, ephemeral: bool| {
        InteractionResponse::Message(AppMessageDraft {
            content: content.into(),
            ephemeral,
            ..Default::default()
        })
    };
    assert_eq!(
        err(as_bot(
            &app,
            &mut core,
            &bot,
            Command::Apps(AppCommand::Respond {
                answer: InteractionAnswer {
                    interaction_id: i.id,
                    token: "x".repeat(43)
                },
                response: message("Sunny", false)
            })
        )
        .await)
        .field
        .as_deref(),
        Some("interaction")
    );
    let AppResult::Message(m) = apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: message("Sunny", false),
        }),
    )
    .await)) else {
        panic!()
    };
    assert_eq!(
        m.extra
            .app
            .as_ref()
            .unwrap()
            .interaction
            .as_ref()
            .unwrap()
            .name,
        "forecast"
    );
    assert_eq!(
        err(as_bot(
            &app,
            &mut core,
            &bot,
            Command::Apps(AppCommand::Respond {
                answer: answer(&i),
                response: message("again", false)
            })
        )
        .await)
        .code,
        ErrorCode::Conflict
    );
    // Ephemeral: only the person's own connection sees it.
    clear(&mut core, &key(1));
    clear(&mut core, &key(2));
    ok(as_user(&app, &mut core, 2, invoke(good.clone())).await);
    let i = interaction_of(&core, &bot);
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: message("Just for you", true),
        }),
    )
    .await);
    let private = |core: &Core, k: &[u8]| {
        events(core, k).into_iter().any(
            |e| matches!(e, Event::Apps(AppEvent::Ephemeral(m)) if m.content == "Just for you"),
        )
    };
    assert!(private(&core, &key(2)) && !private(&core, &key(1)));
    // Deferred, then the answer.
    ok(as_user(&app, &mut core, 2, invoke(good.clone())).await);
    let i = interaction_of(&core, &bot);
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: InteractionResponse::Defer { ephemeral: false },
        }),
    )
    .await);
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::Apps(AppEvent::Ephemeral(m)) if m.thinking)));
    let AppResult::Message(late) = apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::EditOriginal {
            answer: answer(&i),
            draft: AppMessageDraft {
                content: "Rain later".into(),
                ..Default::default()
            },
        }),
    )
    .await)) else {
        panic!()
    };
    assert_eq!(late.content, "Rain later");
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::Apps(AppEvent::EphemeralRemoved { .. }))));
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::FollowUp {
            answer: answer(&i),
            draft: AppMessageDraft {
                content: "Also wind".into(),
                ..Default::default()
            },
        }),
    )
    .await);
    // A form, checked on submit, submitted once.
    ok(as_user(&app, &mut core, 2, invoke(good.clone())).await);
    let i = interaction_of(&core, &bot);
    let modal = Modal {
        custom_id: "feedback".into(),
        title: "Feedback".into(),
        inputs: vec![TextInput {
            custom_id: "note".into(),
            label: "Note".into(),
            style: TextInputStyle::Short,
            placeholder: None,
            value: None,
            required: true,
            min_length: 0,
            max_length: Some(10),
        }],
    };
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: InteractionResponse::Modal(modal),
        }),
    )
    .await);
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::Apps(AppEvent::Modal { .. }))));
    let submit = |v: &str| {
        Command::Apps(AppCommand::SubmitModal {
            interaction_id: i.id,
            values: vec![ModalValue {
                custom_id: "note".into(),
                value: v.into(),
            }],
        })
    };
    assert_eq!(
        err(as_user(&app, &mut core, 2, submit("far too long a note")).await)
            .field
            .as_deref(),
        Some("modal")
    );
    assert_eq!(
        err(as_user(&app, &mut core, 2, submit("  ")).await)
            .field
            .as_deref(),
        Some("modal")
    );
    assert_eq!(
        err(as_user(&app, &mut core, 1, submit("hi")).await)
            .field
            .as_deref(),
        Some("modal"),
        "only its user submits it"
    );
    ok(as_user(&app, &mut core, 2, submit("great")).await);
    let sub = interaction_of(&core, &bot);
    assert!(matches!(&sub.data, InteractionData::ModalSubmit(d) if d.values[0].value == "great"));
    assert_eq!(
        err(as_user(&app, &mut core, 2, submit("again")).await)
            .field
            .as_deref(),
        Some("modal")
    );
    // Autocomplete answers go to the asking connection only.
    let mut typing = option("city", OptionValue::String("Os".into()));
    typing.focused = true;
    ok(as_user(
        &app,
        &mut core,
        2,
        Command::Apps(AppCommand::Autocomplete {
            invocation: Invocation {
                channel_id: Id(11),
                command_id: forecast,
                options: vec![typing],
                target_id: None,
            },
            request: 7,
        }),
    )
    .await);
    let i = interaction_of(&core, &bot);
    assert!(matches!(i.data, InteractionData::Autocomplete(_)));
    assert_eq!(
        err(as_bot(
            &app,
            &mut core,
            &bot,
            Command::Apps(AppCommand::Respond {
                answer: answer(&i),
                response: message("no", false)
            })
        )
        .await)
        .field
        .as_deref(),
        Some("response")
    );
    ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: InteractionResponse::Autocomplete {
                choices: vec![Choice {
                    name: "Oslo".into(),
                    value: OptionValue::String("oslo".into()),
                }],
            },
        }),
    )
    .await);
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::Apps(AppEvent::Autocomplete { request: 7, .. }))));
    // No answer in time: the person is told, and a late answer is refused.
    clear(&mut core, &key(2));
    ok(as_user(&app, &mut core, 2, invoke(good.clone())).await);
    let i = interaction_of(&core, &bot);
    core.apps
        .interactions
        .backdate(i.id, INTERACTION_RESPONSE_MS + 10);
    interactions::expire(&mut core);
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::Apps(AppEvent::Failed { .. }))));
    assert_eq!(
        err(as_bot(
            &app,
            &mut core,
            &bot,
            Command::Apps(AppCommand::Respond {
                answer: answer(&i),
                response: message("late", false)
            })
        )
        .await)
        .field
        .as_deref(),
        Some("interaction")
    );
    // Context-menu commands need a valid target.
    let wave = |t: u64| {
        Command::Apps(AppCommand::Invoke(Invocation {
            channel_id: Id(11),
            command_id: cmds[2].id,
            options: vec![],
            target_id: Some(Id(t)),
        }))
    };
    assert_eq!(
        err(as_user(&app, &mut core, 2, wave(4)).await).code,
        ErrorCode::NotFound
    );
    ok(as_user(&app, &mut core, 2, wave(1)).await);
    assert!(
        matches!(&interaction_of(&core, &bot).data, InteractionData::Command(d) if d.target_id == Some(Id(1)))
    );
}

#[tokio::test]
async fn components_are_checked_on_the_server_before_the_bot_hears_of_them() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let mut keep = vec![];
    core.cursor(&key(2), Id(2));
    let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
    online(&app, &mut core, &bot, &mut keep).await;
    let button = |id: &str, disabled: bool| {
        Component::Button(Button {
            style: ButtonStyle::Primary,
            label: Some(id.into()),
            emoji: None,
            custom_id: Some(id.into()),
            url: None,
            disabled,
        })
    };
    let rows = vec![
        ActionRow {
            components: vec![
                button("yes", false),
                button("no", true),
                Component::Button(Button {
                    style: ButtonStyle::Link,
                    label: Some("Docs".into()),
                    emoji: None,
                    custom_id: None,
                    url: Some("https://example.com".into()),
                    disabled: false,
                }),
            ],
        },
        ActionRow {
            components: vec![Component::Select(SelectMenu {
                kind: SelectKind::String,
                custom_id: "pick".into(),
                placeholder: None,
                min_values: 1,
                max_values: 2,
                options: vec![
                    SelectOption {
                        label: "A".into(),
                        value: "a".into(),
                        description: None,
                        emoji: None,
                        default: false,
                    },
                    SelectOption {
                        label: "B".into(),
                        value: "b".into(),
                        description: None,
                        emoji: None,
                        default: false,
                    },
                ],
                disabled: false,
            })],
        },
        ActionRow {
            components: vec![Component::Select(SelectMenu {
                kind: SelectKind::User,
                custom_id: "who".into(),
                placeholder: None,
                min_values: 1,
                max_values: 1,
                options: vec![],
                disabled: false,
            })],
        },
    ];
    let AppResult::Message(m) = apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Send {
            channel_id: Id(11),
            draft: AppMessageDraft {
                content: "Vote".into(),
                components: rows,
                ..Default::default()
            },
        }),
    )
    .await)) else {
        panic!()
    };
    let press = |id: &str, values: Vec<&str>| {
        Command::Apps(AppCommand::Component {
            channel_id: Id(11),
            message_id: m.id,
            custom_id: id.into(),
            values: values.into_iter().map(str::to_owned).collect(),
        })
    };
    for (id, values) in [
        ("no", vec![]),
        ("missing", vec![]),
        ("yes", vec!["x"]),
        ("pick", vec!["c"]),
        ("pick", vec![]),
        ("pick", vec!["a", "a"]),
        ("who", vec!["4"]),
        ("Docs", vec![]),
    ] {
        assert_eq!(
            err(as_user(&app, &mut core, 2, press(id, values.clone())).await)
                .field
                .as_deref(),
            Some("component"),
            "{id} {values:?}"
        );
    }
    ok(as_user(&app, &mut core, 2, press("who", vec!["1"])).await);
    ok(as_user(&app, &mut core, 2, press("pick", vec!["a", "b"])).await);
    ok(as_user(&app, &mut core, 2, press("yes", vec![])).await);
    let i = interaction_of(&core, &bot);
    assert!(matches!(&i.data, InteractionData::Component(d) if d.custom_id == "yes"));
    assert_eq!(i.message.as_ref().map(|m| m.id), Some(m.id));
    // Updating the message the component is on.
    clear(&mut core, &key(2));
    let AppResult::Message(edited) = apps(ok(as_bot(
        &app,
        &mut core,
        &bot,
        Command::Apps(AppCommand::Respond {
            answer: answer(&i),
            response: InteractionResponse::UpdateMessage(AppMessageDraft {
                content: "Voted".into(),
                ..Default::default()
            }),
        }),
    )
    .await)) else {
        panic!()
    };
    assert_eq!(
        (
            edited.content.as_str(),
            edited.extra.app.as_ref().unwrap().components.len()
        ),
        ("Voted", 0)
    );
    assert!(events(&core, &key(2))
        .iter()
        .any(|e| matches!(e, Event::MessageUpdated(u) if u.id == m.id)));
    // A component on a message from someone else's app or a person does nothing.
    let mine = match ok(as_user(&app, &mut core, 2, say("plain")).await) {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::Component {
                channel_id: Id(11),
                message_id: mine.id,
                custom_id: "yes".into(),
                values: vec![]
            })
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
}

async fn create_webhook(app: &App, core: &mut Core, by: u64, ch: u64) -> CommandOutcome {
    as_user(
        app,
        core,
        by,
        Command::Apps(AppCommand::CreateWebhook {
            channel_id: Id(ch),
            name: "CI".into(),
            avatar_id: None,
        }),
    )
    .await
}
fn path_of(url: &str) -> String {
    url[url.find("/api/").unwrap()..].to_owned()
}
fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn webhooks_post_through_their_secret_url_with_their_own_narrow_rights() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    assert_eq!(
        err(create_webhook(&app, &mut core, 2, 11).await).code,
        ErrorCode::Forbidden
    );
    let AppResult::WebhookSecret(secret) = apps(ok(create_webhook(&app, &mut core, 1, 11).await))
    else {
        panic!()
    };
    let hook = secret.webhook.clone();
    let url = path_of(&secret.url);
    let mut c = app.0.pool.acquire().await.unwrap();
    // Its account reads nothing and is a member of nothing.
    assert!(role(&mut c, hook.user_id, Id(10)).await.is_err());
    assert_eq!(
        text(&mut c, hook.user_id, Id(11), Permissions::READ_HISTORY)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::Forbidden
    );
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'other',1,0)").execute(&mut *c).await.unwrap();
    assert_eq!(
        text(&mut c, hook.user_id, Id(12), Permissions::VIEW_CHANNEL)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    drop(c);
    drop(core);
    let r = send(&app, post_json(&format!("{url}?wait=true"), serde_json::json!({
        "content": "Deployed @everyone <@2> <@&10>",
        "username": "Release Bot",
        "embeds": [{"title": "Build #42", "color": 3066993, "fields": [{"name": "Branch", "value": "main", "inline": true}], "footer": {"text": "ci"}}],
        "allowed_mentions": {"parse": ["users"], "users": ["2"]}
    }))).await;
    assert_eq!(r.status(), 200);
    let m: Message = serde_json::from_value(json(r).await).unwrap();
    assert!(
        m.content.contains("@\u{200B}everyone"),
        "no @everyone by default: {}",
        m.content
    );
    assert!(!m.content.contains("<@&10>"));
    assert_eq!(m.mention_ids, vec![Id(2)]);
    let extra = m.extra.app.clone().unwrap();
    assert_eq!(
        (
            extra.username.as_deref(),
            extra.embeds.len(),
            extra.webhook_id
        ),
        (Some("Release Bot"), 1, Some(hook.id))
    );
    assert_eq!(m.author_id, hook.user_id);
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = message(&mut c, Id(2), Id(11), m.id, false).await.unwrap();
    assert!(seen.extra.app.is_some(), "viewers get the embeds");
    assert!(
        user(&mut c, hook.user_id).await.unwrap().style.bot,
        "APP tag"
    );
    drop(c);
    // Embeds only, no text.
    let r = send(
        &app,
        post_json(
            &url,
            serde_json::json!({"embeds": [{"description": "quiet"}]}),
        ),
    )
    .await;
    assert_eq!(r.status(), 204);
    // Wrong secret, unknown webhook, wrong media type, bad embeds.
    assert_eq!(
        send(
            &app,
            post_json(
                &format!("/api/v1/webhooks/{}/{}", hook.id.0, "A".repeat(43)),
                serde_json::json!({"content": "x"})
            )
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        send(
            &app,
            Request::post(&url)
                .header("content-type", "text/plain")
                .body(Body::from("x"))
                .unwrap()
        )
        .await
        .status(),
        415
    );
    assert_eq!(
        send(
            &app,
            post_json(
                &url,
                serde_json::json!({"embeds": [{"url": "javascript:alert(1)", "title": "x"}]})
            )
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        send(&app, post_json(&url, serde_json::json!({"content": ""})))
            .await
            .status(),
        400
    );
    // GET shows the webhook without its secret.
    let info = json(send(&app, Request::get(&url).body(Body::empty()).unwrap()).await).await;
    assert_eq!(info["id"], hook.id.0);
    assert!(!info.to_string().contains(url.rsplit('/').next().unwrap()));
    // Edit and delete its own message.
    let r = send(
        &app,
        Request::patch(format!("{url}/messages/{}", m.id.0))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"content":"Deployed (edited)"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(json(r).await["content"], "Deployed (edited)");
    let mut core = app.0.core.lock().await;
    let person = match ok(as_user(&app, &mut core, 2, say("mine")).await) {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    drop(core);
    assert_eq!(
        send(
            &app,
            Request::delete(format!("{url}/messages/{}", person.id.0))
                .body(Body::empty())
                .unwrap()
        )
        .await
        .status(),
        404,
        "only its own messages"
    );
    assert_eq!(
        send(
            &app,
            Request::delete(format!("{url}/messages/{}", m.id.0))
                .body(Body::empty())
                .unwrap()
        )
        .await
        .status(),
        204
    );
    // AutoMod screens webhook posts.
    let mut core = app.0.core.lock().await;
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::CreateAutoModRule(AutoModRule {
                id: Id(0),
                guild_id: Id(10),
                name: "words".into(),
                enabled: true,
                trigger: AutoModTrigger::Keyword {
                    keywords: vec!["badword".into()],
                    allow: vec![],
                },
                actions: AutoModActions {
                    block: true,
                    ..Default::default()
                },
                exempt_roles: vec![],
                exempt_channels: vec![],
            }),
        )),
    )
    .await);
    drop(core);
    assert_eq!(
        send(
            &app,
            post_json(
                &url,
                serde_json::json!({"embeds": [{"title": "a badword"}]})
            )
        )
        .await
        .status(),
        403
    );
    // Threads in its channel only.
    let mut core = app.0.core.lock().await;
    let thread = match ok(as_user(
        &app,
        &mut core,
        1,
        Command::Threads(ThreadCommand::Create(NewThread {
            parent_id: Id(11),
            name: "Releases".into(),
            message_id: None,
            private: false,
            invitable: true,
            auto_archive_minutes: 1440,
        })),
    )
    .await)
    {
        CommandResult::Threads(ThreadResult::Thread(t)) => t,
        other => panic!("{other:?}"),
    };
    drop(core);
    assert_eq!(
        send(
            &app,
            post_json(
                &format!("{url}?thread_id={}", thread.id.0),
                serde_json::json!({"content": "in thread"})
            )
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        send(
            &app,
            post_json(
                &format!("{url}?thread_id=12"),
                serde_json::json!({"content": "elsewhere"})
            )
        )
        .await
        .status(),
        404
    );
    // Slack and GitHub shapes.
    let r = send(&app, post_json(&format!("{url}/slack"), serde_json::json!({"text": "Deploy <https://ci.example.com|done>", "attachments": [{"color": "good", "title": "Build", "text": "All green"}]}))).await;
    assert_eq!(r.status(), 200);
    let r = send(&app, Request::post(format!("{url}/github")).header("content-type", "application/json").header("x-github-event", "issues").body(Body::from(serde_json::json!({"action": "opened", "repository": {"full_name": "o/r"}, "issue": {"number": 3, "title": "Crash", "html_url": "https://github.com/o/r/issues/3"}, "sender": {"login": "alex"}}).to_string())).unwrap()).await;
    assert_eq!(r.status(), 204);
    let mut c = app.0.pool.acquire().await.unwrap();
    let titles: Vec<String> =
        sqlx::query_scalar("SELECT data FROM app_messages WHERE webhook_id=? ORDER BY message_id")
            .bind(hook.id.0 as i64)
            .fetch_all(&mut *c)
            .await
            .unwrap();
    assert!(
        titles
            .iter()
            .any(|d| d.contains("[o/r] Issue opened: #3 Crash")),
        "{titles:?}"
    );
    assert!(titles.iter().any(|d| d.contains("All green")));
    drop(c);
    // Multipart with a file.
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let boundary = "XBOUNDARY";
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n\r\n{{\"content\":\"screenshot\"}}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"shot.png\"\r\nContent-Type: image/png\r\n\r\n").into_bytes();
    body.extend_from_slice(&png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let r = send(
        &app,
        Request::post(format!("{url}?wait=true"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(json(r).await["attachments"].as_array().unwrap().len(), 1);
    // Rotating replaces the URL; deleting removes it and its credential.
    let mut core = app.0.core.lock().await;
    assert_eq!(
        err(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::RotateWebhook {
                webhook_id: hook.id
            })
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    let AppResult::WebhookSecret(rotated) = apps(ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::RotateWebhook {
            webhook_id: hook.id,
        }),
    )
    .await)) else {
        panic!()
    };
    drop(core);
    assert_eq!(
        send(&app, post_json(&url, serde_json::json!({"content": "old"})))
            .await
            .status(),
        404
    );
    let new_url = path_of(&rotated.url);
    assert_eq!(
        send(
            &app,
            post_json(&new_url, serde_json::json!({"content": "new"}))
        )
        .await
        .status(),
        204
    );
    let mut core = app.0.core.lock().await;
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Apps(AppCommand::DeleteWebhook {
            webhook_id: hook.id,
        }),
    )
    .await);
    drop(core);
    assert_eq!(
        send(
            &app,
            post_json(&new_url, serde_json::json!({"content": "gone"}))
        )
        .await
        .status(),
        404
    );
    let creds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_sessions WHERE kind='webhook'")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(creds, 0);
    let actions = audit_actions(&app).await;
    for a in ["webhook_create", "webhook_update", "webhook_delete"] {
        assert!(actions.contains(&a.to_owned()), "{a} in {actions:?}");
    }
    // Webhooks only go in text-like channels; limits per channel.
    let mut core = app.0.core.lock().await;
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(13,'voice',10,'talk',2,0)").execute(&app.0.pool).await.unwrap();
    assert_eq!(
        err(create_webhook(&app, &mut core, 1, 13).await)
            .field
            .as_deref(),
        Some("channel")
    );
    for _ in 0..MAX_WEBHOOKS_PER_CHANNEL {
        ok(create_webhook(&app, &mut core, 1, 12).await);
    }
    assert_eq!(
        err(create_webhook(&app, &mut core, 1, 12).await).code,
        ErrorCode::LimitExceeded
    );
}
use image::ImageEncoder as _;

#[tokio::test]
async fn embed_images_are_fetched_by_the_hardened_fetcher_and_served_from_here() {
    let dir = tempfile::tempdir().unwrap();
    let mut operations = crate::OperationsConfig {
        min_free_disk_gb: 0,
        ..Default::default()
    };
    operations.content.mock_link_preview_host = Some("images.example.com".into());
    let app = App::open(crate::Config {
        operations,
        browser: Default::default(),
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().into(),
        public_url: "http://localhost".into(),
        max_upload_mb: 25,
        max_total_upload_mb: 10240,
        worker_id: 0,
        trusted_proxy: "127.0.0.1".parse().unwrap(),
        google_client_id: None,
        mock_google_jwks: None,
        mock_google_issuer: None,
        gifs: Default::default(),
        voice: crate::voice::VoiceOptions {
            voice_bind_ip: "127.0.0.1".parse().unwrap(),
            voice_advertised_ip: Some("127.0.0.1".parse().unwrap()),
            ..Default::default()
        },
    })
    .await
    .unwrap();
    for u in 1..=2_i64 {
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'x','online',0)").bind(u).bind(format!("user{u}")).execute(&app.0.pool).await.unwrap();
        sqlx::query("INSERT INTO sessions VALUES(?,?,0,?,0)")
            .bind(vec![u as u8; 32])
            .bind(u)
            .bind((now() + 86_400_000) as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO guilds VALUES(10,1,'guild',NULL,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'member',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(11,'text',10,'general',0,0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO read_states(user_id,channel_id) VALUES(1,11),(2,11)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    // A tiny image server on loopback, reachable only through the mock name.
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[0, 128, 255, 255].repeat(64 * 64),
            64,
            64,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = png.clone();
    tokio::spawn(async move {
        let router = axum::Router::new().route(
            "/a.png",
            axum::routing::get(move || {
                let b = served.clone();
                async move { ([("content-type", "image/png")], b) }
            }),
        );
        axum::serve(listener, router).await.unwrap();
    });
    let mut core = app.0.core.lock().await;
    let AppResult::WebhookSecret(secret) = apps(ok(create_webhook(&app, &mut core, 1, 11).await))
    else {
        panic!()
    };
    assert!(
        secret.url.starts_with("http://localhost/api/v1/webhooks/")
            || secret.url.starts_with("/api/v1/webhooks/")
    );
    drop(core);
    let url = path_of(&secret.url);
    let r = send(&app, post_json(&format!("{url}?wait=true"), serde_json::json!({
        "avatar_url": format!("http://images.example.com:{port}/a.png"),
        "embeds": [
            {"title": "ok", "image": {"url": format!("http://images.example.com:{port}/a.png")}},
            {"title": "private", "image": {"url": format!("http://10.0.0.1:{port}/a.png")}}
        ]
    }))).await;
    let status = r.status();
    let body = json(r).await;
    assert_eq!(status, 200, "{body}");
    let m: Message = serde_json::from_value(body).unwrap();
    let jobs = app.0.core.lock().await.apps.images.take();
    assert_eq!(jobs.len(), 3);
    messages::fetch_images(&app, jobs).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = message(&mut c, Id(2), Id(11), m.id, false).await.unwrap();
    let extra = seen.extra.app.unwrap();
    let image = extra.embeds[0].image.clone().expect("fetched");
    assert!(image.url.starts_with("/api/v1/channels/11/messages/") && image.width <= 480);
    assert!(
        extra.embeds[1].image.is_none(),
        "a private address is never fetched"
    );
    assert!(extra.avatar.is_some());
    let r = send(
        &app,
        Request::get(&image.url)
            .header("authorization", format!("Bearer {}", "x"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 401);
    let mut core = app.0.core.lock().await;
    let mut c2 = app.0.pool.acquire().await.unwrap();
    let (kind, bytes) = messages::image(&mut c2, Id(2), Id(11), m.id, 1)
        .await
        .unwrap();
    assert!(kind.starts_with("image/") && !bytes.is_empty());
    sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(3,'user3','x','online',0)").execute(&mut *c2).await.unwrap();
    assert!(
        messages::image(&mut c2, Id(3), Id(11), m.id, 1)
            .await
            .is_err(),
        "only readers of the channel"
    );
    core.limits.clear();
}

/// A bot may download a file a person passed to it in an interaction (an
/// attachment option), and only that file, while it can read the channel the
/// command was used in, as a member reading the message would.
#[tokio::test]
async fn a_bot_downloads_only_the_files_passed_to_it_in_an_interaction() {
    let (_dir, app) = fixture().await;
    let mut keep = vec![];
    let bot = {
        let mut core = app.0.core.lock().await;
        core.cursor(&key(2), Id(2));
        let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
        ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
        let scan = CommandDraft {
            kind: CommandKind::ChatInput,
            name: "scan".into(),
            description: "Scan a file".into(),
            options: vec![CommandOption {
                kind: OptionKind::Attachment,
                name: "file".into(),
                description: "File".into(),
                required: true,
                choices: vec![],
                options: vec![],
                min_value: None,
                max_value: None,
                min_length: None,
                max_length: None,
                autocomplete: false,
            }],
            default_member_permissions: None,
        };
        let AppResult::Commands(cmds) = apps(ok(as_bot(
            &app,
            &mut core,
            &bot,
            Command::Apps(AppCommand::SetCommands {
                guild_id: None,
                commands: vec![scan],
            }),
        )
        .await)) else {
            panic!()
        };
        online(&app, &mut core, &bot, &mut keep).await;
        // Two of member 2's uploads in #general, waiting to be sent.
        sqlx::raw_sql(
            "INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at,expires_at) VALUES(80,2,'message',11,'report.txt','text/plain',6,'k80',0,99999999999999),(81,2,'message',11,'other.txt','text/plain',6,'k81',0,99999999999999);",
        )
        .execute(&app.0.pool)
        .await
        .unwrap();
        for (k, body) in [("k80", "report"), ("k81", "secret")] {
            tokio::fs::write(app.0.config.data_dir.join("files").join(k), body)
                .await
                .unwrap();
        }
        // Only 80 is passed to the bot.
        ok(as_user(
            &app,
            &mut core,
            2,
            Command::Apps(AppCommand::Invoke(Invocation {
                channel_id: Id(11),
                command_id: cmds[0].id,
                options: vec![option("file", OptionValue::Attachment(Id(80)))],
                target_id: None,
            })),
        )
        .await);
        let i = interaction_of(&core, &bot);
        let InteractionData::Command(data) = &i.data else {
            panic!()
        };
        assert_eq!(data.resolved.attachments[0].id, Id(80));
        bot
    };
    let get = |id: u64, auth: String| {
        Request::get(format!("/api/v1/bot/attachments/{id}"))
            .header("authorization", auth)
            .body(Body::empty())
            .unwrap()
    };
    let r = send(&app, get(80, format!("Bot {}", bot.token))).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        to_bytes(r.into_body(), 1 << 20).await.unwrap().as_ref(),
        b"report"
    );
    // Not a file it was not given, and the route is for bots only.
    assert_eq!(
        send(&app, get(81, format!("Bot {}", bot.token)))
            .await
            .status(),
        404
    );
    let person = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, key(2));
    assert_eq!(
        send(&app, get(80, format!("Bearer {person}")))
            .await
            .status(),
        401
    );
    // Once the bot can no longer read the channel's messages, not even that one.
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',?,0,?)")
        .bind(bot.user.0 as i64)
        .bind(Permissions::READ_HISTORY.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        send(&app, get(80, format!("Bot {}", bot.token)))
            .await
            .status(),
        404
    );
}

/// A server's AutoMod keyword rules apply to the commands its apps offer
/// (names, descriptions, choices), for that server's commands and for an
/// app's global ones in every server it is in.
#[tokio::test]
async fn automod_screens_command_names_and_descriptions() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let bot = create_app(&app, &mut core, 1, Intents::empty(), BASIC).await;
    ok(install(&app, &mut core, &bot, 1, BASIC, false).await);
    ok(as_user(
        &app,
        &mut core,
        1,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::CreateAutoModRule(AutoModRule {
                id: Id(0),
                guild_id: Id(10),
                name: "Words".into(),
                enabled: true,
                trigger: AutoModTrigger::Keyword {
                    keywords: vec!["boo*".into()],
                    allow: vec![],
                },
                actions: AutoModActions {
                    block: true,
                    ..Default::default()
                },
                exempt_roles: vec![],
                exempt_channels: vec![],
            }),
        )),
    )
    .await);
    let set = |guild_id: Option<Id>, description: &str| {
        let mut commands = ping_commands();
        commands[0].description = description.into();
        Command::Apps(AppCommand::SetCommands { guild_id, commands })
    };
    for scope in [None, Some(Id(10))] {
        let e = err(as_bot(&app, &mut core, &bot, set(scope, "Booze forecasts")).await);
        assert_eq!(e.field.as_deref(), Some("automod"), "{scope:?}");
        ok(as_bot(&app, &mut core, &bot, set(scope, "Weather")).await);
    }
}
