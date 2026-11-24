use crate::{
    audit_tests::fixture,
    commands, db, launch, permissions as acl,
    state::{now, Core},
    App,
};
use sqlx::Connection;
use sticktalk_proto::*;

async fn command(app: &App, core: &mut Core, u: u64, nonce: u64, cmd: Command) -> CommandOutcome {
    core.limits.clear();
    commands::execute(
        app,
        core,
        &[u as u8; 32],
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
async fn run(app: &App, core: &mut Core, u: u64, n: u64, cmd: LaunchCommand) -> CommandOutcome {
    command(app, core, u, n, Command::Launch(cmd)).await
}
fn ok(outcome: CommandOutcome) -> CommandResult {
    match outcome {
        CommandOutcome::Ok(v) => v,
        CommandOutcome::Err(e) => panic!("{e:?}"),
    }
}
fn denied(outcome: CommandOutcome) {
    assert!(matches!(outcome, CommandOutcome::Err(_)), "{outcome:?}");
}
fn events(core: &Core, u: u64) -> Vec<Event> {
    core.streams
        .get(&vec![u as u8; 32])
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
fn overwrite(
    target: u64,
    kind: OverwriteKind,
    allow: Permissions,
    deny: Permissions,
) -> PermissionOverwrite {
    PermissionOverwrite {
        target_id: Id(target),
        kind,
        allow,
        deny,
    }
}
async fn hide(app: &App, core: &mut Core, nonce: u64) {
    ok(run(
        app,
        core,
        1,
        nonce,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![overwrite(
                10,
                OverwriteKind::Role,
                Permissions::empty(),
                Permissions::VIEW_CHANNEL,
            )],
        },
    )
    .await);
}
async fn seed_roles(app: &App) {
    sqlx::raw_sql("INSERT INTO guild_members VALUES(10,3,'member',0); UPDATE roles SET position=4 WHERE id=11; INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'low',1,0),(10,30,'manager',3,0); INSERT INTO member_roles VALUES(10,2,30),(10,3,20); INSERT INTO read_states(user_id,channel_id) VALUES(3,11);").execute(&app.0.pool).await.unwrap();
    sqlx::query("UPDATE roles SET permissions=? WHERE id=30 AND guild_id=10")
        .bind(
            (Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS | Permissions::MANAGE_NICKNAMES)
                .bits() as i64,
        )
        .execute(&app.0.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn evaluator_full_bit_matrix_and_owner_admin_bypass() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    sqlx::query("UPDATE roles SET permissions=0 WHERE guild_id=10")
        .execute(&mut *c)
        .await
        .unwrap();
    for bit in 0..28 {
        let p = Permissions::from_bits_retain(1 << bit);
        sqlx::query("UPDATE roles SET permissions=? WHERE id=30 AND guild_id=10")
            .bind(p.bits() as i64)
            .execute(&mut *c)
            .await
            .unwrap();
        let actual = acl::evaluate(&mut c, Id(2), Id(10), None).await.unwrap();
        let expected = if p == Permissions::OWNER {
            Permissions::empty()
        } else if p == Permissions::ADMINISTRATOR {
            Permissions::all() - Permissions::OWNER
        } else {
            p
        };
        assert_eq!(actual, expected, "bit {bit}");
    }
    sqlx::query("UPDATE roles SET permissions=? WHERE id=30")
        .bind(Permissions::ADMINISTRATOR.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'role',10,0,?)")
        .bind(Permissions::all().bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        acl::evaluate(&mut c, Id(1), Id(10), Some(Id(11)))
            .await
            .unwrap(),
        Permissions::all()
    );
    assert_eq!(
        acl::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
            .await
            .unwrap(),
        Permissions::all() - Permissions::OWNER
    );
    assert!(acl::evaluate(&mut c, Id(4), Id(10), None).await.is_err());
    assert!(acl::evaluate(&mut c, Id(1), Id(10), Some(Id(999)))
        .await
        .is_err());
}

#[tokio::test]
async fn overwrite_order_aggregate_roles_member_and_implicit_dependencies() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    sqlx::raw_sql("INSERT INTO member_roles VALUES(10,2,20); INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(50,'category',10,'private',1,0); UPDATE channels SET parent_id=50 WHERE id=11;").execute(&mut *c).await.unwrap();
    let view = Permissions::VIEW_CHANNEL;
    let send = Permissions::SEND_MESSAGES;
    sqlx::query("INSERT INTO permission_overwrites VALUES(50,'role',10,0,?)")
        .bind((view | send).bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(!acl::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
        .await
        .unwrap()
        .contains(view));
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'role',20,?,0),(11,'role',30,0,?)")
        .bind((view | send).bits() as i64)
        .bind((view | send).bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    // Aggregated role allow wins even against a higher role's deny.
    assert!(acl::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
        .await
        .unwrap()
        .contains(view | send));
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,0,?)")
        .bind(send.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    let p = acl::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
        .await
        .unwrap();
    assert!(p.contains(view));
    assert!(!p.intersects(send | Permissions::ATTACH_FILES | Permissions::EMBED_LINKS));
    sqlx::query("UPDATE permission_overwrites SET deny=? WHERE kind='member'")
        .bind((view | Permissions::CONNECT).bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    let p = acl::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
        .await
        .unwrap();
    assert!(!p.intersects(
        view | send
            | Permissions::READ_HISTORY
            | Permissions::SPEAK
            | Permissions::STREAM
            | Permissions::VIDEO
    ));
    assert!(p.contains(Permissions::KICK_MEMBERS));
}

#[tokio::test]
async fn role_hierarchy_assignment_escalation_reorder_and_owner_bypass() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    let mut manager = acl::roles(&mut c, Id(10))
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == Id(30))
        .unwrap();
    denied(
        run(
            &app,
            &mut core,
            2,
            1,
            LaunchCommand::UpdateRole {
                role: manager.clone(),
            },
        )
        .await,
    ); // own highest
    manager.id = Id(20);
    manager.position = 1;
    manager.permissions = Permissions::ADMINISTRATOR;
    denied(
        run(
            &app,
            &mut core,
            2,
            2,
            LaunchCommand::UpdateRole {
                role: manager.clone(),
            },
        )
        .await,
    ); // cannot grant unheld permission
    denied(
        run(
            &app,
            &mut core,
            2,
            3,
            LaunchCommand::SetMemberRoles {
                guild_id: Id(10),
                user_id: Id(2),
                role_ids: vec![Id(11), Id(30)],
            },
        )
        .await,
    );
    denied(
        run(
            &app,
            &mut core,
            2,
            4,
            LaunchCommand::ReorderRoles {
                guild_id: Id(10),
                role_ids: vec![Id(10), Id(20), Id(11), Id(30)],
            },
        )
        .await,
    );
    manager.permissions = Permissions::KICK_MEMBERS;
    manager.color = 0x123456;
    manager.hoist = true;
    manager.mentionable = true;
    ok(run(
        &app,
        &mut core,
        2,
        5,
        LaunchCommand::UpdateRole { role: manager },
    )
    .await);
    denied(
        command(
            &app,
            &mut core,
            3,
            6,
            Command::KickMember {
                guild_id: Id(10),
                user_id: Id(2),
            },
        )
        .await,
    );
    denied(
        command(
            &app,
            &mut core,
            2,
            7,
            Command::KickMember {
                guild_id: Id(10),
                user_id: Id(1),
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        1,
        8,
        LaunchCommand::ReorderRoles {
            guild_id: Id(10),
            role_ids: vec![Id(10), Id(11), Id(30), Id(20)],
        },
    )
    .await);
    let r = ok(run(
        &app,
        &mut core,
        2,
        9,
        LaunchCommand::CreateRole {
            guild_id: Id(10),
            name: "helper".into(),
        },
    )
    .await);
    let CommandResult::Role(r) = r else { panic!() };
    assert!(r.position < acl::highest(&mut c, Id(10), Id(2)).await.unwrap());
    denied(
        run(
            &app,
            &mut core,
            2,
            10,
            LaunchCommand::DeleteRole {
                guild_id: Id(10),
                role_id: Id(20),
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        1,
        11,
        LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: r.id,
        },
    )
    .await);
    denied(
        run(
            &app,
            &mut core,
            1,
            12,
            LaunchCommand::DeleteRole {
                guild_id: Id(10),
                role_id: Id(10),
            },
        )
        .await,
    );
}

#[tokio::test]
async fn hidden_channel_denies_every_existing_channel_command_and_snapshot_projection() {
    let (_dir, app) = fixture().await;
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,2,'secret',0)").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    hide(&app, &mut core, 1).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    for p in [
        Permissions::VIEW_CHANNEL,
        Permissions::READ_HISTORY,
        Permissions::SEND_MESSAGES,
        Permissions::ATTACH_FILES,
        Permissions::ADD_REACTIONS,
        Permissions::MANAGE_MESSAGES,
        Permissions::MANAGE_CHANNELS,
        Permissions::CONNECT,
    ] {
        assert!(db::access(&mut c, Id(2), Id(11), p).await.is_err());
    }
    let commands = vec![
        Command::SendMessage(MessageDraft {
            channel_id: Id(11),
            content: "no".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        }),
        Command::EditMessage(MessageEdit {
            channel_id: Id(11),
            message_id: Id(12),
            content: "no".into(),
            mention_ids: vec![],
        }),
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: Id(12),
        },
        Command::AddReaction {
            channel_id: Id(11),
            message_id: Id(12),
            emoji: "👍".into(),
        },
        Command::RemoveReaction {
            channel_id: Id(11),
            message_id: Id(12),
            emoji: "👍".into(),
        },
        Command::Typing { channel_id: Id(11) },
        Command::MarkRead {
            channel_id: Id(11),
            message_id: Id(12),
        },
        Command::UpdateChannel {
            channel_id: Id(11),
            patch: ChannelPatch::default(),
        },
        Command::DeleteChannel { channel_id: Id(11) },
        Command::Launch(LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: true,
        }),
        Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![],
        }),
    ];
    for (i, cmd) in commands.into_iter().enumerate() {
        denied(command(&app, &mut core, 2, 100 + i as u64, cmd).await);
    }
    assert!(db::channels(&mut c, Id(2), None).await.unwrap().is_empty());
    assert!(db::read_states(&mut c, Id(2)).await.unwrap().is_empty());
    assert!(!db::audience(&mut c, Id(11)).await.unwrap().contains(&Id(2)));
    let ready = db::ready(&mut c, &mut core, &[2; 32], Id(2)).await.unwrap();
    assert!(ready.channels.is_empty() && ready.read_states.is_empty());
    assert!(!ready.roles.is_empty());
    assert_eq!(ready.memberships[0].role_ids, vec![Id(10)]);
}

#[tokio::test]
async fn private_channel_fanout_replay_receipts_and_visibility_restoration() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[2; 32], Id(2));
    let (close, closed) = tokio::sync::watch::channel(None);
    let (tx, _rx) = tokio::sync::mpsc::channel(128);
    core.streams.get_mut(&vec![2; 32]).unwrap().connection = Some(crate::state::Connection {
        focused: false,
        alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        id: "test".into(),
        tx,
        budget: std::sync::Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
        close,
    });
    let draft = MessageDraft {
        channel_id: Id(11),
        content: "before".into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    };
    ok(command(&app, &mut core, 2, 1, Command::SendMessage(draft.clone())).await);
    assert!(events(&core, 2)
        .iter()
        .any(|e| matches!(e, Event::MessageCreated { .. })));
    hide(&app, &mut core, 2).await;
    assert_eq!(*closed.borrow(), Some(4003));
    assert!(events(&core, 2)
        .iter()
        .all(|e| !matches!(e, Event::MessageCreated { .. })));
    denied(command(&app, &mut core, 2, 1, Command::SendMessage(draft)).await); // historical receipt reauthorizes
    core.invalidate(&[Id(2)]);
    let result = ok(command(
        &app,
        &mut core,
        1,
        3,
        Command::SendMessage(MessageDraft {
            channel_id: Id(11),
            content: "private".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        }),
    )
    .await);
    let CommandResult::Message(m) = result else {
        panic!()
    };
    ok(command(
        &app,
        &mut core,
        1,
        4,
        Command::AddReaction {
            channel_id: Id(11),
            message_id: m.id,
            emoji: "👍".into(),
        },
    )
    .await);
    ok(command(
        &app,
        &mut core,
        1,
        5,
        Command::Typing { channel_id: Id(11) },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        6,
        LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: m.id,
            pinned: true,
        },
    )
    .await);
    ok(command(
        &app,
        &mut core,
        1,
        7,
        Command::ReorderChannels {
            guild_id: Id(10),
            placements: vec![ChannelPlacement {
                channel_id: Id(11),
                parent_id: None,
                position: 0,
            }],
        },
    )
    .await);
    for e in events(&core, 2) {
        match e {
            Event::ChannelsReordered { placements, .. } => assert!(placements.is_empty()),
            _ => panic!("private fanout {e:?}"),
        }
    }
    ok(run(
        &app,
        &mut core,
        1,
        8,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![],
        },
    )
    .await);
    let restored = events(&core, 2);
    assert!(restored
        .iter()
        .any(|e| matches!(e,Event::ChannelUpsert(ch) if ch.summary.id==Id(11))));
    assert!(restored
        .iter()
        .any(|e| matches!(e,Event::ReadStateUpdated(r) if r.channel_id==Id(11))));
}

#[tokio::test]
async fn channel_overwrite_controls_management_not_just_guild_base() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE id=30")
        .bind((Permissions::MANAGE_MESSAGES | Permissions::MANAGE_CHANNELS).bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'owner',0)").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    ok(run(
        &app,
        &mut core,
        1,
        1,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![overwrite(
                2,
                OverwriteKind::Member,
                Permissions::empty(),
                Permissions::MANAGE_CHANNELS | Permissions::MANAGE_MESSAGES,
            )],
        },
    )
    .await);
    for (i, cmd) in [
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: Id(12),
        },
        Command::UpdateChannel {
            channel_id: Id(11),
            patch: ChannelPatch::default(),
        },
        Command::DeleteChannel { channel_id: Id(11) },
    ]
    .into_iter()
    .enumerate()
    {
        denied(command(&app, &mut core, 2, i as u64 + 2, cmd).await);
    }
}

#[tokio::test]
async fn profiles_unicode_limits_mutuals_preview_and_nicknames() {
    let (_dir, app) = fixture().await;
    sqlx::raw_sql("INSERT INTO relationships VALUES(1,3,'friend',0),(2,3,'friend',0); INSERT INTO guilds VALUES(40,4,'not mutual',NULL,50); INSERT INTO guild_members VALUES(40,4,'owner',50),(40,2,'member',60);").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    ok(run(
        &app,
        &mut core,
        2,
        1,
        LaunchCommand::UpdateUserProfile {
            banner_color: 0x123456,
            pronouns: "they/them".into(),
            about_me: "🦀".repeat(190),
        },
    )
    .await);
    denied(
        run(
            &app,
            &mut core,
            2,
            2,
            LaunchCommand::UpdateUserProfile {
                banner_color: 0,
                pronouns: "".into(),
                about_me: "a".repeat(191),
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        1,
        3,
        LaunchCommand::UpdateGuildProfile {
            guild_id: Id(10),
            description: "🦀".repeat(120),
            banner_color: 9,
        },
    )
    .await);
    denied(
        run(
            &app,
            &mut core,
            1,
            4,
            LaunchCommand::UpdateGuildProfile {
                guild_id: Id(10),
                description: "a".repeat(121),
                banner_color: 0,
            },
        )
        .await,
    );
    denied(
        run(
            &app,
            &mut core,
            1,
            5,
            LaunchCommand::UpdateGuildProfile {
                guild_id: Id(10),
                description: "".into(),
                banner_color: 10,
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        2,
        6,
        LaunchCommand::SetNickname {
            guild_id: Id(10),
            user_id: Id(2),
            nickname: Some("Crab".into()),
        },
    )
    .await);
    denied(
        run(
            &app,
            &mut core,
            2,
            7,
            LaunchCommand::SetNickname {
                guild_id: Id(10),
                user_id: Id(1),
                nickname: Some("no".into()),
            },
        )
        .await,
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let p = launch::profile(&mut c, Id(1), Id(2)).await.unwrap();
    assert_eq!(p.mutual_servers.len(), 1);
    assert_eq!(p.mutual_servers[0].guild.id, Id(10));
    assert_eq!(p.mutual_friends[0].id, Id(3));
    assert_eq!(p.profile.about_me.chars().count(), 190);
    assert_eq!(
        db::member(&mut c, Id(10), Id(2))
            .await
            .unwrap()
            .nickname
            .as_deref(),
        Some("Crab")
    );
    sqlx::query("UPDATE roles SET permissions=permissions & ? WHERE id=10")
        .bind(!(Permissions::CHANGE_NICKNAME.bits() as i64))
        .execute(&mut *c)
        .await
        .unwrap();
    denied(
        run(
            &app,
            &mut core,
            2,
            8,
            LaunchCommand::SetNickname {
                guild_id: Id(10),
                user_id: Id(2),
                nickname: None,
            },
        )
        .await,
    );
}

#[tokio::test]
async fn pins_require_channel_manage_messages_and_deleted_messages_drop_pins() {
    let (_dir, app) = fixture().await;
    sqlx::query(
        "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'pin',0)",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
    let mut core = app.0.core.lock().await;
    denied(
        run(
            &app,
            &mut core,
            2,
            1,
            LaunchCommand::PinMessage {
                channel_id: Id(11),
                message_id: Id(12),
                pinned: true,
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        1,
        2,
        LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: true,
        },
    )
    .await);
    assert!(events(&core, 1)
        .iter()
        .any(|e| matches!(e, Event::PinsUpdated { pinned: true, .. })));
    ok(run(
        &app,
        &mut core,
        1,
        3,
        LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: false,
        },
    )
    .await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pins")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
    ok(run(
        &app,
        &mut core,
        1,
        4,
        LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: true,
        },
    )
    .await);
    ok(command(
        &app,
        &mut core,
        1,
        5,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: Id(12),
        },
    )
    .await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pins")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
    assert!(events(&core, 1).iter().any(
        |e| matches!(e,Event::PinsUpdated {message_id,pinned:false,..} if *message_id==Id(12))
    ));
}

#[tokio::test]
async fn notifications_are_durable_device_synced_expiring_and_visibility_filtered() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[2; 32], Id(2));
    core.cursor(&[99; 32], Id(2));
    core.streams.get_mut(&vec![99; 32]).unwrap().launch_features = true;
    core.cursor(&[1; 32], Id(1));
    let mut s = NotificationSettings {
        guild_id: Some(Id(10)),
        channel_id: Some(Id(11)),
        level: NotificationLevel::Mentions,
        muted: true,
        mute_until: Some(now() + 60_000),
        suppress_everyone: true,
        hide_muted_channels: true,
    };
    ok(run(
        &app,
        &mut core,
        2,
        1,
        LaunchCommand::SetNotifications(s.clone()),
    )
    .await);
    assert!(core.streams[&vec![99;32]].ring.iter().any(|(_,_,b)|matches!(rmp_serde::from_slice::<ServerMsg>(b).unwrap(),ServerMsg::Event(e) if matches!(e.event,Event::NotificationSettingsUpdated(_)))));
    assert!(events(&core, 1).is_empty());
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        launch::notifications(&mut c, Id(2)).await.unwrap(),
        vec![s.clone()]
    );
    s.mute_until = Some(now() - 1);
    s.level = NotificationLevel::Nothing;
    ok(run(&app, &mut core, 2, 2, LaunchCommand::SetNotifications(s)).await);
    let settings = launch::notifications(&mut c, Id(2)).await.unwrap();
    assert!(!settings[0].muted);
    assert_eq!(settings[0].level, NotificationLevel::Nothing);
    hide(&app, &mut core, 3).await;
    assert!(launch::notifications(&mut c, Id(2))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn devices_are_opaque_user_scoped_and_revocation_closes_live_streams() {
    let (_dir, app) = fixture().await;
    sqlx::query("INSERT INTO sessions VALUES(?,2,0,?,0)")
        .bind(vec![9u8; 32])
        .bind((now() + 60_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut core = app.0.core.lock().await;
    core.cursor(&[9; 32], Id(2));
    core.cursor(&[2; 32], Id(2));
    let mut c = app.0.pool.acquire().await.unwrap();
    let mut sessions = launch::sessions(&mut c, Id(2), &[2; 32]).await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions.iter().filter(|s| s.current).count(), 1);
    assert!(sessions
        .iter()
        .all(|s| s.place == "this device" && s.id.len() == 32));
    ok(run(
        &app,
        &mut core,
        2,
        1,
        LaunchCommand::UpdateDevice {
            device_name: "Desktop".into(),
            platform: "Windows".into(),
        },
    )
    .await);
    sessions = launch::sessions(&mut c, Id(2), &[2; 32]).await.unwrap();
    assert_eq!(
        sessions.iter().find(|s| s.current).unwrap().device_name,
        "Desktop"
    );
    let other = sessions.iter().find(|s| !s.current).unwrap().id.clone();
    denied(
        run(
            &app,
            &mut core,
            1,
            2,
            LaunchCommand::LogoutSession {
                session_id: other.clone(),
            },
        )
        .await,
    );
    ok(run(
        &app,
        &mut core,
        2,
        3,
        LaunchCommand::LogoutSession { session_id: other },
    )
    .await);
    assert!(!core.streams.contains_key(&vec![9; 32]));
    sqlx::query("INSERT INTO sessions VALUES(?,2,0,?,0)")
        .bind(vec![8u8; 32])
        .bind((now() + 60_000) as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    core.cursor(&[8; 32], Id(2));
    ok(run(&app, &mut core, 2, 4, LaunchCommand::LogoutAllOthers).await);
    assert!(!core.streams.contains_key(&vec![8; 32]));
    assert_eq!(
        launch::sessions(&mut c, Id(2), &[2; 32])
            .await
            .unwrap()
            .len(),
        1
    );
    let current = launch::sessions(&mut c, Id(2), &[2; 32]).await.unwrap()[0]
        .id
        .clone();
    ok(run(
        &app,
        &mut core,
        2,
        5,
        LaunchCommand::LogoutSession {
            session_id: current,
        },
    )
    .await);
    assert!(!core.streams.contains_key(&vec![2; 32]));
}

#[tokio::test]
async fn migration_preserves_legacy_member_admin_owner_and_wire_bits() {
    let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0001_phase1.sql"))
        .execute(&mut c)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(1,'owner','','online',0),(2,'admin','','online',0),(3,'member','','online',0); INSERT INTO guilds VALUES(10,1,'old',NULL,0); INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'admin',0),(10,3,'member',0);").execute(&mut c).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/202611120001_launch.sql"))
        .execute(&mut c)
        .await
        .unwrap();
    // Evaluation also reads timeouts and rules screening, later migrations.
    sqlx::raw_sql("CREATE TABLE member_timeouts(guild_id INTEGER,user_id INTEGER,until INTEGER,reason TEXT,actor_id INTEGER); CREATE TABLE community_members(guild_id INTEGER,user_id INTEGER,rules_pending INTEGER,questions_pending INTEGER)")
        .execute(&mut c)
        .await
        .unwrap();
    assert!(acl::evaluate(&mut c, Id(1), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::OWNER));
    assert!(acl::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::ADMINISTRATOR | Permissions::MANAGE_ROLES));
    let p = acl::evaluate(&mut c, Id(3), Id(10), None).await.unwrap();
    assert!(p.contains(Permissions::from_bits_retain(287)));
    assert!(!p.contains(Permissions::ADMINISTRATOR));
    assert_eq!(Permissions::OWNER.bits(), 1 << 13);
}
#[tokio::test]
async fn rest_private_channel_matrix_profiles_pins_and_device_endpoints() {
    let (_dir, mut app) = fixture().await;
    // Isolate ACL behavior from the developer PC's changing free-space reserve.
    std::sync::Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .min_free_disk_gb = 0;
    let secret = crate::state::token();
    let key = crate::state::hash(secret.as_bytes());
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=2")
        .bind(&key)
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'secret',0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(60,1,'message',11,'secret.txt','text/plain',6,'secret',0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO message_attachments VALUES(12,60,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    tokio::fs::write(app.0.config.data_dir.join("files/secret"), b"secret")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let serving = app.clone();
    let task = tokio::spawn(async move { serving.serve(listener).await });
    let client = reqwest::Client::new();
    for path in [
        "channels/11",
        "channels/11/messages",
        "channels/11/messages/12",
        "channels/11/pins",
        "channels/11/permissions",
        "files/60",
        "users/1/profile",
        "guilds/10/preview",
        "guilds/10/roles",
        "me/sessions",
        "me/notifications",
    ] {
        let response = client
            .get(format!("{url}/{path}"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{path}: {}",
            response.status()
        );
    }
    let preview: Snapshot<GuildPreview> = client
        .get(format!("{url}/guilds/10/preview"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview.data.guild.member_count, 2);
    assert_eq!(preview.data.online_count, 0);
    assert_eq!(preview.data.profile.created_at, 0);
    {
        let mut core = app.0.core.lock().await;
        let stream = core.streams.get_mut(&key).unwrap();
        stream.ever_connected = true;
        stream.touched = now();
    }
    let preview: Snapshot<GuildPreview> = client
        .get(format!("{url}/guilds/10/preview"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview.data.online_count, 1);
    {
        let mut core = app.0.core.lock().await;
        hide(&app, &mut core, 1).await;
    }
    for path in [
        "channels/11",
        "channels/11/messages",
        "channels/11/messages/12",
        "channels/11/pins",
        "channels/11/permissions",
        "files/60",
        "files/60/thumbnail",
        "channels/11/overwrites",
    ] {
        let response = client
            .get(format!("{url}/{path}"))
            .bearer_auth(&secret)
            .header("if-none-match", "*")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND, "{path}");
    }
    for path in ["guilds/10/channels", "read-states"] {
        let response: serde_json::Value = client
            .get(format!("{url}/{path}"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response["items"], serde_json::json!([]), "{path}");
    }
    let response = client
        .post(format!("{url}/uploads/resumable"))
        .bearer_auth(&secret)
        .json(&CreateUploadRequest {
            purpose: UploadPurpose::Message,
            channel_id: Some(Id(11)),
            filename: "no.txt".into(),
            content_type: "text/plain".into(),
            byte_size: 1,
            sha256: "0".repeat(64),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    task.abort();
}

#[tokio::test]
async fn hidden_category_parent_ids_and_delete_reorder_events_do_not_leak() {
    let (_dir, app) = fixture().await;
    sqlx::raw_sql("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(50,'category',10,'hidden',1,0); UPDATE channels SET parent_id=50 WHERE id=11;").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    core.cursor(&[2; 32], Id(2));
    ok(run(
        &app,
        &mut core,
        1,
        1,
        LaunchCommand::SetOverwrites {
            channel_id: Id(50),
            overwrites: vec![overwrite(
                10,
                OverwriteKind::Role,
                Permissions::empty(),
                Permissions::VIEW_CHANNEL,
            )],
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        2,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![overwrite(
                2,
                OverwriteKind::Member,
                Permissions::VIEW_CHANNEL,
                Permissions::empty(),
            )],
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let ch = db::access(&mut c, Id(2), Id(11), Permissions::VIEW_CHANNEL)
        .await
        .unwrap();
    assert_eq!(ch.summary.parent_id, None);
    let list = db::channels(&mut c, Id(2), Some(Id(10))).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].parent_id, None);
    core.invalidate(&[Id(2)]);
    ok(command(
        &app,
        &mut core,
        1,
        3,
        Command::DeleteChannel { channel_id: Id(50) },
    )
    .await);
    assert!(events(&core, 2)
        .iter()
        .all(|e| !matches!(e,Event::ChannelRemoved {channel_id,..} if *channel_id==Id(50))));
    // A private channel's deletion is sent only to its previous audience.
    hide(&app, &mut core, 4).await;
    core.invalidate(&[Id(2)]);
    // Remove the member allow so hiding is effective.
    ok(run(
        &app,
        &mut core,
        1,
        5,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![overwrite(
                10,
                OverwriteKind::Role,
                Permissions::empty(),
                Permissions::VIEW_CHANNEL,
            )],
        },
    )
    .await);
    core.invalidate(&[Id(2)]);
    ok(command(
        &app,
        &mut core,
        1,
        6,
        Command::DeleteChannel { channel_id: Id(11) },
    )
    .await);
    assert!(events(&core, 2)
        .iter()
        .all(|e| !matches!(e,Event::ChannelRemoved {channel_id,..} if *channel_id==Id(11))));
}
#[tokio::test]
async fn deleted_legacy_admin_can_be_recreated_by_old_assignment_and_transfer() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    ok(command(
        &app,
        &mut core,
        1,
        1,
        Command::SetMemberRole {
            guild_id: Id(10),
            user_id: Id(2),
            role: MemberRole::Admin,
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        2,
        LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: Id(11),
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(!acl::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::ADMINISTRATOR));
    ok(command(
        &app,
        &mut core,
        1,
        3,
        Command::SetMemberRole {
            guild_id: Id(10),
            user_id: Id(2),
            role: MemberRole::Admin,
        },
    )
    .await);
    assert!(acl::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::ADMINISTRATOR));
    ok(run(
        &app,
        &mut core,
        1,
        4,
        LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: Id(11),
        },
    )
    .await);
    ok(command(
        &app,
        &mut core,
        1,
        5,
        Command::TransferGuildOwnership {
            guild_id: Id(10),
            user_id: Id(2),
        },
    )
    .await);
    assert!(acl::evaluate(&mut c, Id(2), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::OWNER));
    assert!(acl::evaluate(&mut c, Id(1), Id(10), None)
        .await
        .unwrap()
        .contains(Permissions::ADMINISTRATOR));
}

#[tokio::test]
async fn legacy_sessions_never_receive_unknown_launch_event_variants() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[2; 32], Id(2));
    core.cursor(&[99; 32], Id(2));
    core.streams.get_mut(&vec![99; 32]).unwrap().launch_features = true;
    core.emit(
        &[Id(2)],
        Event::RolesUpdated {
            guild_id: Id(10),
            roles: vec![],
        },
    )
    .unwrap();
    core.emit(&[Id(2)], Event::SessionsUpdated).unwrap();
    assert!(events(&core, 2).is_empty());
    assert_eq!(core.streams[&vec![99; 32]].ring.len(), 2);
    // Existing events and named-field snapshots remain available to legacy peers.
    core.emit(
        &[Id(2)],
        Event::ReadStateUpdated(ReadState {
            channel_id: Id(11),
            last_read_id: None,
            latest_message_id: None,
            unread_count: 0,
            mention_count: 0,
        }),
    )
    .unwrap();
    assert_eq!(events(&core, 2).len(), 1);
}

#[tokio::test]
async fn s1_empty_partial_and_bit_removal_preserve_protected_acl() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,0,?),(11,'member',3,0,?)")
        .bind(Permissions::READ_HISTORY.bits() as i64)
        .bind(Permissions::READ_HISTORY.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut core = app.0.core.lock().await;
    let before = acl::overwrites(&mut app.0.pool.acquire().await.unwrap(), Id(11))
        .await
        .unwrap();
    for (nonce, overwrites) in [
        (1, vec![]),
        (2, vec![before[0].clone()]),
        (
            3,
            vec![
                overwrite(
                    2,
                    OverwriteKind::Member,
                    Permissions::empty(),
                    Permissions::empty(),
                ),
                before[1].clone(),
            ],
        ),
    ] {
        denied(
            run(
                &app,
                &mut core,
                2,
                nonce,
                LaunchCommand::SetOverwrites {
                    channel_id: Id(11),
                    overwrites,
                },
            )
            .await,
        );
        assert_eq!(
            acl::overwrites(&mut app.0.pool.acquire().await.unwrap(), Id(11))
                .await
                .unwrap(),
            before
        );
    }
    let mut replacement = before.clone();
    replacement[0].allow = Permissions::VIEW_CHANNEL;
    ok(run(
        &app,
        &mut core,
        2,
        4,
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: replacement,
        },
    )
    .await);
}

async fn nest(
    app: &App,
    core: &mut Core,
    nonce: u64,
    kind: ChannelKind,
    name: &str,
    parent: Option<Id>,
) -> Id {
    match ok(command(
        app,
        core,
        1,
        nonce,
        Command::CreateChannel {
            guild_id: Id(10),
            kind,
            name: name.into(),
            parent_id: parent,
            topic: None,
        },
    )
    .await)
    {
        CommandResult::Channel(ch) => {
            assert_eq!(ch.summary.parent_id, parent);
            ch.summary.id
        }
        other => panic!("{other:?}"),
    }
}
/// Channel IDs with the parent each is shown under, in sidebar order.
async fn tree(app: &App, user: u64) -> Vec<(Id, Option<Id>)> {
    let mut c = app.0.pool.acquire().await.unwrap();
    db::channels(&mut c, Id(user), Some(Id(10)))
        .await
        .unwrap()
        .into_iter()
        .map(|ch| (ch.id, ch.parent_id))
        .collect()
}
fn place(channel: Id, parent: Option<Id>, position: u16) -> ChannelPlacement {
    ChannelPlacement {
        channel_id: channel,
        parent_id: parent,
        position,
    }
}
async fn reorder(
    app: &App,
    core: &mut Core,
    nonce: u64,
    placements: Vec<ChannelPlacement>,
) -> CommandOutcome {
    command(
        app,
        core,
        1,
        nonce,
        Command::ReorderChannels {
            guild_id: Id(10),
            placements,
        },
    )
    .await
}

#[tokio::test]
async fn any_channel_nests_in_any_other_and_inherits_every_level() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    // Category in category, text in category, text in text, voice in text.
    let a = nest(&app, &mut core, 1, ChannelKind::Category, "a", None).await;
    let b = nest(&app, &mut core, 2, ChannelKind::Category, "b", Some(a)).await;
    let c = nest(&app, &mut core, 3, ChannelKind::Text, "c", Some(b)).await;
    let d = nest(&app, &mut core, 4, ChannelKind::Text, "d", Some(c)).await;
    let e = nest(&app, &mut core, 5, ChannelKind::Voice, "e", Some(d)).await;
    let general = Id(11);
    assert_eq!(
        tree(&app, 2).await,
        [
            (general, None),
            (a, None),
            (b, Some(a)),
            (c, Some(b)),
            (d, Some(c)),
            (e, Some(d))
        ]
    );
    // A category still cannot have a topic, and a parent must be in the same guild.
    denied(
        command(
            &app,
            &mut core,
            1,
            6,
            Command::CreateChannel {
                guild_id: Id(10),
                kind: ChannelKind::Category,
                name: "topic".into(),
                parent_id: Some(e),
                topic: Some("no".into()),
            },
        )
        .await,
    );
    denied(
        command(
            &app,
            &mut core,
            1,
            7,
            Command::CreateChannel {
                guild_id: Id(10),
                kind: ChannelKind::Text,
                name: "orphan".into(),
                parent_id: Some(Id(999)),
                topic: None,
            },
        )
        .await,
    );
    // Hiding the outermost channel hides everything inside it, four levels down.
    let everyone = |allow, deny| overwrite(10, OverwriteKind::Role, allow, deny);
    let member = |allow, deny| overwrite(2, OverwriteKind::Member, allow, deny);
    let view = Permissions::VIEW_CHANNEL;
    let none = Permissions::empty();
    ok(run(
        &app,
        &mut core,
        1,
        8,
        LaunchCommand::SetOverwrites {
            channel_id: a,
            overwrites: vec![everyone(none, view)],
        },
    )
    .await);
    assert_eq!(tree(&app, 2).await, [(general, None)]);
    {
        let mut conn = app.0.pool.acquire().await.unwrap();
        assert!(!acl::evaluate(&mut conn, Id(2), Id(10), Some(e))
            .await
            .unwrap()
            .contains(view));
    }
    // An allow part-way down applies to everything below it. Channels inside a
    // hidden one are shown under the nearest visible channel above them.
    ok(run(
        &app,
        &mut core,
        1,
        9,
        LaunchCommand::SetOverwrites {
            channel_id: d,
            overwrites: vec![member(view, none)],
        },
    )
    .await);
    assert_eq!(
        tree(&app, 2).await,
        [(general, None), (d, None), (e, Some(d))]
    );
    ok(run(
        &app,
        &mut core,
        1,
        10,
        LaunchCommand::SetOverwrites {
            channel_id: b,
            overwrites: vec![member(view, none)],
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        11,
        LaunchCommand::SetOverwrites {
            channel_id: c,
            overwrites: vec![member(none, view)],
        },
    )
    .await);
    // b is visible, c (inside it) is hidden again, d re-allows: d is shown in b.
    assert_eq!(
        tree(&app, 2).await,
        [(general, None), (b, None), (d, Some(b)), (e, Some(d))]
    );
    let mut conn = app.0.pool.acquire().await.unwrap();
    for (channel, parent) in [(b, None), (d, Some(b)), (e, Some(d))] {
        let seen = db::access(&mut conn, Id(2), channel, view).await.unwrap();
        assert_eq!(seen.summary.parent_id, parent);
    }
    assert!(db::access(&mut conn, Id(2), c, view).await.is_err());
    // The owner still sees the real tree.
    drop(conn);
    assert_eq!(
        tree(&app, 1).await[1..],
        [
            (a, None),
            (b, Some(a)),
            (c, Some(b)),
            (d, Some(c)),
            (e, Some(d))
        ]
    );
}

#[tokio::test]
async fn nesting_is_as_deep_as_the_guild_channel_limit() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    // `general` plus a chain of 499, each inside the one before it: far past the
    // old 64-level evaluation bound, so the bound must follow the channel limit.
    let depth = MAX_CHANNELS_PER_GUILD - 1;
    // Stored directly up to the last level, which goes through the command.
    let mut chain: Vec<Id> = vec![];
    for n in 0..depth as u64 - 1 {
        let kind = if n % 2 == 0 { "text" } else { "category" };
        sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(?,?,10,?,?,0,0)")
            .bind(100_000 + n as i64)
            .bind(kind)
            .bind(chain.last().map(|v| v.0 as i64))
            .bind(format!("level{n}"))
            .execute(&app.0.pool)
            .await
            .unwrap();
        chain.push(Id(100_000 + n));
    }
    let parent = chain.last().copied();
    chain.push(nest(&app, &mut core, 1, ChannelKind::Text, "deepest", parent).await);
    denied(
        command(
            &app,
            &mut core,
            1,
            1000,
            Command::CreateChannel {
                guild_id: Id(10),
                kind: ChannelKind::Text,
                name: "one-too-many".into(),
                parent_id: chain.last().copied(),
                topic: None,
            },
        )
        .await,
    );
    let listed = tree(&app, 2).await;
    assert_eq!(listed.len(), MAX_CHANNELS_PER_GUILD);
    assert_eq!(listed[depth], (chain[depth - 1], Some(chain[depth - 2])));
    // A deny on the outermost level reaches the innermost one.
    ok(run(
        &app,
        &mut core,
        1,
        1001,
        LaunchCommand::SetOverwrites {
            channel_id: chain[0],
            overwrites: vec![overwrite(
                10,
                OverwriteKind::Role,
                Permissions::empty(),
                Permissions::SEND_MESSAGES,
            )],
        },
    )
    .await);
    let mut conn = app.0.pool.acquire().await.unwrap();
    let deepest = acl::evaluate(&mut conn, Id(2), Id(10), Some(chain[depth - 1]))
        .await
        .unwrap();
    assert!(deepest.contains(Permissions::VIEW_CHANNEL));
    assert!(!deepest.contains(Permissions::SEND_MESSAGES));
}

#[tokio::test]
async fn moving_channels_keeps_subtrees_rejects_loops_and_delete_moves_children_up() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[2; 32], Id(2));
    let general = Id(11);
    let x = nest(&app, &mut core, 1, ChannelKind::Text, "x", None).await;
    let y = nest(&app, &mut core, 2, ChannelKind::Text, "y", Some(x)).await;
    let z = nest(&app, &mut core, 3, ChannelKind::Text, "z", Some(y)).await;
    // A channel cannot go inside itself, directly or through what it contains.
    denied(
        reorder(
            &app,
            &mut core,
            4,
            vec![
                place(general, None, 0),
                place(x, Some(z), 0),
                place(y, Some(x), 0),
                place(z, Some(y), 0),
            ],
        )
        .await,
    );
    denied(
        reorder(
            &app,
            &mut core,
            5,
            vec![
                place(general, None, 0),
                place(x, Some(x), 0),
                place(y, Some(x), 0),
                place(z, Some(y), 0),
            ],
        )
        .await,
    );
    // Sibling positions that change must stay dense.
    denied(
        reorder(
            &app,
            &mut core,
            6,
            vec![
                place(general, None, 0),
                place(x, None, 1),
                place(y, None, 3),
                place(z, Some(y), 0),
            ],
        )
        .await,
    );
    assert_eq!(
        tree(&app, 1).await,
        [(general, None), (x, None), (y, Some(x)), (z, Some(y))]
    );
    // Moving y to the top takes z with it.
    core.invalidate(&[Id(2)]);
    ok(reorder(
        &app,
        &mut core,
        7,
        vec![
            place(general, None, 0),
            place(x, None, 1),
            place(y, None, 2),
            place(z, Some(y), 0),
        ],
    )
    .await);
    assert_eq!(
        tree(&app, 1).await,
        [(general, None), (x, None), (y, None), (z, Some(y))]
    );
    assert!(events(&core, 2).iter().any(|e| matches!(
        e,
        Event::ChannelsReordered { placements, .. }
            if placements.contains(&place(y, None, 2)) && placements.contains(&place(z, Some(y), 0))
    )));
    // A text channel can go three levels down, inside other text channels.
    ok(reorder(
        &app,
        &mut core,
        8,
        vec![
            place(general, Some(z), 0),
            place(x, None, 0),
            place(y, None, 1),
            place(z, Some(y), 0),
        ],
    )
    .await);
    assert_eq!(
        tree(&app, 1).await,
        [(x, None), (y, None), (z, Some(y)), (general, Some(z))]
    );
    // Deleting a channel keeps what is inside it, one level up.
    ok(command(
        &app,
        &mut core,
        1,
        9,
        Command::DeleteChannel { channel_id: z },
    )
    .await);
    assert_eq!(
        tree(&app, 1).await,
        [(x, None), (y, None), (general, Some(y))]
    );
    ok(command(
        &app,
        &mut core,
        1,
        10,
        Command::DeleteChannel { channel_id: y },
    )
    .await);
    assert_eq!(tree(&app, 1).await, [(x, None), (general, None)]);
    let mut conn = app.0.pool.acquire().await.unwrap();
    let moved = db::channel(&mut conn, general).await.unwrap();
    assert_eq!(moved.summary.position, 1);
}

async fn reorder_as(
    app: &App,
    core: &mut Core,
    user: u64,
    nonce: u64,
    placements: Vec<ChannelPlacement>,
) -> CommandOutcome {
    command(
        app,
        core,
        user,
        nonce,
        Command::ReorderChannels {
            guild_id: Id(10),
            placements,
        },
    )
    .await
}
/// The checks a dragged channel's `ReorderChannels` meets (task `chandrag`):
/// the server-wide permission, every channel being this server's, each moved
/// channel and both ends of its move manageable through channel overwrites,
/// and no loop.
#[tokio::test]
async fn reordering_checks_membership_overwrites_servers_and_loops() {
    let (_dir, app) = fixture().await;
    seed_roles(&app).await;
    let mut core = app.0.core.lock().await;
    let general = Id(11);
    let a = nest(&app, &mut core, 1, ChannelKind::Category, "a", None).await;
    let b = nest(&app, &mut core, 2, ChannelKind::Text, "b", Some(a)).await;
    let v = nest(&app, &mut core, 3, ChannelKind::Voice, "v", None).await;
    let start = [(general, None), (a, None), (b, Some(a)), (v, None)];
    assert_eq!(tree(&app, 1).await, start);
    let swap = move || {
        vec![
            place(v, None, 0),
            place(general, None, 1),
            place(a, None, 2),
            place(b, Some(a), 0),
        ]
    };
    // A member without Manage Channels cannot reorder at all.
    denied(reorder_as(&app, &mut core, 2, 10, swap()).await);
    assert_eq!(tree(&app, 1).await, start);
    // With it (a role), they can.
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE id=30")
        .bind(Permissions::MANAGE_CHANNELS.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    core.invalidate(&[Id(2)]);
    ok(reorder_as(&app, &mut core, 2, 11, swap()).await);
    assert_eq!(
        tree(&app, 1).await,
        [(v, None), (general, None), (a, None), (b, Some(a))]
    );
    // A channel of another server, or a parent from there, is refused.
    sqlx::raw_sql("INSERT INTO guilds VALUES(20,1,'other',NULL,0); INSERT INTO guild_members VALUES(20,1,'owner',0); INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'elsewhere',0,0),(22,'category',20,'away',1,0);").execute(&app.0.pool).await.unwrap();
    for (n, placements) in [
        (
            12,
            vec![
                place(general, None, 0),
                place(v, None, 1),
                place(a, None, 2),
                place(Id(21), None, 3),
            ],
        ),
        (
            13,
            vec![
                place(general, Some(Id(22)), 0),
                place(v, None, 0),
                place(a, None, 1),
                place(b, Some(a), 0),
            ],
        ),
    ] {
        denied(reorder(&app, &mut core, n, placements).await);
    }
    // A loop through two channels is refused.
    denied(
        reorder(
            &app,
            &mut core,
            14,
            vec![
                place(v, Some(b), 0),
                place(general, None, 0),
                place(a, Some(v), 0),
                place(b, Some(a), 0),
            ],
        )
        .await,
    );
    assert_eq!(
        tree(&app, 1).await,
        [(v, None), (general, None), (a, None), (b, Some(a))]
    );
    // A channel locked for this manager by an overwrite cannot be moved, nor can
    // anything be put into it, but the rest of the list can still be arranged.
    let deny = |target| {
        vec![overwrite(
            target,
            OverwriteKind::Member,
            Permissions::empty(),
            Permissions::MANAGE_CHANNELS,
        )]
    };
    ok(run(
        &app,
        &mut core,
        1,
        15,
        LaunchCommand::SetOverwrites {
            channel_id: general,
            overwrites: deny(2),
        },
    )
    .await);
    denied(
        reorder_as(
            &app,
            &mut core,
            2,
            16,
            vec![
                place(general, None, 0),
                place(v, None, 1),
                place(a, None, 2),
                place(b, Some(a), 0),
            ],
        )
        .await,
    );
    denied(
        reorder_as(
            &app,
            &mut core,
            2,
            17,
            vec![
                place(v, None, 0),
                place(general, None, 1),
                place(a, None, 2),
                place(b, Some(general), 0),
            ],
        )
        .await,
    );
    // b moves into v; general stays where it is.
    ok(reorder_as(
        &app,
        &mut core,
        2,
        18,
        vec![
            place(v, None, 0),
            place(general, None, 1),
            place(a, None, 2),
            place(b, Some(v), 0),
        ],
    )
    .await);
    assert_eq!(
        tree(&app, 1).await,
        [(v, None), (b, Some(v)), (general, None), (a, None)]
    );
    // Taking a channel out of one the manager may not manage is refused, even
    // when the channel itself allows it.
    ok(run(
        &app,
        &mut core,
        1,
        19,
        LaunchCommand::SetOverwrites {
            channel_id: v,
            overwrites: deny(2),
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        20,
        LaunchCommand::SetOverwrites {
            channel_id: b,
            overwrites: vec![overwrite(
                2,
                OverwriteKind::Member,
                Permissions::MANAGE_CHANNELS,
                Permissions::empty(),
            )],
        },
    )
    .await);
    denied(
        reorder_as(
            &app,
            &mut core,
            2,
            21,
            vec![
                place(v, None, 0),
                place(general, None, 1),
                place(a, None, 2),
                place(b, None, 3),
            ],
        )
        .await,
    );
    assert_eq!(
        tree(&app, 1).await,
        [(v, None), (b, Some(v)), (general, None), (a, None)]
    );
    // A channel shown at the top only because its real parent is hidden from
    // the manager is not pulled out of that parent by renumbering the top.
    ok(run(
        &app,
        &mut core,
        1,
        22,
        LaunchCommand::SetOverwrites {
            channel_id: v,
            overwrites: vec![overwrite(
                2,
                OverwriteKind::Member,
                Permissions::empty(),
                Permissions::VIEW_CHANNEL,
            )],
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        23,
        LaunchCommand::SetOverwrites {
            channel_id: b,
            overwrites: vec![overwrite(
                2,
                OverwriteKind::Member,
                Permissions::VIEW_CHANNEL | Permissions::MANAGE_CHANNELS,
                Permissions::empty(),
            )],
        },
    )
    .await);
    ok(run(
        &app,
        &mut core,
        1,
        24,
        LaunchCommand::SetOverwrites {
            channel_id: general,
            overwrites: vec![],
        },
    )
    .await);
    assert_eq!(tree(&app, 2).await, [(general, None), (a, None), (b, None)]);
    denied(
        reorder_as(
            &app,
            &mut core,
            2,
            25,
            vec![
                place(general, None, 0),
                place(a, None, 1),
                place(b, None, 2),
            ],
        )
        .await,
    );
    assert_eq!(
        tree(&app, 1).await,
        [(v, None), (b, Some(v)), (general, None), (a, None)]
    );
}

#[tokio::test]
async fn nested_channel_migration_keeps_data_and_lets_categories_nest() {
    let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut c)
        .await
        .unwrap();
    let mut old = sqlx::migrate!();
    old.migrations = std::borrow::Cow::Owned(
        old.iter()
            .filter(|m| m.version < 202611220001)
            .cloned()
            .collect(),
    );
    old.run(&mut c).await.unwrap();
    sqlx::raw_sql("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(1,'owner','','online',0),(2,'guest','','online',0); INSERT INTO guilds VALUES(10,1,'old',NULL,0); INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'member',0); INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(11,'category',10,'cat',0,0); INSERT INTO channels(id,kind,guild_id,parent_id,name,topic,position,created_at) VALUES(12,'text',10,11,'chat','topic',0,0); INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(13,12,1,'retained',0); INSERT INTO read_states(user_id,channel_id) VALUES(2,12); INSERT INTO permission_overwrites VALUES(11,'member',2,0,2);").execute(&mut c).await.unwrap();
    // The old schema refused a category inside anything.
    assert!(sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(14,'category',10,12,'inner',0,0)")
        .execute(&mut c)
        .await
        .is_err());
    db::migrate(&mut c).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    let kept: (i64, String, String) = sqlx::query_as(
        "SELECT c.parent_id,c.topic,m.content FROM channels c JOIN messages m ON m.channel_id=c.id WHERE c.id=12",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(kept, (11, "topic".into(), "retained".into()));
    let rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM read_states),(SELECT COUNT(*) FROM permission_overwrites),(SELECT COUNT(*) FROM history_entries)",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(rows, (1, 1, 1));
    // The category's deny still reaches the channel inside it.
    assert!(!acl::evaluate(&mut c, Id(2), Id(10), Some(Id(12)))
        .await
        .unwrap()
        .contains(Permissions::SEND_MESSAGES));
    sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(14,'category',10,12,'inner',0,0)")
        .execute(&mut c)
        .await
        .unwrap();
    // A category still has no topic, and deleting a channel still cascades.
    assert!(sqlx::query("UPDATE channels SET topic='no' WHERE id=14")
        .execute(&mut c)
        .await
        .is_err());
    sqlx::query("DELETE FROM channels WHERE id=12")
        .execute(&mut c)
        .await
        .unwrap();
    let left: (i64, Option<i64>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages),(SELECT parent_id FROM channels WHERE id=14)",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(left, (0, None));
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut c)
        .await
        .unwrap()
        .is_empty());
}
