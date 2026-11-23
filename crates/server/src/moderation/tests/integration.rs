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
