//! Exploit regressions. All listeners used by fixtures bind only to loopback.
use crate::{audit_tests::fixture, db, http, state, App};
use axum::{
    body::{to_bytes, Body},
    http::Request,
    response::Response,
};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use sticktalk_proto::*;
use tower::ServiceExt;

async fn setup() -> (tempfile::TempDir, App, String) {
    let (dir, mut app) = fixture().await;
    let config = &mut Arc::get_mut(&mut app.0).unwrap().config;
    config.max_upload_mb = 0;
    config.max_total_upload_mb = 0;
    let token = state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(state::hash(token.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    (dir, app, token)
}
fn req(token: &str, path: &str, value: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}
async fn call(app: &App, r: Request<Body>) -> Response {
    app.0.core.lock().await.limits.clear();
    http::router(app.clone()).oneshot(r).await.unwrap()
}
async fn value(r: Response) -> Value {
    serde_json::from_slice(&to_bytes(r.into_body(), 2 * 1024 * 1024).await.unwrap()).unwrap()
}
async fn create(app: &App, token: &str, size: u64) -> Response {
    call(app, req(token, "/api/v1/uploads/resumable", json!({"purpose":"message","channel_id":11,"filename":"a.bin","content_type":"application/octet-stream","byte_size":size,"sha256":"0".repeat(64)}))).await
}
fn multipart(token: &str, bytes: usize) -> Request<Body> {
    let mut body = b"--t\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nmessage\r\n--t\r\nContent-Disposition: form-data; name=\"channel_id\"\r\n\r\n11\r\n--t\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    body.extend(std::iter::repeat_n(b'a', bytes));
    body.extend_from_slice(b"\r\n--t--\r\n");
    Request::builder()
        .method("POST")
        .uri("/api/v1/uploads")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "multipart/form-data; boundary=t")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn s4_shared_daily_ingress_no_declared_charge_or_abort_refund() {
    let (_dir, mut app, token) = setup().await;
    Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .user_daily_upload_mb = 1;
    let r = create(&app, &token, 26 * 1024 * 1024).await;
    assert_eq!(r.status(), 201);
    let upload = value(r).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(charged_bytes),0) FROM upload_daily_usage"
        )
        .fetch_one(&app.0.pool)
        .await
        .unwrap(),
        0
    );
    let bytes = 800 * 1024;
    let r = Request::builder()
        .method("PUT")
        .uri(format!(
            "/api/v1/uploads/resumable/{}",
            upload["upload_id"].as_str().unwrap()
        ))
        .header("authorization", format!("Bearer {token}"))
        .header("upload-offset", "0")
        .header("content-length", bytes.to_string())
        .body(Body::from(vec![b'a'; bytes]))
        .unwrap();
    assert_eq!(call(&app, r).await.status(), 200);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(charged_bytes) FROM upload_daily_usage")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        bytes as i64
    );
    assert_eq!(
        call(&app, multipart(&token, 300 * 1024)).await.status(),
        409
    );
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(charged_bytes) FROM upload_daily_usage")
            .fetch_one(&app.0.pool)
            .await
            .unwrap()
            > 1024 * 1024
    );
}

#[tokio::test]
async fn s4_three_shared_unfinished_slots_idle_expiry_and_quota() {
    let (_dir, mut app, token) = setup().await;
    Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .user_quota_mb = 1;
    assert_eq!(create(&app, &token, 700 * 1024).await.status(), 201);
    assert_eq!(create(&app, &token, 400 * 1024).await.status(), 409);
    sqlx::query("INSERT INTO upload_reservations(key,user_id,utc_day,reserved_bytes,expires_at) VALUES('m1',1,0,0,?),('m2',1,0,0,?)")
        .bind((state::now()+3_600_000) as i64).bind((state::now()+3_600_000) as i64).execute(&app.0.pool).await.unwrap();
    assert_eq!(create(&app, &token, 1).await.status(), 409);
    assert_eq!(call(&app, multipart(&token, 1)).await.status(), 409);
    sqlx::query("UPDATE partial_uploads SET expires_at=0")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(create(&app, &token, 1).await.status(), 201);
    let remaining: i64 = sqlx::query_scalar("SELECT MAX(expires_at)-? FROM partial_uploads")
        .bind(state::now() as i64)
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert!((3_590_000..=3_600_000).contains(&remaining));
}

#[tokio::test]
async fn s4_zero_means_unlimited_for_multipart_too() {
    let (_dir, app, token) = setup().await;
    let r = call(&app, multipart(&token, 26 * 1024 * 1024)).await;
    assert_eq!(r.status(), 201, "{}", value(r).await);
}

#[tokio::test]
async fn s6_password_change_rotates_and_revokes_sockets_and_voice() {
    let (_dir, app, token) = setup().await;
    let other = state::token();
    let other_key = state::hash(other.as_bytes());
    let key = state::hash(token.as_bytes());
    sqlx::query("INSERT INTO sessions VALUES(?,1,0,9999999999999,0)")
        .bind(&other_key)
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels VALUES(12,'voice',10,NULL,'voice',NULL,1,NULL,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut receivers = vec![];
    let mut packets = vec![];
    {
        let mut core = app.0.core.lock().await;
        for (key, conn) in [(&key, "current"), (&other_key, "other")] {
            core.cursor(key, Id(1));
            let (tx, rx) = tokio::sync::mpsc::channel(128);
            packets.push(rx);
            let (close, watch) = tokio::sync::watch::channel(None);
            receivers.push(watch);
            core.streams.get_mut(key).unwrap().connection = Some(state::Connection {
                id: conn.into(),
                focused: false,
                alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                tx,
                budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
                close,
            });
        }
        crate::voice::signal(
            &app,
            &mut core,
            &other_key,
            Id(1),
            "other",
            sticktalk_sfu::encode_signal(
                1,
                &VoiceSignal::Join {
                    channel_id: Id(12),
                    capabilities: VoiceCapabilities {
                        video: vec![VideoCodec::H264],
                        hardware_decode: vec![],
                    },
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(core.voice.len(), 1);
        // A congested signaling queue must not prevent committed revocation.
        let budget = core.streams[&other_key]
            .connection
            .as_ref()
            .unwrap()
            .budget
            .clone();
        let available = budget.available_permits() as u32;
        budget.try_acquire_many_owned(available).unwrap().forget();
    }
    let r=call(&app,req(&token,"/api/auth/password",json!({"current_password":"unused dummy credential","new_password":"a brand new long password"}))).await;
    assert_eq!(r.status(), 200);
    let body = value(r).await;
    let rotated = body["session"]["token"].as_str().unwrap();
    assert_ne!(rotated, token);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(db::auth(&mut c, &token).await.is_err());
    assert!(db::auth(&mut c, &other).await.is_err());
    assert!(db::auth(&mut c, rotated).await.is_ok());
    assert!(app.0.core.lock().await.voice.is_empty());
    for receiver in receivers {
        assert_eq!(*receiver.borrow(), Some(4001));
    }
}

#[tokio::test]
async fn s9_guild_storage_hides_other_guilds_dms_and_host_capacity() {
    let (_dir, app, token) = setup().await;
    sqlx::raw_sql("INSERT INTO guilds VALUES(20,2,'other',NULL,0);INSERT INTO channels VALUES(21,'text',20,NULL,'other',NULL,0,NULL,0);INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(100,1,'message',11,'own','application/octet-stream',7,'own',0),(101,2,'message',21,'other','application/octet-stream',999,'other',0),(102,2,'avatar',NULL,'dm','application/octet-stream',9999,'dm',0);").execute(&app.0.pool).await.unwrap();
    assert_eq!(create(&app, &token, 19).await.status(), 201);
    let r = Request::builder()
        .uri("/api/v1/guilds/10/storage")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let usage = value(call(&app, r).await).await;
    assert_eq!(usage["committed_bytes"], 7);
    assert_eq!(usage["reserved_bytes"], 19 + 65536);
    for key in ["total_bytes", "available_bytes", "min_free_bytes"] {
        assert_eq!(usage[key], 0);
    }
}

#[tokio::test]
async fn account_deletion_requires_proof_erases_files_and_holds_name_30_days() {
    let (_dir, app, token) = setup().await;
    sqlx::raw_sql("INSERT INTO messages VALUES(100,11,1,'private text',0,NULL,NULL,NULL);INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(101,1,'message',11,'private','application/octet-stream',6,'private.bin',0);INSERT INTO message_attachments VALUES(100,101,0);").execute(&app.0.pool).await.unwrap();
    let other = state::token();
    let other_key = state::hash(other.as_bytes());
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=2")
        .bind(&other_key)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let (tx, _packets) = tokio::sync::mpsc::channel(128);
    let (close, watch) = tokio::sync::watch::channel(None);
    {
        let mut core = app.0.core.lock().await;
        core.cursor(&other_key, Id(2));
        core.streams.get_mut(&other_key).unwrap().connection = Some(state::Connection {
            id: "other".into(),
            focused: false,
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
    }
    let file = app.0.config.data_dir.join("files/private.bin");
    tokio::fs::write(&file, b"secret").await.unwrap();
    assert_eq!(
        call(&app, req(&token, "/api/v1/auth/account/delete", json!({})))
            .await
            .status(),
        401
    );
    let proof=value(call(&app,req(&token,"/api/v1/auth/reauth",json!({"operation":"delete_account","current_password":"unused dummy credential","id_token":null}))).await).await;
    let mut r = req(&token, "/api/v1/auth/account/delete", json!({}));
    r.headers_mut().insert(
        "x-sticktalk-reauth",
        proof["proof"].as_str().unwrap().parse().unwrap(),
    );
    assert_eq!(call(&app, r).await.status(), 204);
    assert_eq!(*watch.borrow(), Some(4010));
    assert!(db::auth(&mut app.0.pool.acquire().await.unwrap(), &other)
        .await
        .is_ok());
    assert!(!file.exists());
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(db::auth(&mut c, &token).await.is_err());
    let row = sqlx::query("SELECT content,deleted_at FROM messages WHERE id=100")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("content"), "");
    assert!(row.get::<Option<i64>, _>("deleted_at").is_some());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id=1")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        "Deleted User"
    );
    assert!(crate::account::reserved(&mut c, "user1").await.unwrap());
    sqlx::query("UPDATE reserved_usernames SET release_at=?")
        .bind((state::now() - 1) as i64)
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(!crate::account::reserved(&mut c, "user1").await.unwrap());
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *c)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn large_account_erasure_finishes_before_http_deadline_and_revokes_every_device() {
    let (_dir, app, token) = setup().await;
    let other = state::token();
    let keys = [state::hash(token.as_bytes()), state::hash(other.as_bytes())];
    sqlx::query("INSERT INTO sessions VALUES(?,1,0,9999999999999,0)")
        .bind(&keys[1])
        .execute(&app.0.pool)
        .await
        .unwrap();
    // The phone QA fixture: a sole-owned guild and 10,000 messages. Cascading
    // deletion used to scan messages.reply_to 10,000 times and time out/rollback.
    sqlx::raw_sql("DELETE FROM guild_members WHERE user_id=2; WITH RECURSIVE n(x) AS (SELECT 100 UNION ALL SELECT x+1 FROM n WHERE x<10099) INSERT INTO messages(id,channel_id,author_id,content,created_at) SELECT x,11,1,'private history',0 FROM n;")
        .execute(&app.0.pool).await.unwrap();
    let plan = sqlx::query("EXPLAIN QUERY PLAN SELECT rowid FROM messages WHERE reply_to=100")
        .fetch_all(&app.0.pool)
        .await
        .unwrap();
    assert!(
        plan.iter()
            .any(|r| r.get::<String, _>("detail").contains("message_reply")),
        "the cascading child-key lookup must use the migration's index"
    );
    let mut watches = vec![];
    let mut packets = vec![];
    {
        let mut core = app.0.core.lock().await;
        for key in &keys {
            core.cursor(key, Id(1));
            let (tx, rx) = tokio::sync::mpsc::channel(128);
            packets.push(rx);
            let (close, watch) = tokio::sync::watch::channel(None);
            watches.push(watch);
            core.streams.get_mut(key).unwrap().connection = Some(state::Connection {
                id: "device".into(),
                focused: false,
                alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                tx,
                budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
                close,
            });
        }
    }
    let proof = value(
        call(
            &app,
            req(
                &token,
                "/api/v1/auth/reauth",
                json!({
                    "operation":"delete_account", "current_password":"unused dummy credential"
                }),
            ),
        )
        .await,
    )
    .await;
    let mut request = req(&token, "/api/v1/auth/account/delete", json!({}));
    request.headers_mut().insert(
        "x-sticktalk-reauth",
        proof["proof"].as_str().unwrap().parse().unwrap(),
    );
    let started = std::time::Instant::now();
    assert_eq!(call(&app, request).await.status(), 204);
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    for watch in watches {
        assert_eq!(*watch.borrow(), Some(4001));
    }
    let mut c = app.0.pool.acquire().await.unwrap();
    for token in [&token, &other] {
        assert!(db::auth(&mut c, token).await.is_err());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM deleted_accounts WHERE user_id=1")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        1
    );
    drop(c);
    app.housekeeping().await.unwrap();
}

#[tokio::test]
async fn admin_disable_revokes_all_live_sessions_and_rejects_reauthentication() {
    let (_dir, app, token) = setup().await;
    let key = state::hash(token.as_bytes());
    let (tx, _packets) = tokio::sync::mpsc::channel(128);
    let (close, watch) = tokio::sync::watch::channel(None);
    {
        let mut core = app.0.core.lock().await;
        core.cursor(&key, Id(1));
        core.streams.get_mut(&key).unwrap().connection = Some(state::Connection {
            id: "disabled-device".into(),
            focused: false,
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
    }
    crate::admin::run(
        &app.0.config,
        &crate::admin::AdminCommand::DisableUser {
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    app.housekeeping().await.unwrap();
    assert_eq!(*watch.borrow(), Some(4001));
    assert!(!app.0.core.lock().await.streams.contains_key(&key));
    assert!(db::auth(&mut app.0.pool.acquire().await.unwrap(), &token)
        .await
        .is_err());
}

#[tokio::test]
async fn s10_bundled_sqlite_is_security_patched() {
    let (_dir, app) = fixture().await;
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    let v: Vec<u32> = version.split('.').map(|s| s.parse().unwrap()).collect();
    assert!(v.as_slice() >= [3, 54, 0].as_slice(), "{version}");
}
#[test]
fn s7_backups_require_recipient_encrypt_before_sync_and_expire_in_14_days() {
    let script = include_str!("../../../deploy/pi/backup.sh");
    assert!(
        script.contains("BACKUP_RECIPIENT:?")
            && script.contains("age -r \"$BACKUP_RECIPIENT\"")
            && script.contains("-mmin +20160")
    );
    assert!(script.find("age -r").unwrap() < script.rfind("rsync -a --delete").unwrap());
}
