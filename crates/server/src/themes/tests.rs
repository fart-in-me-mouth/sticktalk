use super::*;
use crate::{audit_tests::fixture, commands, state::App};

/// Guild 10 (#11): 1 (owner) and 2 (member). Guild 20 (#21): 1 (owner) and 3.
/// User 4 is in neither. Direct message 30 between 2 and 3.
async fn world() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sqlx::raw_sql(
        "INSERT INTO guilds VALUES(20,1,'other',NULL,0);
         INSERT INTO guild_members VALUES(20,1,'owner',0),(20,3,'member',0);
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'elsewhere',0,0);
         INSERT INTO channels(id,kind,position,created_at) VALUES(30,'direct',0,0);
         INSERT INTO dm_pairs VALUES(2,3,30);
         INSERT INTO dm_members VALUES(30,2,0,1),(30,3,0,1);
         INSERT INTO read_states(user_id,channel_id) VALUES(1,21),(3,21),(2,30),(3,30);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
    (dir, app)
}
async fn run(app: &App, u: u64, nonce: u64, command: AppearanceCommand) -> CommandOutcome {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest {
            nonce,
            command: Command::Appearance(command),
        },
    )
    .await
    .unwrap()
    .outcome
}
fn library(outcome: CommandOutcome) -> ThemeLibrary {
    match outcome {
        CommandOutcome::Ok(CommandResult::Appearance(AppearanceResult::Library(l))) => *l,
        other => panic!("{other:?}"),
    }
}
fn code(outcome: CommandOutcome) -> ErrorCode {
    match outcome {
        CommandOutcome::Err(e) => e.code,
        CommandOutcome::Ok(r) => panic!("expected an error, got {r:?}"),
    }
}
fn events(core: &Core, key: u8) -> Vec<AppearanceEvent> {
    core.streams[&vec![key; 32]]
        .ring
        .iter()
        .filter_map(|(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
            ServerMsg::Event(e) => match e.event {
                Event::Appearance(a) => Some(a),
                _ => None,
            },
            _ => None,
        })
        .collect()
}
async fn streams(app: &App, keys: &[(u8, u64)]) {
    let mut core = app.0.core.lock().await;
    for (key, user) in keys {
        core.cursor(&[*key; 32], Id(*user));
    }
}
fn spec() -> ThemeSpec {
    PRESETS[0].spec()
}

#[tokio::test]
async fn custom_themes_are_validated_bounded_private_and_synced() {
    let (_dir, app) = world().await;
    // User 2 has this stream (subscribes) and an older one that never does.
    streams(&app, &[(2, 2), (9, 2), (3, 3)]).await;
    let lib = library(run(&app, 2, 1, AppearanceCommand::Subscribe).await);
    assert!(lib.themes.is_empty() && lib.settings.server_themes);
    let save = |id: Option<Id>, name: &str, spec: ThemeSpec| AppearanceCommand::SaveTheme {
        id,
        name: name.into(),
        spec,
    };
    let lib = library(run(&app, 2, 2, save(None, "  Mine  ", spec())).await);
    assert_eq!(lib.themes.len(), 1);
    assert_eq!(lib.themes[0].name, "Mine");
    let mine = lib.themes[0].id;
    // Untrusted input is refused.
    let bad = ThemeSpec { colors: vec![0; 6], ..spec() };
    assert_eq!(code(run(&app, 2, 3, save(None, "x", bad)).await), ErrorCode::InvalidRequest);
    let bad = ThemeSpec { accent: 0x1_000_000, ..spec() };
    assert_eq!(code(run(&app, 2, 4, save(None, "x", bad)).await), ErrorCode::InvalidRequest);
    let bad = ThemeSpec { angle: 400, ..spec() };
    assert_eq!(code(run(&app, 2, 5, save(None, "x", bad)).await), ErrorCode::InvalidRequest);
    assert_eq!(code(run(&app, 2, 6, save(None, "", spec())).await), ErrorCode::InvalidRequest);
    assert_eq!(code(run(&app, 2, 7, save(None, &"n".repeat(40), spec())).await), ErrorCode::InvalidRequest);
    // Someone else's theme cannot be changed or deleted.
    assert_eq!(code(run(&app, 3, 8, save(Some(mine), "taken", spec())).await), ErrorCode::NotFound);
    assert_eq!(code(run(&app, 3, 9, AppearanceCommand::DeleteTheme { id: mine }).await), ErrorCode::NotFound);
    // Renaming keeps the id.
    let lib = library(run(&app, 2, 10, save(Some(mine), "Renamed", spec())).await);
    assert_eq!((lib.themes[0].id, lib.themes[0].name.as_str()), (mine, "Renamed"));
    // At most 20.
    for n in 0..19 {
        library(run(&app, 2, 100 + n, save(None, &format!("t{n}"), spec())).await);
    }
    assert_eq!(code(run(&app, 2, 200, save(None, "one too many", spec())).await), ErrorCode::LimitExceeded);
    let lib = library(run(&app, 2, 201, AppearanceCommand::DeleteTheme { id: mine }).await);
    assert_eq!(lib.themes.len(), 19);
    let core = app.0.core.lock().await;
    // The subscribed stream heard about every change; the old one and others did not.
    assert!(events(&core, 2).iter().any(|e| matches!(e, AppearanceEvent::Library(l) if l.themes.len() == 19)));
    assert!(events(&core, 9).is_empty());
    assert!(events(&core, 3).is_empty());
}

#[tokio::test]
async fn server_themes_need_manage_server_and_reach_members() {
    let (_dir, app) = world().await;
    streams(&app, &[(1, 1), (2, 2), (3, 3), (4, 4)]).await;
    for u in [1, 2, 3, 4] {
        library(run(&app, u, 1, AppearanceCommand::Subscribe).await);
    }
    let set = |g: u64, theme: Option<ThemeChoice>| AppearanceCommand::SetServerTheme {
        guild_id: Id(g),
        theme,
    };
    let ocean = Some(ThemeChoice::Preset("ocean".into()));
    // A member without Manage Server, and someone outside the server.
    assert_eq!(code(run(&app, 2, 2, set(10, ocean.clone())).await), ErrorCode::Forbidden);
    assert!(matches!(run(&app, 4, 2, set(10, ocean.clone())).await, CommandOutcome::Err(_)));
    // Unknown presets and broken custom themes.
    assert_eq!(code(run(&app, 1, 3, set(10, Some(ThemeChoice::Preset("nope".into())))).await), ErrorCode::InvalidRequest);
    let broken = ThemeSpec { intensity: 200, ..spec() };
    assert_eq!(code(run(&app, 1, 4, set(10, Some(ThemeChoice::Custom(broken)))).await), ErrorCode::InvalidRequest);
    // The owner sets it; members hear it, others do not.
    library(run(&app, 1, 5, set(10, ocean.clone())).await);
    {
        let core = app.0.core.lock().await;
        let heard = |k| events(&core, k).iter().any(|e| matches!(e, AppearanceEvent::ServerTheme { guild_id, theme } if *guild_id == Id(10) && *theme == ocean));
        assert!(heard(1) && heard(2));
        assert!(!heard(3) && !heard(4));
    }
    // A member's library lists it; someone outside the server's does not.
    let lib = library(run(&app, 2, 6, AppearanceCommand::Subscribe).await);
    assert_eq!(lib.servers, vec![ServerTheme { guild_id: Id(10), theme: ThemeChoice::Preset("ocean".into()) }]);
    assert!(library(run(&app, 3, 6, AppearanceCommand::Subscribe).await).servers.is_empty());
    // Granting Manage Server to @everyone lets a member set it; removing works.
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE id=10")
        .bind(Permissions::MANAGE_GUILD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let custom = Some(ThemeChoice::Custom(spec()));
    library(run(&app, 2, 7, set(10, custom)).await);
    let lib = library(run(&app, 2, 8, set(10, None)).await);
    assert!(lib.servers.is_empty());
}

#[tokio::test]
async fn settings_keep_only_servers_you_are_in() {
    let (_dir, app) = world().await;
    streams(&app, &[(2, 2)]).await;
    let o = |g: u64, show| ServerThemeOverride { guild_id: Id(g), show };
    let lib = library(
        run(
            &app,
            2,
            1,
            AppearanceCommand::SetSettings(ThemeSettings {
                server_themes: false,
                server_overrides: vec![o(10, true), o(20, true), o(999, false)],
            }),
        )
        .await,
    );
    assert!(!lib.settings.server_themes);
    assert_eq!(lib.settings.server_overrides, vec![o(10, true)]);
    // Duplicates are refused.
    let dup = ThemeSettings { server_themes: true, server_overrides: vec![o(10, true), o(10, false)] };
    assert_eq!(code(run(&app, 2, 2, AppearanceCommand::SetSettings(dup)).await), ErrorCode::InvalidRequest);
}

#[tokio::test]
async fn tts_messages_need_the_permission_and_are_marked() {
    let (_dir, app) = world().await;
    streams(&app, &[(1, 1), (2, 2)]).await;
    // @everyone has Send TTS Messages by default (migration and trigger).
    let p: i64 = sqlx::query_scalar("SELECT permissions FROM roles WHERE id=10")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert!(Permissions::from_bits_retain(p as u64).contains(Permissions::SEND_TTS_MESSAGES));
    let tts = |ch: u64, text: &str| {
        AppearanceCommand::SendTts(MessageDraft {
            channel_id: Id(ch),
            content: text.into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        })
    };
    let CommandOutcome::Ok(CommandResult::Message(m)) = run(&app, 2, 1, tts(11, "hello there")).await else {
        panic!()
    };
    assert!(m.extra.tts && m.content == "hello there");
    {
        let core = app.0.core.lock().await;
        let created = core.streams[&vec![1; 32]].ring.iter().any(|(_, _, b)| {
            matches!(rmp_serde::from_slice::<ServerMsg>(b).unwrap(),
                ServerMsg::Event(e) if matches!(&e.event, Event::MessageCreated { message, .. } if message.id == m.id && message.extra.tts))
        });
        assert!(created, "the other member's MessageCreated carries the flag");
    }
    // Empty text, someone outside the channel, and a denied permission.
    assert_eq!(code(run(&app, 2, 2, tts(11, "   ")).await), ErrorCode::InvalidRequest);
    assert!(matches!(run(&app, 3, 3, tts(11, "hi")).await, CommandOutcome::Err(_)));
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'role',10,0,?)")
        .bind(Permissions::SEND_TTS_MESSAGES.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(code(run(&app, 2, 4, tts(11, "hi")).await), ErrorCode::Forbidden);
    // An ordinary message still goes through.
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    let plain = commands::execute(
        &app,
        &mut core,
        &[2; 32],
        Id(2),
        CommandRequest {
            nonce: 5,
            command: Command::SendMessage(MessageDraft {
                channel_id: Id(11),
                content: "plain".into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }),
        },
    )
    .await
    .unwrap()
    .outcome;
    assert!(matches!(plain, CommandOutcome::Ok(CommandResult::Message(m)) if !m.extra.tts));
    drop(core);
    // Direct messages allow it.
    assert!(matches!(run(&app, 2, 6, tts(30, "dm")).await, CommandOutcome::Ok(CommandResult::Message(_))));
}
