use super::*;
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get};
use rsa::{pkcs8::EncodePrivateKey, traits::PublicKeyParts, RsaPrivateKey};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};
use tokio::sync::RwLock;

const CLIENT_ID: &str = "sticktalk-test-client.apps.googleusercontent.com";
fn signing_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap())
}
fn jwks(kid: &str) -> Value {
    let key = signing_key();
    json!({"keys":[{"kid":kid,"kty":"RSA","alg":"RS256","use":"sig",
        "n":URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())}]})
}
fn claims(subject: &str) -> Value {
    let now = state::now() / 1000;
    json!({"iss":"https://accounts.google.com","aud":CLIENT_ID,"sub":subject,
        "exp":now+3600,"iat":now,"email_verified":true,"email":"same@example.invalid"})
}
fn signed(header: Value, claims: Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
    let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    let payload = format!("{header}.{claims}");
    let key =
        ring::signature::RsaKeyPair::from_pkcs8(signing_key().to_pkcs8_der().unwrap().as_bytes())
            .unwrap();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        payload.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{payload}.{}", URL_SAFE_NO_PAD.encode(signature))
}
fn id_token(subject: &str) -> String {
    signed(json!({"alg":"RS256","kid":"one"}), claims(subject))
}
#[derive(Clone)]
struct Mock {
    body: Arc<RwLock<Value>>,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicUsize>,
}
struct Fixture {
    app: App,
    url: String,
    mock: Mock,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    jwks_task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.app.shutdown();
        self.task.abort();
        self.jwks_task.abort();
    }
}
impl Fixture {
    async fn new(enabled: bool) -> Self {
        // Generated locally, never persisted; all JWKS traffic is loopback.
        tokio::task::spawn_blocking(signing_key).await.unwrap();
        let mock = Mock {
            body: Arc::new(RwLock::new(jwks("one"))),
            calls: Arc::new(AtomicUsize::new(0)),
            fail: Arc::new(AtomicUsize::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let jwks_url = format!("http://{}/certs", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .route(
                "/certs",
                get(|State(mock): State<Mock>| async move {
                    mock.calls.fetch_add(1, Ordering::SeqCst);
                    if mock.fail.load(Ordering::SeqCst) != 0 {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    (
                        [("cache-control", "public, max-age=3600"), ("age", "10")],
                        axum::Json(mock.body.read().await.clone()),
                    )
                        .into_response()
                }),
            )
            .with_state(mock.clone());
        let jwks_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let app = App::open_with_verifier(
            crate::Config {
                operations: Default::default(),
                browser: Default::default(),
                bind: "127.0.0.1:0".parse().unwrap(),
                data_dir: dir.path().into(),
                public_url: "http://localhost".into(),
                max_upload_mb: 25,
                max_total_upload_mb: 10240,
                worker_id: 0,
                trusted_proxy: "127.0.0.1".parse().unwrap(),
                google_client_id: enabled.then(|| CLIENT_ID.into()),
                mock_google_jwks: None,
                mock_google_issuer: None,
                gifs: Default::default(),
                voice: crate::voice::VoiceOptions {
                    voice_bind_ip: "127.0.0.1".parse().unwrap(),
                    voice_advertised_ip: Some("127.0.0.1".parse().unwrap()),
                    ..Default::default()
                },
            },
            Verifier::with_url(jwks_url).unwrap(),
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving = app.clone();
        let task = tokio::spawn(async move { serving.serve(listener).await });
        Self {
            app,
            url,
            mock,
            task,
            jwks_task,
            _dir: dir,
        }
    }
    async fn raw(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> reqwest::Response {
        let client = reqwest::Client::new();
        let mut request = client.request(method.parse().unwrap(), format!("{}{path}", self.url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.json(&body).send().await.unwrap()
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> reqwest::Response {
        // Behavior tests are independent of time-dependent limiter credit.
        // The dedicated rate test below does not clear the limiter.
        self.app.0.core.lock().await.limits.clear();
        self.raw(method, path, token, body).await
    }
    async fn google(&self, sub: &str) -> GoogleAuthResponse {
        let r = self
            .call(
                "POST",
                "/api/auth/google",
                None,
                json!({"id_token":id_token(sub)}),
            )
            .await;
        assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
        assert_eq!(r.headers()["cache-control"], "no-store");
        r.json().await.unwrap()
    }
    async fn ticket(&self, sub: &str) -> GoogleSignupRequired {
        match self.google(sub).await {
            GoogleAuthResponse::SignupRequired(v) => v,
            _ => panic!("expected signup"),
        }
    }
    async fn complete(&self, ticket: &str, username: &str) -> reqwest::Response {
        self.call(
            "POST",
            "/api/v1/auth/google/complete",
            None,
            json!({"ticket":ticket,"username":username}),
        )
        .await
    }
    async fn signup(&self, sub: &str, name: &str) -> AuthResponse {
        let ticket = self.ticket(sub).await;
        let r = self.complete(&ticket.ticket, name).await;
        assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
        r.json().await.unwrap()
    }
    async fn register(&self, name: &str) -> AuthResponse {
        let r = self
            .call(
                "POST",
                "/api/v1/auth/register",
                None,
                json!({"username":name,"password":"correct horse battery"}),
            )
            .await;
        assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
        r.json().await.unwrap()
    }
    async fn allow_refresh(&self) {
        self.app.0.google.cache.lock().await.attempted =
            Some(Instant::now() - Duration::from_secs(61));
    }
}
async fn error(r: reqwest::Response, status: u16, code: ErrorCode) {
    assert_eq!(r.status(), status);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let e: ApiError = r.json().await.unwrap();
    assert_eq!(e.code, code);
}

#[tokio::test]
async fn valid_token_and_claim_signature_rejections() {
    let f = Fixture::new(true).await;
    assert_eq!(
        f.app
            .0
            .google
            .verify(&id_token("valid-sub"), CLIENT_ID)
            .await
            .unwrap(),
        "valid-sub"
    );
    let now = state::now() / 1000;
    for (field, value) in [
        ("aud", json!("wrong")),
        ("iss", json!("https://attacker.invalid")),
        ("exp", json!(now - 61)),
        ("iat", json!(now + 61)),
        ("iat", json!(now + 3600)),
        ("email_verified", json!(false)),
        ("email_verified", json!("true")),
        ("sub", json!("")),
        ("sub", json!("x".repeat(256))),
    ] {
        let mut c = claims("valid-sub");
        c[field] = value;
        let e = f
            .app
            .0
            .google
            .verify(&signed(json!({"alg":"RS256","kid":"one"}), c), CLIENT_ID)
            .await
            .unwrap_err();
        assert_eq!(e.0.code, ErrorCode::InvalidCredentials, "{field}");
    }
    for field in ["exp", "iat", "aud", "iss", "sub", "email_verified"] {
        let mut c = claims("valid-sub");
        c.as_object_mut().unwrap().remove(field);
        assert!(f
            .app
            .0
            .google
            .verify(&signed(json!({"alg":"RS256","kid":"one"}), c), CLIENT_ID)
            .await
            .is_err());
    }
    for header in [
        json!({"alg":"none","kid":"one"}),
        json!({"alg":"HS256","kid":"one"}),
        json!({"alg":"RS256","kid":"one","crit":["b64"]}),
    ] {
        assert!(f
            .app
            .0
            .google
            .verify(&signed(header, claims("valid-sub")), CLIENT_ID)
            .await
            .is_err());
    }
    let token = id_token("valid-sub");
    let mut pieces: Vec<_> = token.split('.').map(str::to_owned).collect();
    pieces[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims("forged-sub")).unwrap());
    assert!(f
        .app
        .0
        .google
        .verify(&pieces.join("."), CLIENT_ID)
        .await
        .is_err());
    for token in ["", "a.b", "a.b.c.d", "a.b.c", &"x".repeat(12289)] {
        assert!(f.app.0.google.verify(token, CLIENT_ID).await.is_err());
    }
    let mut c = claims("skew");
    c["iss"] = json!("accounts.google.com");
    c["exp"] = json!(now - 30);
    c["iat"] = json!(now - 3600);
    assert!(f
        .app
        .0
        .google
        .verify(&signed(json!({"alg":"RS256","kid":"one"}), c), CLIENT_ID)
        .await
        .is_ok());
    let mut c = claims("skew");
    c["iat"] = json!(now + 30);
    assert!(f
        .app
        .0
        .google
        .verify(&signed(json!({"alg":"RS256","kid":"one"}), c), CLIENT_ID)
        .await
        .is_ok());
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn jwks_cache_rotation_throttle_expiry_and_failure() {
    let f = Fixture::new(true).await;
    let v = &f.app.0.google;
    v.verify(&id_token("one"), CLIENT_ID).await.unwrap();
    v.verify(&id_token("two"), CLIENT_ID).await.unwrap();
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 1);
    assert!(
        v.cache
            .lock()
            .await
            .expires
            .unwrap()
            .duration_since(Instant::now())
            .as_secs()
            <= 3590
    );
    let unknown = signed(json!({"alg":"RS256","kid":"two"}), claims("one"));
    assert!(v.verify(&unknown, CLIENT_ID).await.is_err());
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 1);
    *f.mock.body.write().await = jwks("two");
    f.allow_refresh().await;
    v.verify(&unknown, CLIENT_ID).await.unwrap();
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 2);
    // Unknown-kid floods do not cause repeated network refreshes.
    for i in 0..10 {
        let t = signed(
            json!({"alg":"RS256","kid":format!("unknown-{i}")}),
            claims("one"),
        );
        assert!(v.verify(&t, CLIENT_ID).await.is_err());
    }
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 2);
    v.cache.lock().await.expires = Some(Instant::now() - Duration::from_secs(1));
    f.allow_refresh().await;
    f.mock.fail.store(1, Ordering::SeqCst);
    assert_eq!(
        v.verify(&unknown, CLIENT_ID).await.unwrap_err().0.code,
        ErrorCode::Unavailable
    );
    assert_eq!(
        v.verify(&unknown, CLIENT_ID).await.unwrap_err().0.code,
        ErrorCode::Unavailable
    );
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 3);
    f.mock.fail.store(0, Ordering::SeqCst);
    f.allow_refresh().await;
    v.verify(&unknown, CLIENT_ID).await.unwrap();
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 4);
}

#[test]
fn cache_control_bounds() {
    for (value, age, expected) in [
        ("public, max-age=120", "20", 100),
        ("MAX-AGE=\"3600\"", "0", 3600),
        ("max-age=1", "5", 0),
        ("max-age=999999", "0", 86400),
        ("no-cache, max-age=3600", "0", 0),
        ("no-store, max-age=3600", "0", 0),
        ("max-age=oops", "0", 300),
    ] {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("cache-control", value.parse().unwrap());
        h.insert("age", age.parse().unwrap());
        assert_eq!(cache_ttl(&h), Duration::from_secs(expected));
    }
}

#[tokio::test]
async fn signup_session_ticket_hash_replay_collision_and_subject_not_email() {
    let f = Fixture::new(true).await;
    let ticket = f.ticket("google-one").await;
    assert!((state::now() + 599_000..=state::now() + 600_000).contains(&ticket.expires_at));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM oauth_signup_tickets")
        .fetch_one(&f.app.0.pool)
        .await
        .unwrap();
    assert_eq!(stored, state::hash(ticket.ticket.as_bytes()));
    assert_ne!(stored, ticket.ticket.as_bytes());
    let r = f.complete(&ticket.ticket, "  Alice  ").await;
    assert_eq!(r.status(), 200);
    let alice: AuthResponse = r.json().await.unwrap();
    assert_eq!(alice.user.user.username, "alice");
    error(
        f.complete(&ticket.ticket, "other_name").await,
        401,
        ErrorCode::InvalidSignupTicket,
    )
    .await;
    match f.google("google-one").await {
        GoogleAuthResponse::Session(s) => {
            assert_eq!(s.user, alice.user);
            assert_ne!(s.token, alice.token);
        }
        _ => panic!("expected session"),
    }
    // Both tokens carry the same email; only Google's sub identifies an account.
    let second = f.ticket("google-two").await;
    error(
        f.complete(&second.ticket, "ALICE").await,
        409,
        ErrorCode::UsernameTaken,
    )
    .await;
    let bob: AuthResponse = f
        .complete(&second.ticket, "bob")
        .await
        .json()
        .await
        .unwrap();
    assert_ne!(alice.user.user.id, bob.user.user.id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM oauth_signup_tickets")
            .fetch_one(&f.app.0.pool)
            .await
            .unwrap(),
        0
    );
    let hash: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=? LIMIT 1")
            .bind(id(alice.user.user.id).unwrap())
            .fetch_one(&f.app.0.pool)
            .await
            .unwrap();
    assert_eq!(hash.len(), 32);
}

#[tokio::test]
async fn tickets_expire_rotate_and_are_atomically_single_use() {
    let f = Fixture::new(true).await;
    let old = f.ticket("rotate").await;
    let new = f.ticket("rotate").await;
    error(
        f.complete(&old.ticket, "old_name").await,
        401,
        ErrorCode::InvalidSignupTicket,
    )
    .await;
    sqlx::query("UPDATE oauth_signup_tickets SET expires_at=?")
        .bind(state::now() as i64 - 1)
        .execute(&f.app.0.pool)
        .await
        .unwrap();
    error(
        f.complete(&new.ticket, "new_name").await,
        401,
        ErrorCode::InvalidSignupTicket,
    )
    .await;
    let ticket = f.ticket("race").await;
    let (a, b) = tokio::join!(
        f.complete(&ticket.ticket, "race_one"),
        f.complete(&ticket.ticket, "race_two")
    );
    assert!((a.status() == 200 && b.status() == 401) || (b.status() == 200 && a.status() == 401));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&f.app.0.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn link_unlink_and_password_recovery_rules() {
    let f = Fixture::new(true).await;
    let alice = f.signup("alice-google", "alice").await;
    let status: AuthMethodsResponse = f
        .call("GET", "/api/auth/methods", Some(&alice.token), json!(null))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        status,
        AuthMethodsResponse {
            has_password: false,
            google_linked: true
        }
    );
    error(
        f.call(
            "POST",
            "/api/auth/google/unlink",
            Some(&alice.token),
            json!({}),
        )
        .await,
        409,
        ErrorCode::LastLoginMethod,
    )
    .await;
    // Google-only accounts never authenticate using the server's dummy password.
    error(
        f.call(
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"username":"alice","password":"unused dummy credential"}),
        )
        .await,
        401,
        ErrorCode::InvalidCredentials,
    )
    .await;
    let r = f
        .call(
            "POST",
            "/api/auth/password",
            Some(&alice.token),
            json!({"current_password":null,"new_password":"my new long password"}),
        )
        .await;
    assert_eq!(r.status(), 200);
    let status: AuthMethodsResponse = r.json().await.unwrap();
    assert!(status.has_password && status.google_linked);
    let r = f
        .call(
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"username":"alice","password":"my new long password"}),
        )
        .await;
    assert_eq!(r.status(), 200);
    error(
        f.call(
            "POST",
            "/api/auth/password",
            Some(&alice.token),
            json!({"current_password":"wrong","new_password":"another long password"}),
        )
        .await,
        401,
        ErrorCode::InvalidCredentials,
    )
    .await;
    error(
        f.call(
            "POST",
            "/api/auth/password",
            Some(&alice.token),
            json!({"current_password":null,"new_password":"another long password"}),
        )
        .await,
        401,
        ErrorCode::InvalidCredentials,
    )
    .await;
    let r = f.call("POST", "/api/auth/password", Some(&alice.token), json!({"current_password":"my new long password","new_password":"another long password"})).await;
    assert_eq!(r.status(), 200);
    let r = f
        .call(
            "POST",
            "/api/auth/google/unlink",
            Some(&alice.token),
            json!({}),
        )
        .await;
    assert_eq!(r.status(), 200);
    let status: AuthMethodsResponse = r.json().await.unwrap();
    assert!(status.has_password && !status.google_linked);
    // Unlink is idempotent, and the same Google identity can be linked again.
    assert_eq!(
        f.call(
            "POST",
            "/api/auth/google/unlink",
            Some(&alice.token),
            json!({})
        )
        .await
        .status(),
        200
    );
    let r = f
        .call(
            "POST",
            "/api/auth/google/link",
            Some(&alice.token),
            json!({"id_token":id_token("alice-google")}),
        )
        .await;
    assert_eq!(r.status(), 200);
    let bob = f.register("bob").await;
    error(
        f.call(
            "POST",
            "/api/auth/google/link",
            Some(&bob.token),
            json!({"id_token":id_token("alice-google")}),
        )
        .await,
        409,
        ErrorCode::IdentityAlreadyLinked,
    )
    .await;
    error(
        f.call(
            "POST",
            "/api/auth/google/link",
            Some(&alice.token),
            json!({"id_token":id_token("other-google")}),
        )
        .await,
        409,
        ErrorCode::IdentityAlreadyLinked,
    )
    .await;
    assert_eq!(
        f.call(
            "POST",
            "/api/auth/google/link",
            Some(&alice.token),
            json!({"id_token":id_token("alice-google")})
        )
        .await
        .status(),
        200
    );
    error(
        f.call(
            "POST",
            "/api/auth/google/link",
            None,
            json!({"id_token":id_token("unused")}),
        )
        .await,
        401,
        ErrorCode::Unauthenticated,
    )
    .await;
    error(
        f.call("POST", "/api/auth/google/unlink", None, json!({}))
            .await,
        401,
        ErrorCode::Unauthenticated,
    )
    .await;
    let unused = f.ticket("bob-google").await;
    assert_eq!(
        f.call(
            "POST",
            "/api/auth/google/link",
            Some(&bob.token),
            json!({"id_token":id_token("bob-google")})
        )
        .await
        .status(),
        200
    );
    error(
        f.complete(&unused.ticket, "surprise").await,
        401,
        ErrorCode::InvalidSignupTicket,
    )
    .await;
}

#[tokio::test]
async fn disabled_meta_and_routes() {
    let f = Fixture::new(false).await;
    for path in ["/api/meta", "/api/v1/meta"] {
        let r = f.call("GET", path, None, json!(null)).await;
        assert_eq!(r.status(), 200);
        let meta: MetaResponse = r.json().await.unwrap();
        assert_eq!(meta.google_client_id, None);
        assert_eq!(meta.protocol_version, PROTOCOL_VERSION);
    }
    for path in [
        "/api/auth/google",
        "/api/v1/auth/google/complete",
        "/api/auth/google/link",
        "/api/auth/google/unlink",
    ] {
        error(
            f.call("POST", path, None, json!({})).await,
            404,
            ErrorCode::FeatureDisabled,
        )
        .await;
    }
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 0);
    let enabled = Fixture::new(true).await;
    let meta: MetaResponse = enabled
        .call("GET", "/api/meta", None, json!(null))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(meta.google_client_id.as_deref(), Some(CLIENT_ID));
    error(
        enabled
            .call(
                "POST",
                "/api/auth/google",
                None,
                json!({"id_token":"broken"}),
            )
            .await,
        401,
        ErrorCode::InvalidCredentials,
    )
    .await;
}

#[tokio::test]
async fn google_endpoints_share_login_rate_limit_and_body_bound() {
    let f = Fixture::new(true).await;
    for _ in 0..5 {
        error(
            f.raw(
                "POST",
                "/api/auth/google",
                None,
                json!({"id_token":"broken"}),
            )
            .await,
            401,
            ErrorCode::InvalidCredentials,
        )
        .await;
    }
    let r = f
        .raw(
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"username":"nobody","password":"not a real password"}),
        )
        .await;
    assert_eq!(r.status(), 429);
    assert!(r.headers().get("retry-after").is_some());
    let e: ApiError = r.json().await.unwrap();
    assert_eq!(e.code, ErrorCode::RateLimited);
    assert!(e.retry_after_ms.unwrap() > 0);
    for path in [
        "/api/auth/google/complete",
        "/api/auth/google/link",
        "/api/auth/google/unlink",
    ] {
        error(
            f.raw("POST", path, None, json!({})).await,
            429,
            ErrorCode::RateLimited,
        )
        .await;
    }
    error(
        f.call(
            "POST",
            "/api/auth/google",
            None,
            json!({"id_token":"x".repeat(17000)}),
        )
        .await,
        413,
        ErrorCode::PayloadTooLarge,
    )
    .await;
}

#[tokio::test]
async fn existing_google_login_obeys_session_cap_and_ticket_table_is_bounded() {
    let f = Fixture::new(true).await;
    let alice = f.signup("session-cap", "alice").await;
    for _ in 0..4 {
        assert!(matches!(
            f.google("session-cap").await,
            GoogleAuthResponse::Session(_)
        ));
    }
    error(
        f.call(
            "POST",
            "/api/auth/google",
            None,
            json!({"id_token":id_token("session-cap")}),
        )
        .await,
        409,
        ErrorCode::LimitExceeded,
    )
    .await;
    // The cap is account-wide, including password sessions.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id=?")
            .bind(id(alice.user.user.id).unwrap())
            .fetch_one(&f.app.0.pool)
            .await
            .unwrap(),
        5
    );
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO oauth_signup_tickets SELECT randomblob(32), 'bounded-'||x, ? FROM n")
        .bind((state::now()+600_000) as i64).execute(&f.app.0.pool).await.unwrap();
    error(
        f.call(
            "POST",
            "/api/auth/google",
            None,
            json!({"id_token":id_token("over-cap")}),
        )
        .await,
        409,
        ErrorCode::LimitExceeded,
    )
    .await;
}

#[tokio::test]
async fn malformed_or_oversized_jwks_fails_closed_and_backs_off() {
    let f = Fixture::new(true).await;
    for body in [
        json!({"keys":[]}),
        json!({"keys":[{"kty":"RSA","kid":"one","n":"bad","e":"AQAB"}]}),
        json!({"keys":vec![jwks("one")["keys"][0].clone();33]}),
        json!({"padding":"x".repeat(65537)}),
    ] {
        *f.mock.body.write().await = body;
        f.allow_refresh().await;
        let v = &f.app.0.google;
        assert_eq!(
            v.verify(&id_token("test"), CLIENT_ID)
                .await
                .unwrap_err()
                .0
                .code,
            ErrorCode::Unavailable
        );
        let calls = f.mock.calls.load(Ordering::SeqCst);
        assert_eq!(
            v.verify(&id_token("test"), CLIENT_ID)
                .await
                .unwrap_err()
                .0
                .code,
            ErrorCode::Unavailable
        );
        assert_eq!(f.mock.calls.load(Ordering::SeqCst), calls);
    }
    *f.mock.body.write().await = jwks("one");
    f.allow_refresh().await;
    f.app
        .0
        .google
        .verify(&id_token("test"), CLIENT_ID)
        .await
        .unwrap();
}

#[tokio::test]
async fn disabled_google_still_allows_session_password_setup_and_methods_are_strict() {
    let f = Fixture::new(false).await;
    // Model a persisted Google-only account after the owner disables Google.
    sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(1,'alice','','online',0)").execute(&f.app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO oauth_identities VALUES('google','disabled-sub',1)")
        .execute(&f.app.0.pool)
        .await
        .unwrap();
    let token = state::token();
    sqlx::query("INSERT INTO sessions VALUES(?,1,?,?,?)")
        .bind(state::hash(token.as_bytes()))
        .bind(state::now() as i64)
        .bind((state::now() + 60_000) as i64)
        .bind(state::now() as i64)
        .execute(&f.app.0.pool)
        .await
        .unwrap();
    let r = f
        .call("GET", "/api/auth/methods", Some(&token), json!(null))
        .await;
    let status: AuthMethodsResponse = r.json().await.unwrap();
    assert!(!status.has_password && status.google_linked);
    let r = f
        .call(
            "POST",
            "/api/auth/password",
            Some(&token),
            json!({"new_password":"new password for recovery","current_password":null}),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<AuthMethodsResponse>().await.unwrap().has_password);
    assert_eq!(f.mock.calls.load(Ordering::SeqCst), 0);
    for (method, path) in [
        ("POST", "/api/meta"),
        ("POST", "/api/auth/methods"),
        ("GET", "/api/auth/password"),
    ] {
        error(
            f.call(method, path, Some(&token), json!({})).await,
            404,
            ErrorCode::NotFound,
        )
        .await;
    }
}

#[tokio::test]
async fn web_redirect_pkce_state_cookie_signup_and_existing_session() {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        routing::post,
    };
    use std::collections::HashMap;
    use tower::ServiceExt;
    let f = Fixture::new(true).await;
    let current = Arc::new(RwLock::new(id_token("web-subject")));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<HashMap<String, String>>(8);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let token_url = format!("http://{}/token", listener.local_addr().unwrap());
    let mock_current = current.clone();
    let upstream = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/token",
                post(
                    move |axum::Form(form): axum::Form<HashMap<String, String>>| {
                        let current = mock_current.clone();
                        let tx = tx.clone();
                        async move {
                            tx.send(form).await.unwrap();
                            axum::Json(json!({"id_token":current.read().await.clone()}))
                        }
                    },
                ),
            ),
        )
        .await
        .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = f.app.0.config.clone();
    config.data_dir = dir.path().into();
    config.public_url = "https://sticktalk.test".into();
    config.google_client_id = None;
    config.browser = crate::browser::BrowserOptions {
        google_web_client_id: Some(CLIENT_ID.into()),
        google_web_client_secret: Some("test-only-secret".into()),
        mock_google_token_url: Some(token_url),
        ..Default::default()
    };
    let app = App::open_with_verifier(
        config,
        Verifier::with_url(f.app.0.google.url.clone()).unwrap(),
    )
    .await
    .unwrap();
    let router = crate::http::router(app.clone());
    let start = || {
        Request::builder()
            .uri("/api/v1/browser/google/start")
            .body(Body::empty())
            .unwrap()
    };
    let cookie_pairs = |r: &axum::response::Response| {
        r.headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join("; ")
    };
    let r = router.clone().oneshot(start()).await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let state_cookie = cookie_pairs(&r);
    assert!(r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("SameSite=Lax"));
    assert!(r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("HttpOnly"));
    let authorization = reqwest::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    let params: HashMap<_, _> = authorization.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(
        params["redirect_uri"],
        "https://sticktalk.test/api/v1/browser/google/callback"
    );
    let callback_path = format!(
        "/api/v1/browser/google/callback?state={}&code=mock-code",
        params["state"]
    );
    let callback = |cookie: &str| {
        Request::builder()
            .uri(&callback_path)
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(callback("__Host-sticktalk_oauth=wrong"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let r = router
        .clone()
        .oneshot(callback(&state_cookie))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()["location"], "/?google=signup");
    let signup_cookies = cookie_pairs(&r)
        .split("; ")
        .filter(|s| !s.ends_with('='))
        .collect::<Vec<_>>()
        .join("; ");
    assert!(signup_cookies.contains("__Host-sticktalk_signup="));
    assert!(!signup_cookies.contains("__Host-sticktalk_session="));
    let form = rx.recv().await.unwrap();
    assert_eq!(form["code"], "mock-code");
    assert_eq!(form["client_secret"], "test-only-secret");
    assert_eq!(
        params["code_challenge"],
        URL_SAFE_NO_PAD.encode(state::hash(form["code_verifier"].as_bytes()))
    );
    assert_eq!(
        router
            .clone()
            .oneshot(callback(&state_cookie))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let csrf = signup_cookies
        .split("; ")
        .find_map(|s| s.strip_prefix("__Host-sticktalk_csrf="))
        .unwrap();
    let complete = || {
        Request::builder()
            .method("POST")
            .uri("/api/v1/browser/session/google/complete")
            .header("origin", "https://sticktalk.test")
            .header("cookie", &signup_cookies)
            .header("x-sticktalk-csrf", csrf)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"ticket":"","username":"web_user"}).to_string(),
            ))
            .unwrap()
    };
    let r = router.clone().oneshot(complete()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let session_cookie = cookie_pairs(&r);
    let body: Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 16384).await.unwrap()).unwrap();
    assert!(body.get("token").is_none());
    assert_eq!(body["user"]["user"]["username"], "web_user");
    assert!(session_cookie.contains("__Host-sticktalk_session="));
    assert_eq!(
        router.clone().oneshot(complete()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    app.0.core.lock().await.limits.clear();
    let r = router.clone().oneshot(start()).await.unwrap();
    let cookie = cookie_pairs(&r);
    let url = reqwest::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let r = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/browser/google/callback?state={state}&code=another-code"
                ))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()["location"], "/");
    assert!(cookie_pairs(&r).contains("__Host-sticktalk_session="));
    let _ = rx.recv().await;
    app.0.core.lock().await.limits.clear();
    let r = router.clone().oneshot(start()).await.unwrap();
    let cookie = cookie_pairs(&r);
    let url = reqwest::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let mut wrong = claims("wrong-aud");
    wrong["aud"] = json!("other-client");
    *current.write().await = signed(json!({"alg":"RS256","kid":"one"}), wrong);
    let r = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/browser/google/callback?state={state}&code=wrong-audience"
                ))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    app.0.core.lock().await.limits.clear();
    let r = router.clone().oneshot(start()).await.unwrap();
    let cookie = cookie_pairs(&r);
    let url = reqwest::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    sqlx::query("UPDATE web_oauth_states SET expires_at=0")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let r = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/browser/google/callback?state={state}&code=expired"
                ))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    app.shutdown();
    upstream.abort();
}
