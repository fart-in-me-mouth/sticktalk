use super::*;
use crate::{audit_tests::fixture, state};
use axum::http::StatusCode;
use tower::ServiceExt;

async fn token(app: &App, u: i64) -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u as u8 + 100; 32]);
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(state::hash(token.as_bytes()))
        .bind(u)
        .bind(state::now() as i64)
        .bind((state::now() + 600_000) as i64)
        .bind(state::now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
async fn call(
    app: &App,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    app.0.core.lock().await.limits.clear();
    let mut r = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        r = r.header("content-type", "application/json");
    }
    let r = r
        .body(axum::body::Body::from(
            body.map_or(vec![], |b| serde_json::to_vec(&b).unwrap()),
        ))
        .unwrap();
    let response = http::router(app.clone()).oneshot(r).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
/// Navigation events this session's stream has received.
async fn events(app: &App, token: &str) -> Vec<Event> {
    let key = state::hash(token.as_bytes());
    let core = app.0.core.lock().await;
    core.streams.get(&key).map_or(vec![], |s| {
        s.ring
            .iter()
            .filter_map(
                |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).ok()? {
                    ServerMsg::Event(e) => Some(e.event),
                    _ => None,
                },
            )
            .collect()
    })
}
async fn sql(app: &App, q: &'static str) {
    sqlx::raw_sql(q).execute(&app.0.pool).await.unwrap();
}
fn ids(v: &serde_json::Value) -> Vec<u64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| {
            i.as_str()
                .map(|s| s.parse().unwrap())
                .unwrap_or_else(|| i.as_u64().unwrap())
        })
        .collect()
}
fn id_json(n: u64) -> serde_json::Value {
    serde_json::to_value(Id(n)).unwrap()
}

#[tokio::test]
async fn pins_and_favorites_are_private_bounded_and_limited_to_visible_conversations() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'hidden',1,0),(13,'category',10,'cat',2,0),(20,'direct',NULL,NULL,0,0);INSERT INTO dm_members(channel_id,user_id,joined_at,visible) VALUES(20,1,0,1),(20,3,0,1);").await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let t1 = token(&app, 1).await;
    let t2 = token(&app, 2).await;
    // Opt in, then pin.
    assert_eq!(
        call(&app, &t2, "GET", "/api/v1/users/@me/nav", None)
            .await
            .0,
        StatusCode::OK
    );
    let (s, v) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/nav/pins",
        Some(serde_json::json!({"channel_ids":[id_json(11)]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(ids(&v["pinned"]), [11]);
    assert!(events(&app, &t2)
        .await
        .iter()
        .any(|e| matches!(e, Event::NavUpdated(n) if n.pinned == [Id(11)])));
    // A stream that never asked for navigation events gets none.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(events(&app, &t1).await.is_empty());
    // Hidden channel, someone else's DM, a category, duplicates and too many are refused.
    for (body, code) in [
        (
            serde_json::json!({"channel_ids":[id_json(12)]}),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({"channel_ids":[id_json(20)]}),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({"channel_ids":[id_json(13)]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"channel_ids":[id_json(11),id_json(11)]}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (s, v) = call(&app, &t2, "PUT", "/api/v1/users/@me/nav/pins", Some(body)).await;
        assert_eq!(s, code, "{v}");
    }
    let many: Vec<_> = (0..=MAX_PINNED_CONVERSATIONS as u64)
        .map(|n| id_json(1000 + n))
        .collect();
    let (s, _) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/nav/pins",
        Some(serde_json::json!({"channel_ids":many})),
    )
    .await;
    assert!(s.is_client_error());
    // User 1 pins their DM; user 2's pins are unaffected and not visible to them.
    let (s, v) = call(
        &app,
        &t1,
        "PUT",
        "/api/v1/users/@me/nav/pins",
        Some(serde_json::json!({"channel_ids":[id_json(20),id_json(12)]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(ids(&v["pinned"]), [20, 12]);
    assert_eq!(settings(&mut c, Id(2)).await.unwrap().pinned, [Id(11)]);
    // Losing access hides the pin rather than leaking the channel.
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,0,?)")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(settings(&mut c, Id(2)).await.unwrap().pinned.is_empty());
    sqlx::query("DELETE FROM permission_overwrites WHERE channel_id=11")
        .execute(&mut *c)
        .await
        .unwrap();

    // Favorites: categories, order, bounds and validation.
    let body = serde_json::json!({
        "categories":[{"id":1,"name":"Work","collapsed":false}],
        "favorites":[{"channel_id":id_json(20),"category_id":1},{"channel_id":id_json(11)}]
    });
    let (s, v) = call(
        &app,
        &t1,
        "PUT",
        "/api/v1/users/@me/nav/favorites",
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let n = settings(&mut c, Id(1)).await.unwrap();
    assert_eq!(n.favorites.len(), 2);
    assert_eq!(n.favorites[0].category_id, Some(1));
    assert_eq!(n.favorite_categories[0].name, "Work");
    assert!(settings(&mut c, Id(2)).await.unwrap().favorites.is_empty());
    for body in [
        serde_json::json!({"categories":[],"favorites":[{"channel_id":id_json(11),"category_id":9}]}),
        serde_json::json!({"categories":[{"id":0,"name":"x"}],"favorites":[]}),
        serde_json::json!({"categories":[{"id":1,"name":"\nx"}],"favorites":[]}),
        serde_json::json!({"categories":[{"id":1,"name":"a"},{"id":1,"name":"b"}],"favorites":[]}),
        serde_json::json!({"categories":[],"favorites":[{"channel_id":id_json(13)}]}),
    ] {
        let (s, v) = call(
            &app,
            &t1,
            "PUT",
            "/api/v1/users/@me/nav/favorites",
            Some(body),
        )
        .await;
        assert!(s.is_client_error(), "{v}");
    }
    let (s, _) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/nav/favorites",
        Some(serde_json::json!({"categories":[],"favorites":[{"channel_id":id_json(20)}]})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let too_many: Vec<_> = (0..=MAX_FAVORITES as u64)
        .map(|n| serde_json::json!({"channel_id":id_json(1000+n)}))
        .collect();
    let (s, _) = call(
        &app,
        &t1,
        "PUT",
        "/api/v1/users/@me/nav/favorites",
        Some(serde_json::json!({"categories":[],"favorites":too_many})),
    )
    .await;
    assert!(s.is_client_error());
    // Deleting a channel removes it everywhere it was arranged.
    sqlx::query("DELETE FROM dm_members WHERE channel_id=20")
        .execute(&mut *c)
        .await
        .unwrap();
    sqlx::query("DELETE FROM channels WHERE id=20")
        .execute(&mut *c)
        .await
        .unwrap();
    let n = settings(&mut c, Id(1)).await.unwrap();
    assert_eq!(n.pinned, [Id(12)]);
    assert_eq!(n.favorites.len(), 1);
}

#[tokio::test]
async fn rail_order_and_folders_are_validated_together_and_membership_scoped() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO guilds VALUES(30,1,'second',NULL,0),(40,3,'theirs',NULL,0);INSERT INTO guild_members VALUES(30,1,'owner',0),(40,3,'owner',0);").await;
    let t1 = token(&app, 1).await;
    let body = serde_json::json!({
        "folders":[{"id":5,"name":"Games","color":0x3ddc97,"collapsed":false,"guild_ids":[id_json(10)]}],
        "order":[{"type":"guild","id":id_json(30)},{"type":"folder","id":5}]
    });
    let (s, v) = call(&app, &t1, "PUT", "/api/v1/users/@me/nav/rail", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut c = app.0.pool.acquire().await.unwrap();
    let n = settings(&mut c, Id(1)).await.unwrap();
    assert_eq!(n.rail, [RailItem::Guild(Id(30)), RailItem::Folder(5)]);
    assert_eq!(n.folders[0].guild_ids, [Id(10)]);
    for body in [
        // A server that is also in a folder, a folder that does not exist,
        // someone else's server, a duplicate.
        serde_json::json!({"folders":[{"id":5,"name":"G","color":0,"collapsed":false,"guild_ids":[id_json(10)]}],"order":[{"type":"guild","id":id_json(10)}]}),
        serde_json::json!({"folders":[],"order":[{"type":"folder","id":9}]}),
        serde_json::json!({"folders":[],"order":[{"type":"guild","id":id_json(40)}]}),
        serde_json::json!({"folders":[],"order":[{"type":"guild","id":id_json(30)},{"type":"guild","id":id_json(30)}]}),
        serde_json::json!({"folders":[{"id":5,"name":"G","color":0,"collapsed":false,"guild_ids":[id_json(40)]}],"order":[]}),
    ] {
        let (s, v) = call(&app, &t1, "PUT", "/api/v1/users/@me/nav/rail", Some(body)).await;
        assert!(s.is_client_error(), "{v}");
    }
    // A refused write changed nothing.
    assert_eq!(settings(&mut c, Id(1)).await.unwrap().rail, n.rail);
    // Leaving a server drops it from the order and its folder.
    sqlx::query("DELETE FROM guild_members WHERE guild_id=30 AND user_id=1")
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        settings(&mut c, Id(1)).await.unwrap().rail,
        [RailItem::Folder(5)]
    );
}

#[tokio::test]
async fn saved_messages_need_read_access_are_bounded_and_never_leak_after_access_is_lost() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'hidden',1,0);INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,11,1,'launch plan',1),(101,12,1,'secret',2),(102,11,1,'later deleted',3);").await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let t2 = token(&app, 2).await;
    call(&app, &t2, "GET", "/api/v1/users/@me/nav", None).await;
    let (s, v) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/saved/100",
        Some(serde_json::json!({"channel_id":id_json(11)})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["message"]["content"], "launch plan");
    assert!(events(&app, &t2)
        .await
        .iter()
        .any(|e| matches!(e, Event::SavedUpdated(s) if s.count == 1)));
    // Cannot save what you cannot read, or claim it is in another channel.
    for (uri, ch) in [
        ("/api/v1/users/@me/saved/101", 12),
        ("/api/v1/users/@me/saved/100", 12),
        ("/api/v1/users/@me/saved/999", 11),
    ] {
        let (s, _) = call(
            &app,
            &t2,
            "PUT",
            uri,
            Some(serde_json::json!({"channel_id":id_json(ch)})),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{uri}");
    }
    // Reminder bounds.
    let now = state::now();
    for at in [now - 60_000, now + MAX_REMINDER_AHEAD_MS + 60_000] {
        let (s, _) = call(
            &app,
            &t2,
            "PUT",
            "/api/v1/users/@me/saved/100",
            Some(serde_json::json!({"channel_id":id_json(11),"remind_at":at})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, v) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/saved/102",
        Some(serde_json::json!({"channel_id":id_json(11),"remind_at":now+3_600_000})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, page) = call(&app, &t2, "GET", "/api/v1/users/@me/saved?limit=1", None).await;
    assert_eq!(page["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"]["has_more"], true);
    assert_eq!(page["data"]["summary"]["pending"], 1);
    // Deleted and no-longer-readable messages are listed without any content.
    sql(&app, "UPDATE messages SET deleted_at=1 WHERE id=102").await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,0,?)")
        .bind(Permissions::READ_HISTORY.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let (_, page) = call(&app, &t2, "GET", "/api/v1/users/@me/saved", None).await;
    let items = page["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i["message"].is_null()));
    assert!(
        !page.to_string().contains("launch plan") && !page.to_string().contains("later deleted")
    );
    // Someone else cannot remove it; the owner can clear a reminder, then remove.
    let t1 = token(&app, 1).await;
    assert_eq!(
        call(&app, &t1, "DELETE", "/api/v1/users/@me/saved/100", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (s, v) = call(
        &app,
        &t2,
        "DELETE",
        "/api/v1/users/@me/saved/102?reminder_only=true",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["pending"], 0);
    assert_eq!(
        call(&app, &t2, "DELETE", "/api/v1/users/@me/saved/100", None)
            .await
            .0,
        StatusCode::OK
    );
    // The limit counts every saved message.
    sql(
        &app,
        "DELETE FROM permission_overwrites WHERE channel_id=11",
    )
    .await;
    let mut c = app.0.pool.acquire().await.unwrap();
    for n in 0..MAX_SAVED_MESSAGES as i64 {
        sqlx::query(
            "INSERT INTO saved_messages(user_id,message_id,channel_id,saved_at) VALUES(2,?,11,0)",
        )
        .bind(5000 + n)
        .execute(&mut *c)
        .await
        .unwrap();
    }
    let (s, _) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/saved/100",
        Some(serde_json::json!({"channel_id":id_json(11)})),
    )
    .await;
    assert!(s.is_client_error(), "{s}");
}

#[tokio::test]
async fn reminders_fire_from_one_timer_and_reach_only_their_owner() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,11,1,'stand-up',1);").await;
    let t1 = token(&app, 1).await;
    let t2 = token(&app, 2).await;
    call(&app, &t1, "GET", "/api/v1/users/@me/nav", None).await;
    call(&app, &t2, "GET", "/api/v1/users/@me/nav", None).await;
    spawn(&app);
    let at = state::now() + 300;
    let (s, v) = call(
        &app,
        &t2,
        "PUT",
        "/api/v1/users/@me/saved/100",
        Some(serde_json::json!({"channel_id":id_json(11),"remind_at":at})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut due = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        due = events(&app, &t2).await.into_iter().find_map(|e| match e {
            Event::ReminderDue(item) => Some(item),
            _ => None,
        });
        if due.is_some() {
            break;
        }
    }
    let due = due.expect("reminder fired");
    assert!(state::now() >= at);
    assert_eq!(due.message.unwrap().content, "stand-up");
    assert!(!events(&app, &t1)
        .await
        .iter()
        .any(|e| matches!(e, Event::ReminderDue(_))));
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(summary(&mut c, Id(2)).await.unwrap().due, 1);
    // Firing again finds nothing: each reminder fires once.
    assert_eq!(fire_due(&app).await.unwrap(), 0);
    // A reminder for a message that can no longer be read fires without content.
    sql(&app, "INSERT INTO saved_messages(user_id,message_id,channel_id,saved_at,remind_at) VALUES(2,100,11,0,1) ON CONFLICT(user_id,message_id) DO UPDATE SET remind_at=1,fired_at=NULL;").await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,0,?)")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(fire_due(&app).await.unwrap(), 1);
    let last = events(&app, &t2)
        .await
        .into_iter()
        .rev()
        .find_map(|e| match e {
            Event::ReminderDue(item) => Some(item),
            _ => None,
        });
    assert!(last.unwrap().message.is_none());
}

#[tokio::test]
async fn inbox_lists_readable_unreads_and_filtered_mentions_and_acks_them() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'hidden',1,0);INSERT INTO read_states(user_id,channel_id) VALUES(2,12),(3,11);INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,11,1,'hi <@2>',1),(101,11,1,'@everyone',2),(102,11,1,'<@&20>',3),(103,12,1,'secret <@2>',4),(104,11,3,'blocked <@2>',5);INSERT INTO message_mentions VALUES(100,2),(101,2),(102,2),(103,2),(104,2);INSERT INTO message_broadcast_mentions VALUES(101,2,'everyone'),(102,2,'role');INSERT INTO relationships(user_id,other_id,kind,since) VALUES(2,3,'blocked',0);").await;
    sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
        .bind(Permissions::VIEW_CHANNEL.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::db::recount(&mut c, Id(11)).await.unwrap();
    crate::db::recount(&mut c, Id(12)).await.unwrap();
    let all = mention_feed(&mut c, Id(2), None, 50, true, true)
        .await
        .unwrap();
    assert_eq!(
        all.items.iter().map(|m| m.id.0).collect::<Vec<_>>(),
        [102, 101, 100]
    );
    let explicit = mention_feed(&mut c, Id(2), None, 50, false, false)
        .await
        .unwrap();
    assert_eq!(
        explicit.items.iter().map(|m| m.id.0).collect::<Vec<_>>(),
        [100]
    );
    let roles = mention_feed(&mut c, Id(2), None, 50, false, true)
        .await
        .unwrap();
    assert_eq!(
        roles.items.iter().map(|m| m.id.0).collect::<Vec<_>>(),
        [102, 100]
    );
    let page = mention_feed(&mut c, Id(2), None, 1, true, true)
        .await
        .unwrap();
    assert!(page.has_more);
    let next = mention_feed(&mut c, Id(2), page.next_before, 50, true, true)
        .await
        .unwrap();
    assert_eq!(next.items[0].id, Id(101));
    let unreads = unread_feed(&mut c, Id(2)).await.unwrap();
    assert_eq!(
        unreads.channels.len(),
        1,
        "the hidden channel is not listed"
    );
    assert_eq!(unreads.channels[0].read.channel_id, Id(11));
    assert!(unreads.channels[0].messages.len() <= MAX_INBOX_MESSAGES_PER_CHANNEL);
    assert!(!serde_json::to_string(&unreads).unwrap().contains("secret"));
    drop(c);
    let t2 = token(&app, 2).await;
    let (s, v) = call(
        &app,
        &t2,
        "GET",
        "/api/v1/users/@me/mentions?everyone=false&roles=false",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1);
    // Acknowledging needs access to every channel named.
    let (s, _) = call(
        &app,
        &t2,
        "POST",
        "/api/v1/users/@me/unreads/ack",
        Some(serde_json::json!({"channel_ids":[id_json(11),id_json(12)]})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = call(
        &app,
        &t2,
        "POST",
        "/api/v1/users/@me/unreads/ack",
        Some(serde_json::json!({"channel_ids":[id_json(11)]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["read_states"][0]["unread_count"], 0);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(unread_feed(&mut c, Id(2))
        .await
        .unwrap()
        .channels
        .is_empty());
    assert_eq!(
        read(&mut c, Id(3), Id(11)).await.unwrap().unread_count,
        3,
        "only my read state changes"
    );
}

#[tokio::test]
async fn direct_message_search_covers_only_my_conversations_and_new_filters() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(20,'direct',NULL,NULL,0,0),(21,'group_dm',NULL,'crew',0,0),(22,'direct',NULL,NULL,0,0);INSERT INTO dm_members(channel_id,user_id,joined_at,visible) VALUES(20,1,0,1),(20,2,0,0),(21,1,0,1),(21,2,0,1),(21,3,0,1),(22,3,0,1),(22,4,0,1);INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,20,1,'trip photos',1767225600001),(101,21,3,'trip clip',1767312000001),(102,22,3,'trip secret',1767225600002),(103,11,1,'trip guild',1767225600003);INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(200,3,'message',21,'clip.mp4','video/mp4',1,'k',0);INSERT INTO message_attachments VALUES(101,200,0);").await;
    let q = |s: &str| MessageSearchQuery {
        q: s.into(),
        before: None,
        limit: Some(25),
    };
    let mut c = app.0.pool.acquire().await.unwrap();
    let ids = |r: crate::search::SearchResult| r.0.iter().map(|m| m.id.0).collect::<Vec<_>>();
    // A closed DM is still searchable by its member; others' DMs and servers are not included.
    assert_eq!(
        ids(crate::search::direct(&mut c, Id(2), None, &q("trip"))
            .await
            .unwrap()),
        [101, 100]
    );
    assert_eq!(
        ids(
            crate::search::direct(&mut c, Id(2), None, &q("trip has:video"))
                .await
                .unwrap()
        ),
        [101]
    );
    assert_eq!(
        ids(
            crate::search::direct(&mut c, Id(2), None, &q("trip during:2026-01-01"))
                .await
                .unwrap()
        ),
        [100]
    );
    assert_eq!(
        ids(
            crate::search::direct(&mut c, Id(2), None, &q("trip in:crew"))
                .await
                .unwrap()
        ),
        [101]
    );
    assert_eq!(
        ids(
            crate::search::direct(&mut c, Id(2), None, &q("trip in:user1"))
                .await
                .unwrap()
        ),
        [101, 100]
    );
    assert_eq!(
        ids(
            crate::search::direct(&mut c, Id(2), Some(Id(21)), &q("trip"))
                .await
                .unwrap()
        ),
        [101]
    );
    assert!(
        crate::search::direct(&mut c, Id(2), Some(Id(22)), &q("trip"))
            .await
            .is_err()
    );
    assert!(crate::search::direct(&mut c, Id(4), None, &q("trip"))
        .await
        .unwrap()
        .0
        .iter()
        .all(|m| m.id == Id(102)));
    drop(c);
    let t2 = token(&app, 2).await;
    let (s, v) = call(
        &app,
        &t2,
        "GET",
        "/api/v1/users/@me/messages/search?q=trip&channel_id=22",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = call(
        &app,
        &t2,
        "GET",
        "/api/v1/users/@me/messages/search?q=trip",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains("secret") && !v.to_string().contains("guild"));
}
