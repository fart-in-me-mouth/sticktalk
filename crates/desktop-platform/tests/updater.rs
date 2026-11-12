use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use sticktalk_desktop_platform::updater::{Updater, CHECK_INTERVAL, TARGET};
use sticktalk_proto::{SignedUpdateManifest, UpdateManifest};

const BINARY: &[u8] = b"test installer bytes - this is never executed";
struct StateData {
    envelope: Mutex<SignedUpdateManifest>,
    mode: AtomicUsize,
    ranges: Mutex<Vec<String>>,
    checks: AtomicUsize,
}
struct Server {
    state: Arc<StateData>,
    origin: String,
    task: tokio::task::JoinHandle<()>,
    key: Ed25519KeyPair,
    dir: tempfile::TempDir,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let state = Arc::new(StateData {
            envelope: Mutex::new(sign(&key, &manifest(&origin))),
            mode: AtomicUsize::new(0),
            ranges: Mutex::new(vec![]),
            checks: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route("/api/update/latest", get(latest))
            .route("/api/update/artifact", get(artifact))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            state,
            origin,
            task,
            key,
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn updater(&self) -> Updater {
        Updater::new(
            &self.origin,
            "0.1.0",
            self.dir.path().into(),
            self.key.public_key().as_ref().try_into().unwrap(),
        )
        .unwrap()
    }
    fn resign(&self, edit: impl FnOnce(&mut UpdateManifest)) {
        let mut m = manifest(&self.origin);
        edit(&mut m);
        *self.state.envelope.lock().unwrap() = sign(&self.key, &m);
    }
}
fn manifest(origin: &str) -> UpdateManifest {
    let sha256 = format!("{:x}", Sha256::digest(BINARY));
    UpdateManifest {
        version: "0.2.0".into(),
        target: TARGET.into(),
        url: format!("{origin}api/update/artifact?sha256={sha256}"),
        byte_size: BINARY.len() as u64,
        sha256,
    }
}
fn sign(key: &Ed25519KeyPair, m: &UpdateManifest) -> SignedUpdateManifest {
    let manifest = serde_json::to_string(m).unwrap();
    let signature = STANDARD.encode(key.sign(manifest.as_bytes()).as_ref());
    SignedUpdateManifest {
        manifest,
        signature,
    }
}
async fn latest(State(s): State<Arc<StateData>>) -> Response<Body> {
    s.checks.fetch_add(1, Ordering::SeqCst);
    let bytes = serde_json::to_vec(&*s.envelope.lock().unwrap()).unwrap();
    if s.mode.load(Ordering::SeqCst) == 5 {
        return Response::builder().status(404).body(Body::empty()).unwrap();
    }
    Response::new(Body::from(bytes))
}
async fn artifact(State(s): State<Arc<StateData>>, headers: HeaderMap) -> Response<Body> {
    let mode = s.mode.load(Ordering::SeqCst);
    let range = headers.get("range").and_then(|v| v.to_str().ok());
    if let Some(r) = range {
        s.ranges.lock().unwrap().push(r.into());
    }
    let mut bytes = BINARY.to_vec();
    if mode == 1 {
        bytes[0] ^= 1;
    }
    if mode == 4 {
        bytes.push(0);
    }
    if let Some(range) = range.filter(|_| mode != 2) {
        let offset = range
            .strip_prefix("bytes=")
            .unwrap()
            .trim_end_matches('-')
            .parse::<usize>()
            .unwrap();
        return Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                "content-range",
                format!(
                    "bytes {}-{}/{}",
                    if mode == 3 { offset + 1 } else { offset },
                    bytes.len() - 1,
                    bytes.len()
                ),
            )
            .body(Body::from(bytes[offset..].to_vec()))
            .unwrap();
    }
    Response::new(Body::from(bytes))
}

#[tokio::test]
async fn verifies_downloads_stages_and_reverifies_after_restart() {
    let s = Server::start().await;
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    let staged = u.download(&v).await.unwrap();
    assert_eq!(std::fs::read(staged.installer()).unwrap(), BINARY);
    assert_eq!(staged.version(), "0.2.0");
    let pending = s.updater().pending().await.unwrap().unwrap();
    assert_eq!(pending.installer(), staged.installer());
    // Already downloaded stages and an existing pending.json are idempotent.
    assert_eq!(
        u.download(&v).await.unwrap().installer(),
        staged.installer()
    );
    std::fs::write(staged.installer(), vec![0; BINARY.len()]).unwrap();
    assert!(u.pending().await.is_err());
}
#[tokio::test]
async fn tampered_manifest_and_wrong_key_fail_before_download() {
    let s = Server::start().await;
    let mut e = s.state.envelope.lock().unwrap().clone();
    e.manifest.push(' ');
    assert!(s.updater().verify(e.clone()).is_err());
    *s.state.envelope.lock().unwrap() = e;
    assert!(s.updater().check().await.is_err());
    let other = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
    assert!(s
        .updater()
        .verify(sign(&other, &manifest(&s.origin)))
        .is_err());
    assert!(!s.dir.path().join("pending.json").exists());
}
#[tokio::test]
async fn tampered_binary_is_deleted_and_not_staged() {
    let s = Server::start().await;
    s.state.mode.store(1, Ordering::SeqCst);
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    assert!(u.download(&v).await.is_err());
    assert!(!s.dir.path().join("pending.json").exists());
    assert!(!s
        .dir
        .path()
        .join(format!("{}.part", v.manifest().sha256))
        .exists());
}
#[tokio::test]
async fn range_resume_survives_restart() {
    let s = Server::start().await;
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    std::fs::write(
        s.dir.path().join(format!("{}.part", v.manifest().sha256)),
        &BINARY[..10],
    )
    .unwrap();
    s.updater().download(&v).await.unwrap();
    assert_eq!(*s.state.ranges.lock().unwrap(), vec!["bytes=10-"]);
}
#[tokio::test]
async fn ignored_range_restarts_safely() {
    let s = Server::start().await;
    s.state.mode.store(2, Ordering::SeqCst);
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    std::fs::write(
        s.dir.path().join(format!("{}.part", v.manifest().sha256)),
        &BINARY[..10],
    )
    .unwrap();
    let staged = u.download(&v).await.unwrap();
    assert_eq!(std::fs::read(staged.installer()).unwrap(), BINARY);
}
#[tokio::test]
async fn invalid_content_range_is_rejected() {
    let s = Server::start().await;
    s.state.mode.store(3, Ordering::SeqCst);
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    std::fs::write(
        s.dir.path().join(format!("{}.part", v.manifest().sha256)),
        &BINARY[..10],
    )
    .unwrap();
    assert!(u.download(&v).await.is_err());
    assert!(u.pending().await.unwrap().is_none());
}
#[tokio::test]
async fn signed_bounds_origin_target_and_version_are_checked() {
    let s = Server::start().await;
    let u = s.updater();
    for edit in [
        (|m: &mut UpdateManifest| m.url = "https://evil.invalid/setup.exe".into())
            as fn(&mut UpdateManifest),
        |m| m.target = "linux".into(),
        |m| m.byte_size = 0,
        |m| m.byte_size = 1024 * 1024 * 1024 + 1,
        |m| m.sha256 = "F".repeat(64),
        |m| m.version = "../../escape".into(),
        |m| m.version = "0.0.9".into(),
    ] {
        let mut m = manifest(&s.origin);
        edit(&mut m);
        assert!(u.verify(sign(&s.key, &m)).is_err());
    }
    assert!(Updater::new("http://example.com/", "0.1.0", s.dir.path().into(), [0; 32]).is_err());
}
#[tokio::test]
async fn no_release_current_and_older_are_normal() {
    let s = Server::start().await;
    s.resign(|m| m.version = "0.1.0".into());
    assert!(s.updater().check().await.unwrap().is_none());
    s.resign(|m| m.version = "0.0.1".into());
    assert!(s.updater().check().await.unwrap().is_none());
    s.state.mode.store(5, Ordering::SeqCst);
    assert!(s.updater().check().await.unwrap().is_none());
}
#[tokio::test]
async fn larger_than_signed_binary_never_stages() {
    let s = Server::start().await;
    s.state.mode.store(4, Ordering::SeqCst);
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    assert!(u.download(&v).await.is_err());
    assert!(u.pending().await.unwrap().is_none());
}
#[tokio::test]
async fn persisted_manifest_is_not_trusted() {
    let s = Server::start().await;
    let u = s.updater();
    let v = u.check().await.unwrap().unwrap();
    u.download(&v).await.unwrap();
    let mut e = s.state.envelope.lock().unwrap().clone();
    e.manifest.push(' ');
    std::fs::write(
        s.dir.path().join("pending.json"),
        serde_json::to_vec(&e).unwrap(),
    )
    .unwrap();
    assert!(u.pending().await.is_err());
}
#[tokio::test]
async fn timer_is_immediate_then_six_hours_and_stops_on_receiver_drop() {
    let s = Server::start().await;
    let u = s.updater();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let task = tokio::spawn(async move {
        u.watch(tx).await;
    });
    rx.recv().await.unwrap().unwrap();
    assert_eq!(s.state.checks.load(Ordering::SeqCst), 1);
    tokio::time::pause();
    tokio::time::advance(CHECK_INTERVAL - Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_err());
    assert_eq!(s.state.checks.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    rx.recv().await.unwrap().unwrap();
    assert_eq!(s.state.checks.load(Ordering::SeqCst), 2);
    drop(rx);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}
