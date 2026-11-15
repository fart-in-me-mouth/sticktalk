//! Real desktop transport/client-core projection against a TCP/SQLite/blob server.
//! Normal tests never open windows; the explicit ignored desktop gate opens two
//! secondary/no-activate windows using synthetic loopback accounts and isolated
//! settings homes. No keyring writes, external services, browsers, or OS effects.
//! Expensive gates are opt-in: 2 GiB upload, native decoder, native desktop, and
//! 2,048-message production-rate soak (about 34 minutes). Run them individually
//! with `cargo test -p sticktalk --test live <name> -- --ignored --exact`.
//! Use CARGO_INCREMENTAL=0 and the single task target directory from AGENTS.md.
use axum::{
    extract::{Query, State},
    response::Redirect,
    routing::{get, post},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::StreamExt;
use rsa::{pkcs8::EncodePrivateKey, traits::PublicKeyParts, RsaPrivateKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use sticktalk::{
    backend::{self, Data, Handle, HistoryAnchor, Http, Request},
    model::Model,
};
use sticktalk_proto::*;
use sticktalk_server::{App, Config};
use tokio::sync::{mpsc, watch, Mutex, Notify};

const PASSWORD: &str = "correct horse battery staple";
const CLIENT_ID: &str = "sticktalk-e2e.apps.googleusercontent.com";
const DEADLINE: Duration = Duration::from_secs(20);

// Tests honor real token buckets instead of weakening production rate limits.
async fn post_retry<T: serde::de::DeserializeOwned>(
    http: &Http,
    path: &str,
    body: &impl serde::Serialize,
) -> Result<T, String> {
    loop {
        match http.post(path, body).await {
            Err(error) if error.starts_with("You're doing that too quickly") => {
                tokio::time::sleep(Duration::from_secs(6)).await
            }
            result => return result,
        }
    }
}
async fn account_retry(
    http: &Http,
    action: backend::AccountAction,
    proof: &str,
) -> Result<AccountUpdateResponse, String> {
    loop {
        match http
            .core()
            .account_authorized(action.clone(), Some(proof))
            .await
        {
            Err(error) if error.starts_with("You're doing that too quickly") => {
                tokio::time::sleep(Duration::from_secs(6)).await
            }
            result => return result,
        }
    }
}
async fn password_auth(
    url: &str,
    name: &str,
    password: &str,
    register: bool,
) -> Result<AuthResponse, String> {
    loop {
        match backend::authenticate_unstored(
            url,
            name.into(),
            password.into(),
            register,
            name.into(),
        )
        .await
        {
            Err(error) if error.starts_with("You're doing that too quickly") => {
                tokio::time::sleep(Duration::from_secs(6)).await
            }
            result => return result,
        }
    }
}

#[derive(Clone)]
struct MockIssuer {
    key: Arc<RsaPrivateKey>,
    issuer: String,
    challenge: Arc<Mutex<Option<String>>>,
    nonce: Arc<std::sync::Mutex<String>>,
}
impl MockIssuer {
    fn token(&self, audience: &str, expires: u64, verified: bool) -> String {
        let now = sticktalk::mock::now() / 1000;
        let h = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&json!({"alg":"RS256","kid":"e2e"})).unwrap());
        let c = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"iss":self.issuer,"aud":audience,"sub":"carol-subject","exp":expires,"iat":now-3600,"email_verified":verified,"nonce":self.nonce.lock().unwrap().clone()})).unwrap());
        let data = format!("{h}.{c}");
        let key =
            ring::signature::RsaKeyPair::from_pkcs8(self.key.to_pkcs8_der().unwrap().as_bytes())
                .unwrap();
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            data.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{data}.{}", URL_SAFE_NO_PAD.encode(signature))
    }
}
struct Fixture {
    app: App,
    url: String,
    mock: MockIssuer,
    _dir: tempfile::TempDir,
    serving: tokio::task::JoinHandle<std::io::Result<()>>,
    upstream: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.app.shutdown();
        self.serving.abort();
        self.upstream.abort();
    }
}
impl Fixture {
    async fn start() -> Self {
        let key = tokio::task::spawn_blocking(|| {
            RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap()
        })
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let mock = MockIssuer {
            key: Arc::new(key),
            issuer: issuer.clone(),
            challenge: Default::default(),
            nonce: Default::default(),
        };
        let router = axum::Router::new()
            .route("/jwks", get(|State(m): State<MockIssuer>| async move {
                ([("cache-control", "max-age=3600")], Json(json!({"keys":[{"kid":"e2e","kty":"RSA","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(m.key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(m.key.e().to_bytes_be())}]})))
            }))
            .route("/authorize", get(|State(m): State<MockIssuer>, Query(q): Query<HashMap<String,String>>| async move {
                assert_eq!(q["code_challenge_method"], "S256");
                assert_eq!(q["client_id"], CLIENT_ID);
                *m.challenge.lock().await = Some(q["code_challenge"].clone());
                *m.nonce.lock().unwrap()=q["nonce"].clone();
                let mut redirect = reqwest::Url::parse(&q["redirect_uri"]).unwrap();
                redirect.query_pairs_mut().append_pair("state", &q["state"]).append_pair("code", "mock-code");
                Redirect::temporary(redirect.as_str())
            }))
            .route("/token", post(|State(m): State<MockIssuer>, axum::Form(q): axum::Form<HashMap<String,String>>| async move {
                assert_eq!(q["code"], "mock-code");
                assert_eq!(Some(URL_SAFE_NO_PAD.encode(Sha256::digest(q["code_verifier"].as_bytes()))), m.challenge.lock().await.take());
                Json(json!({"id_token":m.token(CLIENT_ID, sticktalk::mock::now()/1000+3600, true)}))
            }))
            .route("/v2/{endpoint}", get(|axum::extract::Path(endpoint): axum::extract::Path<String>| async move {
                if endpoint == "categories" { return Json(json!({"tags":[{"name":"Happy","searchterm":"happy"}]})); }
                Json(json!({"results":[{"id":"123","title":"Wave","media_formats":{"nanowebm":{"url":"https://static.klipy.com/preview.webm","dims":[150,100]},"mediumgif":{"url":"https://static.klipy.com/send.gif","dims":[498,332]}}}],"next":"page2"}))
            }))
            .with_state(mock.clone());
        let upstream = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(Config {
            operations: sticktalk_server::OperationsConfig {
                min_free_disk_gb: 0,
                ..Default::default()
            },
            voice: Default::default(),
            browser: Default::default(),
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: dir.path().into(),
            public_url: "http://localhost".into(),
            max_upload_mb: 0,
            max_total_upload_mb: 0,
            worker_id: 0,
            trusted_proxy: "127.0.0.1".parse().unwrap(),
            google_client_id: Some(CLIENT_ID.into()),
            mock_google_jwks: Some(format!("{issuer}/jwks")),
            mock_google_issuer: Some(issuer.clone()),
            gifs: sticktalk_server::gifs::GifOptions {
                gif_provider: Some(sticktalk_server::gifs::ProviderOption::Klipy),
                gif_mode: sticktalk_server::gifs::GifMode::Proxy,
                gif_api_key: Some("mock-key".into()),
                gif_proxy_approved: true,
                mock_gif_base: Some(format!("{issuer}/v2")),
                ..Default::default()
            },
        })
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving_app = app.clone();
        let serving = tokio::spawn(async move { serving_app.serve(listener).await });
        Self {
            app,
            url,
            mock,
            _dir: dir,
            serving,
            upstream,
        }
    }
    async fn register(&self, name: &str) -> AuthResponse {
        password_auth(&self.url, name, PASSWORD, true)
            .await
            .unwrap()
    }
    async fn fresh_google_token(&self, http: &Http, operation: AccountOperation) -> String {
        let challenge: GoogleChallengeResponse = post_retry(
            http,
            "/auth/google/challenge",
            &GoogleChallengeRequest { operation },
        )
        .await
        .unwrap();
        *self.mock.nonce.lock().unwrap() = challenge.nonce;
        self.mock
            .token(CLIENT_ID, sticktalk::mock::now() / 1000 + 3600, true)
    }
    async fn proof(
        &self,
        http: &Http,
        operation: AccountOperation,
        google: bool,
    ) -> ReauthResponse {
        let request = ReauthRequest {
            operation,
            current_password: (!google).then(|| PASSWORD.into()),
            id_token: if google {
                Some(self.fresh_google_token(http, operation).await)
            } else {
                None
            },
        };
        post_retry(http, "/auth/reauth", &request).await.unwrap()
    }
    async fn google(&self) -> AuthResponse {
        // The opener is an HTTP script, never open::that/system browser.
        let challenge = Http::new(&self.url, "")
            .unwrap()
            .core()
            .google_challenge(AccountOperation::SignIn)
            .await
            .unwrap();
        let id_token = tokio::time::timeout(
            DEADLINE,
            sticktalk::oauth::flow(
                CLIENT_ID.into(),
                challenge.nonce,
                "mock-secret",
                format!("{}/authorize", self.mock.issuer),
                format!("{}/token", self.mock.issuer),
                None,
                |url| async move {
                    tokio::spawn(async move {
                        assert!(reqwest::get(url).await.unwrap().status().is_success());
                    });
                    Ok(())
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let http = Http::new(&self.url, "").unwrap();
        for token in [
            self.mock
                .token("wrong", sticktalk::mock::now() / 1000 + 3600, true),
            self.mock
                .token(CLIENT_ID, sticktalk::mock::now() / 1000 - 120, true),
            self.mock
                .token(CLIENT_ID, sticktalk::mock::now() / 1000 + 3600, false),
        ] {
            assert!(post_retry::<GoogleAuthResponse>(
                &http,
                "/auth/google",
                &GoogleAuthRequest { id_token: token }
            )
            .await
            .unwrap_err()
            .starts_with("Incorrect username or password."));
        }
        let result: GoogleAuthResponse = post_retry(
            &http,
            "/auth/google",
            &GoogleAuthRequest {
                id_token: id_token.clone(),
            },
        )
        .await
        .unwrap();
        let GoogleAuthResponse::SignupRequired(ticket) = result else {
            panic!("expected signup");
        };
        assert!(post_retry::<AuthResponse>(
            &http,
            "/auth/google/complete",
            &GoogleCompleteRequest {
                ticket: ticket.ticket.clone(),
                username: "alice".into()
            }
        )
        .await
        .is_err());
        let complete = GoogleCompleteRequest {
            ticket: ticket.ticket,
            username: "carol".into(),
        };
        let auth: AuthResponse = post_retry(&http, "/auth/google/complete", &complete)
            .await
            .unwrap();
        assert!(
            post_retry::<AuthResponse>(&http, "/auth/google/complete", &complete)
                .await
                .is_err()
        );
        // Sign-in nonces are single-use: a new login requires a fresh challenge.
        assert!(post_retry::<GoogleAuthResponse>(
            &http,
            "/auth/google",
            &GoogleAuthRequest { id_token }
        )
        .await
        .is_err());
        let id_token = self
            .fresh_google_token(&http, AccountOperation::SignIn)
            .await;
        let result: GoogleAuthResponse =
            post_retry(&http, "/auth/google", &GoogleAuthRequest { id_token })
                .await
                .unwrap();
        assert!(
            matches!(result, GoogleAuthResponse::Session(ref a) if a.user.user.id == auth.user.user.id)
        );
        auth
    }
}

/// Each actor runs the *production* subscription, bounded queues, reconnect logic,
/// event reducer and REST page merger, not an independently implemented WS peer.
struct Peer {
    http: Http,
    handle: Handle,
    model: Arc<Mutex<Model>>,
    changed: Arc<Notify>,
    data: mpsc::Receiver<Data>,
    pump: tokio::task::JoinHandle<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.pump.abort();
    }
}
impl Peer {
    async fn connect(url: &str, auth: &AuthResponse) -> Self {
        let config = backend::Config {
            server: url.into(),
            token: auth.token.clone(),
            mock: false,
            generation: 1,
            scenario: String::new(),
        };
        let model = Arc::new(Mutex::new(Model::default()));
        let changed = Arc::new(Notify::new());
        let (tx, mut data) = mpsc::channel(128);
        let projection = model.clone();
        let notify = changed.clone();
        let pump = tokio::spawn(async move {
            let stream = backend::subscription(&config);
            tokio::pin!(stream);
            let mut handle: Option<Handle> = None;
            while let Some(delivery) = stream.next().await {
                let data = (*delivery.data).clone();
                let mut model = projection.lock().await;
                match &data {
                    Data::Connected(h) => handle = Some(h.clone()),
                    Data::Frame(ServerMsg::Ready(r)) => model.ready(*r.clone()),
                    Data::Frame(ServerMsg::Event(e)) => {
                        model.apply(*e.clone());
                    }
                    Data::Frame(ServerMsg::Voice(v)) => {
                        model.voice_payload(v);
                    }
                    Data::Launch { data, .. } => {
                        assert!(model.snapshot_valid(data.cursor()));
                        model.launch.snapshot(data.clone());
                    }
                    Data::Frame(ServerMsg::Resumed(r)) => {
                        let cursor = model.cursor.as_mut().unwrap();
                        assert_eq!(cursor.stream_id, r.cursor.stream_id);
                        cursor.seq = cursor.seq.max(r.cursor.seq);
                    }
                    Data::History { channel, page, .. }
                    | Data::AroundHistory { channel, page, .. }
                    | Data::NewerHistory { channel, page, .. } => {
                        assert!(model.merge_page(*channel, page.clone()));
                    }
                    Data::Settings { snapshot, .. } => {
                        assert!(model.snapshot_valid(&snapshot.cursor));
                        model.settings.ingest(snapshot.data.clone());
                        model.reconcile_metadata(&snapshot.cursor);
                    }
                    _ => {}
                }
                if matches!(
                    data,
                    Data::Frame(ServerMsg::Ready(_) | ServerMsg::Event(_) | ServerMsg::Resumed(_))
                ) {
                    if let (Some(handle), Some(cursor)) = (&handle, &model.cursor) {
                        let _ = handle.try_send(Request::Applied(cursor.clone()));
                    }
                }
                drop(model);
                notify.notify_one();
                if !matches!(data, Data::Frame(ServerMsg::Event(_) | ServerMsg::Hello(_)))
                    && tx.send(data).await.is_err()
                {
                    break;
                }
            }
        });
        let handle = match tokio::time::timeout(DEADLINE, data.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Data::Connected(h) => h,
            d => panic!("{d:?}"),
        };
        let mut peer = Self {
            http: Http::new(url, &auth.token).unwrap(),
            handle,
            model,
            changed,
            data,
            pump,
        };
        peer.until(|d| matches!(d, Data::Frame(ServerMsg::Ready(_))))
            .await;
        peer
    }
    async fn until(&mut self, predicate: impl Fn(&Data) -> bool) -> Data {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let data = self.data.recv().await.expect("client stream stopped");
                if predicate(&data) {
                    return data;
                }
                assert!(
                    !matches!(&data, Data::Error(_) | Data::Rejected { .. }),
                    "{data:?}"
                );
            }
        })
        .await
        .expect("timed out waiting for client delivery")
    }
    async fn projection(&self, predicate: impl Fn(&Model) -> bool) {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let notified = self.changed.notified();
                if predicate(&*self.model.lock().await) {
                    return;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for reducer");
    }
    async fn outcome(&mut self, command: Command) -> CommandOutcome {
        let nonce = backend::nonce();
        self.handle
            .try_send(Request::Command(CommandRequest { nonce, command }))
            .unwrap();
        let Data::Frame(ServerMsg::Response(response)) = self
            .until(|d| matches!(d, Data::Frame(ServerMsg::Response(r)) if r.nonce == nonce))
            .await
        else {
            unreachable!()
        };
        response.outcome
    }
    async fn command(&mut self, command: Command) -> CommandResult {
        loop {
            match self.outcome(command.clone()).await {
                CommandOutcome::Ok(result) => return result,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::RateLimited,
                    retry_after_ms,
                    ..
                }) => {
                    tokio::time::sleep(Duration::from_millis(u64::from(
                        retry_after_ms.unwrap_or(1000),
                    )))
                    .await
                }
                e => panic!("{e:?} while executing {command:?}"),
            }
        }
    }
    async fn message(&mut self, channel: Id, content: impl Into<String>) -> Message {
        let CommandResult::Message(message) = self
            .command(Command::SendMessage(MessageDraft {
                channel_id: channel,
                content: content.into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }))
            .await
        else {
            panic!()
        };
        message
    }
    async fn history(&mut self, channel: Id, before: Option<Id>) -> Page<Message> {
        self.model.lock().await.touch(channel);
        self.handle
            .try_send(Request::History {
                seek: None,
                channel,
                before,
                generation: 1,
            })
            .unwrap();
        let Data::History { page, .. } = self.until(|d| matches!(d, Data::History { .. })).await
        else {
            panic!()
        };
        page
    }
}

/// Cut an established TCP stream without closing the listener or server session.
struct Cable {
    url: String,
    cut: watch::Sender<u64>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Cable {
    fn drop(&mut self) {
        self.task.abort();
        self.cut.send_modify(|n| *n += 1);
    }
}
impl Cable {
    async fn new(server: &str) -> Self {
        let upstream = server.trim_start_matches("http://").to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (cut, rx) = watch::channel(0);
        let task = tokio::spawn(async move {
            loop {
                let (mut downstream, _) = listener.accept().await.unwrap();
                let mut upstream = tokio::net::TcpStream::connect(&upstream).await.unwrap();
                let mut rx = rx.clone();
                rx.borrow_and_update();
                tokio::spawn(async move {
                    tokio::select! { _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream) => {}, _ = rx.changed() => {} }
                });
            }
        });
        Self { url, cut, task }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase_one_real_clients() {
    tokio::time::timeout(Duration::from_secs(300), phase_one())
        .await
        .expect("whole E2E deadline");
}
async fn phase_one() {
    eprintln!("E2E: auth and mock Google");
    let fixture = Fixture::start().await;
    let a = fixture.register("alice").await;
    let b = fixture.register("bob").await;
    let c = fixture.google().await;
    let alice_id = a.user.user.id;
    let bob_id = b.user.user.id;
    let carol_id = c.user.user.id;
    let cable = Cable::new(&fixture.url).await;
    let mut alice = Peer::connect(&cable.url, &a).await;
    let mut bob = Peer::connect(&fixture.url, &b).await;
    let mut carol = Peer::connect(&fixture.url, &c).await;
    assert_eq!(
        backend::meta(fixture.url.clone(), false)
            .await
            .unwrap()
            .google_client_id
            .as_deref(),
        Some(CLIENT_ID)
    );
    assert!(password_auth(&fixture.url, "alice", "bad password", false)
        .await
        .is_err());
    let logged_in = password_auth(&fixture.url, "alice", PASSWORD, false)
        .await
        .unwrap();
    let other_session = Http::new(&fixture.url, &logged_in.token).unwrap();
    other_session.logout().await;
    assert!(other_session
        .json::<Snapshot<SelfUser>>("/me")
        .await
        .is_err());

    eprintln!("E2E: relationships, presence and messaging");
    alice
        .command(Command::SendFriendRequest {
            username: "bob".into(),
        })
        .await;
    bob.projection(|m| m.relationships.contains_key(&alice_id))
        .await;
    bob.command(Command::AcceptFriendRequest { user_id: alice_id })
        .await;
    bob.command(Command::SetPresence {
        status: PresenceStatus::Idle,
    })
    .await;
    alice
        .projection(|m| m.presences.get(&bob_id) == Some(&PresenceStatus::Idle))
        .await;
    alice
        .command(Command::SendFriendRequest {
            username: "carol".into(),
        })
        .await;
    carol
        .command(Command::AcceptFriendRequest { user_id: alice_id })
        .await;
    let CommandResult::Channel(dm) = alice.command(Command::OpenDm { user_id: bob_id }).await
    else {
        panic!()
    };
    let dm = dm.summary.id;
    bob.history(dm, None).await;
    alice.history(dm, None).await;
    let message = alice
        .message(
            dm,
            "**bold** _italic_ `code`\n> quote\nhttps://example.invalid",
        )
        .await;
    bob.projection(|m| {
        m.messages
            .get(&dm)
            .is_some_and(|messages| messages.contains_key(&message.id))
    })
    .await;
    assert!(
        sticktalk::markdown::parse(&bob.model.lock().await.messages[&dm][&message.id].content)
            .blocks
            .len()
            >= 2
    );
    let CommandResult::Message(reply) = bob
        .command(Command::SendMessage(MessageDraft {
            channel_id: dm,
            content: format!("reply <@{}>", alice_id.0),
            reply_to: Some(message.id),
            attachment_ids: vec![],
            mention_ids: vec![alice_id],
        }))
        .await
    else {
        panic!()
    };
    alice
        .projection(|m| {
            m.reads
                .get(&dm)
                .is_some_and(|r| r.unread_count > 0 && r.mention_count == 1)
        })
        .await;
    assert_eq!(reply.reply_to, Some(message.id));
    bob.command(Command::EditMessage(MessageEdit {
        channel_id: dm,
        message_id: reply.id,
        content: "edited".into(),
        mention_ids: vec![],
    }))
    .await;
    alice
        .projection(|m| {
            m.messages[&dm]
                .get(&reply.id)
                .is_some_and(|m| m.content == "edited")
        })
        .await;
    bob.command(Command::AddReaction {
        channel_id: dm,
        message_id: message.id,
        emoji: "👍".into(),
    })
    .await;
    alice
        .projection(|m| !m.messages[&dm][&message.id].reactions.is_empty())
        .await;
    bob.command(Command::RemoveReaction {
        channel_id: dm,
        message_id: message.id,
        emoji: "👍".into(),
    })
    .await;
    alice
        .command(Command::MarkRead {
            channel_id: dm,
            message_id: reply.id,
        })
        .await;
    alice.projection(|m| m.reads[&dm].unread_count == 0).await;
    bob.command(Command::Typing { channel_id: dm }).await;
    alice
        .projection(|m| m.typing.contains_key(&(dm, bob_id)))
        .await;
    bob.command(Command::DeleteMessage {
        channel_id: dm,
        message_id: reply.id,
    })
    .await;
    alice
        .projection(|m| !m.messages[&dm].contains_key(&reply.id))
        .await;

    let CommandResult::Channel(group) = alice
        .command(Command::CreateGroupDm {
            name: "Group".into(),
            user_ids: vec![bob_id, carol_id],
        })
        .await
    else {
        panic!()
    };
    let group = group.summary.id;
    carol.projection(|m| m.channels.contains_key(&group)).await;
    carol.history(group, None).await;
    let group_message = bob.message(group, "three-way group message").await;
    carol
        .projection(|m| m.messages[&group].contains_key(&group_message.id))
        .await;
    alice
        .command(Command::RemoveGroupRecipient {
            channel_id: group,
            user_id: carol_id,
        })
        .await;
    carol.projection(|m| !m.channels.contains_key(&group)).await;
    alice
        .command(Command::AddGroupRecipient {
            channel_id: group,
            user_id: carol_id,
        })
        .await;
    alice
        .command(Command::TransferGroupOwnership {
            channel_id: group,
            user_id: bob_id,
        })
        .await;
    carol
        .command(Command::LeaveGroupDm { channel_id: group })
        .await;
    alice.command(Command::CloseDm { channel_id: dm }).await;
    alice.command(Command::OpenDm { user_id: bob_id }).await;

    let CommandResult::Guild(guild) = alice
        .command(Command::CreateGuild {
            name: "E2E server".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let guild = guild.summary.guild.id;
    let CommandResult::Invite(invite) = alice
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 3600,
            max_uses: 10,
        })
        .await
    else {
        panic!()
    };
    bob.command(Command::JoinGuild {
        invite_code: invite.code.clone(),
    })
    .await;
    carol
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    let CommandResult::Channel(category) = alice
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Category,
            name: "Projects".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Channel(channel) = alice
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Text,
            name: "testing".into(),
            parent_id: Some(category.summary.id),
            topic: Some("Live".into()),
        })
        .await
    else {
        panic!()
    };
    let channel = channel.summary.id;
    bob.projection(|m| m.channels.contains_key(&channel)).await;
    alice
        .command(Command::UpdateChannel {
            channel_id: channel,
            patch: ChannelPatch {
                name: Patch::Set("tested".into()),
                topic: Patch::Set("Updated".into()),
            },
        })
        .await;
    bob.projection(|m| m.channels[&channel].name.as_deref() == Some("tested"))
        .await;
    let members: Page<Member> = alice
        .http
        .json(&format!("/guilds/{}/members", guild.0))
        .await
        .unwrap();
    assert_eq!(members.items.len(), 3);

    eprintln!("E2E: uploads");
    uploads(&fixture, &mut alice, &bob, channel).await;
    eprintln!("E2E: pagination and bounded regions");
    history_regions(&mut alice, channel).await;
    eprintln!("E2E: GIFs");
    gifs(&mut alice, &bob).await;
    let methods: AuthMethodsResponse = carol.http.json("/auth/methods").await.unwrap();
    assert!(!methods.has_password && methods.google_linked);
    let proof = fixture
        .proof(&carol.http, AccountOperation::Unlink, true)
        .await;
    assert!(
        account_retry(&carol.http, backend::AccountAction::Unlink, &proof.proof)
            .await
            .err()
            .unwrap()
            .contains("only sign-in method")
    );
    let proof = fixture
        .proof(&alice.http, AccountOperation::Link, false)
        .await;
    let token = fixture
        .fresh_google_token(&alice.http, AccountOperation::Link)
        .await;
    assert!(account_retry(
        &alice.http,
        backend::AccountAction::Link(token),
        &proof.proof
    )
    .await
    .err()
    .unwrap()
    .starts_with("That Google account is linked to another"));
    let proof = fixture
        .proof(&carol.http, AccountOperation::Password, true)
        .await;
    let changed = account_retry(
        &carol.http,
        backend::AccountAction::Password {
            current: None,
            new: PASSWORD.into(),
        },
        &proof.proof,
    )
    .await
    .unwrap();
    let rotated = changed
        .session
        .expect("password creation rotates the login");
    drop(carol);
    let mut carol = Peer::connect(&fixture.url, &rotated).await;
    let proof = fixture
        .proof(&carol.http, AccountOperation::Unlink, false)
        .await;
    let methods = account_retry(&carol.http, backend::AccountAction::Unlink, &proof.proof)
        .await
        .unwrap()
        .methods;
    assert!(methods.has_password && !methods.google_linked);
    let proof = fixture
        .proof(&carol.http, AccountOperation::Link, false)
        .await;
    let token = fixture
        .fresh_google_token(&carol.http, AccountOperation::Link)
        .await;
    let methods = account_retry(
        &carol.http,
        backend::AccountAction::Link(token),
        &proof.proof,
    )
    .await
    .unwrap()
    .methods;
    assert!(methods.google_linked);

    // Server-side socket disappearance: receive the real client's Backoff, send
    // during disconnection, then verify production resume/replay (not fresh Ready).
    eprintln!("E2E: reconnect/resume and moderation");
    alice.history(dm, None).await;
    let cursor = alice.model.lock().await.cursor.clone().unwrap();
    cable.cut.send_modify(|n| *n += 1);
    alice
        .until(|d| {
            matches!(
                d,
                Data::State(sticktalk::model::ConnectionState::Backoff(_))
            )
        })
        .await;
    eprintln!("E2E: socket cut observed");
    let offline = bob.message(dm, "sent while Alice's socket was dead").await;
    let resumed = alice
        .until(|d| matches!(d, Data::Frame(ServerMsg::Resumed(_))))
        .await;
    assert!(
        matches!(resumed, Data::Frame(ServerMsg::Resumed(ref r)) if r.cursor.stream_id == cursor.stream_id && r.replayed > 0)
    );
    alice
        .projection(|m| m.messages[&dm].contains_key(&offline.id))
        .await;
    eprintln!("E2E: replay received");

    alice
        .command(Command::KickMember {
            guild_id: guild,
            user_id: carol_id,
        })
        .await;
    carol
        .projection(|m| !m.guilds.contains_key(&guild) && !m.channels.contains_key(&channel))
        .await;
    carol
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    alice
        .command(Command::BanMember {
            guild_id: guild,
            user_id: carol_id,
            reason: Some("test".into()),
        })
        .await;
    carol.projection(|m| !m.guilds.contains_key(&guild)).await;
    assert!(matches!(
        carol
            .outcome(Command::JoinGuild {
                invite_code: invite.code.clone()
            })
            .await,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Banned,
            ..
        })
    ));
    assert!(carol
        .http
        .json::<Page<Message>>(&format!(
            "/channels/{}/messages?around={}",
            channel.0, message.id.0
        ))
        .await
        .is_err());
    alice
        .command(Command::UnbanMember {
            guild_id: guild,
            user_id: carol_id,
        })
        .await;
    alice
        .command(Command::RevokeInvite {
            guild_id: guild,
            code: invite.code,
        })
        .await;
    alice
        .command(Command::DeleteChannel {
            channel_id: channel,
        })
        .await;
    bob.projection(|m| !m.channels.contains_key(&channel)).await;
    bob.http.logout().await;
    bob.until(|d| {
        matches!(
            d,
            Data::State(sticktalk::model::ConnectionState::SessionRevoked)
        )
    })
    .await;
    assert!(bob.http.json::<Snapshot<SelfUser>>("/me").await.is_err());
}

async fn history_regions(alice: &mut Peer, channel: Id) {
    let mut ids = Vec::new();
    for n in 0..64 {
        // Respect the server's real send rate, don't disable production limits.
        let command = Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: format!("history {n}"),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        });
        loop {
            match alice.outcome(command.clone()).await {
                CommandOutcome::Ok(CommandResult::Message(m)) => {
                    ids.push(m);
                    break;
                }
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::RateLimited,
                    retry_after_ms,
                    ..
                }) => {
                    tokio::time::sleep(Duration::from_millis(u64::from(
                        retry_after_ms.unwrap_or(1000),
                    )))
                    .await
                }
                result => panic!("{result:?}"),
            }
        }
    }
    let newest = alice.history(channel, None).await;
    assert_eq!(newest.items.len(), 50);
    assert!(newest.has_more);
    let older = alice.history(channel, newest.next_before).await;
    assert!(older
        .items
        .iter()
        .all(|m| m.id < newest.items.last().unwrap().id));
    let target = ids[32].id;
    alice.model.lock().await.messages.remove(&channel);
    alice
        .handle
        .try_send(Request::AroundHistory {
            channel,
            anchor: HistoryAnchor::Message(target),
            seek: 1,
            generation: 1,
        })
        .unwrap();
    let Data::AroundHistory { page, .. } = alice
        .until(|d| matches!(d, Data::AroundHistory { .. }))
        .await
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 50);
    assert!(page.items.iter().any(|m| m.id == target));
    assert!(page.next_before.is_some() && page.next_after.is_some());
    assert_eq!(alice.model.lock().await.messages[&channel].len(), 50);
    alice
        .handle
        .try_send(Request::NewerHistory {
            seek: 1,
            channel,
            after: page.next_after.unwrap(),
            generation: 1,
        })
        .unwrap();
    let Data::NewerHistory { page: newer, .. } = alice
        .until(|d| matches!(d, Data::NewerHistory { .. }))
        .await
    else {
        panic!()
    };
    assert!(newer.items.iter().all(|m| m.id > page.items[0].id));
    alice
        .handle
        .try_send(Request::AroundHistory {
            channel,
            anchor: HistoryAnchor::Timestamp(ids[32].created_at),
            seek: 2,
            generation: 1,
        })
        .unwrap();
    let Data::AroundHistory { page: by_time, .. } = alice
        .until(|d| matches!(d, Data::AroundHistory { .. }))
        .await
    else {
        panic!()
    };
    assert_eq!(by_time.items.len(), 50);
    let after: Page<Message> = alice
        .http
        .json(&format!(
            "/channels/{}/messages?after={}&limit=5",
            channel.0,
            page.next_after.unwrap().0
        ))
        .await
        .unwrap();
    assert!(after.items.iter().all(|m| m.id > page.items[0].id));
    for query in [
        "around=0",
        "around=1&before=2",
        "around=1&around_timestamp=1735689600000",
        "around_timestamp=0",
        "around_timestamp=18446744073709551615",
        "around=1&limit=51",
    ] {
        assert!(
            alice
                .http
                .json::<Page<Message>>(&format!("/channels/{}/messages?{query}", channel.0))
                .await
                .is_err(),
            "{query}"
        );
    }
    let empty: Page<Message> = alice
        .http
        .json(&format!(
            "/channels/{}/messages?around=1&limit=1",
            channel.0
        ))
        .await
        .unwrap();
    assert_eq!(empty.items.len(), 1);
}

async fn uploads(fixture: &Fixture, alice: &mut Peer, bob: &Peer, channel: Id) {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("photo.jpg");
    let bytes = sticktalk_client_core::mock::IMAGE;
    tokio::fs::write(&image_path, bytes).await.unwrap();
    let legacy = alice
        .http
        .upload_legacy(&image_path, UploadPurpose::Message, Some(channel))
        .await
        .unwrap();
    assert!(legacy.attachment.thumbnail.is_some());
    // The production Request path now uses resumable uploads too.
    alice
        .handle
        .try_send(Request::Upload {
            form_revision: 0,
            path: image_path.clone(),
            purpose: UploadPurpose::Message,
            channel: Some(channel),
            generation: 1,
        })
        .unwrap();
    let Data::Uploaded {
        response: resumed, ..
    } = alice.until(|d| matches!(d, Data::Uploaded { .. })).await
    else {
        panic!()
    };
    assert!(resumed.attachment.thumbnail.is_some());
    let CommandResult::Message(sent) = alice
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "uploads".into(),
            reply_to: None,
            attachment_ids: vec![legacy.attachment.id, resumed.attachment.id],
            mention_ids: vec![],
        }))
        .await
    else {
        panic!()
    };
    assert_eq!(sent.attachments.len(), 2);
    for attachment in &sent.attachments {
        let response = bob
            .http
            .get(&format!("/files/{}/thumbnail", attachment.id.0))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert!(!response.bytes().await.unwrap().is_empty());
        let response = bob
            .http
            .get(&format!("/files/{}", attachment.id.0))
            .header("Range", "bytes=2-8")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap().as_ref(), &bytes[2..9]);
    }
    // Persisted offset survives a new Http client (simulated client restart).
    let large = dir.path().join("large.bin");
    let mut file = tokio::fs::File::create(&large).await.unwrap();
    use tokio::io::AsyncWriteExt;
    let chunk = vec![0x5a; 1024 * 1024];
    let mut hash = Sha256::new();
    for _ in 0..26 {
        file.write_all(&chunk).await.unwrap();
        hash.update(&chunk);
    }
    file.flush().await.unwrap();
    drop(file);
    let upload: ResumableUpload = post_retry(
        &alice.http,
        "/uploads/resumable",
        &CreateUploadRequest {
            purpose: UploadPurpose::Message,
            channel_id: Some(channel),
            filename: "large.bin".into(),
            content_type: "application/octet-stream".into(),
            byte_size: 26 * 1024 * 1024,
            sha256: format!("{:x}", hash.finalize()),
        },
    )
    .await
    .unwrap();
    let route = format!(
        "{}/api/v1/uploads/resumable/{}",
        fixture.url, upload.upload_id
    );
    let auth = alice.http.get("/me").build().unwrap().headers()["authorization"].clone();
    let partial = alice
        .http
        .client
        .put(&route)
        .header("authorization", auth.clone())
        .header("Upload-Offset", 0)
        .body(chunk.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(partial.status(), 200);
    let status: ResumableUpload = partial.json().await.unwrap();
    assert_eq!(status.offset, 1024 * 1024);
    let bad = alice
        .http
        .client
        .put(&route)
        .header("authorization", auth)
        .header("Upload-Offset", 0)
        .body(chunk)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 409);
    let uploaded = alice
        .http
        .resume_upload(&large, upload.clone())
        .await
        .unwrap();
    let retry = alice.http.resume_upload(&large, upload).await.unwrap();
    assert_eq!(uploaded, retry);
    alice
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "large".into(),
            reply_to: None,
            attachment_ids: vec![uploaded.attachment.id],
            mention_ids: vec![],
        }))
        .await;
    let downloaded = dir.path().join("download.bin");
    bob.http
        .download(uploaded.attachment.id, downloaded.clone())
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::metadata(downloaded).await.unwrap().len(),
        26 * 1024 * 1024
    );
}

async fn gifs(alice: &mut Peer, bob: &Peer) {
    let trending: GifPage = alice.http.json("/gifs/trending").await.unwrap();
    assert_eq!(trending.items[0].id, "klipy:123");
    assert_eq!(trending.next_cursor.as_deref(), Some("page2"));
    let search: GifPage = alice
        .http
        .json("/gifs/search?q=wave&cursor=page2")
        .await
        .unwrap();
    assert_eq!(trending, search);
    let categories: GifCategoriesResponse = alice.http.json("/gifs/categories").await.unwrap();
    assert_eq!(categories.items[0].title, "Happy");
    let items: GifPage = alice
        .http
        .json("/gifs/items?ids=klipy%3A123")
        .await
        .unwrap();
    assert_eq!(items.items[0].id, "klipy:123");
    // Same authenticated client request builder, no user bearer reaches the provider.
    let url = format!("{}/api/v1/gifs/favorites/klipy:123", alice.http.base);
    let auth = alice.http.get("/me").build().unwrap().headers()["authorization"].clone();
    assert_eq!(
        alice
            .http
            .client
            .put(&url)
            .header("authorization", auth.clone())
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let favorites: GifFavoritesResponse = alice.http.json("/gifs/favorites").await.unwrap();
    assert_eq!(favorites.ids, ["klipy:123"]);
    assert!(bob
        .http
        .json::<GifFavoritesResponse>("/gifs/favorites")
        .await
        .unwrap()
        .ids
        .is_empty());
    assert_eq!(
        alice
            .http
            .client
            .delete(&url)
            .header("authorization", auth)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert!(alice
        .http
        .json::<GifFavoritesResponse>("/gifs/favorites")
        .await
        .unwrap()
        .ids
        .is_empty());
    let _: Value = alice.http.json("/meta").await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cut_socket_replays_into_the_real_client_reducer() {
    let fixture = Fixture::start().await;
    let a = fixture.register("alice").await;
    let b = fixture.register("bob").await;
    let cable = Cable::new(&fixture.url).await;
    let mut alice = Peer::connect(&cable.url, &a).await;
    let mut bob = Peer::connect(&fixture.url, &b).await;
    alice
        .command(Command::SendFriendRequest {
            username: "bob".into(),
        })
        .await;
    bob.command(Command::AcceptFriendRequest {
        user_id: a.user.user.id,
    })
    .await;
    let CommandResult::Channel(dm) = alice
        .command(Command::OpenDm {
            user_id: b.user.user.id,
        })
        .await
    else {
        panic!()
    };
    let dm = dm.summary.id;
    alice.history(dm, None).await;
    cable.cut.send_modify(|n| *n += 1);
    alice
        .until(|d| {
            matches!(
                d,
                Data::State(sticktalk::model::ConnectionState::Backoff(_))
            )
        })
        .await;
    let missed = bob.message(dm, "replay me").await;
    alice
        .until(|d| matches!(d, Data::Frame(ServerMsg::Resumed(_))))
        .await;
    alice
        .projection(|m| m.messages[&dm].contains_key(&missed.id))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn launch_main_window_real_clients() {
    tokio::time::timeout(Duration::from_secs(180), async {
        use sticktalk_client_core::launch::{Read, SnapshotData};
        let fixture = Fixture::start().await;
        let a = fixture.register("parity_owner").await;
        let b = fixture.register("parity_member").await;
        let mut owner = Peer::connect(&fixture.url, &a).await;
        let mut member = Peer::connect(&fixture.url, &b).await;
        let CommandResult::Guild(g) = owner
            .command(Command::CreateGuild {
                name: "Parity server".into(),
                icon_id: None,
            })
            .await
        else {
            panic!()
        };
        let guild = g.summary.guild.id;
        let CommandResult::Invite(invite) = owner
            .command(Command::CreateInvite {
                guild_id: guild,
                ttl_seconds: 3600,
                max_uses: 5,
            })
            .await
        else {
            panic!()
        };
        member
            .command(Command::JoinGuild {
                invite_code: invite.code,
            })
            .await;
        let CommandResult::Role(mut role) = owner
            .command(Command::Launch(LaunchCommand::CreateRole {
                guild_id: guild,
                name: "Moderators".into(),
            }))
            .await
        else {
            panic!()
        };
        role.color = 0x79b8ff;
        role.hoist = true;
        role.permissions = Permissions::MANAGE_MESSAGES;
        owner
            .command(Command::Launch(LaunchCommand::UpdateRole {
                role: role.clone(),
            }))
            .await;
        owner
            .command(Command::Launch(LaunchCommand::SetMemberRoles {
                guild_id: guild,
                user_id: b.user.user.id,
                role_ids: vec![role.id],
            }))
            .await;
        owner
            .command(Command::Launch(LaunchCommand::SetNickname {
                guild_id: guild,
                user_id: b.user.user.id,
                nickname: Some("Blue Nickname".into()),
            }))
            .await;
        member
            .projection(|m| {
                m.launch.name(Some(guild), b.user.user.id) == Some("Blue Nickname")
                    && m.launch.color(Some(guild), b.user.user.id) == Some(0x79b8ff)
            })
            .await;
        {
            let model = member.model.lock().await;
            assert_eq!(
                model.launch.hoist(guild, b.user.user.id).unwrap().name,
                "Moderators"
            );
        }
        let text = g
            .channels
            .iter()
            .find(|c| c.kind == ChannelKind::Text)
            .unwrap()
            .id;
        let message = owner
            .message(text, "A pinned message from the real server")
            .await;
        member
            .command(Command::Launch(LaunchCommand::PinMessage {
                channel_id: text,
                message_id: message.id,
                pinned: true,
            }))
            .await;
        owner
            .handle
            .try_send(Request::Launch {
                read: Read::Pins(text),
                generation: 1,
            })
            .unwrap();
        let Data::Launch {
            data: SnapshotData::Pins(_, pins),
            ..
        } = owner
            .until(|d| {
                matches!(
                    d,
                    Data::Launch {
                        data: SnapshotData::Pins(..),
                        ..
                    }
                )
            })
            .await
        else {
            panic!()
        };
        assert_eq!(pins.data[0].id, message.id);
        owner
            .handle
            .try_send(Request::Launch {
                read: Read::Profile(b.user.user.id),
                generation: 1,
            })
            .unwrap();
        let Data::Launch {
            data: SnapshotData::Profile(profile),
            ..
        } = owner
            .until(|d| {
                matches!(
                    d,
                    Data::Launch {
                        data: SnapshotData::Profile(..),
                        ..
                    }
                )
            })
            .await
        else {
            panic!()
        };
        assert!(profile
            .data
            .mutual_servers
            .iter()
            .any(|s| s.guild.id == guild));
        owner
            .handle
            .try_send(Request::Launch {
                read: Read::Extent(text),
                generation: 1,
            })
            .unwrap();
        let Data::Launch {
            data: SnapshotData::Extent(extent),
            ..
        } = owner
            .until(|d| {
                matches!(
                    d,
                    Data::Launch {
                        data: SnapshotData::Extent(..),
                        ..
                    }
                )
            })
            .await
        else {
            panic!()
        };
        assert_eq!(extent.data.message_count, 1);
        assert_eq!(extent.data.anchors[0].message_id, message.id);
        let CommandResult::Channel(private) = owner
            .command(Command::CreateChannel {
                guild_id: guild,
                parent_id: None,
                kind: ChannelKind::Text,
                name: "private".into(),
                topic: None,
            })
            .await
        else {
            panic!()
        };
        owner
            .command(Command::Launch(LaunchCommand::SetOverwrites {
                channel_id: private.summary.id,
                overwrites: vec![PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: guild,
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                }],
            }))
            .await;
        member
            .projection(|m| !m.channels.contains_key(&private.summary.id))
            .await;
        owner
            .handle
            .try_send(Request::Launch {
                read: Read::Overwrites(private.summary.id),
                generation: 1,
            })
            .unwrap();
        owner
            .until(|d| {
                matches!(
                    d,
                    Data::Launch {
                        data: SnapshotData::Overwrites(..),
                        ..
                    }
                )
            })
            .await;
        {
            let model = owner.model.lock().await;
            assert!(model.launch.is_private(&private.summary));
        }
        let CommandResult::Channel(voice) = owner
            .command(Command::CreateChannel {
                guild_id: guild,
                parent_id: None,
                kind: ChannelKind::Voice,
                name: "Lounge".into(),
                topic: None,
            })
            .await
        else {
            panic!()
        };
        owner
            .command(Command::SubscribeGuild {
                guild_id: Some(guild),
                presence_user_ids: vec![b.user.user.id],
            })
            .await;
        member
            .command(Command::SubscribeGuild {
                guild_id: Some(guild),
                presence_user_ids: vec![a.user.user.id],
            })
            .await;
        let signal = |s| {
            Request::Voice(VoicePayload {
                nonce: backend::nonce(),
                version: VOICE_VERSION,
                kind: "signal".into(),
                payload: rmp_serde::to_vec_named(&s).unwrap(),
            })
        };
        member
            .handle
            .try_send(signal(VoiceSignal::Join {
                channel_id: voice.summary.id,
                capabilities: VoiceCapabilities {
                    video: vec![VideoCodec::H264],
                    hardware_decode: vec![],
                },
            }))
            .unwrap();
        owner
            .projection(|m| {
                m.launch.voice.values().any(|v| {
                    v.channel_id == voice.summary.id
                        && v.participant.session.user_id == b.user.user.id
                })
            })
            .await;
        member
            .handle
            .try_send(signal(VoiceSignal::State {
                muted: true,
                deafened: true,
            }))
            .unwrap();
        owner
            .projection(|m| {
                m.launch.voice.values().any(|v| {
                    v.participant.session.user_id == b.user.user.id
                        && v.participant.muted
                        && v.participant.deafened
                })
            })
            .await;
        member.handle.try_send(signal(VoiceSignal::Leave)).unwrap();
        owner
            .projection(|m| {
                m.launch
                    .voice
                    .values()
                    .all(|v| v.participant.session.user_id != b.user.user.id)
            })
            .await;
        member
            .command(Command::Launch(LaunchCommand::PinMessage {
                channel_id: text,
                message_id: message.id,
                pinned: false,
            }))
            .await;
        owner
            .projection(|m| m.launch.pins.get(&text).is_none_or(|p| p.is_empty()))
            .await;
        let n = NotificationSettings {
            guild_id: Some(guild),
            channel_id: Some(text),
            level: NotificationLevel::Mentions,
            muted: true,
            mute_until: None,
            suppress_everyone: true,
            hide_muted_channels: false,
        };
        member
            .command(Command::Launch(LaunchCommand::SetNotifications(n)))
            .await;
        member
            .projection(|m| m.launch.muted(Some(guild), text, sticktalk::mock::now()))
            .await;
    })
    .await
    .expect("main window live deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_upload_worker_checkpoints_completion_and_download_replaces_atomically() {
    let fixture = Fixture::start().await;
    let a = fixture.register("queue_alice").await;
    let b = fixture.register("queue_bob").await;
    let mut alice = Peer::connect(&fixture.url, &a).await;
    let mut bob = Peer::connect(&fixture.url, &b).await;
    alice
        .command(Command::SendFriendRequest {
            username: "queue_bob".into(),
        })
        .await;
    bob.command(Command::AcceptFriendRequest {
        user_id: a.user.user.id,
    })
    .await;
    let CommandResult::Channel(channel) = alice
        .command(Command::OpenDm {
            user_id: b.user.user.id,
        })
        .await
    else {
        panic!()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.jpg");
    tokio::fs::write(&path, sticktalk_client_core::mock::IMAGE)
        .await
        .unwrap();
    let (size, modified_ms) = sticktalk::uploads::fingerprint(&path).unwrap();
    let config = sticktalk::backend::Config {
        server: fixture.url.clone(),
        token: a.token.clone(),
        mock: false,
        generation: 1,
        scenario: String::new(),
    };
    let id = sticktalk::backend::nonce();
    let account = a.user.user.id;
    let job = sticktalk::uploads::Job {
        id,
        path,
        channel: channel.summary.id,
        forward: false,
        size,
        modified_ms,
        upload: None,
        attachment: None,
    };
    sticktalk::uploads::save(&config, account, &job).unwrap();
    let worker = sticktalk::uploads::Worker {
        config: config.clone(),
        account,
        id,
        journal: serde_json::to_string(&job).unwrap(),
    };
    let mut stream = Box::pin(sticktalk::uploads::subscription(&worker));
    let attachment = loop {
        let event = tokio::time::timeout(Duration::from_secs(15), stream.next())
            .await
            .unwrap()
            .expect("worker ended without completion");
        match event {
            sticktalk::uploads::Event::Complete(_, attachment) => break attachment,
            sticktalk::uploads::Event::Failed(_, e) => panic!("{e}"),
            _ => {}
        }
    };
    drop(stream);
    let restored = sticktalk::uploads::restore(&config, account)
        .into_iter()
        .find(|j| j.id == id)
        .unwrap();
    assert_eq!(restored.attachment.as_ref().unwrap().id, attachment.id);
    assert!(!std::fs::read_to_string(
        sticktalk::uploads::preferences(&config, account)
            .parent()
            .unwrap()
            .join(format!("{id}.json"))
    )
    .unwrap()
    .contains(&a.token));
    // Bind the pending upload before downloading it, just like the composer.
    alice
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel.summary.id,
            content: String::new(),
            reply_to: None,
            attachment_ids: vec![attachment.id],
            mention_ids: vec![],
        }))
        .await;
    let destination = dir.path().join("saved.jpg");
    tokio::fs::write(&destination, b"old destination")
        .await
        .unwrap();
    alice
        .http
        .download(attachment.id, destination.clone())
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read(&destination).await.unwrap(),
        sticktalk_client_core::mock::IMAGE
    );
    sticktalk::uploads::remove(&config, account, id);
    let journal_dir = sticktalk::uploads::preferences(&config, account)
        .parent()
        .unwrap()
        .to_owned();
    let _ = std::fs::remove_file(journal_dir.join(format!("{id}.cancelled")));
    let _ = std::fs::remove_dir(journal_dir);
}

/// Launch settings exercise the production socket, REST adapter, SQLite authorization,
/// and shared projection. Every listener is loopback and no OS UI is constructed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_roles_overwrites_profiles_devices_and_notification_sync() {
    use sticktalk_client_core::settings::{Query as SettingsQuery, SnapshotData};
    let fixture = Fixture::start().await;
    let owner_auth = fixture.register("settings_owner").await;
    let member_auth = fixture.register("settings_member").await;
    let outsider_auth = fixture.register("settings_observer").await;
    let mut owner = Peer::connect(&fixture.url, &owner_auth).await;
    let mut member = Peer::connect(&fixture.url, &member_auth).await;
    let mut observer = Peer::connect(&fixture.url, &outsider_auth).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Settings E2E".into(),
            icon_id: None,
        })
        .await
    else {
        panic!("guild")
    };
    let gid = detail.summary.guild.id;
    let CommandResult::Channel(channel) = owner
        .command(Command::CreateChannel {
            guild_id: gid,
            kind: ChannelKind::Text,
            name: "private-lab".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!("channel")
    };
    let channel_id = channel.summary.id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: gid,
            ttl_seconds: 3600,
            max_uses: 10,
        })
        .await
    else {
        panic!("invite")
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    observer
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    let CommandResult::Role(mut role) = owner
        .command(Command::Launch(LaunchCommand::CreateRole {
            guild_id: gid,
            name: "Designers".into(),
        }))
        .await
    else {
        panic!("role")
    };
    let CommandResult::Role(other) = owner
        .command(Command::Launch(LaunchCommand::CreateRole {
            guild_id: gid,
            name: "Moderators".into(),
        }))
        .await
    else {
        panic!("role")
    };
    role.name = "Product Designers".into();
    role.color = 0x3ddc97;
    role.hoist = true;
    role.mentionable = true;
    role.permissions = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
    // Creation shifts role positions; fetch current replacement before editing.
    let roles: Snapshot<Vec<Role>> = owner
        .http
        .json(&format!("/guilds/{}/roles", gid.0))
        .await
        .unwrap();
    role.position = roles
        .data
        .iter()
        .find(|r| r.id == role.id)
        .unwrap()
        .position;
    owner
        .command(Command::Launch(LaunchCommand::UpdateRole {
            role: role.clone(),
        }))
        .await;
    let mut expected_roles: Vec<_> = roles
        .data
        .iter()
        .filter(|r| r.id != role.id && r.id != other.id)
        .collect();
    expected_roles.sort_by_key(|r| r.position);
    let mut expected_roles: Vec<_> = expected_roles.into_iter().map(|r| r.id).collect();
    expected_roles.insert(1, role.id);
    expected_roles.insert(2, other.id);
    owner
        .command(Command::Launch(LaunchCommand::ReorderRoles {
            guild_id: gid,
            role_ids: expected_roles.clone(),
        }))
        .await;
    owner
        .command(Command::Launch(LaunchCommand::SetMemberRoles {
            guild_id: gid,
            user_id: member_auth.user.user.id,
            role_ids: vec![role.id],
        }))
        .await;
    owner
        .projection(|m| {
            m.settings
                .roles
                .get(&role.id)
                .is_some_and(|r| r.name == "Product Designers" && r.position == 1)
        })
        .await;
    let roles: Snapshot<Vec<Role>> = owner
        .http
        .json(&format!("/guilds/{}/roles", gid.0))
        .await
        .unwrap();
    let mut ordered = roles.data;
    ordered.sort_by_key(|r| r.position);
    assert_eq!(
        ordered.iter().map(|r| r.id).collect::<Vec<_>>(),
        expected_roles
    );
    let members: Page<Member> = owner
        .http
        .json(&format!("/guilds/{}/members", gid.0))
        .await
        .unwrap();
    assert!(members
        .items
        .iter()
        .find(|m| m.user.id == member_auth.user.user.id)
        .unwrap()
        .role_ids
        .contains(&role.id));
    owner
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id,
            overwrites: vec![
                PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: gid,
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                },
                PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: role.id,
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                },
            ],
        }))
        .await;
    let overwrites: Snapshot<Vec<PermissionOverwrite>> = owner
        .http
        .json(&format!("/channels/{}/overwrites", channel_id.0))
        .await
        .unwrap();
    assert_eq!(overwrites.data.len(), 2);
    assert!(observer
        .http
        .json::<Snapshot<Channel>>(&format!("/channels/{}", channel_id.0))
        .await
        .is_err());
    assert!(member
        .http
        .json::<Snapshot<Channel>>(&format!("/channels/{}", channel_id.0))
        .await
        .is_ok());
    owner
        .command(Command::UpdateProfile(ProfilePatch {
            display_name: Patch::Set("Settings Owner".into()),
            ..Default::default()
        }))
        .await;
    owner
        .command(Command::Launch(LaunchCommand::UpdateUserProfile {
            banner_color: 0x9878d5,
            pronouns: "they / them".into(),
            about_me: "Testing the real settings round trip.".into(),
        }))
        .await;
    let profile: Snapshot<ProfileDetail> = owner
        .http
        .json(&format!("/users/{}/profile", owner_auth.user.user.id.0))
        .await
        .unwrap();
    assert_eq!(
        profile.data.profile.user.display_name.as_deref(),
        Some("Settings Owner")
    );
    assert_eq!(profile.data.profile.banner_color, 0x9878d5);
    assert_eq!(profile.data.profile.pronouns, "they / them");
    owner
        .command(Command::Launch(LaunchCommand::UpdateGuildProfile {
            guild_id: gid,
            description: "A real server profile".into(),
            banner_color: 8,
        }))
        .await;
    let preview: Snapshot<GuildPreview> = owner
        .http
        .json(&format!("/guilds/{}/preview", gid.0))
        .await
        .unwrap();
    assert_eq!(preview.data.profile.banner_color, 8);
    let second_auth = password_auth(&fixture.url, "settings_owner", PASSWORD, false)
        .await
        .unwrap();
    let second = Peer::connect(&fixture.url, &second_auth).await;
    let notification = NotificationSettings {
        guild_id: Some(gid),
        channel_id: None,
        level: NotificationLevel::Mentions,
        muted: true,
        mute_until: Some(sticktalk::mock::now() + 3_600_000),
        suppress_everyone: true,
        hide_muted_channels: true,
    };
    owner
        .command(Command::Launch(LaunchCommand::SetNotifications(
            notification.clone(),
        )))
        .await;
    second
        .projection(|m| m.settings.notifications.get(&(Some(gid), None)) == Some(&notification))
        .await;
    let notifications: Snapshot<Vec<NotificationSettings>> =
        second.http.json("/me/notifications").await.unwrap();
    assert_eq!(notifications.data, vec![notification]);
    owner
        .handle
        .try_send(Request::Settings {
            query: SettingsQuery::Sessions,
            generation: 1,
        })
        .unwrap();
    let Data::Settings { snapshot, .. } = owner
        .until(|d| {
            matches!(
                d,
                Data::Settings {
                    snapshot: Snapshot {
                        data: SnapshotData::Sessions(_),
                        ..
                    },
                    ..
                }
            )
        })
        .await
    else {
        panic!("sessions")
    };
    let SnapshotData::Sessions(sessions) = snapshot.data else {
        unreachable!()
    };
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions.iter().filter(|s| s.current).count(), 1);
    let other_session = sessions.iter().find(|s| !s.current).unwrap();
    owner
        .command(Command::Launch(LaunchCommand::LogoutSession {
            session_id: other_session.id.clone(),
        }))
        .await;
    assert!(second
        .http
        .json::<Snapshot<Vec<DeviceSession>>>("/me/sessions")
        .await
        .is_err());
    let third_auth = password_auth(&fixture.url, "settings_owner", PASSWORD, false)
        .await
        .unwrap();
    let third_http = Http::new(&fixture.url, &third_auth.token).unwrap();
    owner
        .command(Command::Launch(LaunchCommand::LogoutAllOthers))
        .await;
    assert!(third_http
        .json::<Snapshot<Vec<DeviceSession>>>("/me/sessions")
        .await
        .is_err());
    let sessions: Snapshot<Vec<DeviceSession>> = owner.http.json("/me/sessions").await.unwrap();
    assert_eq!(sessions.data.len(), 1);
    assert!(sessions.data[0].current);
    owner
        .command(Command::Launch(LaunchCommand::DeleteRole {
            guild_id: gid,
            role_id: other.id,
        }))
        .await;
    owner
        .projection(|m| !m.settings.roles.contains_key(&other.id))
        .await;
}

/// A deterministic virtual source: hashing and upload use 64 KiB chunks, without
/// needing a multi-gigabyte input allocation or an extra on-disk source copy.
struct GeneratedFile(u64);
impl sticktalk_client_core::upload::UploadSource for GeneratedFile {
    fn filename(&self) -> String {
        "large-qa.bin".into()
    }
    fn byte_size(&self) -> sticktalk_client_core::io::Task<'_, Result<u64, String>> {
        Box::pin(async { Ok(self.0) })
    }
    fn chunks(
        &self,
        offset: u64,
        count: u64,
    ) -> sticktalk_client_core::io::Task<'_, Result<sticktalk_client_core::http::ByteStream, String>>
    {
        assert!(offset + count <= self.0);
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::unfold(
                count,
                |remaining| async move {
                    if remaining == 0 {
                        return None;
                    }
                    let n = remaining.min(64 * 1024);
                    Some((Ok(vec![0x5a_u8; n as usize]), remaining - n))
                },
            )) as sticktalk_client_core::http::ByteStream)
        })
    }
}

async fn large_attachment_round_trip(size: u64) {
    let fixture = Fixture::start().await;
    let auth = fixture.register("large_owner").await;
    let member_auth = fixture.register("large_member").await;
    let outsider_auth = fixture.register("large_outsider").await;
    let mut owner = Peer::connect(&fixture.url, &auth).await;
    let mut member = Peer::connect(&fixture.url, &member_auth).await;
    let outsider = Peer::connect(&fixture.url, &outsider_auth).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Large-file QA".into(),
            icon_id: None,
        })
        .await
    else {
        panic!("guild")
    };
    let channel = detail.channels[0].id;
    let guild = detail.summary.guild.id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 60,
            max_uses: 1,
        })
        .await
    else {
        panic!("invite")
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    let started = std::time::Instant::now();
    let uploaded = owner
        .http
        .core()
        .upload_resumable(&GeneratedFile(size), UploadPurpose::Message, Some(channel))
        .await
        .unwrap();
    assert_eq!(uploaded.attachment.byte_size, size);
    let CommandResult::Message(message) = owner
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "Stream me without caching the whole file".into(),
            reply_to: None,
            attachment_ids: vec![uploaded.attachment.id],
            mention_ids: vec![],
        }))
        .await
    else {
        panic!("message")
    };
    member
        .projection(|m| {
            m.reads
                .get(&channel)
                .is_some_and(|r| r.latest_message_id == Some(message.id))
        })
        .await;
    assert_eq!(
        outsider
            .http
            .get(&format!("/files/{}", uploaded.attachment.id.0))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    // This is the exact source now passed to Media Foundation for inline video.
    // Its construction performs no full-file download, regardless of size.
    let source = sticktalk::video_stream::Source::attachment(
        &fixture.url,
        &member_auth.token,
        uploaded.attachment.id,
    )
    .await
    .unwrap();
    let url = source.path.to_str().unwrap().to_owned();
    assert!(url.starts_with("http://127.0.0.1:"));
    assert!(!url.contains(&member_auth.token));
    let client = reqwest::Client::new();
    let tail = client
        .get(&url)
        .header("Range", "bytes=-33")
        .send()
        .await
        .unwrap();
    assert_eq!(tail.status(), 206);
    assert_eq!(
        tail.headers()["content-range"],
        format!("bytes {}-{}/{size}", size - 33, size - 1)
    );
    assert_eq!(tail.bytes().await.unwrap().as_ref(), &[0x5a; 33]);
    let invalid = client
        .get(&url)
        .header("Range", format!("bytes={size}-"))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 416);
    assert_eq!(
        invalid.headers()["content-range"],
        format!("bytes */{size}")
    );
    // Permission changes affect subsequent range requests even though the local
    // URL has not changed. Revoked clips cannot keep playing from a disk cache.
    owner
        .command(Command::KickMember {
            guild_id: guild,
            user_id: member_auth.user.user.id,
        })
        .await;
    let denied = client
        .get(&url)
        .header("Range", "bytes=0-32")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    drop(source);
    eprintln!(
        "QA: uploaded and range-verified {size} bytes in {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attachments_over_128_mib_stream_ranges_and_recheck_access() {
    tokio::time::timeout(
        Duration::from_secs(180),
        large_attachment_round_trip(129 * 1024 * 1024),
    )
    .await
    .expect("large attachment deadline");
}

/// Explicit launch gate; intentionally opt-in because it writes >2 GiB to the
/// temporary real server. Run with --ignored --exact and no other large test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "launch gate: streams a real 2 GiB + 1 byte upload to temporary disk"]
async fn files_above_two_gib_have_no_size_ceiling() {
    tokio::time::timeout(
        Duration::from_secs(900),
        large_attachment_round_trip(2 * 1024 * 1024 * 1024 + 1),
    )
    .await
    .expect("2 GiB upload deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accounts_profiles_presence_rotation_and_erasure_real_clients() {
    let fixture = Fixture::start().await;
    let auth = fixture.register("account_owner").await;
    let member_auth = fixture.register("account_member").await;
    let outsider_auth = fixture.register("account_outsider").await;
    let mut owner = Peer::connect(&fixture.url, &auth).await;
    let mut member = Peer::connect(&fixture.url, &member_auth).await;
    let mut outsider = Peer::connect(&fixture.url, &outsider_auth).await;
    let second_auth = password_auth(&fixture.url, "account_owner", PASSWORD, false)
        .await
        .unwrap();
    let mut second = Peer::connect(&fixture.url, &second_auth).await;
    owner
        .command(Command::SendFriendRequest {
            username: "account_member".into(),
        })
        .await;
    member
        .command(Command::AcceptFriendRequest {
            user_id: auth.user.user.id,
        })
        .await;
    outsider
        .command(Command::SendFriendRequest {
            username: "account_member".into(),
        })
        .await;
    member
        .command(Command::DeclineFriendRequest {
            user_id: outsider_auth.user.user.id,
        })
        .await;
    outsider
        .projection(|m| !m.relationships.contains_key(&member_auth.user.user.id))
        .await;
    let dir = tempfile::tempdir().unwrap();
    let avatar_path = dir.path().join("avatar.jpg");
    tokio::fs::write(&avatar_path, sticktalk_client_core::mock::IMAGE)
        .await
        .unwrap();
    let avatar = owner
        .http
        .upload_resumable(&avatar_path, UploadPurpose::Avatar, None)
        .await
        .unwrap();
    owner
        .command(Command::UpdateProfile(ProfilePatch {
            display_name: Patch::Set("A long but valid QA display name".into()),
            avatar_id: Patch::Set(avatar.attachment.id),
            status_text: Patch::Set("Custom status persists".into()),
            ..Default::default()
        }))
        .await;
    owner
        .command(Command::Launch(LaunchCommand::UpdateUserProfile {
            banner_color: 0x3ddc97,
            pronouns: "they/them".into(),
            about_me: "A profile from the real server".into(),
        }))
        .await;
    member
        .projection(|m| {
            m.users.get(&auth.user.user.id).is_some_and(|u| {
                u.avatar_id == Some(avatar.attachment.id)
                    && u.status_text.as_deref() == Some("Custom status persists")
            })
        })
        .await;
    for status in [
        PresenceStatus::Online,
        PresenceStatus::Idle,
        PresenceStatus::Dnd,
        PresenceStatus::Invisible,
    ] {
        owner.command(Command::SetPresence { status }).await;
        let observed = if status == PresenceStatus::Invisible {
            PresenceStatus::Offline
        } else {
            status
        };
        member
            .projection(|m| m.presences.get(&auth.user.user.id) == Some(&observed))
            .await;
        second
            .projection(|m| m.me.as_ref().is_some_and(|u| u.desired_presence == status))
            .await;
    }
    let sessions: Snapshot<Vec<DeviceSession>> = owner.http.json("/me/sessions").await.unwrap();
    assert_eq!(sessions.data.len(), 2);
    let own_session = sessions.data.iter().find(|s| s.current).unwrap();
    assert!(matches!(
        outsider
            .outcome(Command::Launch(LaunchCommand::LogoutSession {
                session_id: own_session.id.clone()
            }))
            .await,
        CommandOutcome::Err(_)
    ));
    assert!(owner
        .http
        .core()
        .reauthenticate(&ReauthRequest {
            operation: AccountOperation::Password,
            current_password: Some("wrong password".into()),
            id_token: None
        })
        .await
        .is_err());
    let proof: ReauthResponse = post_retry(
        &owner.http,
        "/auth/reauth",
        &ReauthRequest {
            operation: AccountOperation::Password,
            current_password: Some(PASSWORD.into()),
            id_token: None,
        },
    )
    .await
    .unwrap();
    let changed = account_retry(
        &owner.http,
        backend::AccountAction::Password {
            current: Some(PASSWORD.into()),
            new: "a fresh correct password".into(),
        },
        &proof.proof,
    )
    .await
    .unwrap();
    let rotated = changed
        .session
        .expect("password changes rotate the caller, too");
    assert!(owner.http.json::<Snapshot<SelfUser>>("/me").await.is_err());
    assert!(second.http.json::<Snapshot<SelfUser>>("/me").await.is_err());
    second
        .until(|d| {
            matches!(
                d,
                Data::State(sticktalk::model::ConnectionState::SessionRevoked)
            )
        })
        .await;
    assert!(member.http.json::<Snapshot<SelfUser>>("/me").await.is_ok());
    let mut owner = Peer::connect(&fixture.url, &rotated).await;
    assert_eq!(
        owner
            .model
            .lock()
            .await
            .me
            .as_ref()
            .unwrap()
            .user
            .status_text
            .as_deref(),
        Some("Custom status persists")
    );
    let profile: Snapshot<ProfileDetail> = owner
        .http
        .json(&format!("/users/{}/profile", auth.user.user.id.0))
        .await
        .unwrap();
    assert_eq!(profile.data.profile.pronouns, "they/them");
    assert_eq!(profile.data.profile.banner_color, 0x3ddc97);
    assert!(owner.http.core().delete_account("no-proof").await.is_err());
    let proof: ReauthResponse = post_retry(
        &owner.http,
        "/auth/reauth",
        &ReauthRequest {
            operation: AccountOperation::DeleteAccount,
            current_password: Some("a fresh correct password".into()),
            id_token: None,
        },
    )
    .await
    .unwrap();
    owner
        .http
        .core()
        .delete_account(&proof.proof)
        .await
        .unwrap();
    owner
        .until(|d| {
            matches!(
                d,
                Data::State(sticktalk::model::ConnectionState::SessionRevoked)
            )
        })
        .await;
    assert!(owner.http.json::<Snapshot<SelfUser>>("/me").await.is_err());
    member
        .until(|d| matches!(d, Data::Frame(ServerMsg::Ready(_))))
        .await;
    member
        .projection(|m| !m.relationships.contains_key(&auth.user.user.id))
        .await;
    assert!(
        member.http.json::<Snapshot<SelfUser>>("/me").await.is_ok(),
        "erasure must not revoke other accounts"
    );
    assert!(password_auth(
        &fixture.url,
        "account_owner",
        "a fresh correct password",
        false
    )
    .await
    .is_err());
}

/// Opt-in OS-window gate. Uses the real executable and real server, never --mock.
/// Each process has isolated preferences and an ephemeral loopback test token;
/// the existing agent window mode disables tray/notifications/focus activation.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "opens two real desktop windows on a secondary monitor"]
async fn desktop_two_real_clients_render_and_send() {
    use std::os::windows::process::CommandExt;
    let fixture = Fixture::start().await;
    let a = fixture.register("desktop_owner").await;
    let b = fixture.register("desktop_member").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Real-data launch QA with a long server name".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let channel = detail.channels[0].id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: detail.summary.guild.id,
            ttl_seconds: 3600,
            max_uses: 3,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    owner
        .command(Command::UpdateProfile(ProfilePatch {
            display_name: Patch::Set("A deliberately long display name for visual QA".into()),
            ..Default::default()
        }))
        .await;
    let first = owner.message(channel, "**Real server data** — messages here came through the production transport.\n> Reply, reactions, markdown, and long names").await;
    member
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: format!(
                "Hello <@{}> — ||a spoiler|| and `inline code`",
                a.user.user.id.0
            ),
            reply_to: Some(first.id),
            attachment_ids: vec![],
            mention_ids: vec![a.user.user.id],
        }))
        .await;
    owner
        .command(Command::AddReaction {
            channel_id: channel,
            message_id: first.id,
            emoji: "👍".into(),
        })
        .await;
    let image_dir = tempfile::tempdir().unwrap();
    let path = image_dir.path().join("real-photo.jpg");
    tokio::fs::write(&path, sticktalk_client_core::mock::IMAGE)
        .await
        .unwrap();
    let image = owner
        .http
        .upload_resumable(&path, UploadPurpose::Message, Some(channel))
        .await
        .unwrap();
    owner
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "Image loaded from the real blob directory".into(),
            reply_to: None,
            attachment_ids: vec![image.attachment.id],
            mention_ids: vec![],
        }))
        .await;
    let output = std::env::var_os("STICKTALK_QA_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("screenshots/qa-live")
        });
    std::fs::create_dir_all(&output).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut children = Vec::new();
    let mut homes = Vec::new();
    for (name, auth, send) in [
        (
            "owner",
            &a,
            "Sent through the owner's real desktop composer",
        ),
        (
            "member",
            &b,
            "Sent through the member's real desktop composer",
        ),
    ] {
        let home = tempfile::tempdir().unwrap();
        for path in [
            output.join(format!("{name}-1280x800.png")),
            output.join(format!("{name}-window.json")),
        ] {
            if path.is_file() {
                std::fs::remove_file(path).unwrap();
            }
        }
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_sticktalk"))
            .creation_flags(0x08000000) // no debug console window
            .args(["--size", "1280x800", "--capture"])
            .arg(output.join(format!("{name}-1280x800.png")))
            .arg("--agent-window-report")
            .arg(output.join(format!("{name}-window.json")))
            .env("LOCALAPPDATA", home.path())
            .env("STICKTALK_AGENT_WINDOW", "secondary")
            .env("STICKTALK_QA_SERVER", &fixture.url)
            .env("STICKTALK_QA_TOKEN", &auth.token)
            .env("STICKTALK_QA_CHANNEL", channel.0.to_string())
            .env("STICKTALK_QA_SEND", send)
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(output.join(format!("{name}.log"))).unwrap())
            .spawn()
            .unwrap();
        children.push(Child(child));
        homes.push(home);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let mut done = true;
        for child in &mut children {
            match child.0.try_wait().unwrap() {
                Some(status) => assert!(status.success(), "desktop process failed: {status}"),
                None => done = false,
            }
        }
        if done {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "desktop screenshot deadline"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let history: Page<Message> = owner
        .http
        .json(&format!("/channels/{}/messages", channel.0))
        .await
        .unwrap();
    for name in ["owner", "member"] {
        assert!(output.join(format!("{name}-1280x800.png")).is_file());
        let report: Value = serde_json::from_slice(
            &std::fs::read(output.join(format!("{name}-window.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(report["shown"], true, "a secondary monitor is required");
        assert_eq!(report["foreground_unchanged"], true);
        assert_eq!(report["client_foreground"], false);
        assert!(history
            .items
            .iter()
            .any(|m| m.content == format!("Sent through the {name}'s real desktop composer")));
    }
    // Reconnect the test driver after its token was deliberately handed to the
    // native process. Additional captures exercise permissions and real empties.
    drop(owner);
    let mut control = Peer::connect(&fixture.url, &a).await;
    let CommandResult::Channel(empty) = control
        .command(Command::CreateChannel {
            guild_id: detail.summary.guild.id,
            kind: ChannelKind::Text,
            name: "empty-real-channel".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    control
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: channel,
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: detail.summary.guild.id,
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES | Permissions::ADD_REACTIONS,
            }],
        }))
        .await;
    for (label, channel_id) in [("read-only", channel), ("empty", empty.summary.id)] {
        for size in ["1280x800", "1920x1080"] {
            let home = tempfile::tempdir().unwrap();
            let capture = output.join(format!("{label}-{size}.png"));
            if capture.is_file() {
                std::fs::remove_file(&capture).unwrap();
            }
            let child = std::process::Command::new(env!("CARGO_BIN_EXE_sticktalk"))
                .creation_flags(0x08000000)
                .args(["--size", size, "--capture"])
                .arg(output.join(format!("{label}-{size}.png")))
                .env("LOCALAPPDATA", home.path())
                .env("STICKTALK_AGENT_WINDOW", "secondary")
                .env("STICKTALK_QA_SERVER", &fixture.url)
                .env("STICKTALK_QA_TOKEN", &b.token)
                .env("STICKTALK_QA_CHANNEL", channel_id.0.to_string())
                .env_remove("STICKTALK_QA_SEND")
                .stdout(std::process::Stdio::null())
                .stderr(std::fs::File::create(output.join(format!("{label}-{size}.log"))).unwrap())
                .spawn()
                .unwrap();
            let mut child = Child(child);
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{label} desktop capture deadline"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(output.join(format!("{label}-{size}.png")).is_file());
        }
    }
    // Capture actual validation feedback and an actual cut TCP connection, not
    // mock UI states. The normal Ready capture timer fires after these changes.
    for label in ["error-toast", "disconnected"] {
        let home = tempfile::tempdir().unwrap();
        let capture = output.join(format!("{label}-1280x800.png"));
        if capture.is_file() {
            std::fs::remove_file(&capture).unwrap();
        }
        let cable = if label == "disconnected" {
            Some(Cable::new(&fixture.url).await)
        } else {
            None
        };
        let server = cable
            .as_ref()
            .map_or(fixture.url.as_str(), |c| c.url.as_str());
        let content = if cable.is_some() {
            "Native connection-cut proof".into()
        } else {
            "x".repeat(MAX_MESSAGE_CHARS + 1)
        };
        let mut child = Child(
            std::process::Command::new(env!("CARGO_BIN_EXE_sticktalk"))
                .creation_flags(0x08000000)
                .args(["--size", "1280x800", "--capture"])
                .arg(&capture)
                .env("LOCALAPPDATA", home.path())
                .env("STICKTALK_AGENT_WINDOW", "secondary")
                .env("STICKTALK_QA_SERVER", server)
                .env("STICKTALK_QA_TOKEN", &a.token)
                .env("STICKTALK_QA_CHANNEL", channel.0.to_string())
                .env("STICKTALK_QA_SEND", &content)
                .stdout(std::process::Stdio::null())
                .stderr(std::fs::File::create(output.join(format!("{label}.log"))).unwrap())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        if let Some(cable) = cable {
            // The persisted send proves that the native process reached Ready.
            // Closing both proxy listener and active streams makes retries fail.
            loop {
                let page: Page<Message> = member
                    .http
                    .json(&format!("/channels/{}/messages", channel.0))
                    .await
                    .unwrap();
                if page.items.iter().any(|m| m.content == content) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "native send before cable cut"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(!capture.exists(), "connection must be cut before capture");
            drop(cable);
        }
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{label} capture deadline"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(capture.is_file());
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "launch gate: opens the native hardware decoder, muted and windowless"]
async fn native_decoder_plays_video_above_128_mib_before_full_download() {
    use sticktalk_client_core::{http::ByteStream, io::Task, upload::UploadSource};
    struct Video {
        prefix: Arc<Vec<u8>>,
        size: u64,
    }
    impl UploadSource for Video {
        fn filename(&self) -> String {
            "padded-real-clip.mp4".into()
        }
        fn byte_size(&self) -> Task<'_, Result<u64, String>> {
            Box::pin(async { Ok(self.size) })
        }
        fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<ByteStream, String>> {
            let prefix = self.prefix.clone();
            Box::pin(async move {
                Ok(Box::pin(futures_util::stream::unfold(
                    (offset, count, prefix),
                    |(offset, remaining, prefix)| async move {
                        if remaining == 0 {
                            return None;
                        }
                        let n = remaining.min(64 * 1024);
                        let mut bytes = vec![0_u8; n as usize];
                        if offset < prefix.len() as u64 {
                            let copy = n.min(prefix.len() as u64 - offset) as usize;
                            bytes[..copy]
                                .copy_from_slice(&prefix[offset as usize..offset as usize + copy]);
                        }
                        Some((Ok(bytes), (offset + n, remaining - n, prefix)))
                    },
                )) as ByteStream)
            })
        }
    }
    let fixture = Fixture::start().await;
    let auth = fixture.register("video_owner").await;
    let mut owner = Peer::connect(&fixture.url, &auth).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Native video QA".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let channel = detail.channels[0].id;
    let size = 129 * 1024 * 1024;
    let mut prefix = include_bytes!("../../server/tests/fixtures/clip.mp4").to_vec();
    // A legitimate trailing free atom makes a tiny deterministic clip >128 MiB
    // without spending test time encoding a huge movie or shipping a large asset.
    prefix.extend_from_slice(&((size - prefix.len() as u64) as u32).to_be_bytes());
    prefix.extend_from_slice(b"free");
    let upload = owner
        .http
        .core()
        .upload_resumable(
            &Video {
                prefix: Arc::new(prefix),
                size,
            },
            UploadPurpose::Message,
            Some(channel),
        )
        .await
        .unwrap();
    assert_eq!(upload.attachment.content_type, "video/mp4");
    owner
        .command(Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "Playable video".into(),
            reply_to: None,
            attachment_ids: vec![upload.attachment.id],
            mention_ids: vec![],
        }))
        .await;
    let source = sticktalk::video_stream::Source::attachment(
        &fixture.url,
        &auth.token,
        upload.attachment.id,
    )
    .await
    .unwrap();
    let path = source.path.clone();
    let frame = tokio::task::spawn_blocking(move || {
        let player = sticktalk_media::FilePlayer::open(
            &path,
            sticktalk_media::Size {
                width: 160,
                height: 90,
            },
            true,
            0.,
            false,
        )
        .unwrap();
        player.play(0.).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(frame) = player.frame().unwrap() {
                return frame;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "decoder did not produce a frame"
            );
            std::thread::sleep(Duration::from_millis(33));
        }
    })
    .await
    .unwrap();
    assert_eq!((frame.size.width, frame.size.height), (160, 90));
    assert!(
        source.bytes_received() < size / 2,
        "decoder must begin playing before downloading the clip: {} of {size}",
        source.bytes_received()
    );
    eprintln!(
        "QA: native decoder produced {}x{} frame after {} of {size} bytes",
        frame.size.width,
        frame.size.height,
        source.bytes_received()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_member_controls_use_authoritative_effective_channel_permissions() {
    use sticktalk_client_core::launch::{Read, SnapshotData};
    async fn permissions(peer: &mut Peer, channel: Id) -> Permissions {
        peer.handle
            .try_send(Request::Launch {
                read: Read::Permissions(channel),
                generation: 1,
            })
            .unwrap();
        let Data::Launch { data: SnapshotData::Permissions(_, value), .. } = peer.until(|d| matches!(d, Data::Launch { data: SnapshotData::Permissions(id, _), .. } if *id == channel)).await else { panic!() };
        assert_eq!(
            sticktalk_client_core::launch::permissions(&*peer.model.lock().await, channel),
            value.data
        );
        value.data
    }
    let fixture = Fixture::start().await;
    let a = fixture.register("acl_owner").await;
    let b = fixture.register("acl_member").await;
    let c = fixture.register("acl_outsider").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let outsider = Peer::connect(&fixture.url, &c).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "ACL controls QA".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let guild = detail.summary.guild.id;
    let CommandResult::Channel(category) = owner
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Category,
            name: "Read-only category".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Channel(channel) = owner
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Text,
            name: "read-only".into(),
            parent_id: Some(category.summary.id),
            topic: Some("Permissions come from the server".into()),
        })
        .await
    else {
        panic!()
    };
    let channel = channel.summary.id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 3600,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    assert!(permissions(&mut member, channel)
        .await
        .contains(Permissions::SEND_MESSAGES | Permissions::ATTACH_FILES));
    assert_eq!(
        outsider
            .http
            .get(&format!("/channels/{}/permissions", channel.0))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    // Nobody learns the underlying roles/member ACLs just by reading their mask.
    assert_eq!(
        member
            .http
            .get(&format!("/channels/{}/overwrites", channel.0))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    owner
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: category.summary.id,
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: guild,
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            }],
        }))
        .await;
    let denied = permissions(&mut member, channel).await;
    assert!(denied.contains(
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY | Permissions::ADD_REACTIONS
    ));
    assert!(!denied.intersects(
        Permissions::SEND_MESSAGES
            | Permissions::ATTACH_FILES
            | Permissions::EMBED_LINKS
            | Permissions::MENTION_EVERYONE
    ));
    for command in [
        Command::SendMessage(MessageDraft {
            channel_id: channel,
            content: "must fail".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        }),
        Command::Typing {
            channel_id: channel,
        },
    ] {
        assert!(matches!(
            member.outcome(command).await,
            CommandOutcome::Err(ApiError {
                code: ErrorCode::Forbidden,
                ..
            })
        ));
    }
    assert!(member
        .http
        .core()
        .upload_resumable(&GeneratedFile(16), UploadPurpose::Message, Some(channel))
        .await
        .is_err());
    owner
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: channel,
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: b.user.user.id,
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
            }],
        }))
        .await;
    assert!(permissions(&mut member, channel)
        .await
        .contains(Permissions::SEND_MESSAGES | Permissions::ATTACH_FILES));
    member
        .message(channel, "Member allow overrides category denial")
        .await;
    owner
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: channel,
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: b.user.user.id,
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            }],
        }))
        .await;
    member
        .projection(|m| !m.channels.contains_key(&channel))
        .await;
    assert_eq!(
        member
            .http
            .get(&format!("/channels/{}/permissions", channel.0))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(
        sticktalk_client_core::launch::permissions(&*member.model.lock().await, channel).is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "launch gate: 2048 messages at production 60/min; approximately 34 minutes"]
async fn thousands_of_messages_paginate_jump_and_keep_unreads_across_client_restart() {
    let fixture = Fixture::start().await;
    let a = fixture.register("history_owner").await;
    let b = fixture.register("history_member").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Thousands of messages QA".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let channel = detail.channels[0].id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: detail.summary.guild.id,
            ttl_seconds: 3600,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    member.history(channel, None).await;
    let mut ids = Vec::new();
    let started = std::time::Instant::now();
    for n in 0..2048 {
        // Receipt quotas are deliberately enforced. Rotate a genuine login after
        // each 200 commands, just as independent devices/accounts would do.
        if n > 0 && n % 200 == 0 {
            owner.http.logout().await;
            drop(owner);
            let auth = password_auth(&fixture.url, "history_owner", PASSWORD, false)
                .await
                .unwrap();
            owner = Peer::connect(&fixture.url, &auth).await;
        }
        ids.push(
            owner
                .message(
                    channel,
                    format!("History message {n}: **bold** and `code` with a realistic row length"),
                )
                .await
                .id,
        );
    }
    member
        .projection(|m| {
            m.reads.get(&channel).is_some_and(|r| {
                r.latest_message_id == ids.last().copied() && r.unread_count == 2048
            })
        })
        .await;
    assert!(member.model.lock().await.retained_count() <= 1000);
    assert!(member.model.lock().await.retained_bytes() <= 8 * 1024 * 1024);
    let mut collected = std::collections::BTreeSet::new();
    let mut before = None;
    loop {
        let page = owner.history(channel, before).await;
        assert!(page.items.len() <= 50);
        for message in page.items {
            assert!(collected.insert(message.id), "pagination duplicated an ID");
        }
        if !page.has_more {
            break;
        }
        let next = page.next_before.expect("older cursor");
        assert!(before.is_none_or(|old| next < old));
        before = Some(next);
    }
    assert_eq!(collected, ids.iter().copied().collect());
    owner
        .handle
        .try_send(Request::AroundHistory {
            channel,
            anchor: HistoryAnchor::Message(ids[1024]),
            seek: 1,
            generation: 1,
        })
        .unwrap();
    let Data::AroundHistory { page, .. } = owner
        .until(|d| matches!(d, Data::AroundHistory { .. }))
        .await
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 50);
    assert!(page.items.iter().any(|m| m.id == ids[1024]));
    member
        .command(Command::MarkRead {
            channel_id: channel,
            message_id: ids[1000],
        })
        .await;
    member
        .projection(|m| m.reads[&channel].unread_count == 1047)
        .await;
    drop(member);
    let member = Peer::connect(&fixture.url, &b).await;
    assert_eq!(
        member.model.lock().await.reads[&channel].last_read_id,
        Some(ids[1000])
    );
    assert_eq!(member.model.lock().await.reads[&channel].unread_count, 1047);
    eprintln!("QA: 2048 real messages, full pagination, jump, bounded reducer and restart unreads in {:?}", started.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_icons_channels_invites_and_moderation_real_clients() {
    let fixture = Fixture::start().await;
    let a = fixture.register("server_owner").await;
    let b = fixture.register("server_member").await;
    let c = fixture.register("server_outsider").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let mut outsider = Peer::connect(&fixture.url, &c).await;
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("icon.jpg");
    tokio::fs::write(&path, sticktalk_client_core::mock::IMAGE)
        .await
        .unwrap();
    let icon = owner
        .http
        .upload_resumable(&path, UploadPurpose::GuildIcon, None)
        .await
        .unwrap();
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Server lifecycle".into(),
            icon_id: Some(icon.attachment.id),
        })
        .await
    else {
        panic!()
    };
    let guild = detail.summary.guild.id;
    let general = detail.channels[0].id;
    assert_eq!(detail.summary.guild.icon_id, Some(icon.attachment.id));
    for (ttl_seconds, max_uses) in [(59, 1), (60, 0)] {
        assert!(matches!(
            owner
                .outcome(Command::CreateInvite {
                    guild_id: guild,
                    ttl_seconds,
                    max_uses
                })
                .await,
            CommandOutcome::Err(ApiError {
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
    }
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 60,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    assert!(matches!(
        outsider
            .outcome(Command::JoinGuild {
                invite_code: invite.code
            })
            .await,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::InviteExhausted,
            ..
        })
    ));
    owner
        .command(Command::UpdateGuild {
            guild_id: guild,
            patch: GuildPatch {
                name: Patch::Set("Edited real server name".into()),
                icon_id: Patch::Clear,
            },
        })
        .await;
    member
        .projection(|m| {
            m.guilds.get(&guild).is_some_and(|g| {
                g.guild.name == "Edited real server name" && g.guild.icon_id.is_none()
            })
        })
        .await;
    for command in [
        Command::DeleteGuild { guild_id: guild },
        Command::UpdateGuild {
            guild_id: guild,
            patch: GuildPatch {
                name: Patch::Set("must fail".into()),
                icon_id: Patch::Keep,
            },
        },
        Command::KickMember {
            guild_id: guild,
            user_id: a.user.user.id,
        },
        Command::BanMember {
            guild_id: guild,
            user_id: a.user.user.id,
            reason: None,
        },
    ] {
        assert!(matches!(
            member.outcome(command).await,
            CommandOutcome::Err(_)
        ));
    }
    let CommandResult::Channel(category) = owner
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Category,
            name: "Projects".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Channel(text) = owner
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Text,
            name: "work".into(),
            parent_id: Some(category.summary.id),
            topic: Some("A real topic".into()),
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Channel(voice) = owner
        .command(Command::CreateChannel {
            guild_id: guild,
            kind: ChannelKind::Voice,
            name: "Voice metadata only".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    owner
        .command(Command::UpdateChannel {
            channel_id: text.summary.id,
            patch: ChannelPatch {
                name: Patch::Set("edited-work".into()),
                topic: Patch::Set("Edited topic".into()),
            },
        })
        .await;
    member
        .projection(|m| {
            m.channels
                .get(&text.summary.id)
                .is_some_and(|c| c.name.as_deref() == Some("edited-work"))
        })
        .await;
    let read: Snapshot<Channel> = member
        .http
        .json(&format!("/channels/{}", text.summary.id.0))
        .await
        .unwrap();
    assert_eq!(read.data.topic.as_deref(), Some("Edited topic"));
    owner
        .command(Command::ReorderChannels {
            guild_id: guild,
            placements: vec![
                ChannelPlacement {
                    channel_id: category.summary.id,
                    parent_id: None,
                    position: 0,
                },
                ChannelPlacement {
                    channel_id: voice.summary.id,
                    parent_id: None,
                    position: 1,
                },
                ChannelPlacement {
                    channel_id: general,
                    parent_id: Some(category.summary.id),
                    position: 0,
                },
                ChannelPlacement {
                    channel_id: text.summary.id,
                    parent_id: Some(category.summary.id),
                    position: 1,
                },
            ],
        })
        .await;
    member
        .projection(|m| {
            m.channels
                .get(&general)
                .is_some_and(|c| c.parent_id == Some(category.summary.id))
        })
        .await;
    owner
        .command(Command::DeleteChannel {
            channel_id: category.summary.id,
        })
        .await;
    member
        .projection(|m| {
            !m.channels.contains_key(&category.summary.id)
                && m.channels[&general].parent_id.is_none()
                && m.channels[&text.summary.id].parent_id.is_none()
        })
        .await;
    owner
        .command(Command::DeleteChannel {
            channel_id: voice.summary.id,
        })
        .await;
    member
        .command(Command::LeaveGuild { guild_id: guild })
        .await;
    member
        .projection(|m| !m.guilds.contains_key(&guild) && !m.channels.contains_key(&general))
        .await;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 60,
            max_uses: 10,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    owner
        .command(Command::KickMember {
            guild_id: guild,
            user_id: b.user.user.id,
        })
        .await;
    member.projection(|m| !m.guilds.contains_key(&guild)).await;
    owner
        .command(Command::BanMember {
            guild_id: guild,
            user_id: b.user.user.id,
            reason: Some("QA ban".into()),
        })
        .await;
    assert!(matches!(
        member
            .outcome(Command::JoinGuild {
                invite_code: invite.code.clone()
            })
            .await,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Banned,
            ..
        })
    ));
    owner
        .command(Command::UnbanMember {
            guild_id: guild,
            user_id: b.user.user.id,
        })
        .await;
    member
        .command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    owner
        .command(Command::RevokeInvite {
            guild_id: guild,
            code: invite.code.clone(),
        })
        .await;
    assert!(matches!(
        outsider
            .outcome(Command::JoinGuild {
                invite_code: invite.code
            })
            .await,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::InvalidInvite,
            ..
        })
    ));
    owner
        .command(Command::DeleteGuild { guild_id: guild })
        .await;
    member
        .projection(|m| !m.guilds.contains_key(&guild) && !m.channels.contains_key(&general))
        .await;
    assert_eq!(
        member
            .http
            .get(&format!("/channels/{}", text.summary.id.0))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_expiry_is_enforced_after_real_elapsed_time() {
    let fixture = Fixture::start().await;
    let a = fixture.register("expiry_owner").await;
    let b = fixture.register("expiry_member").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Invite expiry".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: detail.summary.guild.id,
            ttl_seconds: 60,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    // No private DB clock edits or bypass of the 60-second production minimum.
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(matches!(
        member
            .outcome(Command::JoinGuild {
                invite_code: invite.code
            })
            .await,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::InviteExpired,
            ..
        })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn history_reads_back_off_instead_of_failing_at_the_real_rest_bucket() {
    let fixture = Fixture::start().await;
    let a = fixture.register("read_throttle_owner").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "REST backoff QA".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let channel = detail.channels[0].id;
    let message = owner
        .message(
            channel,
            "Read this through more requests than the REST burst",
        )
        .await;
    // The production bucket is 120/minute, burst 30. Forty-five immediate
    // reads MUST hit it; the native/core adapter waits, rather than emitting Error.
    let started = std::time::Instant::now();
    for _ in 0..45 {
        let page = owner.history(channel, None).await;
        assert_eq!(page.items[0].id, message.id);
    }
    assert!(started.elapsed() >= Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn features3_emoji_search_slowmode_and_private_folders_real_transport() {
    let fixture = Fixture::start().await;
    let a = fixture.register("features_owner").await;
    let b = fixture.register("features_member").await;
    let c = fixture.register("features_outsider").await;
    let mut owner = Peer::connect(&fixture.url, &a).await;
    let mut member = Peer::connect(&fixture.url, &b).await;
    let outside = Http::new(&fixture.url, &c.token).unwrap();
    let CommandResult::Guild(detail) = owner
        .command(Command::CreateGuild {
            name: "Feature checks".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let guild = detail.summary.guild.id;
    let channel = detail
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Text)
        .unwrap()
        .id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: guild,
            ttl_seconds: 600,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    member
        .command(Command::JoinGuild {
            invite_code: invite.code,
        })
        .await;
    member.projection(|m| m.guilds.contains_key(&guild)).await;
    member.history(channel, None).await; // The bounded reducer retains only channels explicitly opened by the host.
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(512, 256)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let upload_url = format!(
        "{}/api/v1/guilds/{}/emoji?name=ship_it",
        fixture.url, guild.0
    );
    assert_eq!(
        member
            .http
            .client
            .post(&upload_url)
            .bearer_auth(&b.token)
            .body(png.get_ref().clone())
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let response = owner
        .http
        .client
        .post(&upload_url)
        .bearer_auth(&a.token)
        .body(png.into_inner())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let uploaded: Snapshot<Vec<GuildEmoji>> = response.json().await.unwrap();
    let emoji = uploaded.data[0].clone();
    owner
        .projection(|m| {
            m.launch
                .emoji
                .get(&guild)
                .is_some_and(|v| v.iter().any(|e| e.id == emoji.id))
        })
        .await;
    member
        .projection(|m| {
            m.launch
                .emoji
                .get(&guild)
                .is_some_and(|v| v.iter().any(|e| e.id == emoji.id))
        })
        .await;
    let preview = member
        .http
        .get(&format!("/emoji/{}/preview", emoji.id.0))
        .send()
        .await
        .unwrap();
    assert!(preview.status().is_success());
    let preview = image::load_from_memory(&preview.bytes().await.unwrap()).unwrap();
    assert_eq!((preview.width(), preview.height()), (128, 64));
    assert!(!outside
        .get(&format!("/emoji/{}", emoji.id.0))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    let message = member
        .message(
            channel,
            format!("launch prototype {} https://example.test", emoji.token()),
        )
        .await;
    member
        .command(Command::AddReaction {
            channel_id: channel,
            message_id: message.id,
            emoji: emoji.token(),
        })
        .await;
    member
        .projection(|m| {
            m.messages
                .get(&channel)
                .and_then(|ms| ms.get(&message.id))
                .is_some_and(|m| {
                    m.reactions
                        .iter()
                        .any(|r| r.emoji == format!("<:emoji:{}>", emoji.id.0) && r.me)
                })
        })
        .await;
    owner
        .command(Command::Launch(LaunchCommand::RenameEmoji {
            guild_id: guild,
            emoji_id: emoji.id,
            name: "shipped".into(),
        }))
        .await;
    member
        .projection(|m| {
            m.launch
                .emoji
                .get(&guild)
                .is_some_and(|v| v.iter().any(|e| e.name == "shipped"))
        })
        .await;
    let results = owner
        .http
        .core()
        .search(
            guild,
            &MessageSearchQuery {
                q: "launch from:features_member in:general has:link".into(),
                before: None,
                limit: Some(25),
            },
        )
        .await
        .unwrap();
    assert_eq!(results.data.items.len(), 1);
    assert_eq!(results.data.items[0].id, message.id);
    member
        .command(Command::EditMessage(MessageEdit {
            channel_id: channel,
            message_id: message.id,
            content: format!("updated launch {}", emoji.token()),
            mention_ids: vec![],
        }))
        .await;
    let old = owner
        .http
        .core()
        .search(
            guild,
            &MessageSearchQuery {
                q: "prototype".into(),
                before: None,
                limit: Some(25),
            },
        )
        .await
        .unwrap();
    assert!(old.data.items.is_empty());
    let updated = owner
        .http
        .core()
        .search(
            guild,
            &MessageSearchQuery {
                q: "updated launch".into(),
                before: None,
                limit: Some(25),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.data.items[0].id, message.id);
    owner
        .command(Command::Launch(LaunchCommand::SetSlowmode {
            channel_id: channel,
            seconds: 5,
        }))
        .await;
    let status: Snapshot<SlowmodeStatus> = member
        .http
        .json(&format!("/channels/{}/slowmode", channel.0))
        .await
        .unwrap();
    assert_eq!(status.data.seconds, 5);
    assert!(!status.data.bypassed);
    // A fresh send is refused across the real WebSocket while the previous persisted send is recent.
    let draft = MessageDraft {
        channel_id: channel,
        content: "too soon".into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    };
    if status.data.retry_after_ms > 0 {
        assert!(
            matches!(member.outcome(Command::SendMessage(draft)).await,CommandOutcome::Err(ApiError{code:ErrorCode::RateLimited,field:Some(field),..}) if field=="slowmode")
        );
    }
    owner.message(channel, "launch manager bypass one").await;
    owner.message(channel, "launch manager bypass two").await;
    owner
        .command(Command::Launch(LaunchCommand::SetSlowmode {
            channel_id: channel,
            seconds: 0,
        }))
        .await;
    let folder = ServerFolder {
        id: 1,
        name: "Work".into(),
        color: 0x3ddc97,
        collapsed: true,
        guild_ids: vec![guild],
    };
    owner
        .command(Command::Launch(LaunchCommand::SetServerFolders {
            folders: vec![folder.clone()],
        }))
        .await;
    owner
        .projection(|m| m.launch.server_folders == vec![folder.clone()])
        .await;
    let saved: Snapshot<Vec<ServerFolder>> =
        owner.http.json("/users/@me/server-folders").await.unwrap();
    assert_eq!(saved.data, vec![folder]);
    let private: Snapshot<Vec<ServerFolder>> =
        member.http.json("/users/@me/server-folders").await.unwrap();
    assert!(private.data.is_empty());
    owner
        .command(Command::Launch(LaunchCommand::DeleteEmoji {
            guild_id: guild,
            emoji_id: emoji.id,
        }))
        .await;
    member
        .projection(|m| m.launch.emoji.get(&guild).is_some_and(Vec::is_empty))
        .await;
    member
        .command(Command::RemoveReaction {
            channel_id: channel,
            message_id: message.id,
            emoji: emoji.token(),
        })
        .await;
    member
        .command(Command::DeleteMessage {
            channel_id: channel,
            message_id: message.id,
        })
        .await;
    let deleted = owner
        .http
        .core()
        .search(
            guild,
            &MessageSearchQuery {
                q: "updated".into(),
                before: None,
                limit: Some(25),
            },
        )
        .await
        .unwrap();
    assert!(deleted.data.items.is_empty());
    owner
        .command(Command::Launch(LaunchCommand::SetOverwrites {
            channel_id: channel,
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: b.user.user.id,
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            }],
        }))
        .await;
    member
        .projection(|m| !m.channels.contains_key(&channel))
        .await;
    let hidden = member
        .http
        .core()
        .search(
            guild,
            &MessageSearchQuery {
                q: "launch".into(),
                before: None,
                limit: Some(25),
            },
        )
        .await
        .unwrap();
    assert!(hidden.data.items.is_empty());
}
