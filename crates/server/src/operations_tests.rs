use crate::{admin, audit_tests::fixture, resumable, state::*, updates, App};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{HeaderMap, Request},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use sticktalk_proto::*;

async fn setup() -> (tempfile::TempDir, App, HeaderMap) {
    let (dir, mut app) = fixture().await;
    let config = &mut Arc::get_mut(&mut app.0).unwrap().config;
    config.max_upload_mb = 0;
    config.max_total_upload_mb = 0;
    config.operations.min_free_disk_gb = 0;
    let secret = token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(hash(secret.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut h = HeaderMap::new();
    h.insert("authorization", format!("Bearer {secret}").parse().unwrap());
    (dir, app, h)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn req(h: &HeaderMap, value: &impl serde::Serialize) -> Request<Body> {
    let mut r = Request::new(Body::from(serde_json::to_vec(value).unwrap()));
    *r.headers_mut() = h.clone();
    r.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    r
}
async fn create(app: &App, h: &HeaderMap, size: u64, sha: String) -> ResumableUpload {
    app.0.core.lock().await.limits.clear();
    let response = resumable::create(
        State(app.clone()),
        req(
            h,
            &CreateUploadRequest {
                purpose: UploadPurpose::Message,
                channel_id: Some(Id(11)),
                filename: "../../safe.bin".into(),
                content_type: "application/octet-stream".into(),
                byte_size: size,
                sha256: sha,
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 201);
    serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap()
}
async fn chunk(
    app: &App,
    h: &HeaderMap,
    key: &str,
    offset: u64,
    data: &[u8],
) -> crate::Result<ResumableUpload> {
    let mut r = Request::new(Body::from(data.to_vec()));
    *r.headers_mut() = h.clone();
    r.headers_mut()
        .insert("upload-offset", offset.to_string().parse().unwrap());
    r.headers_mut()
        .insert("content-length", data.len().to_string().parse().unwrap());
    resumable::chunk(State(app.clone()), Path(key.into()), r)
        .await
        .map(|r| r.0)
}
#[tokio::test]
async fn resumable_crash_restart_hash_acl_range_and_idempotent_finish() {
    let (_dir, app, h) = setup().await;
    let upload = create(&app, &h, 8, digest(b"abcdefgh")).await;
    assert_eq!(
        chunk(&app, &h, &upload.upload_id, 0, b"abcd")
            .await
            .unwrap()
            .offset,
        4
    );
    assert_eq!(
        chunk(&app, &h, &upload.upload_id, 0, b"abcd")
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::Conflict
    );
    // A crash leaves extra bytes beyond the durable offset. Startup keeps the file.
    let path = app
        .0
        .config
        .data_dir
        .join("files")
        .join(format!("{}.part", upload.upload_id));
    tokio::fs::write(&path, b"abcdCRASH").await.unwrap();
    let config = app.0.config.clone();
    drop(app);
    let app = App::open(config).await.unwrap();
    assert!(path.exists());
    assert_eq!(
        resumable::status(
            State(app.clone()),
            Path(upload.upload_id.clone()),
            h.clone()
        )
        .await
        .unwrap()
        .offset,
        4
    );
    chunk(&app, &h, &upload.upload_id, 4, b"efgh")
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"abcdefgh");
    let a = resumable::finish(
        State(app.clone()),
        Path(upload.upload_id.clone()),
        h.clone(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(a.attachment.filename, "safe.bin");
    assert_eq!(
        resumable::finish(
            State(app.clone()),
            Path(upload.upload_id.clone()),
            h.clone()
        )
        .await
        .unwrap()
        .0,
        a
    );
    let mut range = h.clone();
    range.insert("range", "bytes=2-5".parse().unwrap());
    let response = crate::files::original(
        State(app.clone()),
        Path(a.attachment.id.0.to_string()),
        range,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        &b"cdef"[..]
    );
    let bad = create(&app, &h, 3, digest(b"yes")).await;
    chunk(&app, &h, &bad.upload_id, 0, b"bad").await.unwrap();
    assert_eq!(
        resumable::finish(State(app.clone()), Path(bad.upload_id.clone()), h.clone())
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::InvalidAttachment
    );
    // Revoked channel access also blocks the owner of a staged upload.
    sqlx::query("DELETE FROM guild_members WHERE user_id=1")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert!(
        resumable::status(State(app.clone()), Path(bad.upload_id), h)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn unlimited_admission_quota_storage_guard_gc_and_owner_usage() {
    let (_dir, mut app, h) = setup().await;
    let a = create(&app, &h, 32 * 1024 * 1024, digest(b"large")).await;
    let usage = resumable::usage(State(app.clone()), Path("10".into()), h.clone())
        .await
        .unwrap()
        .0;
    assert_eq!(usage.reserved_bytes, 32 * 1024 * 1024 + 65536);
    let secret = token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=2")
        .bind(hash(secret.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut other = h.clone();
    other.insert("authorization", format!("Bearer {secret}").parse().unwrap());
    assert!(
        resumable::status(State(app.clone()), Path(a.upload_id.clone()), other.clone())
            .await
            .is_err()
    );
    assert_eq!(
        resumable::usage(State(app.clone()), Path("10".into()), other)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::OwnerRequired
    );
    sqlx::query("UPDATE partial_uploads SET expires_at=0")
        .execute(&app.0.pool)
        .await
        .unwrap();
    app.housekeeping().await.unwrap();
    assert!(!app
        .0
        .config
        .data_dir
        .join("files")
        .join(format!("{}.part", a.upload_id))
        .exists());
    Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .min_free_disk_gb = u64::MAX;
    let error = resumable::disk_guard(&app, 1).await.unwrap_err();
    assert_eq!(error.0.message, "Server storage is full.");
    Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .min_free_disk_gb = 0;
    Arc::get_mut(&mut app.0)
        .unwrap()
        .config
        .operations
        .user_quota_mb = 1;
    let request = CreateUploadRequest {
        purpose: UploadPurpose::Message,
        channel_id: Some(Id(11)),
        filename: "a".into(),
        content_type: "text/plain".into(),
        byte_size: 2 * 1024 * 1024,
        sha256: digest(b""),
    };
    assert_eq!(
        resumable::create(State(app.clone()), req(&h, &request))
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::LimitExceeded
    );
}

#[tokio::test]
async fn duplicate_chunks_are_serialized_and_cancel_frees_reservations() {
    let (_dir, app, h) = setup().await;
    let a = create(&app, &h, 4, digest(b"abcd")).await;
    let (one, two) = tokio::join!(
        chunk(&app, &h, &a.upload_id, 0, b"abcd"),
        chunk(&app, &h, &a.upload_id, 0, b"abcd")
    );
    assert_ne!(one.is_ok(), two.is_ok());
    assert_eq!(
        one.err().or_else(|| two.err()).unwrap().0.code,
        ErrorCode::Conflict
    );
    let mut oversized = Request::new(Body::empty());
    *oversized.headers_mut() = h.clone();
    oversized.headers_mut().insert(
        "content-length",
        (resumable::CHUNK + 1).to_string().parse().unwrap(),
    );
    oversized
        .headers_mut()
        .insert("upload-offset", "4".parse().unwrap());
    assert_eq!(
        resumable::chunk(State(app.clone()), Path(a.upload_id.clone()), oversized)
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::PayloadTooLarge
    );
    assert_eq!(
        resumable::cancel(State(app.clone()), Path(a.upload_id.clone()), h.clone())
            .await
            .unwrap(),
        204
    );
    assert!(!app
        .0
        .config
        .data_dir
        .join("files")
        .join(format!("{}.part", a.upload_id))
        .exists());
    assert_eq!(
        resumable::usage(State(app.clone()), Path("10".into()), h)
            .await
            .unwrap()
            .reserved_bytes,
        0
    );
}
#[tokio::test]
async fn reset_code_single_use_and_disable_covers_all_sessions() {
    let (_dir, app, h) = setup().await;
    admin::run(
        &app.0.config,
        &admin::AdminCommand::DisableUser {
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    assert!(crate::db::auth(
        &mut app.0.pool.acquire().await.unwrap(),
        crate::http::bearer(&h).unwrap()
    )
    .await
    .is_err());
    assert!(
        crate::db::enabled(&mut app.0.pool.acquire().await.unwrap(), Id(1))
            .await
            .is_err()
    );
    admin::run(
        &app.0.config,
        &admin::AdminCommand::EnableUser {
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    let code = token();
    sqlx::query("INSERT INTO password_resets VALUES(1,?,?)")
        .bind(hash(code.as_bytes()))
        .bind((now() + 10000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let request = ResetPasswordRequest {
        username: "user1".into(),
        code,
        new_password: "a brand new password".into(),
    };
    assert_eq!(
        admin::reset(State(app.clone()), req(&HeaderMap::new(), &request))
            .await
            .unwrap(),
        204
    );
    assert_eq!(
        admin::reset(State(app.clone()), req(&HeaderMap::new(), &request))
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::InvalidCredentials
    );
    let p: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=1")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert!(crate::http::verify_password("a brand new password", &p));
    admin::run(
        &app.0.config,
        &admin::AdminCommand::ResetPassword {
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    let size: i64 =
        sqlx::query_scalar("SELECT length(code_hash) FROM password_resets WHERE user_id=1")
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
    assert_eq!(size, 32);
}
#[tokio::test]
async fn signed_release_publish_round_trip_and_tamper_rejection() {
    let (_dir, mut app, _) = setup().await;
    let pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let public = STANDARD.encode(pair.public_key().as_ref());
    let config = &mut Arc::get_mut(&mut app.0).unwrap().config;
    config.public_url = "https://chat.example.ts.net".into();
    config.operations.update_public_key = Some(public.clone());
    config.operations.release_token = Some("test-publishing-token-with-32-bytes".into());
    let bytes = b"installer-fixture";
    let sha = digest(bytes);
    let manifest = serde_json::to_string(&UpdateManifest {
        version: "0.1.1".into(),
        target: "x86_64-pc-windows-msvc".into(),
        url: format!("https://chat.example.ts.net/api/update/artifact?sha256={sha}"),
        byte_size: bytes.len() as u64,
        sha256: sha,
    })
    .unwrap();
    let signed = SignedUpdateManifest {
        signature: STANDARD.encode(pair.sign(manifest.as_bytes()).as_ref()),
        manifest,
    };
    assert!(updates::verify(&public, &signed).is_ok());
    let mut tampered = signed.clone();
    tampered.manifest.push(' ');
    assert!(updates::verify(&public, &tampered).is_err());
    let mut r = Request::new(Body::from(bytes.to_vec()));
    r.headers_mut().insert(
        "authorization",
        "Bearer test-publishing-token-with-32-bytes"
            .parse()
            .unwrap(),
    );
    r.headers_mut().insert(
        "x-sticktalk-manifest",
        STANDARD
            .encode(serde_json::to_vec(&signed).unwrap())
            .parse()
            .unwrap(),
    );
    assert_eq!(updates::publish(State(app.clone()), r).await.unwrap(), 201);
    let latest = updates::latest(State(app.clone())).await.unwrap();
    let returned: SignedUpdateManifest =
        serde_json::from_slice(&to_bytes(latest.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(returned, signed);
    // Exercise the actual Axum routes/middleware, not only handler calls.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = app.clone();
    let task = tokio::spawn(async move { serving.serve(listener).await.unwrap() });
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{url}/api/update/latest"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<SignedUpdateManifest>().await.unwrap(),
        signed
    );
    let metadata = updates::verify(&public, &signed).unwrap();
    let artifact = client
        .get(format!(
            "{url}/api/update/artifact?sha256={}",
            metadata.sha256
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(artifact.status(), 200);
    assert_eq!(artifact.bytes().await.unwrap(), &bytes[..]);
    assert_eq!(
        client
            .post(format!("{url}/api/update/publish"))
            .body("no-auth")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{url}/api/v1/guilds/10/storage"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    app.shutdown();
    task.await.unwrap();
}
