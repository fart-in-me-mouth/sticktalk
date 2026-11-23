//! A 1:1 call with a non-friend from a shared server follows that server's DM
//! pause, verification level and timeouts (`docs/features/integration.md`).
use super::*;

#[tokio::test]
async fn direct_calls_follow_the_shared_servers_dm_rules_and_timeouts() {
    let (_dir, app) = guild(Permissions::MODERATE_MEMBERS | Permissions::MANAGE_GUILD).await;
    // Users 3 and 4 share guild 10 and a DM; they are not friends.
    sql(
        &app,
        "INSERT INTO channels(id,kind,position,created_at) VALUES(40,'direct',0,0)",
    )
    .await;
    sql(&app, "INSERT INTO dm_pairs VALUES(3,4,40)").await;
    sql(&app, "INSERT INTO dm_members VALUES(40,3,0,1),(40,4,0,1)").await;
    let admit = |u: u64| {
        let app = app.clone();
        async move {
            let mut c = app.0.pool.acquire().await.unwrap();
            let ch = channel(&mut c, Id(40)).await.unwrap();
            crate::calls::admit(&mut c, Id(u), &ch).await
        }
    };
    admit(3).await.unwrap();
    // DMs paused in the shared server: no calls either.
    ok(&run(
        &app,
        1,
        1,
        m(ModerationCommand::PauseDms {
            guild_id: Id(10),
            seconds: 3600,
        }),
    )
    .await);
    assert_eq!(
        admit(3).await.unwrap_err().0.field.as_deref(),
        Some("dms_paused")
    );
    ok(&run(
        &app,
        1,
        2,
        m(ModerationCommand::PauseDms {
            guild_id: Id(10),
            seconds: 0,
        }),
    )
    .await);
    admit(3).await.unwrap();
    // Timed out in the shared server: cannot start or answer such a call.
    ok(&run(&app, 2, 3, timeout(3, 600)).await);
    assert_eq!(
        admit(3).await.unwrap_err().0.field.as_deref(),
        Some("timeout")
    );
    admit(4).await.unwrap();
    // Friends are never affected.
    sql(
        &app,
        "INSERT INTO relationships(user_id,other_id,kind,since) VALUES(3,4,'friend',0),(4,3,'friend',0)",
    )
    .await;
    admit(3).await.unwrap();
}

#[tokio::test]
async fn moderation_state_lists_slowmode_of_visible_channels_only() {
    let (_dir, app) = guild(Permissions::empty()).await;
    // #12 is hidden from @everyone; both have slowmode.
    sql(
        &app,
        "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'staff',1,0)",
    )
    .await;
    sql(
        &app,
        "INSERT INTO permission_overwrites VALUES(12,'role',10,0,1)",
    )
    .await;
    for (n, ch) in [11_u64, 12].into_iter().enumerate() {
        ok(&run(
            &app,
            1,
            10 + n as u64,
            Command::Launch(LaunchCommand::SetSlowmode {
                channel_id: Id(ch),
                seconds: 15,
            }),
        )
        .await);
    }
    let listed = |body: Vec<u8>| -> Vec<(u64, u32)> {
        let s: ModerationState = serde_json::from_slice(&body).unwrap();
        s.slowmode
            .iter()
            .map(|s| (s.channel_id.0, s.seconds))
            .collect()
    };
    let (status, body) = http_get(&app, 3, "/api/v1/guilds/10/moderation/me").await;
    assert_eq!(status, 200);
    assert_eq!(listed(body), vec![(11, 15)]);
    let (_, body) = http_get(&app, 1, "/api/v1/guilds/10/moderation/me").await;
    let mut all = listed(body);
    all.sort();
    assert_eq!(all, vec![(11, 15), (12, 15)]);
    // Turned off: no longer listed.
    ok(&run(
        &app,
        1,
        20,
        Command::Launch(LaunchCommand::SetSlowmode {
            channel_id: Id(11),
            seconds: 0,
        }),
    )
    .await);
    let (_, body) = http_get(&app, 3, "/api/v1/guilds/10/moderation/me").await;
    assert!(listed(body).is_empty());
}

/// Switching conversations reads history pages and extents on their own
/// budget: 60 at once, without spending the budget of other reads.
#[tokio::test]
async fn history_reads_have_their_own_budget() {
    let (_dir, app) = guild(Permissions::empty()).await;
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    assert!(crate::history::is_history_page("channels/11/messages"));
    assert!(!crate::history::is_history_page("channels/11/messages/12"));
    assert!(!crate::history::is_history_page("channels/11"));
    for _ in 0..60 {
        crate::history::rate(&mut core, Id(3)).unwrap();
    }
    assert!(crate::history::rate(&mut core, Id(3)).is_err());
    // Other reads are untouched.
    core.rate("rest:3".into(), 120., 30.).unwrap();
}
