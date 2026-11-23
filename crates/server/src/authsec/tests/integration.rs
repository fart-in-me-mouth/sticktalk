//! The data export covers the account's content from the later features
//! (`docs/features/integration.md`).
use super::*;

#[tokio::test]
async fn data_export_includes_threads_votes_scheduled_sounds_and_backgrounds() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let _bob = user(&app, 2, "bob").await;
    seed_content(&app).await;
    sqlx::raw_sql(
        "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(30,'thread',10,11,'alice plans',0,0);\
         INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(30,11,10,1,60,5,5,99999999999999);\
         INSERT INTO thread_members VALUES(30,1,5,'all');\
         INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(31,30,1,'in the thread',6);\
         INSERT INTO polls VALUES(13,'Lunch?',0,99999999999999,NULL);\
         INSERT INTO poll_answers VALUES(13,0,'Yes',NULL);\
         INSERT INTO poll_votes VALUES(13,1,0,7),(13,2,0,7);\
         INSERT INTO scheduled_messages(id,author_id,channel_id,content,mention_ids,send_at,created_at) VALUES(40,1,11,'later on','[]',99999999999999,8);\
         INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(41,1,'message',11,'plan.txt','text/plain',4,'k41',0);\
         INSERT INTO scheduled_attachments VALUES(40,41,0);\
         INSERT INTO soundboard_sounds(id,guild_id,name,emoji,volume,duration_ms,format,data,created_by,created_at) VALUES(50,10,'Tada',NULL,80,400,'wav',X'5249464600',1,9),(51,10,'Bobs horn',NULL,80,400,'wav',X'5249464601',2,9);\
         INSERT INTO camera_backgrounds(id,user_id,kind,mime,data,created_at) VALUES(60,1,'image','image/png',X'89504E47',10),(61,2,'image','image/png',X'89504E48',10);",
    )
    .execute(&app.0.pool)
    .await
    .unwrap();
    std::fs::create_dir_all(app.0.config.data_dir.join("exports")).unwrap();
    let p = proof(&app, &token, AccountOperation::DataExport).await;
    let (s, _, v) = call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!(s, 200, "{v}");
    export::work(&app).await.unwrap();
    let (_, _, status) = call(Method::GET, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .send(&app)
        .await;
    let path = format!(
        "/api/v1/users/@me/data-export/{}/download",
        status["export"]["id"].as_str().unwrap()
    );
    let r = crate::http::router(app.clone())
        .oneshot(
            axum::http::Request::get(&path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let bytes = axum::body::to_bytes(r.into_body(), 64 << 20).await.unwrap();
    let files = unzip(&bytes);
    let text = |name: &str| {
        String::from_utf8_lossy(
            files
                .get(name)
                .unwrap_or_else(|| panic!("{name}: {:?}", files.keys().collect::<Vec<_>>())),
        )
        .to_string()
    };
    // Threads and forum posts she started; their messages are in messages/.
    assert!(text("threads.json").contains("alice plans"));
    assert!(text("messages/30.json").contains("in the thread"));
    // Her poll vote and thread membership, not Bob's vote.
    let settings: Value = serde_json::from_slice(&files["settings.json"]).unwrap();
    assert_eq!(settings["poll_votes"].as_array().unwrap().len(), 1);
    assert_eq!(settings["thread_members"].as_array().unwrap().len(), 1);
    // Scheduled messages with their held files.
    let scheduled = text("scheduled.json");
    assert!(scheduled.contains("later on") && scheduled.contains("plan.txt"));
    // Her soundboard uploads, with the audio, and only hers.
    let sounds = text("soundboard/index.json");
    assert!(sounds.contains("Tada") && !sounds.contains("Bobs horn"));
    assert_eq!(files["soundboard/50-Tada.wav"], b"RIFF\0");
    // Her camera backgrounds, and only hers.
    assert_eq!(files["camera-backgrounds/60.png"], b"\x89PNG");
    assert!(!files.contains_key("camera-backgrounds/61.png"));
    assert!(text("README.txt").contains("soundboard/"));
}
