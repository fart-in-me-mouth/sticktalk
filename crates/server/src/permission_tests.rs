//! The permission bits feature branches added (task `qa2`, integration.md
//! "Permissions of the later features"): who holds them, who loses them, and
//! what @everyone gets by default.
use crate::{audit_tests::fixture, commands, permissions, state::now};
use sticktalk_proto::*;

/// Every bit added by a feature branch, with whether @everyone gets it in a
/// new (and, by migration, an existing) server.
const LATE: &[(Permissions, bool)] = &[
    (Permissions::VIEW_AUDIT_LOG, false),
    (Permissions::MODERATE_MEMBERS, false),
    (Permissions::CREATE_PUBLIC_THREADS, true),
    (Permissions::CREATE_PRIVATE_THREADS, true),
    (Permissions::SEND_MESSAGES_IN_THREADS, true),
    (Permissions::MANAGE_THREADS, false),
    (Permissions::PUBLISH_MESSAGES, false),
    (Permissions::USE_SOUNDBOARD, true),
    (Permissions::MANAGE_SOUNDBOARD, false),
    (Permissions::USE_EXTERNAL_EMOJI, true),
    (Permissions::USE_EXTERNAL_STICKERS, true),
    (Permissions::MANAGE_EVENTS, false),
    (Permissions::CREATE_EVENTS, false),
    (Permissions::REQUEST_TO_SPEAK, true),
    (Permissions::SEND_TTS_MESSAGES, true),
];

#[tokio::test]
async fn late_bits_follow_owner_admin_timeout_and_screening_rules() {
    let (_dir, app) = fixture().await;
    let late = LATE.iter().fold(Permissions::empty(), |a, (b, _)| a | *b);
    // 2: a role holding every late bit; 3: Administrator; 4: the same role,
    // timed out; and 2 again later, pending rules screening.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO guild_members VALUES(10,3,'member',0),(10,4,'member',0);
         INSERT OR REPLACE INTO roles(guild_id,id,name,position,permissions) VALUES(10,10,'@everyone',0,{});
         INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,20,'all late',2,{}),(10,21,'admin',3,{});
         INSERT INTO member_roles VALUES(10,2,20),(10,4,20),(10,3,21);
         INSERT INTO member_timeouts(guild_id,user_id,until,reason,actor_id) VALUES(10,4,{},NULL,1);",
        (Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY | Permissions::SEND_MESSAGES).bits(),
        late.bits(),
        Permissions::ADMINISTRATOR.bits(),
        now() + 3_600_000,
    )))
    .execute(&app.0.pool)
    .await
    .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    for ch in [None, Some(Id(11))] {
        let of = |p: Permissions, b: Permissions| p.contains(b);
        let owner = permissions::evaluate(&mut c, Id(1), Id(10), ch)
            .await
            .unwrap();
        let holder = permissions::evaluate(&mut c, Id(2), Id(10), ch)
            .await
            .unwrap();
        let admin = permissions::evaluate(&mut c, Id(3), Id(10), ch)
            .await
            .unwrap();
        let timed_out = permissions::evaluate(&mut c, Id(4), Id(10), ch)
            .await
            .unwrap();
        for (bit, _) in LATE {
            assert!(of(owner, *bit), "owner {bit:?}");
            assert!(of(admin, *bit), "administrator {bit:?}");
            assert!(of(holder, *bit), "role {bit:?}");
            assert!(!of(timed_out, *bit), "timed out {bit:?}");
        }
    }
    // Rules screening not passed yet: the same as a timeout.
    sqlx::query("INSERT INTO community_members(guild_id,user_id,rules_pending) VALUES(10,2,1)")
        .execute(&mut *c)
        .await
        .unwrap();
    for ch in [None, Some(Id(11))] {
        let pending = permissions::evaluate(&mut c, Id(2), Id(10), ch)
            .await
            .unwrap();
        for (bit, _) in LATE {
            assert!(!pending.contains(*bit), "pending screening {bit:?}");
        }
    }
    drop(c);
    // A new server: @everyone gets exactly the deliberate defaults.
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    let r = commands::execute(
        &app,
        &mut core,
        &[1u8; 32],
        Id(1),
        CommandRequest {
            nonce: 7,
            command: Command::CreateGuild {
                name: "fresh".into(),
                icon_id: None,
            },
        },
    )
    .await
    .unwrap();
    drop(core);
    let CommandOutcome::Ok(CommandResult::Guild(g)) = r.outcome else {
        panic!("{:?}", r.outcome)
    };
    let g = g.summary.guild.id;
    let everyone: i64 =
        sqlx::query_scalar("SELECT permissions FROM roles WHERE guild_id=? AND id=?")
            .bind(g.0 as i64)
            .bind(g.0 as i64)
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    let everyone = Permissions::from_bits_retain(everyone as u64);
    for (bit, default) in LATE {
        assert_eq!(
            everyone.contains(*bit),
            *default,
            "@everyone default {bit:?}"
        );
    }
}
