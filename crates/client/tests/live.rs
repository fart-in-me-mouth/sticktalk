//! Real client transport + projection against a real TCP/SQLite server. No windows,
//! keyring writes, external services, browser launches, sounds, or notifications.
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
            Err(error) if error.starts_with("RateLimited") => {
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
            Err(error) if error.starts_with("RateLimited") => {
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
    async fn google_id_token(&self) -> String {
        // The opener is an HTTP script, never open::that/system browser.
        let challenge = Http::new(&self.url, "")
            .unwrap()
            .core()
            .google_challenge(AccountOperation::SignIn)
            .await
            .unwrap();
        tokio::time::timeout(
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
        .unwrap()
    }
    async fn google(&self) -> AuthResponse {
        let id_token = self.google_id_token().await;
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
            .starts_with("InvalidCredentials"));
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
        // A nonce challenge is one-use: a returning user must begin a fresh
        // installed-app flow, never replay the token consumed during signup.
        assert!(post_retry::<GoogleAuthResponse>(
            &http,
            "/auth/google",
            &GoogleAuthRequest { id_token }
        )
        .await
        .unwrap_err()
        .starts_with("InvalidCredentials"));
        let id_token = self.google_id_token().await;
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
    assert!(post_retry::<AuthMethodsResponse>(
        &carol.http,
        "/auth/google/unlink",
        &GoogleUnlinkRequest {}
    )
    .await
    .unwrap_err()
    .contains("only sign-in method"));
    let token = fixture
        .mock
        .token(CLIENT_ID, sticktalk::mock::now() / 1000 + 3600, true);
    assert!(post_retry::<AuthMethodsResponse>(
        &alice.http,
        "/auth/google/link",
        &GoogleLinkRequest {
            id_token: token.clone()
        }
    )
    .await
    .unwrap_err()
    .starts_with("IdentityAlreadyLinked"));
    let _: AuthMethodsResponse = post_retry(
        &carol.http,
        "/auth/password",
        &SetPasswordRequest {
            current_password: None,
            new_password: PASSWORD.into(),
        },
    )
    .await
    .unwrap();
    let methods: AuthMethodsResponse =
        post_retry(&carol.http, "/auth/google/unlink", &GoogleUnlinkRequest {})
            .await
            .unwrap();
    assert!(methods.has_password && !methods.google_linked);
    let methods: AuthMethodsResponse = post_retry(
        &carol.http,
        "/auth/google/link",
        &GoogleLinkRequest { id_token: token },
    )
    .await
    .unwrap();
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
            Data::State(sticktalk::model::ConnectionState::Stopped(_))
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
