//! Moderation: every command, route, permission check and limit.
use super::*;
use crate::{audit_tests::fixture, commands};
use axum::body::Body;
use base64::Engine as _;
use tower::ServiceExt;

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
fn m(cmd: ModerationCommand) -> Command {
    Command::Launch(LaunchCommand::Moderation(cmd))
}
fn ok(o: &CommandOutcome) {
    assert!(matches!(o, CommandOutcome::Ok(_)), "{o:?}");
}
fn err(o: &CommandOutcome) -> ApiError {
    match o {
        CommandOutcome::Err(e) => e.clone(),
        o => panic!("expected an error, got {o:?}"),
    }
}
fn send(content: &str) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: Id(11),
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    })
}
async fn sql(app: &App, q: &'static str) {
    sqlx::query(q).execute(&app.0.pool).await.unwrap();
}
async fn scalar(app: &App, q: &'static str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}
/// Guild 10: owner 1; members 2 (mod role 30 with the given rights), 3 and 4.
async fn guild(rights: Permissions) -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(
        &app,
        "INSERT INTO guild_members VALUES(10,3,'member',0),(10,4,'member',0)",
    )
    .await;
    sql(
        &app,
        "INSERT INTO read_states(user_id,channel_id) VALUES(3,11),(4,11)",
    )
    .await;
    sqlx::query(
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,30,'mods',2,?)",
    )
    .bind(rights.bits() as i64)
    .execute(&app.0.pool)
    .await
    .unwrap();
    sql(&app, "INSERT INTO member_roles VALUES(10,2,30)").await;
    // Accounts are old enough for any verification level unless a test says otherwise.
    sql(&app, "UPDATE users SET created_at=0").await;
    (dir, app)
}
fn timeout(user: u64, seconds: u32) -> Command {
    m(ModerationCommand::Timeout {
        guild_id: Id(10),
        user_id: Id(user),
        seconds,
        reason: Some("cool off".into()),
    })
}
async fn entries(app: &App, action: AuditAction) -> Vec<(i64, Option<i64>, Option<String>)> {
    sqlx::query("SELECT actor_id,target_id,reason FROM audit_log WHERE guild_id=10 AND action=? ORDER BY id")
        .bind(action.name())
        .fetch_all(&app.0.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect()
}
/// A GET through the real router, as user `u` with a bearer token.
async fn http_get(app: &App, u: u8, path: &str) -> (u16, Vec<u8>) {
    let t = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u; 32]);
    sqlx::query("INSERT OR IGNORE INTO sessions VALUES(?,?,?,?,?)")
        .bind(crate::state::hash(t.as_bytes()))
        .bind(i64::from(u))
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    app.0.core.lock().await.limits.clear();
    let r = crate::http::router(app.clone())
        .oneshot(
            axum::http::Request::get(path)
                .header("authorization", format!("Bearer {t}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (status, bytes.to_vec())
}
fn ring(core: &Core, u: u8) -> Vec<Event> {
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

#[tokio::test]
async fn timeouts_need_the_permission_respect_hierarchy_and_silence_the_member() {
    let (_dir, app) = guild(Permissions::MODERATE_MEMBERS).await;
    // Without Time Out Members: refused.
    assert_eq!(
        err(&run(&app, 3, 1, timeout(4, 60)).await).code,
        ErrorCode::Forbidden
    );
    // Never yourself, never the owner, never above the moderator's top role.
    assert_eq!(
        err(&run(&app, 2, 1, timeout(2, 60)).await).code,
        ErrorCode::HierarchyViolation
    );
    assert_eq!(
        err(&run(&app, 2, 2, timeout(1, 60)).await).code,
        ErrorCode::HierarchyViolation
    );
    sql(
        &app,
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,31,'senior',3,0)",
    )
    .await;
    sql(&app, "INSERT INTO member_roles VALUES(10,4,31)").await;
    assert_eq!(
        err(&run(&app, 2, 3, timeout(4, 60)).await).code,
        ErrorCode::HierarchyViolation
    );
    // Administrators cannot be timed out, even by the owner.
    sql(&app, "INSERT INTO member_roles VALUES(10,4,11)").await;
    assert_eq!(
        err(&run(&app, 1, 1, timeout(4, 60)).await).code,
        ErrorCode::HierarchyViolation
    );
    // Bounded: 28 days at most.
    assert_eq!(
        err(&run(&app, 2, 4, timeout(3, MAX_TIMEOUT_SECONDS + 1)).await).code,
        ErrorCode::InvalidRequest
    );
    let o = run(&app, 2, 5, timeout(3, 600)).await;
    let CommandOutcome::Ok(CommandResult::Member(member)) = o else {
        panic!("{o:?}")
    };
    assert!(member.timeout_until.unwrap() > now() + 590_000);
    // The member reads but cannot talk, react or connect.
    let e = err(&run(&app, 3, 1, send("hello")).await);
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::Forbidden, Some("timeout"))
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    let p = acl::evaluate(&mut c, Id(3), Id(10), Some(Id(11)))
        .await
        .unwrap();
    assert_eq!(p, TIMED_OUT_KEEPS & p);
    assert!(p.contains(Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY));
    assert!(!p.intersects(
        Permissions::SEND_MESSAGES
            | Permissions::ADD_REACTIONS
            | Permissions::CONNECT
            | Permissions::SPEAK
    ));
    let s = state(&mut c, Id(3), Id(10)).await.unwrap();
    assert_eq!(s.timeout.unwrap().reason.as_deref(), Some("cool off"));
    drop(c);
    let o = run(
        &app,
        3,
        2,
        Command::AddReaction {
            channel_id: Id(11),
            message_id: Id(1),
            emoji: "👍".into(),
        },
    )
    .await;
    assert_eq!(err(&o).code, ErrorCode::Forbidden);
    assert_eq!(
        entries(&app, AuditAction::MemberTimeout).await,
        vec![(2, Some(3), Some("cool off".into()))]
    );
    // Removed early.
    ok(&run(&app, 2, 6, timeout(3, 0)).await);
    ok(&run(&app, 3, 3, send("back")).await);
    assert_eq!(
        entries(&app, AuditAction::MemberTimeoutRemove).await.len(),
        1
    );
}

#[tokio::test]
async fn timeouts_expire_by_themselves_and_notify() {
    let (_dir, app) = guild(Permissions::MODERATE_MEMBERS).await;
    // Member 3 has an event stream.
    let _ = run(&app, 3, 9, Command::Typing { channel_id: Id(11) }).await;
    ok(&run(&app, 2, 1, timeout(3, 60)).await);
    // Due now: the expiry task's single pass clears it and returns no next deadline.
    sql(&app, "UPDATE member_timeouts SET until=1").await;
    assert_eq!(expire(&app).await.unwrap(), None);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM member_timeouts").await,
        0
    );
    let core = app.0.core.lock().await;
    assert!(ring(&core, 3).iter().any(
        |e| matches!(e, Event::MemberUpsert(m) if m.user.id == Id(3) && m.timeout_until.is_none())
    ));
    drop(core);
    ok(&run(&app, 3, 1, send("free")).await);
    // The next deadline is reported for the task to sleep until.
    ok(&run(&app, 2, 2, timeout(3, 60)).await);
    let next = expire(&app).await.unwrap().unwrap();
    assert!(next > now() + 50_000 && next <= now() + 60_000);
}

#[tokio::test]
async fn kick_with_reason_is_permission_checked_and_logged() {
    let (_dir, app) = guild(Permissions::KICK_MEMBERS).await;
    let kick = |u, r: &str| {
        m(ModerationCommand::Kick {
            guild_id: Id(10),
            user_id: Id(u),
            reason: Some(r.into()),
        })
    };
    assert_eq!(
        err(&run(&app, 3, 1, kick(4, "x")).await).code,
        ErrorCode::Forbidden
    );
    assert_eq!(
        err(&run(&app, 2, 1, kick(1, "x")).await).code,
        ErrorCode::HierarchyViolation
    );
    assert_eq!(
        err(&run(&app, 2, 2, kick(4, &"r".repeat(MAX_REASON_CHARS + 1))).await).code,
        ErrorCode::InvalidRequest
    );
    ok(&run(&app, 2, 3, kick(4, "spamming")).await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM guild_members WHERE user_id=4").await,
        0
    );
    assert_eq!(
        entries(&app, AuditAction::MemberKick).await,
        vec![(2, Some(4), Some("spamming".into()))]
    );
    // The legacy kick and ban are logged too, the ban with its reason.
    ok(&run(
        &app,
        2,
        4,
        Command::KickMember {
            guild_id: Id(10),
            user_id: Id(3),
        },
    )
    .await);
    assert_eq!(entries(&app, AuditAction::MemberKick).await.len(), 2);
    ok(&run(
        &app,
        1,
        1,
        Command::BanMember {
            guild_id: Id(10),
            user_id: Id(4),
            reason: Some("again".into()),
        },
    )
    .await);
    assert_eq!(
        entries(&app, AuditAction::MemberBan).await,
        vec![(1, Some(4), Some("again".into()))]
    );
    ok(&run(
        &app,
        1,
        2,
        Command::UnbanMember {
            guild_id: Id(10),
            user_id: Id(4),
        },
    )
    .await);
    assert_eq!(entries(&app, AuditAction::MemberUnban).await.len(), 1);
}

#[tokio::test]
async fn prune_previews_and_removes_only_inactive_members_below_the_actor() {
    let (_dir, app) = guild(Permissions::KICK_MEMBERS).await;
    let prune = |days, roles: Vec<Id>| {
        m(ModerationCommand::Prune {
            guild_id: Id(10),
            days,
            role_ids: roles,
            reason: None,
        })
    };
    assert_eq!(
        err(&run(&app, 3, 1, prune(7, vec![])).await).code,
        ErrorCode::Forbidden
    );
    assert_eq!(
        err(&run(&app, 2, 1, prune(3, vec![])).await).code,
        ErrorCode::InvalidRequest
    );
    // Every command counts as activity, so age the sessions afterwards.
    let old = now().saturating_sub(40 * 86_400_000) as i64;
    sqlx::query("UPDATE sessions SET last_used_at=?")
        .bind(old)
        .execute(&app.0.pool)
        .await
        .unwrap();
    {
        let core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await.unwrap();
        // Members 3 and 4 have no roles and are inactive; the owner and the actor never count.
        let n = prune_candidates(&mut c, &core, Id(10), Id(2), 7, &[])
            .await
            .unwrap();
        assert_eq!(n, vec![Id(3), Id(4)]);
    }
    sql(
        &app,
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,32,'regulars',1,0)",
    )
    .await;
    sql(&app, "INSERT INTO member_roles VALUES(10,4,32)").await;
    // Role-limited prune: only members with that role.
    let o = run(&app, 2, 2, prune(30, vec![Id(32)])).await;
    assert!(
        matches!(
            o,
            CommandOutcome::Ok(CommandResult::Moderation(ModerationResult::Pruned {
                count: 1
            }))
        ),
        "{o:?}"
    );
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM guild_members WHERE guild_id=10").await,
        3
    );
    assert_eq!(entries(&app, AuditAction::MemberPrune).await.len(), 1);
    // Recently active members are kept.
    sql(
        &app,
        "UPDATE sessions SET last_used_at=strftime('%s','now')*1000 WHERE user_id=3",
    )
    .await;
    let o = run(&app, 2, 3, prune(1, vec![])).await;
    assert!(
        matches!(
            o,
            CommandOutcome::Ok(CommandResult::Moderation(ModerationResult::Pruned {
                count: 0
            }))
        ),
        "{o:?}"
    );
}

#[tokio::test]
async fn audit_log_records_existing_actions_in_their_transaction() {
    let (_dir, app) = guild(Permissions::empty()).await;
    let launch = |c| Command::Launch(c);
    let o = run(
        &app,
        1,
        1,
        launch(LaunchCommand::CreateRole {
            guild_id: Id(10),
            name: "helpers".into(),
        }),
    )
    .await;
    let CommandOutcome::Ok(CommandResult::Role(mut role)) = o else {
        panic!("{o:?}")
    };
    role.permissions = Permissions::KICK_MEMBERS;
    role.name = "Helpers".into();
    ok(&run(
        &app,
        1,
        2,
        launch(LaunchCommand::UpdateRole { role: role.clone() }),
    )
    .await);
    ok(&run(
        &app,
        1,
        3,
        launch(LaunchCommand::SetMemberRoles {
            guild_id: Id(10),
            user_id: Id(3),
            role_ids: vec![role.id],
        }),
    )
    .await);
    ok(&run(
        &app,
        1,
        4,
        Command::CreateChannel {
            guild_id: Id(10),
            kind: ChannelKind::Text,
            name: "logs".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await);
    ok(&run(
        &app,
        1,
        5,
        launch(LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: Id(10),
                allow: Permissions::empty(),
                deny: Permissions::ATTACH_FILES,
            }],
        }),
    )
    .await);
    ok(&run(
        &app,
        1,
        6,
        launch(LaunchCommand::SetSlowmode {
            channel_id: Id(11),
            seconds: 15,
        }),
    )
    .await);
    ok(&run(
        &app,
        1,
        7,
        Command::UpdateGuild {
            guild_id: Id(10),
            patch: GuildPatch {
                name: Patch::Set("renamed".into()),
                ..Default::default()
            },
        },
    )
    .await);
    let o = run(
        &app,
        1,
        8,
        Command::CreateInvite {
            guild_id: Id(10),
            ttl_seconds: 600,
            max_uses: 5,
        },
    )
    .await;
    let CommandOutcome::Ok(CommandResult::Invite(invite)) = o else {
        panic!("{o:?}")
    };
    ok(&run(
        &app,
        1,
        9,
        Command::RevokeInvite {
            guild_id: Id(10),
            code: invite.code,
        },
    )
    .await);
    let o = run(&app, 2, 1, send("delete me")).await;
    let CommandOutcome::Ok(CommandResult::Message(msg)) = o else {
        panic!("{o:?}")
    };
    ok(&run(
        &app,
        1,
        10,
        launch(LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: msg.id,
            pinned: true,
        }),
    )
    .await);
    ok(&run(
        &app,
        1,
        11,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: msg.id,
        },
    )
    .await);
    // An author deleting their own message is not a moderation action.
    let o = run(&app, 3, 2, send("mine")).await;
    let CommandOutcome::Ok(CommandResult::Message(own)) = o else {
        panic!("{o:?}")
    };
    ok(&run(
        &app,
        3,
        3,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: own.id,
        },
    )
    .await);
    // A no-op save records nothing.
    ok(&run(
        &app,
        1,
        12,
        launch(LaunchCommand::UpdateRole {
            role: {
                let mut r = role.clone();
                r.name = "Helpers".into();
                r
            },
        }),
    )
    .await);
    ok(&run(
        &app,
        1,
        13,
        launch(LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: role.id,
        }),
    )
    .await);
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE guild_id=10 ORDER BY id")
            .fetch_all(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(
        actions,
        [
            "role_create",
            "role_update",
            "member_roles_update",
            "channel_create",
            "overwrites_update",
            "channel_update",
            "guild_update",
            "invite_create",
            "invite_delete",
            "message_pin",
            "message_delete",
            "role_delete",
        ]
    );
    let changes: String = scalar_text(
        &app,
        "SELECT changes FROM audit_log WHERE action='role_update'",
    )
    .await;
    assert!(
        changes.contains("Kick Members") && changes.contains("Helpers"),
        "{changes}"
    );
    // A refused command leaves no entry.
    let before = scalar(&app, "SELECT COUNT(*) FROM audit_log").await;
    assert_eq!(
        err(&run(
            &app,
            3,
            1,
            launch(LaunchCommand::CreateRole {
                guild_id: Id(10),
                name: "x".into()
            })
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM audit_log").await, before);
}
async fn scalar_text(app: &App, q: &'static str) -> String {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}

#[tokio::test]
async fn audit_log_route_checks_view_audit_log_filters_pages_and_is_bounded() {
    let (_dir, app) = guild(Permissions::VIEW_AUDIT_LOG).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    for i in 0..(MAX_AUDIT_ENTRIES_PER_GUILD + 5) {
        let (actor, action) = if i % 2 == 0 {
            (1, AuditAction::RoleCreate)
        } else {
            (2, AuditAction::ChannelCreate)
        };
        audit::write(
            &mut c,
            Id(actor),
            audit::Entry::new(Id(10), action, AuditTarget::Guild, Some(Id(10))).change(
                "n",
                None,
                Some(i.to_string()),
            ),
        )
        .await
        .unwrap();
    }
    drop(c);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM audit_log").await,
        MAX_AUDIT_ENTRIES_PER_GUILD as i64
    );
    let get = |u: u8, path: &str| {
        let (app, path) = (app.clone(), path.to_owned());
        async move { http_get(&app, u, &path).await }
    };
    // Members without View Audit Log are refused.
    assert_eq!(get(3, "/api/v1/guilds/10/audit-log").await.0, 403);
    let (status, body) = get(
        2,
        "/api/v1/guilds/10/audit-log?limit=10&actor=2&action=channel_create",
    )
    .await;
    assert_eq!(status, 200);
    let page: AuditLogPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.items.len(), 10);
    assert!(page.has_more);
    assert!(page
        .items
        .iter()
        .all(|e| e.actor_id == Id(2) && e.action == AuditAction::ChannelCreate));
    assert!(
        page.items.windows(2).all(|w| w[0].id > w[1].id),
        "newest first"
    );
    let next = page.next_before.unwrap();
    let (_, body) = get(
        2,
        &format!("/api/v1/guilds/10/audit-log?limit=10&before={}", next.0),
    )
    .await;
    let page2: AuditLogPage = serde_json::from_slice(&body).unwrap();
    assert!(page2.items.iter().all(|e| e.id < next));
    assert!(page2.users.iter().any(|u| u.id == Id(1)));
    assert_eq!(
        get(2, "/api/v1/guilds/10/audit-log?action=nonsense")
            .await
            .0,
        400
    );
    assert_eq!(
        get(2, "/api/v1/guilds/10/audit-log?limit=1000").await.0,
        400
    );
    // Other moderation routes check their permissions too.
    assert_eq!(get(3, "/api/v1/guilds/10/automod/rules").await.0, 403);
    assert_eq!(get(3, "/api/v1/guilds/10/prune?days=7").await.0, 403);
    assert_eq!(get(3, "/api/v1/channels/11/access").await.0, 403);
    assert_eq!(get(3, "/api/v1/guilds/10/moderation/me").await.0, 200);
}

fn keyword_rule(alert: Option<Id>, timeout_seconds: u32) -> AutoModRule {
    AutoModRule {
        id: Id(0),
        guild_id: Id(10),
        name: "No swearing".into(),
        enabled: true,
        trigger: AutoModTrigger::Keyword {
            keywords: vec!["heck*".into()],
            allow: vec!["heckin".into()],
        },
        actions: AutoModActions {
            block: true,
            message: None,
            alert_channel_id: alert,
            timeout_seconds,
        },
        exempt_roles: vec![Id(30)],
        exempt_channels: vec![],
    }
}

#[tokio::test]
async fn automod_blocks_on_send_and_edit_alerts_times_out_and_respects_exemptions() {
    let (_dir, app) = guild(Permissions::empty()).await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'mod-log',1,0)").await;
    let create = |r| m(ModerationCommand::CreateAutoModRule(r));
    // Manage Server only.
    assert_eq!(
        err(&run(&app, 3, 1, create(keyword_rule(None, 0))).await).code,
        ErrorCode::Forbidden
    );
    // Validated: wildcards only at the ends, alert channels must be text channels of this server.
    let mut bad = keyword_rule(None, 0);
    bad.trigger = AutoModTrigger::Keyword {
        keywords: vec!["he*ck".into()],
        allow: vec![],
    };
    assert_eq!(
        err(&run(&app, 1, 1, create(bad)).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 1, 2, create(keyword_rule(Some(Id(999)), 0))).await).code,
        ErrorCode::InvalidRequest
    );
    let o = run(&app, 1, 3, create(keyword_rule(Some(Id(12)), 300))).await;
    let CommandOutcome::Ok(CommandResult::Moderation(ModerationResult::AutoModRule(rule))) = o
    else {
        panic!("{o:?}")
    };
    // Blocked before it is stored, with an explanation; the alert and timeout happen anyway.
    let before = scalar(&app, "SELECT COUNT(*) FROM messages WHERE channel_id=11").await;
    let e = err(&run(&app, 3, 1, send("what the HECKING")).await);
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::Forbidden, Some("automod"))
    );
    assert!(e.message.contains("hecking"), "{}", e.message);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM messages WHERE channel_id=11").await,
        before
    );
    let alert: String = scalar_text(
        &app,
        "SELECT content FROM messages WHERE channel_id=12 AND author_id=4194303",
    )
    .await;
    assert!(
        alert.contains("@user3") && alert.contains("No swearing"),
        "{alert}"
    );
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM member_timeouts WHERE user_id=3").await,
        1
    );
    for a in [
        AuditAction::AutomodBlock,
        AuditAction::AutomodAlert,
        AuditAction::AutomodTimeout,
    ] {
        assert_eq!(entries(&app, a).await.len(), 1, "{a:?}");
    }
    // Allow list and whole-word matching.
    ok(&run(&app, 4, 1, send("heckin good")).await);
    // Edits are checked too.
    let o = run(&app, 4, 2, send("clean")).await;
    let CommandOutcome::Ok(CommandResult::Message(msg)) = o else {
        panic!("{o:?}")
    };
    let edit = Command::EditMessage(MessageEdit {
        channel_id: Id(11),
        message_id: msg.id,
        content: "heck".into(),
        mention_ids: vec![],
    });
    assert_eq!(
        err(&run(&app, 4, 3, edit).await).field.as_deref(),
        Some("automod")
    );
    // Exempt role, and Manage Server, bypass rules.
    ok(&run(&app, 2, 1, send("heck")).await);
    ok(&run(&app, 1, 4, send("heck")).await);
    // Disabled rules do nothing; deleting is logged.
    sql(&app, "DELETE FROM member_timeouts").await;
    let mut off = rule.clone();
    off.enabled = false;
    ok(&run(&app, 1, 5, m(ModerationCommand::UpdateAutoModRule(off))).await);
    ok(&run(&app, 4, 4, send("heck")).await);
    ok(&run(
        &app,
        1,
        6,
        m(ModerationCommand::DeleteAutoModRule {
            guild_id: Id(10),
            rule_id: rule.id,
        }),
    )
    .await);
    for a in [
        AuditAction::AutomodRuleCreate,
        AuditAction::AutomodRuleUpdate,
        AuditAction::AutomodRuleDelete,
    ] {
        assert_eq!(entries(&app, a).await.len(), 1, "{a:?}");
    }
}

#[tokio::test]
async fn automod_rule_counts_are_bounded_and_spam_rules_fire() {
    let (_dir, app) = guild(Permissions::empty()).await;
    let create = |r| m(ModerationCommand::CreateAutoModRule(r));
    let mut mention = keyword_rule(None, 0);
    mention.exempt_roles.clear();
    mention.trigger = AutoModTrigger::MentionSpam { limit: 2 };
    ok(&run(&app, 1, 1, create(mention.clone())).await);
    assert_eq!(
        err(&run(&app, 1, 2, create(mention)).await).code,
        ErrorCode::AlreadyExists
    );
    let mut spam = keyword_rule(None, 0);
    spam.exempt_roles.clear();
    spam.trigger = AutoModTrigger::Spam {
        duplicates: 3,
        links: 2,
    };
    ok(&run(&app, 1, 3, create(spam)).await);
    let mut kw = keyword_rule(None, 0);
    kw.trigger = AutoModTrigger::Keyword {
        keywords: (0..=MAX_AUTOMOD_KEYWORDS)
            .map(|i| format!("w{i}"))
            .collect(),
        allow: vec![],
    };
    assert_eq!(
        err(&run(&app, 1, 4, create(kw)).await).code,
        ErrorCode::InvalidRequest
    );
    for n in 5..(5 + MAX_AUTOMOD_RULES as u64 - 2) {
        ok(&run(&app, 1, n, create(keyword_rule(None, 0))).await);
    }
    assert_eq!(
        err(&run(&app, 1, 99, create(keyword_rule(None, 0))).await).code,
        ErrorCode::LimitExceeded
    );
    let mut d = MessageDraft {
        channel_id: Id(11),
        content: "<@1> <@2> <@3>".into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    };
    assert_eq!(
        err(&run(&app, 3, 1, Command::SendMessage(d.clone())).await)
            .field
            .as_deref(),
        Some("automod")
    );
    d.content = "https://a.example https://b.example https://c.example".into();
    assert_eq!(
        err(&run(&app, 3, 2, Command::SendMessage(d)).await)
            .field
            .as_deref(),
        Some("automod")
    );
    ok(&run(&app, 3, 3, send("buy now")).await);
    ok(&run(&app, 3, 4, send("buy now")).await);
    assert_eq!(
        err(&run(&app, 3, 5, send("BUY NOW")).await)
            .field
            .as_deref(),
        Some("automod")
    );
}

#[tokio::test]
async fn raid_alerts_reach_moderators_and_pauses_are_enforced_and_expire() {
    let (_dir, app) = guild(Permissions::KICK_MEMBERS).await;
    // Pauses need Manage Server and a duration from one hour to one day.
    let pause = |s| {
        m(ModerationCommand::PauseInvites {
            guild_id: Id(10),
            seconds: s,
        })
    };
    assert_eq!(
        err(&run(&app, 2, 1, pause(3600)).await).code,
        ErrorCode::Forbidden
    );
    assert_eq!(
        err(&run(&app, 1, 1, pause(1800)).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 1, 2, pause(86_401)).await).code,
        ErrorCode::InvalidRequest
    );
    // A burst of joins raises an alert that only moderators see.
    let code = "c".repeat(22);
    sqlx::query("INSERT INTO invites(id,code_hash,code,guild_id,creator_id,expires_at,max_uses,uses) VALUES(40,?,?,10,1,?,1000,0)")
        .bind(crate::state::hash(code.as_bytes()))
        .bind(&code)
        .bind((now() + 600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    for u in 100..(100 + RAID_JOIN_THRESHOLD as i64) {
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'','online',0)")
            .bind(u)
            .bind(format!("raider{u}"))
            .execute(&app.0.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(vec![u as u8; 32])
            .bind(u)
            .bind(now() as i64)
            .bind((now() + 600_000) as i64)
            .bind(now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        ok(&run(
            &app,
            u as u64,
            1,
            Command::JoinGuild {
                invite_code: code.clone(),
            },
        )
        .await);
    }
    let mut c = app.0.pool.acquire().await.unwrap();
    let raid = state(&mut c, Id(2), Id(10))
        .await
        .unwrap()
        .safety
        .raid
        .unwrap();
    assert_eq!(raid.joins, RAID_JOIN_THRESHOLD);
    assert!(state(&mut c, Id(3), Id(10))
        .await
        .unwrap()
        .safety
        .raid
        .is_none());
    drop(c);
    // Moderators dismiss it; paused invites refuse new members.
    ok(&run(
        &app,
        2,
        2,
        m(ModerationCommand::DismissRaidAlert { guild_id: Id(10) }),
    )
    .await);
    ok(&run(&app, 1, 3, pause(3600)).await);
    sql(&app, "INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(200,'late','','online',0)").await;
    sqlx::query("INSERT INTO sessions VALUES(?,200,?,?,?)")
        .bind(vec![200u8; 32])
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let e = err(&run(
        &app,
        200,
        1,
        Command::JoinGuild {
            invite_code: code.clone(),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("invites_paused"));
    // Expiry restores them without anyone acting.
    sql(&app, "UPDATE guild_safety SET invites_paused_until=1").await;
    expire(&app).await.unwrap();
    ok(&run(&app, 200, 2, Command::JoinGuild { invite_code: code }).await);
    // DM pause: members who are not friends cannot DM each other.
    sql(
        &app,
        "INSERT INTO channels(id,kind,position,created_at) VALUES(50,'direct',0,0)",
    )
    .await;
    sql(&app, "INSERT INTO dm_members VALUES(50,3,0,1),(50,4,0,1)").await;
    sql(
        &app,
        "INSERT INTO read_states(user_id,channel_id) VALUES(3,50),(4,50)",
    )
    .await;
    ok(&run(
        &app,
        1,
        4,
        m(ModerationCommand::PauseDms {
            guild_id: Id(10),
            seconds: 7200,
        }),
    )
    .await);
    let dm = Command::SendMessage(MessageDraft {
        channel_id: Id(50),
        content: "hi".into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    });
    assert_eq!(
        err(&run(&app, 3, 1, dm.clone()).await).field.as_deref(),
        Some("dms_paused")
    );
    sql(
        &app,
        "INSERT INTO relationships VALUES(3,4,'friend',0),(4,3,'friend',0)",
    )
    .await;
    ok(&run(&app, 3, 2, dm).await);
    ok(&run(
        &app,
        1,
        5,
        m(ModerationCommand::PauseDms {
            guild_id: Id(10),
            seconds: 0,
        }),
    )
    .await);
    assert_eq!(entries(&app, AuditAction::DmsPause).await.len(), 2);
    assert_eq!(entries(&app, AuditAction::InvitesPause).await.len(), 1);
}

#[tokio::test]
async fn verification_levels_gate_new_accounts_and_new_members() {
    let (_dir, app) = guild(Permissions::empty()).await;
    let level = |l| {
        m(ModerationCommand::SetVerificationLevel {
            guild_id: Id(10),
            level: l,
        })
    };
    assert_eq!(
        err(&run(&app, 3, 1, level(VerificationLevel::AccountAge)).await).code,
        ErrorCode::Forbidden
    );
    ok(&run(&app, 1, 1, level(VerificationLevel::AccountAge)).await);
    sqlx::query("UPDATE users SET created_at=? WHERE id IN(3,2)")
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let e = err(&run(&app, 3, 1, send("hi")).await);
    assert_eq!(e.field.as_deref(), Some("verification"));
    // Members with a role are trusted.
    ok(&run(&app, 2, 1, send("hi")).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(
        state(&mut c, Id(3), Id(10))
            .await
            .unwrap()
            .send_blocked_until
            .unwrap()
            > now()
    );
    drop(c);
    sql(&app, "UPDATE users SET created_at=0").await;
    ok(&run(&app, 3, 2, send("hi")).await);
    ok(&run(&app, 1, 2, level(VerificationLevel::MemberAge)).await);
    sqlx::query("UPDATE guild_members SET joined_at=? WHERE user_id=4")
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        err(&run(&app, 4, 1, send("hi")).await).field.as_deref(),
        Some("verification")
    );
    ok(&run(&app, 3, 3, send("still fine")).await);
    assert_eq!(
        entries(&app, AuditAction::VerificationUpdate).await.len(),
        2
    );
}

#[tokio::test]
async fn age_restricted_channels_hide_history_until_confirmed() {
    let (_dir, app) = guild(Permissions::empty()).await;
    let set = |on| {
        m(ModerationCommand::SetAgeRestricted {
            channel_id: Id(11),
            age_restricted: on,
        })
    };
    assert_eq!(
        err(&run(&app, 3, 1, set(true)).await).code,
        ErrorCode::Forbidden
    );
    ok(&run(&app, 1, 1, set(true)).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        age_gate(&mut c, Id(3), Id(11))
            .await
            .unwrap_err()
            .0
            .field
            .as_deref(),
        Some("age_restricted")
    );
    let s = state(&mut c, Id(3), Id(10)).await.unwrap();
    assert_eq!((s.age_restricted, s.age_confirmed), (vec![Id(11)], vec![]));
    drop(c);
    ok(&run(
        &app,
        3,
        2,
        m(ModerationCommand::ConfirmAge { channel_id: Id(11) }),
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    age_gate(&mut c, Id(3), Id(11)).await.unwrap();
    assert!(
        age_gate(&mut c, Id(4), Id(11)).await.is_err(),
        "confirmation is per user"
    );
    drop(c);
    ok(&run(&app, 1, 2, set(false)).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    age_gate(&mut c, Id(4), Id(11)).await.unwrap();
}

#[tokio::test]
async fn moderation_events_reach_only_subscribed_streams() {
    let (_dir, app) = guild(Permissions::empty()).await;
    ok(&run(&app, 3, 1, m(ModerationCommand::Subscribe)).await);
    // Stream 4 exists but did not opt in.
    ok(&run(&app, 4, 1, Command::Typing { channel_id: Id(11) }).await);
    ok(&run(
        &app,
        1,
        1,
        m(ModerationCommand::SetVerificationLevel {
            guild_id: Id(10),
            level: VerificationLevel::AccountAge,
        }),
    )
    .await);
    let core = app.0.core.lock().await;
    assert!(ring(&core, 3).iter().any(|e| matches!(e, Event::Moderation(ModerationEvent::SafetyUpdated(s)) if s.verification_level == VerificationLevel::AccountAge && s.raid.is_none())));
    assert!(!ring(&core, 4)
        .iter()
        .any(|e| matches!(e, Event::Moderation(_))));
}

#[tokio::test]
async fn channel_access_summary_and_wider_slowmode_list() {
    let (_dir, app) = guild(Permissions::MANAGE_ROLES).await;
    ok(&run(
        &app,
        1,
        1,
        Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![
                PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: Id(10),
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                },
                PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: Id(30),
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                },
            ],
        }),
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let a = channel_access(&mut c, Id(2), Id(11)).await.unwrap();
    assert_eq!(
        (a.members, a.viewers, a.everyone, a.roles.clone()),
        (4, 2, false, vec![Id(11), Id(30)])
    );
    assert!(channel_access(&mut c, Id(3), Id(11)).await.is_err());
    drop(c);
    for (n, s) in [(2, 15), (3, 120), (4, 600), (5, 1800), (6, 7200)] {
        ok(&run(
            &app,
            1,
            n,
            Command::Launch(LaunchCommand::SetSlowmode {
                channel_id: Id(11),
                seconds: s,
            }),
        )
        .await);
    }
    assert_eq!(
        err(&run(
            &app,
            1,
            9,
            Command::Launch(LaunchCommand::SetSlowmode {
                channel_id: Id(11),
                seconds: 45
            })
        )
        .await)
        .code,
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn raised_capacity_limits_hold_and_stay_enforced() {
    let (_dir, app) = guild(Permissions::empty()).await;
    // Channels: past the old 32, up to 500.
    for i in 0..498_i64 {
        sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(?,'text',10,?,?,0)")
            .bind(1000 + i)
            .bind(format!("c{i}"))
            .bind(i + 1)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    let create = |n: &str| Command::CreateChannel {
        guild_id: Id(10),
        kind: ChannelKind::Text,
        name: n.into(),
        parent_id: None,
        topic: None,
    };
    ok(&run(&app, 1, 1, create("five-hundred")).await);
    assert_eq!(
        err(&run(&app, 1, 2, create("too-many")).await).code,
        ErrorCode::LimitExceeded
    );
    // Listing 500 channels for a member is one batch, and agrees with the slow path.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        channels(&mut c, Id(3), Some(Id(10))).await.unwrap().len(),
        500
    );
    drop(c);
    // Roles: past the old 64.
    for i in 0..(MAX_ROLES_PER_GUILD as i64 - 3) {
        sqlx::query("INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,?,?,?,0)")
            .bind(5000 + i)
            .bind(format!("r{i}"))
            .bind(3 + i)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM roles WHERE guild_id=10").await,
        MAX_ROLES_PER_GUILD as i64
    );
    let o = run(
        &app,
        1,
        3,
        Command::Launch(LaunchCommand::CreateRole {
            guild_id: Id(10),
            name: "one more".into(),
        }),
    )
    .await;
    assert_eq!(err(&o).code, ErrorCode::LimitExceeded);
    // Servers per user: past the old 10, up to 200.
    for g in 0..(MAX_GUILDS_PER_USER as i64 - 1) {
        sqlx::query("INSERT INTO guilds VALUES(?,1,'g',NULL,0)")
            .bind(20_000 + g)
            .execute(&app.0.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO guild_members VALUES(?,3,'member',0)")
            .bind(20_000 + g)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    let o = run(
        &app,
        3,
        1,
        Command::CreateGuild {
            name: "mine".into(),
            icon_id: None,
        },
    )
    .await;
    assert_eq!(err(&o).code, ErrorCode::LimitExceeded);
    sql(
        &app,
        "DELETE FROM guild_members WHERE user_id=3 AND guild_id=20000",
    )
    .await;
    ok(&run(
        &app,
        3,
        2,
        Command::CreateGuild {
            name: "mine".into(),
            icon_id: None,
        },
    )
    .await);
    // Pins: past the old 50, up to 250.
    for i in 0..MAX_PINS_PER_CHANNEL as i64 {
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,11,1,'p',0)",
        )
        .bind(90_000 + i)
        .execute(&app.0.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO pins VALUES(11,?,1,?)")
            .bind(90_000 + i)
            .bind(i)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    sql(
        &app,
        "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(99999,11,1,'p',0)",
    )
    .await;
    let pin = Command::Launch(LaunchCommand::PinMessage {
        channel_id: Id(11),
        message_id: Id(99_999),
        pinned: true,
    });
    assert_eq!(
        err(&run(&app, 1, 4, pin).await).code,
        ErrorCode::LimitExceeded
    );
    // All 250 are listed, newest first.
    let (status, body) = http_get(&app, 1, "/api/v1/channels/11/pins").await;
    assert_eq!(status, 200);
    let pins: Snapshot<Vec<Message>> = serde_json::from_slice(&body).unwrap();
    assert_eq!(pins.data.len(), MAX_PINS_PER_CHANNEL);
    assert_eq!(
        pins.data[0].id,
        Id(90_000 + MAX_PINS_PER_CHANNEL as u64 - 1)
    );
}
