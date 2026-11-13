use crate::{
    blob::{BlobStore, RemoteStore, StorageOptions},
    state::{hash, now, App},
    storage_node::StorageNode,
};
use axum::body::{to_bytes, Body};
use bytes::Bytes;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use sticktalk_proto::*;
const SECRET: &str = "test-only-secret-32-bytes-minimum-never-deploy";
fn sha(bytes: &[u8]) -> String {
    crate::blob::hex(&Sha256::digest(bytes))
}
struct Node {
    node: StorageNode,
    addr: SocketAddr,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Node {
    async fn start(dir: &std::path::Path, addr: Option<SocketAddr>) -> Self {
        let listener =
            tokio::net::TcpListener::bind(addr.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let addr = listener.local_addr().unwrap();
        let node = StorageNode::open(dir.into(), SECRET, 0).await.unwrap();
        let serving = node.clone();
        let task = tokio::spawn(async move { serving.serve(listener).await });
        Self { node, addr, task }
    }
    fn remote(&self) -> RemoteStore {
        RemoteStore::new(&format!("http://{}", self.addr), SECRET).unwrap()
    }
    async fn stop(self) {
        self.node.shutdown();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
async fn fixture(
    node: &Node,
    cache_mb: u64,
) -> (
    tempfile::TempDir,
    App,
    reqwest::Client,
    String,
    String,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (dir, mut app) = crate::audit_tests::fixture().await;
    let inner = Arc::get_mut(&mut app.0).unwrap();
    inner.config.operations.storage = StorageOptions {
        storage_url: Some(format!("http://{}", node.addr)),
        storage_secret: Some(SECRET.into()),
        cache_mb,
        cache_file_mb: 25,
    };
    inner.config.max_upload_mb = 0;
    inner.config.max_total_upload_mb = 0;
    inner.tier = crate::tier::Tier::new(&inner.config).unwrap();
    let token = crate::state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(hash(token.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serve = app.clone();
    let task = tokio::spawn(async move { serve.serve(listener).await });
    (dir, app, reqwest::Client::new(), url, token, task)
}
async fn create(
    app: &App,
    client: &reqwest::Client,
    url: &str,
    token: &str,
    size: u64,
    digest: String,
) -> ResumableUpload {
    app.0.core.lock().await.limits.clear();
    let r = client
        .post(format!("{url}/api/v1/uploads/resumable"))
        .bearer_auth(token)
        .json(&CreateUploadRequest {
            purpose: UploadPurpose::Message,
            channel_id: Some(Id(11)),
            filename: "test.bin".into(),
            content_type: "application/octet-stream".into(),
            byte_size: size,
            sha256: digest,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
    r.json().await.unwrap()
}
async fn chunk(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    key: &str,
    offset: u64,
    bytes: &'static [u8],
) -> ResumableUpload {
    let r = client
        .put(format!("{url}/api/v1/uploads/resumable/{key}"))
        .bearer_auth(token)
        .header("upload-offset", offset)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    r.json().await.unwrap()
}
async fn finish(client: &reqwest::Client, url: &str, token: &str, key: &str) -> UploadResponse {
    let r = client
        .post(format!("{url}/api/v1/uploads/resumable/{key}/finish"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    r.json().await.unwrap()
}
#[tokio::test]
async fn storage_auth_replay_restart_payload_and_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let remote = node.remote();
    let request = remote
        .request(reqwest::Method::GET, "/v1/space", 0, 0, "", "")
        .build()
        .unwrap();
    assert_eq!(
        remote
            .client
            .execute(request.try_clone().unwrap())
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        remote
            .client
            .execute(request.try_clone().unwrap())
            .await
            .unwrap()
            .status(),
        401
    );
    let expired = ((now() / 1000) - 120).to_string();
    let nonce = crate::state::stream_id();
    let signature = crate::storage_node::sign(
        SECRET.as_bytes(),
        "GET",
        "/v1/space",
        &expired,
        &nonce,
        0,
        0,
        "",
        "",
    );
    let response = remote
        .client
        .get(format!("{}/v1/space", remote.url))
        .header("x-time", expired)
        .header("x-nonce", nonce)
        .header("x-signature", signature)
        .header("upload-offset", 0)
        .header("x-size", 0)
        .header("x-sha256", "")
        .header("range", "")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let bad = RemoteStore::new(
        &format!("http://{}", node.addr),
        "different-secret-32-bytes-at-least",
    )
    .unwrap();
    assert!(bad.free_space().await.is_err());
    // Range/method/path/body digest are MAC-bound, not just time and nonce.
    let mut tamper = remote
        .request(reqwest::Method::GET, "/v1/space", 0, 0, "", "")
        .build()
        .unwrap();
    tamper
        .headers_mut()
        .insert("range", "bytes=0-1".parse().unwrap());
    assert_eq!(remote.client.execute(tamper).await.unwrap().status(), 401);
    for key in [
        "../secret",
        "a/b",
        "a\\b",
        ".",
        "..",
        "a..b",
        "%2e%2e",
        "x\0",
    ] {
        assert!(!crate::blob::valid_key(key));
        assert!(remote.stat(key).await.is_err());
    }
    // Exercise URL decoding at the service boundary too, with a valid MAC for
    // the raw path (the HTTP client must not normalize away the attack).
    let response = remote
        .request(reqwest::Method::PUT, "/v1/blob/a%2Fb", 0, 1, &sha(b"a"), "")
        .header("content-length", 1)
        .body("a")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert!(remote
        .put_chunk("bad.bin", 0, 3, &sha(b"yes"), Body::from("bad"))
        .await
        .is_err());
    assert_eq!(remote.stat("bad.bin").await.unwrap().size, 0);
    remote.delete("bad.bin").await.unwrap();
    assert!(remote.stat("bad.bin").await.is_err());
    assert!(!dir.path().join("files/bad.bin").exists());
    remote
        .put_chunk("good.bin", 0, 3, &sha(b"yes"), Body::from("yes"))
        .await
        .unwrap();
    remote.finalize("good.bin", 3, &sha(b"yes")).await.unwrap();
    assert_eq!(
        to_bytes(remote.get("good.bin", 1, 2).await.unwrap(), 16)
            .await
            .unwrap(),
        "es"
    );
    let range = remote
        .request(
            reqwest::Method::GET,
            "/v1/blob/good.bin",
            0,
            0,
            "",
            "bytes=99-",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(range.status(), 416);
    assert_eq!(range.headers()["content-range"], "bytes */3");
    let addr = node.addr;
    node.stop().await;
    let node = Node::start(dir.path(), Some(addr)).await;
    assert_eq!(remote.client.execute(request).await.unwrap().status(), 401);
    assert_eq!(remote.stat("good.bin").await.unwrap().size, 3);
    node.stop().await;
}
#[tokio::test]
async fn storage_interrupted_put_and_download_resume() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let remote = node.remote();
    let (sent, wait) = tokio::sync::oneshot::channel();
    let (release, blocked) = tokio::sync::oneshot::channel::<()>();
    let body = Body::from_stream(
        futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"abc")) })
            .chain(futures_util::stream::once(async move {
                let _ = sent.send(());
                let _ = blocked.await;
                Ok::<_, std::io::Error>(Bytes::from_static(b"def"))
            })),
    );
    let writer = remote.clone();
    let transfer = tokio::spawn(async move {
        writer
            .put_chunk("partial.bin", 0, 6, &sha(b"abcdef"), body)
            .await
    });
    wait.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::fs::metadata(dir.path().join("files/partial.bin"))
                .await
                .is_ok_and(|m| m.len() == 3)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let addr = node.addr;
    node.stop().await;
    let _ = release.send(());
    assert!(transfer.await.unwrap().is_err());
    let node = Node::start(dir.path(), Some(addr)).await;
    assert_eq!(remote.stat("partial.bin").await.unwrap().size, 0);
    remote
        .put_chunk("partial.bin", 0, 6, &sha(b"abcdef"), Body::from("abcdef"))
        .await
        .unwrap();
    remote
        .finalize("partial.bin", 6, &sha(b"abcdef"))
        .await
        .unwrap();
    // A large stream cannot finish into TCP buffers before shutdown.
    let data = Bytes::from(vec![37; 16 * 1024 * 1024]);
    remote
        .put_chunk(
            "download.bin",
            0,
            data.len() as u64,
            &sha(&data),
            Body::from(data.clone()),
        )
        .await
        .unwrap();
    remote
        .finalize("download.bin", data.len() as u64, &sha(&data))
        .await
        .unwrap();
    let mut stream = remote
        .get("download.bin", 0, data.len() as u64)
        .await
        .unwrap()
        .into_data_stream();
    let prefix = stream.next().await.unwrap().unwrap();
    let received = prefix.len() as u64;
    assert!(received < data.len() as u64);
    node.stop().await;
    let mut got_error = false;
    while let Some(bytes) = stream.next().await {
        if bytes.is_err() {
            got_error = true;
            break;
        }
    }
    assert!(got_error);
    let node = Node::start(dir.path(), Some(addr)).await;
    let body = to_bytes(
        remote
            .get("download.bin", received, data.len() as u64 - received)
            .await
            .unwrap(),
        data.len(),
    )
    .await
    .unwrap();
    assert_eq!(body, &data[received as usize..]);
    node.stop().await;
}
#[tokio::test]
async fn hybrid_main_outage_resume_cache_permissions_sync_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let addr = node.addr;
    let (_main_dir, app, http, url, token, task) = fixture(&node, 0).await;
    let up = create(&app, &http, &url, &token, 8, sha(b"abcdefgh")).await;
    chunk(&http, &url, &token, &up.upload_id, 0, b"abcd").await;
    assert_eq!(
        node.remote()
            .stat(&format!("{}.part", up.upload_id))
            .await
            .unwrap()
            .size,
        4
    );
    node.stop().await;
    chunk(&http, &url, &token, &up.upload_id, 4, b"efgh").await;
    let a = finish(&http, &url, &token, &up.upload_id).await;
    let path = format!("{url}/api/v1/files/{}", a.attachment.id.0);
    assert_eq!(
        http.get(&path)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "abcdefgh"
    );
    let node = Node::start(dir.path(), Some(addr)).await;
    crate::tier::maintain(&app).await.unwrap();
    assert!(!app
        .0
        .config
        .data_dir
        .join("files")
        .join(format!("{}.part", up.upload_id))
        .exists());
    let r = http
        .get(&path)
        .bearer_auth(&token)
        .header("range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.bytes().await.unwrap(), "cdef");
    assert_eq!(http.get(&path).send().await.unwrap().status(), 401);
    // A restored DB can rebuild its replica ledger without pulling Pi-only
    // objects back onto Oracle just to hash them again.
    sqlx::query("DELETE FROM blob_replicas")
        .execute(&app.0.pool)
        .await
        .unwrap();
    crate::tier::maintain(&app).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blob_replicas WHERE synced=1")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        1
    );
    // Snapshot is online and includes all remote files before publication.
    let identity = age::x25519::Identity::generate();
    let mut backup_config = app.0.config.clone();
    backup_config.operations.backup_recipient = Some(identity.to_public().to_string());
    crate::backup::run(&backup_config).await.unwrap();
    let mut entries = tokio::fs::read_dir(dir.path().join("backups"))
        .await
        .unwrap();
    let mut backup = None;
    while let Some(e) = entries.next_entry().await.unwrap() {
        if e.file_name().to_string_lossy().starts_with("backup-") {
            backup = Some(e.path());
        }
    }
    let backup = backup.unwrap();
    assert!(backup.to_string_lossy().ends_with(".sqlite3.age"));
    let ciphertext = tokio::fs::read(&backup).await.unwrap();
    let plaintext = age::decrypt(&identity, &ciphertext).unwrap();
    let backup = dir.path().join("decrypted-test.sqlite3");
    tokio::fs::write(&backup, plaintext).await.unwrap();
    let mut db = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(backup)
            .read_only(true),
    )
    .await
    .unwrap();
    use sqlx::Connection;
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA quick_check")
            .fetch_one(&mut db)
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploads")
            .fetch_one(&mut db)
            .await
            .unwrap(),
        1
    );
    node.stop().await;
    let r = http.get(&path).bearer_auth(&token).send().await.unwrap();
    assert_eq!(r.status(), 503);
    assert!(r.text().await.unwrap().contains("temporarily offline"));
    let node = Node::start(dir.path(), Some(addr)).await;
    sqlx::query("DELETE FROM uploads")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM partial_uploads")
        .execute(&app.0.pool)
        .await
        .unwrap();
    crate::tier::maintain(&app).await.unwrap();
    // Tombstone retains bytes for restoring the 14-day snapshots.
    assert!(dir
        .path()
        .join("files")
        .join(format!("{}.part", up.upload_id))
        .exists());
    app.shutdown();
    task.await.unwrap().unwrap();
    node.stop().await;
}
#[test]
fn storage_configuration_rejects_public_destinations_and_redacts_secret() {
    for url in [
        "http://0.0.0.0:9000",
        "http://192.168.1.1:9000",
        "http://example.com",
        "http://100.64.0.1/evil",
        "http://127.0.0.1/?x=1",
    ] {
        assert!(RemoteStore::new(url, SECRET).is_err());
    }
    let options = StorageOptions {
        storage_secret: Some(SECRET.into()),
        ..Default::default()
    };
    assert!(!format!("{options:?}").contains(SECRET));
    assert!(!crate::storage_node::private_address(
        "0.0.0.0".parse().unwrap()
    ));
    assert!(crate::storage_node::private_address(
        "100.64.0.2".parse().unwrap()
    ));
    assert!(now() > 0);
}
#[cfg(windows)]
fn peak_rss() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: OS pseudo-handle and correctly sized writable PROCESS_MEMORY_COUNTERS.
    assert_ne!(
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                std::mem::size_of::<Counters>() as u32,
            )
        },
        0
    );
    counters.peak
}
#[cfg(not(windows))]
fn peak_rss() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| {
            l.strip_prefix("VmHWM:")
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<usize>().ok())
        })
        .unwrap()
        * 1024
}
/// Opt-in disk/RSS acceptance test: two actual loopback HTTP servers, >2 GiB.
/// Run alone to avoid unrelated tests contaminating process peak RSS.
#[tokio::test]
#[ignore = "writes >2 GiB to each tier; run explicitly for memory acceptance"]
async fn hybrid_multigb_stream_flat_rss() {
    const BLOCK: &[u8] = &[b'Z'; 65536];
    const FULL_CHUNKS: u64 = 128;
    let total = FULL_CHUNKS * crate::resumable::CHUNK + BLOCK.len() as u64;
    let mut digest = Sha256::new();
    for _ in 0..total / BLOCK.len() as u64 {
        digest.update(BLOCK);
    }
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let (_main_dir, app, http, url, token, task) = fixture(&node, 0).await;
    let baseline = peak_rss();
    let start = std::time::Instant::now();
    let up = create(
        &app,
        &http,
        &url,
        &token,
        total,
        crate::blob::hex(&digest.finalize()),
    )
    .await;
    let mut offset = 0;
    let mut checkpoints = vec![];
    while offset < total {
        let length = (total - offset).min(crate::resumable::CHUNK);
        let body = reqwest::Body::wrap_stream(futures_util::stream::iter(
            (0..length / BLOCK.len() as u64)
                .map(|_| Ok::<_, std::io::Error>(Bytes::from_static(BLOCK))),
        ));
        let r = http
            .put(format!("{url}/api/v1/uploads/resumable/{}", up.upload_id))
            .bearer_auth(&token)
            .header("upload-offset", offset)
            .header("content-length", length)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
        let status: ResumableUpload = r.json().await.unwrap();
        offset += length;
        assert_eq!(status.offset, offset);
        if offset % (512 * 1024 * 1024) == 0 {
            checkpoints.push((offset, peak_rss()));
        }
    }
    let a = finish(&http, &url, &token, &up.upload_id).await;
    assert_eq!(a.attachment.byte_size, total);
    let response = http
        .get(format!("{url}/api/v1/files/{}", a.attachment.id.0))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let mut downloaded = response.bytes_stream();
    let mut count = 0u64;
    let mut sha = Sha256::new();
    while let Some(bytes) = downloaded.next().await {
        let bytes = bytes.unwrap();
        count += bytes.len() as u64;
        sha.update(&bytes);
    }
    assert_eq!(count, total);
    let mut expected = Sha256::new();
    for _ in 0..total / BLOCK.len() as u64 {
        expected.update(BLOCK);
    }
    assert_eq!(sha.finalize(), expected.finalize());
    let peak = peak_rss();
    println!("HYBRID_RSS bytes={total} baseline={baseline} peak={peak} growth={} elapsed_s={:.2} checkpoints={checkpoints:?}",peak-baseline,start.elapsed().as_secs_f64());
    assert!(
        peak < baseline + 128 * 1024 * 1024,
        "streaming RSS grew beyond 128 MiB"
    );
    app.shutdown();
    task.await.unwrap().unwrap();
    node.stop().await;
}

#[tokio::test]
async fn hybrid_lru_cache_and_backup_retention() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let (_main_dir, app, _http, _url, _token, task) = fixture(&node, 1).await;
    let bytes = vec![7; 768 * 1024];
    for (id, key) in [
        (100, "cachea.bin"),
        (101, "cacheb.bin"),
        (102, "cachec.jpg"),
    ] {
        tokio::fs::write(app.0.tier.local.path(key).unwrap(), &bytes)
            .await
            .unwrap();
        sqlx::query("INSERT INTO uploads(id,owner_id,purpose,filename,content_type,byte_size,storage_key,created_at,expires_at) VALUES(?,1,'avatar','file','application/octet-stream',?,?,0,?)").bind(id).bind(bytes.len() as i64).bind(key).bind((now()+3600000) as i64).execute(&app.0.pool).await.unwrap();
        crate::tier::mirror(&app, key, bytes.len() as u64, true)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE blob_replicas SET accessed_at=1")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let _body = app.0.tier.get(&app, "cachea.bin", 0, 1).await.unwrap();
    crate::tier::maintain(&app).await.unwrap();
    assert!(app.0.tier.local.path("cachea.bin").unwrap().exists());
    assert!(!app.0.tier.local.path("cacheb.bin").unwrap().exists());
    assert!(!app.0.tier.local.path("cachec.jpg").unwrap().exists());
    let remote = node.remote();
    let old = format!("backup-{}.sqlite3", now() - 16 * 86400000);
    let recent = format!("backup-{}.sqlite3", now());
    remote
        .put_chunk(&old, 0, 3, &sha(b"old"), Body::from("old"))
        .await
        .unwrap();
    remote.finalize(&old, 3, &sha(b"old")).await.unwrap();
    remote
        .put_chunk(&recent, 0, 3, &sha(b"new"), Body::from("new"))
        .await
        .unwrap();
    remote.finalize(&recent, 3, &sha(b"new")).await.unwrap();
    assert!(remote.stat(&old).await.is_err());
    assert!(remote.stat(&recent).await.unwrap().complete);
    remote.delete("cachea.bin").await.unwrap();
    assert!(dir.path().join("files/cachea.bin").exists());
    use sqlx::Connection;
    let mut db = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("storage.sqlite3")),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE blobs SET deleted_at=? WHERE key='cachea.bin'")
        .bind(now().saturating_sub(16 * 86400000) as i64)
        .execute(&mut db)
        .await
        .unwrap();
    remote.free_space().await.unwrap();
    assert!(!dir.path().join("files/cachea.bin").exists());
    app.shutdown();
    task.await.unwrap().unwrap();
    node.stop().await;
}

#[tokio::test]
async fn hybrid_proxy_mid_download_range_retry() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::start(dir.path(), None).await;
    let addr = node.addr;
    let (_main_dir, app, http, url, token, task) = fixture(&node, 0).await;
    let data = Bytes::from(vec![53; 16 * 1024 * 1024]);
    let up = create(&app, &http, &url, &token, data.len() as u64, sha(&data)).await;
    let r = http
        .put(format!("{url}/api/v1/uploads/resumable/{}", up.upload_id))
        .bearer_auth(&token)
        .header("upload-offset", 0)
        .body(data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let a = finish(&http, &url, &token, &up.upload_id).await;
    let path = format!("{url}/api/v1/files/{}", a.attachment.id.0);
    let response = http.get(&path).bearer_auth(&token).send().await.unwrap();
    let mut body = response.bytes_stream();
    let prefix = body.next().await.unwrap().unwrap();
    let mut received = prefix.len();
    node.stop().await;
    let mut failed = false;
    while let Some(next) = body.next().await {
        match next {
            Ok(bytes) => received += bytes.len(),
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    assert!(failed && received < data.len());
    let node = Node::start(dir.path(), Some(addr)).await;
    let r = http
        .get(&path)
        .bearer_auth(&token)
        .header("range", format!("bytes={received}-"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.bytes().await.unwrap(), &data[received..]);
    app.shutdown();
    task.await.unwrap().unwrap();
    node.stop().await;
}
