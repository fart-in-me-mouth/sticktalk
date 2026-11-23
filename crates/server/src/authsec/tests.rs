//! Server tests for docs/features/authsec.md: every route, permission and limit,
//! and one test per attack (replay, brute force, wrong origin and RP ID, expired
//! challenge, cloned authenticator, reused links and tickets, session fixation).
//! Everything is loopback-only; the SMTP server and authenticator are fakes.
use super::*;
use axum::http::{Method, StatusCode};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use clap::Parser;
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost";

async fn app_with(extra: &[&str]) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_string_lossy().to_string();
    let mut args = vec![
        "server",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        &data,
        "--public-url",
        ORIGIN,
        "--min-free-disk-gb",
        "0",
        "--voice-bind-ip",
        "127.0.0.1",
        "--voice-advertised-ip",
        "127.0.0.1",
    ];
    args.extend_from_slice(extra);
    let app = App::open(crate::Config::parse_from(args)).await.unwrap();
    (dir, app)
}

/// A user with password "pw" and one session; returns its bearer token.
async fn user(app: &App, n: i64, name: &str) -> String {
    let hash = if n == 1 {
        crate::http::password_hash("pw".into()).unwrap()
    } else {
        app.0.dummy_hash.clone()
    };
    sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,?,'online',?)")
        .bind(n)
        .bind(name)
        .bind(hash)
        .bind(state::now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    session_for(app, n).await
}
async fn session_for(app: &App, n: i64) -> String {
    let token = state::token();
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(state::hash(token.as_bytes()))
        .bind(n)
        .bind(state::now() as i64)
        .bind((state::now() + 86_400_000) as i64)
        .bind(state::now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
/// Forget rate-limit buckets between steps that are not about limits.
async fn fresh_limits(app: &App) {
    app.0.core.lock().await.limits.clear();
}
async fn proof(app: &App, token: &str, op: AccountOperation) -> String {
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::reauth::issue(&mut c, &state::hash(token.as_bytes()), op)
        .await
        .unwrap()
        .proof
}
fn csrf(token: &str) -> String {
    B64.encode(
        ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, token.as_bytes()),
            b"sticktalk-browser-csrf-v1",
        )
        .as_ref(),
    )
}

struct Call {
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Value>,
}
fn call(method: Method, path: &str) -> Call {
    Call {
        method,
        path: path.into(),
        headers: vec![],
        body: None,
    }
}
impl Call {
    fn bearer(mut self, token: &str) -> Self {
        self.headers
            .push(("authorization".into(), format!("Bearer {token}")));
        self
    }
    fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    fn json(mut self, v: Value) -> Self {
        self.body = Some(v);
        self
    }
    /// A browser request carrying these cookies, from the app's own origin.
    fn browser(mut self, cookies: &str, session: Option<&str>) -> Self {
        self.headers.push(("origin".into(), ORIGIN.into()));
        if !cookies.is_empty() {
            self.headers.push(("cookie".into(), cookies.into()));
        }
        if let Some(token) = session {
            self.headers.push(("x-sticktalk-csrf".into(), csrf(token)));
        }
        self
    }
    async fn send(self, app: &App) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut b = axum::http::Request::builder()
            .method(self.method)
            .uri(&self.path);
        for (k, v) in &self.headers {
            b = b.header(k, v);
        }
        let body = match self.body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&v).unwrap())
            }
            None => Body::empty(),
        };
        let r = crate::http::router(app.clone())
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let bytes = axum::body::to_bytes(r.into_body(), 64 << 20).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, value)
    }
}
fn set_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .filter_map(|v| v.split_once('='))
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .next_back()
}
async fn sessions_of(app: &App, user: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id=?")
        .bind(user)
        .fetch_one(&app.0.pool)
        .await
        .unwrap()
}
async fn user_of(app: &App, token: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash=?")
        .bind(state::hash(token.as_bytes()))
        .fetch_optional(&app.0.pool)
        .await
        .unwrap()
}

// ----- A software authenticator (ES256, "none" attestation) ----------------

struct Authenticator {
    key: EcdsaKeyPair,
    credential: Vec<u8>,
    handle: Vec<u8>,
    counter: u32,
}
fn cbor_head(major: u8, n: usize, out: &mut Vec<u8>) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=255 => out.extend([m | 24, n as u8]),
        _ => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
    }
}
fn cbor_text(s: &str, out: &mut Vec<u8>) {
    cbor_head(3, s.len(), out);
    out.extend(s.as_bytes());
}
fn cbor_bytes(b: &[u8], out: &mut Vec<u8>) {
    cbor_head(2, b.len(), out);
    out.extend(b);
}
fn rp_hash(rp_id: &str) -> Vec<u8> {
    state::hash(rp_id.as_bytes())
}
fn challenge(options: &str) -> String {
    let v: Value = serde_json::from_str(options).unwrap();
    v["publicKey"]["challenge"].as_str().unwrap().to_owned()
}
fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"type": kind, "challenge": challenge, "origin": origin, "crossOrigin": false}),
    )
    .unwrap()
}
impl Authenticator {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        Self {
            key: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap(),
            credential: state::random(16),
            handle: vec![],
            counter: 0,
        }
    }
    fn register(&mut self, options: &str, origin: &str, rp_id: &str) -> String {
        let v: Value = serde_json::from_str(options).unwrap();
        self.handle = B64
            .decode(v["publicKey"]["user"]["id"].as_str().unwrap())
            .unwrap();
        let public = self.key.public_key().as_ref();
        let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21];
        cbor_bytes(&public[1..33], &mut cose);
        cose.push(0x22);
        cbor_bytes(&public[33..65], &mut cose);
        let mut auth = rp_hash(rp_id);
        auth.push(0x01 | 0x04 | 0x40); // UP, UV, attested credential data
        auth.extend(self.counter.to_be_bytes());
        auth.extend([0u8; 16]);
        auth.extend((self.credential.len() as u16).to_be_bytes());
        auth.extend(&self.credential);
        auth.extend(cose);
        let mut att = vec![0xa3];
        cbor_text("fmt", &mut att);
        cbor_text("none", &mut att);
        cbor_text("attStmt", &mut att);
        att.push(0xa0);
        cbor_text("authData", &mut att);
        cbor_bytes(&auth, &mut att);
        let id = B64.encode(&self.credential);
        json!({
            "id": id, "rawId": id, "type": "public-key", "extensions": {},
            "response": {
                "attestationObject": B64.encode(att),
                "clientDataJSON": B64.encode(client_data("webauthn.create", &challenge(options), origin)),
            },
        })
        .to_string()
    }
    fn assert_with(
        &self,
        options: &str,
        origin: &str,
        rp_id: &str,
        flags: u8,
        counter: u32,
    ) -> String {
        let mut auth = rp_hash(rp_id);
        auth.push(flags);
        auth.extend(counter.to_be_bytes());
        let data = client_data("webauthn.get", &challenge(options), origin);
        let mut signed = auth.clone();
        signed.extend(state::hash(&data));
        let signature = self.key.sign(&SystemRandom::new(), &signed).unwrap();
        let id = B64.encode(&self.credential);
        json!({
            "id": id, "rawId": id, "type": "public-key", "extensions": {},
            "response": {
                "authenticatorData": B64.encode(auth),
                "clientDataJSON": B64.encode(data),
                "signature": B64.encode(signature.as_ref()),
                "userHandle": B64.encode(&self.handle),
            },
        })
        .to_string()
    }
    fn assert(&mut self, options: &str) -> String {
        self.counter += 1;
        self.assert_with(options, ORIGIN, "localhost", 0x05, self.counter)
    }
}

async fn register_passkey(app: &App, token: &str, a: &mut Authenticator) -> Value {
    fresh_limits(app).await;
    let p = proof(app, token, AccountOperation::Passkeys).await;
    let (s, _, start) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"name": "Laptop"}))
        .send(app)
        .await;
    assert_eq!(s, 200, "{start}");
    let options = start["public_key"].as_str().unwrap();
    // Discoverable credentials, user verification required.
    let o: Value = serde_json::from_str(options).unwrap();
    assert_eq!(
        o["publicKey"]["authenticatorSelection"]["residentKey"],
        "required"
    );
    assert_eq!(
        o["publicKey"]["authenticatorSelection"]["userVerification"],
        "required"
    );
    assert_eq!(o["publicKey"]["rp"]["id"], "localhost");
    let credential = a.register(options, ORIGIN, "localhost");
    let (s, _, done) = call(Method::POST, "/api/v1/users/@me/passkeys/register/finish")
        .bearer(token)
        .json(json!({"challenge_id": start["challenge_id"], "credential": credential}))
        .send(app)
        .await;
    assert_eq!(s, 200, "{done}");
    done
}
async fn passkey_options(app: &App) -> (String, String) {
    fresh_limits(app).await;
    let (s, _, start) = call(Method::POST, "/api/v1/auth/passkey/start")
        .send(app)
        .await;
    assert_eq!(s, 200, "{start}");
    (
        start["challenge_id"].as_str().unwrap().to_owned(),
        start["public_key"].as_str().unwrap().to_owned(),
    )
}
async fn passkey_finish(app: &App, challenge: &str, credential: String) -> (StatusCode, Value) {
    let (s, _, v) = call(Method::POST, "/api/v1/auth/passkey/finish")
        .json(json!({"challenge_id": challenge, "credential": credential}))
        .send(app)
        .await;
    (s, v)
}

#[tokio::test]
async fn passkey_registration_needs_reauth_and_is_bound_to_its_session() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let other = session_for(&app, 1).await;
    // Without fresh proof of an existing login method: refused.
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(&token)
        .json(json!({"name": "Laptop"}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    // A proof for another operation does not count.
    fresh_limits(&app).await;
    let wrong = proof(&app, &token, AccountOperation::Email).await;
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(&token)
        .header("x-sticktalk-reauth", &wrong)
        .json(json!({"name": "Laptop"}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    let p = proof(&app, &token, AccountOperation::Passkeys).await;
    let (_, _, start) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"name": "Laptop"}))
        .send(&app)
        .await;
    let mut a = Authenticator::new();
    let credential = a.register(start["public_key"].as_str().unwrap(), ORIGIN, "localhost");
    // Another session of the same account cannot finish this ceremony...
    fresh_limits(&app).await;
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/finish")
        .bearer(&other)
        .json(json!({"challenge_id": start["challenge_id"], "credential": credential}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    // ...and the attempt spent it: one use only.
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/finish")
        .bearer(&token)
        .json(json!({"challenge_id": start["challenge_id"], "credential": credential}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    // The proof itself was consumed by start.
    fresh_limits(&app).await;
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"name": "Laptop"}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
}

#[tokio::test]
async fn passkey_registration_rejects_wrong_origin_and_rp_id() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    for (origin, rp) in [
        ("https://evil.example", "localhost"),
        (ORIGIN, "evil.example"),
    ] {
        let p = proof(&app, &token, AccountOperation::Passkeys).await;
        let (_, _, start) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
            .bearer(&token)
            .header("x-sticktalk-reauth", &p)
            .json(json!({"name": "Laptop"}))
            .send(&app)
            .await;
        let mut a = Authenticator::new();
        let credential = a.register(start["public_key"].as_str().unwrap(), origin, rp);
        let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/finish")
            .bearer(&token)
            .json(json!({"challenge_id": start["challenge_id"], "credential": credential}))
            .send(&app)
            .await;
        assert_eq!(s, 401, "{origin} {rp}");
    }
    let (_, _, list) = call(Method::GET, "/api/v1/users/@me/passkeys")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(list, json!([]));
}

#[tokio::test]
async fn passkey_sign_in_mints_a_fresh_session_and_updates_last_used() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let mut a = Authenticator::new();
    let summary = register_passkey(&app, &token, &mut a).await;
    assert_eq!(summary["name"], "Laptop");
    assert!(summary["last_used_at"].is_null());
    let before = sessions_of(&app, 1).await;
    let (id, options) = passkey_options(&app).await;
    let o: Value = serde_json::from_str(&options).unwrap();
    // Discoverable: no allowCredentials list leaks which credentials exist.
    assert!(o["publicKey"]["allowCredentials"]
        .as_array()
        .is_none_or(|v| v.is_empty()));
    assert!(o.get("mediation").is_none());
    let (s, auth) = passkey_finish(&app, &id, a.assert(&options)).await;
    assert_eq!(s, 200, "{auth}");
    let fresh = auth["token"].as_str().unwrap();
    assert_ne!(fresh, token);
    assert_eq!(user_of(&app, fresh).await, Some(1));
    assert_eq!(sessions_of(&app, 1).await, before + 1);
    let (_, _, list) = call(Method::GET, "/api/v1/users/@me/passkeys")
        .bearer(&token)
        .send(&app)
        .await;
    assert!(list[0]["last_used_at"].as_u64().is_some());
    // The per-account session limit still applies to passkey sign-in.
    for _ in sessions_of(&app, 1).await..MAX_SESSIONS_PER_USER as i64 {
        session_for(&app, 1).await;
    }
    let (id, options) = passkey_options(&app).await;
    let (s, v) = passkey_finish(&app, &id, a.assert(&options)).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("limit_exceeded"))
    );
}

#[tokio::test]
async fn passkey_sign_in_rejects_replay_wrong_origin_rp_expiry_clones_and_missing_uv() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let mut a = Authenticator::new();
    register_passkey(&app, &token, &mut a).await;

    // Replay 1: the same challenge cannot be answered twice.
    let (id, options) = passkey_options(&app).await;
    let assertion = a.assert(&options);
    assert_eq!(passkey_finish(&app, &id, assertion.clone()).await.0, 200);
    assert_eq!(passkey_finish(&app, &id, assertion.clone()).await.0, 401);
    // Replay 2: a captured assertion does not answer a new challenge.
    let (id, _) = passkey_options(&app).await;
    assert_eq!(passkey_finish(&app, &id, assertion).await.0, 401);

    // Wrong origin (phishing site) and wrong RP ID.
    let (id, options) = passkey_options(&app).await;
    a.counter += 1;
    let bad = a.assert_with(
        &options,
        "https://evil.example",
        "localhost",
        0x05,
        a.counter,
    );
    assert_eq!(passkey_finish(&app, &id, bad).await.0, 401);
    let (id, options) = passkey_options(&app).await;
    a.counter += 1;
    let bad = a.assert_with(&options, ORIGIN, "evil.example", 0x05, a.counter);
    assert_eq!(passkey_finish(&app, &id, bad).await.0, 401);

    // Passwordless sign-in requires user verification (UV), not just presence.
    let (id, options) = passkey_options(&app).await;
    a.counter += 1;
    let bad = a.assert_with(&options, ORIGIN, "localhost", 0x01, a.counter);
    assert_eq!(passkey_finish(&app, &id, bad).await.0, 401);

    // A cloned authenticator: its counter does not advance past the stored one.
    let (id, options) = passkey_options(&app).await;
    let stale = a.assert_with(&options, ORIGIN, "localhost", 0x05, 1);
    assert_eq!(passkey_finish(&app, &id, stale).await.0, 401);

    // An expired challenge.
    let (id, options) = passkey_options(&app).await;
    app.0.authsec.passkeys.expire_all();
    assert_eq!(passkey_finish(&app, &id, a.assert(&options)).await.0, 401);

    // A credential nobody registered.
    let mut stranger = Authenticator::new();
    stranger.handle = a.handle.clone();
    let (id, options) = passkey_options(&app).await;
    assert_eq!(
        passkey_finish(&app, &id, stranger.assert(&options)).await.0,
        401
    );

    // And the real one still works afterwards.
    let (id, options) = passkey_options(&app).await;
    assert_eq!(passkey_finish(&app, &id, a.assert(&options)).await.0, 200);
}

#[tokio::test]
async fn passkey_sign_in_is_rate_limited_per_ip() {
    let (_d, app) = app_with(&[]).await;
    let mut limited = false;
    for _ in 0..12 {
        let (s, _, _) = call(Method::POST, "/api/v1/auth/passkey/finish")
            .json(json!({"challenge_id": "guess", "credential": "{}"}))
            .send(&app)
            .await;
        if s == 429 {
            limited = true;
            break;
        }
        assert_eq!(s, 401);
    }
    assert!(limited, "brute force must hit the limiter");
}

#[tokio::test]
async fn passkeys_rename_and_removal_are_owner_only_and_removal_needs_reauth() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let bob = user(&app, 2, "bob").await;
    let mut a = Authenticator::new();
    let pk = register_passkey(&app, &token, &mut a).await;
    let path = format!("/api/v1/users/@me/passkeys/{}", pk["id"].as_str().unwrap());
    let (s, _, _) = call(Method::PATCH, &path)
        .bearer(&bob)
        .json(json!({"name": "Mine now"}))
        .send(&app)
        .await;
    assert_eq!(s, 404);
    let (s, _, list) = call(Method::PATCH, &path)
        .bearer(&token)
        .json(json!({"name": "  Work key  "}))
        .send(&app)
        .await;
    assert_eq!(
        (s, list[0]["name"].as_str()),
        (StatusCode::OK, Some("Work key"))
    );
    for bad in ["", "\u{7}", &"x".repeat(65)] {
        let (s, _, _) = call(Method::PATCH, &path)
            .bearer(&token)
            .json(json!({"name": bad}))
            .send(&app)
            .await;
        assert_eq!(s, 400);
    }
    let bob_proof = proof(&app, &bob, AccountOperation::Passkeys).await;
    let (s, _, _) = call(Method::DELETE, &path)
        .bearer(&bob)
        .header("x-sticktalk-reauth", &bob_proof)
        .send(&app)
        .await;
    assert_eq!(s, 404);
    let (s, _, _) = call(Method::DELETE, &path).bearer(&token).send(&app).await;
    assert_eq!(s, 401);
    let p = proof(&app, &token, AccountOperation::Passkeys).await;
    let (s, _, list) = call(Method::DELETE, &path)
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!((s, list), (StatusCode::OK, json!([])));
    // A removed passkey can no longer sign in.
    let (id, options) = passkey_options(&app).await;
    assert_eq!(passkey_finish(&app, &id, a.assert(&options)).await.0, 401);
}

#[tokio::test]
async fn passkey_limit_and_disabled_without_a_usable_rp_id() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    for _ in 0..passkey::MAX_PER_USER {
        register_passkey(&app, &token, &mut Authenticator::new()).await;
    }
    let p = proof(&app, &token, AccountOperation::Passkeys).await;
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/passkeys/register/start")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"name": "Eleventh"}))
        .send(&app)
        .await;
    assert_eq!(s, 409);
    // Plain HTTP on an IP address cannot host WebAuthn: the feature is off.
    let (_d2, ip) = {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_string_lossy().to_string();
        let app = App::open(crate::Config::parse_from([
            "server",
            "--bind",
            "127.0.0.1:0",
            "--data-dir",
            &data,
            "--public-url",
            "http://127.0.0.1:9",
            "--min-free-disk-gb",
            "0",
            "--voice-bind-ip",
            "127.0.0.1",
            "--voice-advertised-ip",
            "127.0.0.1",
        ]))
        .await
        .unwrap();
        (dir, app)
    };
    let (_, _, opts) = call(Method::GET, "/api/v1/auth/options").send(&ip).await;
    assert_eq!(opts["passkeys"], false);
    let (s, _, _) = call(Method::POST, "/api/v1/auth/passkey/start")
        .send(&ip)
        .await;
    assert_eq!(s, 404);
    // A self-hosted RP ID that does not cover the public host is a startup error.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_string_lossy().to_string();
    assert!(App::open(crate::Config::parse_from([
        "server",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        &data,
        "--public-url",
        "https://chat.example.com",
        "--webauthn-rp-id",
        "other.example",
        "--min-free-disk-gb",
        "0",
        "--voice-bind-ip",
        "127.0.0.1",
        "--voice-advertised-ip",
        "127.0.0.1",
    ]))
    .await
    .is_err());
    // ...while the parent domain of the public host is accepted.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_string_lossy().to_string();
    let parent = App::open(crate::Config::parse_from([
        "server",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        &data,
        "--public-url",
        "https://chat.example.com",
        "--webauthn-rp-id",
        "example.com",
        "--min-free-disk-gb",
        "0",
        "--voice-bind-ip",
        "127.0.0.1",
        "--voice-advertised-ip",
        "127.0.0.1",
    ]))
    .await
    .unwrap();
    let (_, _, start) = call(Method::POST, "/api/v1/auth/passkey/start")
        .send(&parent)
        .await;
    let o: Value = serde_json::from_str(start["public_key"].as_str().unwrap()).unwrap();
    assert_eq!(o["publicKey"]["rpId"], "example.com");
}

#[tokio::test]
async fn browser_passkey_sign_in_resists_session_fixation() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let attacker = user(&app, 2, "mallory").await;
    let mut a = Authenticator::new();
    register_passkey(&app, &token, &mut a).await;
    let (id, options) = passkey_options(&app).await;
    // The attacker planted their own session cookie in the victim's browser.
    let planted = format!(
        "__Host-sticktalk_session={attacker}; __Host-sticktalk_csrf={}",
        csrf(&attacker)
    );
    let (s, h, v) = call(Method::POST, "/api/v1/browser/session/passkey")
        .browser(&planted, Some(&attacker))
        .json(json!({"challenge_id": id, "credential": a.assert(&options)}))
        .send(&app)
        .await;
    assert_eq!(s, 200, "{v}");
    let issued = set_cookie(&h, "__Host-sticktalk_session").unwrap();
    assert_ne!(issued, attacker);
    assert_eq!(user_of(&app, &issued).await, Some(1));
    // The planted session was not upgraded to the victim's identity.
    assert_eq!(user_of(&app, &attacker).await, Some(2));
    assert!(
        v.get("token").is_none(),
        "the browser never sees a bearer token"
    );
    // A cross-origin page cannot use the browser route.
    let (id, options) = passkey_options(&app).await;
    let (s, _, _) = call(Method::POST, "/api/v1/browser/session/passkey")
        .header("origin", "https://evil.example")
        .json(json!({"challenge_id": id, "credential": a.assert(&options)}))
        .send(&app)
        .await;
    assert_eq!(s, 403);
}

// ----- QR code sign-in ---------------------------------------------------------

async fn serve(app: &App) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = crate::http::router(app.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}
type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn qr_socket(
    addr: std::net::SocketAddr,
    origin: &str,
) -> std::result::Result<Ws, tungstenite::Error> {
    use tungstenite::client::IntoClientRequest;
    let mut request = format!("ws://{addr}/api/v1/auth/qr/socket")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request.headers_mut().insert(
        "user-agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36".parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(ws, _)| ws)
}
async fn next(ws: &mut Ws) -> Value {
    use futures_util::StreamExt;
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .unwrap()
        {
            Some(Ok(tungstenite::Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(tungstenite::Message::Close(_))) | None => return Value::Null,
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("{e}"),
        }
    }
}
fn qr_token(code: &Value) -> String {
    code["url"]
        .as_str()
        .unwrap()
        .split("#qr=")
        .nth(1)
        .unwrap()
        .to_owned()
}
async fn qr(app: &App, token: &str, action: &str, qr: &str) -> (StatusCode, Value) {
    fresh_limits(app).await;
    let (s, _, v) = call(Method::POST, &format!("/api/v1/auth/qr/{action}"))
        .bearer(token)
        .json(json!({"token": qr}))
        .send(app)
        .await;
    (s, v)
}

#[tokio::test]
async fn qr_sign_in_needs_an_explicit_confirmation_and_grants_only_the_waiting_browser() {
    let (_d, app) = app_with(&[]).await;
    let phone = user(&app, 1, "alice").await;
    let other = user(&app, 2, "bob").await;
    let addr = serve(&app).await;
    let mut ws = qr_socket(addr, ORIGIN).await.unwrap();
    let code = next(&mut ws).await;
    assert_eq!(code["type"], "code");
    let size = code["size"].as_u64().unwrap();
    assert_eq!(code["modules"].as_str().unwrap().len() as u64, size * size);
    assert!(code["url"]
        .as_str()
        .unwrap()
        .starts_with("http://localhost/#qr="));
    let token = qr_token(&code);
    // The token alone grants nothing: it is not a ticket.
    let (s, _, _) = call(Method::POST, "/api/v1/auth/qr/redeem")
        .json(json!({"ticket": token}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    // Approving without first seeing the confirmation screen is refused.
    assert_eq!(qr(&app, &phone, "approve", &token).await.0, 404);
    // The confirming device is told who is asking.
    let (s, info) = qr(&app, &phone, "inspect", &token).await;
    assert_eq!(s, 200, "{info}");
    assert_eq!(info["device"], "Chrome on Windows");
    assert_eq!(info["ip"], "127.0.0.1");
    assert_eq!(
        info["same_network"], false,
        "oneshot calls have no peer address"
    );
    assert_eq!(next(&mut ws).await["type"], "scanned");
    // Another signed-in device cannot take over the code once it is opened.
    assert_eq!(qr(&app, &other, "inspect", &token).await.0, 404);
    assert_eq!(qr(&app, &other, "approve", &token).await.0, 404);
    assert_eq!(qr(&app, &phone, "approve", &token).await.0, 204);
    let approved = next(&mut ws).await;
    assert_eq!(approved["type"], "approved");
    let ticket = approved["ticket"].as_str().unwrap().to_owned();
    // The code is spent.
    assert_eq!(qr(&app, &phone, "inspect", &token).await.0, 404);
    // The waiting browser gets its own brand-new session, as a cookie.
    let (s, h, v) = call(Method::POST, "/api/v1/browser/session/qr")
        .browser("", None)
        .json(json!({"ticket": ticket}))
        .send(&app)
        .await;
    assert_eq!(s, 200, "{v}");
    let cookie = set_cookie(&h, "__Host-sticktalk_session").unwrap();
    assert_ne!(cookie, phone);
    assert_eq!(user_of(&app, &cookie).await, Some(1));
    // Replay: a ticket works once.
    let (s, _, _) = call(Method::POST, "/api/v1/auth/qr/redeem")
        .json(json!({"ticket": ticket}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
}

#[tokio::test]
async fn qr_codes_rotate_on_expiry_and_rejection_and_the_socket_ends() {
    let (_d, app) = app_with(&[]).await;
    let phone = user(&app, 1, "alice").await;
    let addr = serve(&app).await;
    let mut ws = qr_socket(addr, ORIGIN).await.unwrap();
    let first = qr_token(&next(&mut ws).await);
    assert_eq!(qr(&app, &phone, "inspect", &first).await.0, 200);
    assert_eq!(next(&mut ws).await["type"], "scanned");
    assert_eq!(qr(&app, &phone, "reject", &first).await.0, 204);
    assert_eq!(next(&mut ws).await["type"], "rejected");
    let second = next(&mut ws).await;
    assert_eq!(second["type"], "code");
    let second = qr_token(&second);
    assert_ne!(first, second);
    assert_eq!(qr(&app, &phone, "inspect", &first).await.0, 404);
    // Server-pushed refresh when a code expires (2 s under test), no polling.
    let started = std::time::Instant::now();
    let third = next(&mut ws).await;
    assert_eq!(third["type"], "code");
    assert!(started.elapsed() >= std::time::Duration::from_millis(1500));
    assert_eq!(
        qr(&app, &phone, "inspect", &second).await.0,
        404,
        "expired code"
    );
    // After the last code the socket says so and closes; state is released.
    assert_eq!(next(&mut ws).await["type"], "expired");
    assert_eq!(next(&mut ws).await, Value::Null);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(qr::token_for_test(&app).is_none());
}

#[tokio::test]
async fn qr_socket_refuses_other_origins_and_inspect_is_rate_limited() {
    let (_d, app) = app_with(&[]).await;
    let phone = user(&app, 1, "alice").await;
    let addr = serve(&app).await;
    assert!(qr_socket(addr, "https://evil.example").await.is_err());
    // Guessing codes runs into the per-account limiter.
    let mut limited = false;
    for _ in 0..12 {
        let (s, _, _) = call(Method::POST, "/api/v1/auth/qr/inspect")
            .bearer(&phone)
            .json(json!({"token": state::token()}))
            .send(&app)
            .await;
        if s == 429 {
            limited = true;
            break;
        }
        assert_eq!(s, 404);
    }
    assert!(limited);
    // Inspecting requires a signed-in session.
    let (s, _, _) = call(Method::POST, "/api/v1/auth/qr/inspect")
        .json(json!({"token": state::token()}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
}

#[tokio::test]
async fn qr_ticket_expires_and_disabled_accounts_cannot_approve() {
    let (_d, app) = app_with(&[]).await;
    let phone = user(&app, 1, "alice").await;
    let addr = serve(&app).await;
    let mut ws = qr_socket(addr, ORIGIN).await.unwrap();
    let token = qr_token(&next(&mut ws).await);
    qr(&app, &phone, "inspect", &token).await;
    next(&mut ws).await;
    qr(&app, &phone, "approve", &token).await;
    let ticket = next(&mut ws).await["ticket"].as_str().unwrap().to_owned();
    qr::expire_tickets(&app);
    let (s, _, _) = call(Method::POST, "/api/v1/auth/qr/redeem")
        .json(json!({"ticket": ticket}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    let mut ws = qr_socket(addr, ORIGIN).await.unwrap();
    let token = qr_token(&next(&mut ws).await);
    qr(&app, &phone, "inspect", &token).await;
    sqlx::query("INSERT INTO disabled_users VALUES(1)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(qr(&app, &phone, "approve", &token).await.0, 401);
}

#[test]
fn qr_matrix_and_device_labels() {
    let (size, modules) = qr::matrix("http://localhost/#qr=abc").unwrap();
    assert_eq!(modules.len() as u32, size * size);
    assert!(modules.bytes().all(|b| b == b'0' || b == b'1'));
    assert_eq!(device_label("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"), "Safari on iOS");
    assert_eq!(device_label("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36 Edg/140.0"), "Edge on Windows");
    assert_eq!(
        device_label("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"),
        "Firefox on Linux"
    );
    assert_eq!(device_label("Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36"), "Chrome on Android");
    assert_eq!(device_label(""), "Unknown device");
}

// ----- Email password reset ----------------------------------------------------

/// A minimal SMTP server on 127.0.0.1 that hands each message's DATA to a channel.
async fn mock_smtp() -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                write
                    .write_all(b"220 localhost ESMTP test\r\n")
                    .await
                    .unwrap();
                let mut data: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(body) = data.as_mut() {
                        if line == "." {
                            let _ = tx.send(std::mem::take(body));
                            data = None;
                            write.write_all(b"250 OK queued\r\n").await.unwrap();
                        } else {
                            body.push_str(&line);
                            body.push('\n');
                        }
                        continue;
                    }
                    let verb = line
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase();
                    let reply: &[u8] = match verb.as_str() {
                        "EHLO" | "HELO" => b"250-localhost\r\n250 8BITMIME\r\n",
                        "DATA" => {
                            data = Some(String::new());
                            b"354 go ahead\r\n"
                        }
                        "QUIT" => {
                            write.write_all(b"221 bye\r\n").await.unwrap();
                            return;
                        }
                        _ => b"250 OK\r\n",
                    };
                    write.write_all(reply).await.unwrap();
                }
            });
        }
    });
    (port, rx)
}
async fn mail_app() -> (
    tempfile::TempDir,
    App,
    tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    let (port, rx) = mock_smtp().await;
    let port = port.to_string();
    let (d, app) = app_with(&[
        "--smtp-host",
        "127.0.0.1",
        "--smtp-port",
        &port,
        "--smtp-security",
        "none",
        "--smtp-from",
        "StickTalk <no-reply@localhost.test>",
    ])
    .await;
    (d, app, rx)
}
async fn mail(rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("an email")
        .unwrap()
}
/// The secret from a link like http://localhost/#reset=TOKEN (quoted-printable safe).
fn link_token(message: &str, kind: &str) -> String {
    let flat = message.replace("=\n", "").replace("=3D", "=");
    let at = flat.find(&format!("#{kind}=")).expect("link") + kind.len() + 2;
    flat[at..at + 43].to_owned()
}
async fn add_verified_email(
    app: &App,
    token: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    email: &str,
) {
    fresh_limits(app).await;
    let p = proof(app, token, AccountOperation::Email).await;
    let (s, _, v) = call(Method::PUT, "/api/v1/users/@me/email")
        .bearer(token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"email": email}))
        .send(app)
        .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["pending"].as_str().unwrap().eq_ignore_ascii_case(email));
    let message = mail(rx).await;
    let secret = link_token(&message, "verify-email");
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email/verify")
        .json(json!({"token": secret}))
        .send(app)
        .await;
    assert_eq!(s, 204);
}

#[tokio::test]
async fn email_is_disabled_without_smtp_and_the_admin_reset_remains() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let (_, _, opts) = call(Method::GET, "/api/v1/auth/options").send(&app).await;
    assert_eq!(opts["email_reset"], false);
    let (s, _, v) = call(Method::GET, "/api/v1/users/@me/email")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(
        (s, v["smtp_enabled"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/request")
        .json(json!({"login": "alice"}))
        .send(&app)
        .await;
    assert_eq!(s, 404);
    // Plain SMTP to a remote host is refused at startup.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_string_lossy().to_string();
    assert!(App::open(crate::Config::parse_from([
        "server",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        &data,
        "--public-url",
        ORIGIN,
        "--smtp-host",
        "mail.example.com",
        "--smtp-security",
        "none",
        "--smtp-from",
        "a@example.com",
        "--min-free-disk-gb",
        "0",
        "--voice-bind-ip",
        "127.0.0.1",
        "--voice-advertised-ip",
        "127.0.0.1",
    ]))
    .await
    .is_err());
}

#[tokio::test]
async fn adding_an_email_needs_reauth_and_verification() {
    let (_d, app, mut rx) = mail_app().await;
    let token = user(&app, 1, "alice").await;
    let (s, _, _) = call(Method::PUT, "/api/v1/users/@me/email")
        .bearer(&token)
        .json(json!({"email": "alice@example.com"}))
        .send(&app)
        .await;
    assert_eq!(
        s, 401,
        "a stolen session cannot redirect resets to its own mailbox"
    );
    for bad in ["no-at-sign", "a@b", "x y@example.com", "a@exa mple.com"] {
        let p = proof(&app, &token, AccountOperation::Email).await;
        let (s, _, _) = call(Method::PUT, "/api/v1/users/@me/email")
            .bearer(&token)
            .header("x-sticktalk-reauth", &p)
            .json(json!({"email": bad}))
            .send(&app)
            .await;
        assert_eq!(s, 400, "{bad}");
    }
    add_verified_email(&app, &token, &mut rx, "Alice@Example.COM").await;
    let (_, _, v) = call(Method::GET, "/api/v1/users/@me/email")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(
        v,
        json!({"smtp_enabled": true, "email": "Alice@example.com", "verified": true, "pending": null})
    );
    // A verification link works once.
    let p = proof(&app, &token, AccountOperation::Email).await;
    call(Method::PUT, "/api/v1/users/@me/email")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .json(json!({"email": "new@example.com"}))
        .send(&app)
        .await;
    let secret = link_token(&mail(&mut rx).await, "verify-email");
    for expected in [204, 401] {
        let (s, _, _) = call(Method::POST, "/api/v1/auth/email/verify")
            .json(json!({"token": secret}))
            .send(&app)
            .await;
        assert_eq!(s, expected);
    }
    // Removal needs reauth too.
    let (s, _, _) = call(Method::DELETE, "/api/v1/users/@me/email")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(s, 401);
    let p = proof(&app, &token, AccountOperation::Email).await;
    let (s, _, v) = call(Method::DELETE, "/api/v1/users/@me/email")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!((s, v["email"].clone()), (StatusCode::OK, Value::Null));
}

#[tokio::test]
async fn reset_requests_reveal_nothing_and_links_are_single_use_and_sign_out_everywhere() {
    let (_d, app, mut rx) = mail_app().await;
    let token = user(&app, 1, "alice").await;
    session_for(&app, 1).await;
    add_verified_email(&app, &token, &mut rx, "alice@example.com").await;
    user(&app, 2, "bob").await; // No email: indistinguishable from no account.
    fresh_limits(&app).await;
    let mut answers = vec![];
    for login in [
        "alice",
        "bob",
        "nobody_here",
        "ghost@example.com",
        "alice@example.com",
    ] {
        let (s, h, v) = call(Method::POST, "/api/v1/auth/email-reset/request")
            .json(json!({"login": login}))
            .send(&app)
            .await;
        answers.push((s, v, h.get("content-length").cloned()));
    }
    assert!(answers.iter().all(|a| a == &answers[0]), "{answers:?}");
    assert_eq!(answers[0].0, StatusCode::ACCEPTED);
    // Mail goes only to the real, verified address; the second request replaced the first link.
    let first = link_token(&mail(&mut rx).await, "reset");
    let second = mail(&mut rx).await;
    assert!(second.contains("@alice"));
    let secret = link_token(&second, "reset");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv())
            .await
            .is_err()
    );
    fresh_limits(&app).await;
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
        .json(json!({"token": first, "new_password": "new-password"}))
        .send(&app)
        .await;
    assert_eq!(s, 401, "a replaced link is dead");
    assert_eq!(sessions_of(&app, 1).await, 2);
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
        .json(json!({"token": secret, "new_password": "new-password"}))
        .send(&app)
        .await;
    assert_eq!(s, 204);
    assert_eq!(sessions_of(&app, 1).await, 0, "every session is signed out");
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=1")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert!(crate::http::verify_password("new-password", &hash));
    // Single use.
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
        .json(json!({"token": secret, "new_password": "again"}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
}

#[tokio::test]
async fn reset_links_expire_die_with_an_email_change_and_guessing_is_limited() {
    let (_d, app, mut rx) = mail_app().await;
    let token = user(&app, 1, "alice").await;
    add_verified_email(&app, &token, &mut rx, "alice@example.com").await;
    call(Method::POST, "/api/v1/auth/email-reset/request")
        .json(json!({"login": "alice"}))
        .send(&app)
        .await;
    let secret = link_token(&mail(&mut rx).await, "reset");
    fresh_limits(&app).await;
    sqlx::query("UPDATE email_tokens SET expires_at=0 WHERE kind='reset'")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
        .json(json!({"token": secret, "new_password": "x"}))
        .send(&app)
        .await;
    assert_eq!(s, 401, "expired");
    call(Method::POST, "/api/v1/auth/email-reset/request")
        .json(json!({"login": "alice"}))
        .send(&app)
        .await;
    let secret = link_token(&mail(&mut rx).await, "reset");
    // The owner verifies a new address: links sent to the old one stop working.
    add_verified_email(&app, &token, &mut rx, "alice2@example.com").await;
    fresh_limits(&app).await;
    let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
        .json(json!({"token": secret, "new_password": "x"}))
        .send(&app)
        .await;
    assert_eq!(s, 401);
    let mut limited = false;
    for _ in 0..12 {
        let (s, _, _) = call(Method::POST, "/api/v1/auth/email-reset/complete")
            .json(json!({"token": state::token(), "new_password": "x"}))
            .send(&app)
            .await;
        if s == 429 {
            limited = true;
            break;
        }
        assert_eq!(s, 401);
    }
    assert!(limited, "guessing links runs into the limiter");
    // Requests for one target are limited too (mail bombing).
    let (_d, app, _rx) = mail_app().await;
    let mut statuses = vec![];
    for _ in 0..4 {
        statuses.push(
            call(Method::POST, "/api/v1/auth/email-reset/request")
                .json(json!({"login": "someone"}))
                .send(&app)
                .await
                .0,
        );
    }
    assert_eq!(statuses, [202, 202, 202, 429]);
}

// ----- Account switcher --------------------------------------------------------

fn jar(active: Option<&str>, stash: &[&str]) -> String {
    let mut parts = vec![];
    if let Some(a) = active {
        parts.push(format!("__Host-sticktalk_session={a}"));
        parts.push(format!("__Host-sticktalk_csrf={}", csrf(a)));
    }
    if !stash.is_empty() {
        parts.push(format!("{}={}", accounts::STASH, stash.join(".")));
    }
    parts.join("; ")
}

#[tokio::test]
async fn account_switcher_keeps_independent_sessions() {
    let (_d, app) = app_with(&[]).await;
    let alice = user(&app, 1, "alice").await;
    let bob = user(&app, 2, "bob").await;
    let carol = user(&app, 3, "carol").await;
    // Add an account: the current one moves to the stash and the browser signs out.
    let (s, h, _) = call(Method::POST, "/api/v1/browser/accounts/add")
        .browser(&jar(Some(&alice), &[]), Some(&alice))
        .send(&app)
        .await;
    assert_eq!(s, 204);
    assert_eq!(
        set_cookie(&h, accounts::STASH).as_deref(),
        Some(alice.as_str())
    );
    assert_eq!(
        set_cookie(&h, "__Host-sticktalk_session").as_deref(),
        Some("")
    );
    assert!(h.get_all("set-cookie").iter().all(|c| {
        let c = c.to_str().unwrap();
        !c.starts_with(accounts::STASH)
            || (c.contains("HttpOnly") && c.contains("SameSite=Strict") && c.contains("Secure"))
    }));
    // After signing in as Bob, both are listed, Bob active.
    let (_, _, list) = call(Method::GET, "/api/v1/browser/accounts")
        .browser(&jar(Some(&bob), &[&alice]), None)
        .send(&app)
        .await;
    let names: Vec<(String, bool)> = list["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["user"]["username"].as_str().unwrap().to_owned(),
                a["active"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(names, [("bob".into(), true), ("alice".into(), false)]);
    assert_eq!(list["max"], 5);
    // Switch to Alice: her own token becomes active, Bob's waits.
    let (s, h, v) = call(Method::POST, "/api/v1/browser/accounts/switch")
        .browser(&jar(Some(&bob), &[&alice]), Some(&bob))
        .json(json!({"user_id": 1}))
        .send(&app)
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["user"]["user"]["username"], "alice");
    assert_eq!(
        set_cookie(&h, "__Host-sticktalk_session").as_deref(),
        Some(alice.as_str())
    );
    assert_eq!(
        set_cookie(&h, accounts::STASH).as_deref(),
        Some(bob.as_str())
    );
    assert_eq!(set_cookie(&h, "__Host-sticktalk_csrf"), Some(csrf(&alice)));
    // Only accounts actually signed in on this browser can be switched to.
    let (s, _, _) = call(Method::POST, "/api/v1/browser/accounts/switch")
        .browser(&jar(Some(&alice), &[&bob]), Some(&alice))
        .json(json!({"user_id": 3}))
        .send(&app)
        .await;
    assert_eq!(s, 404);
    // Switching is a state change: CSRF protected, same origin only.
    let (s, _, _) = call(Method::POST, "/api/v1/browser/accounts/switch")
        .browser(&jar(Some(&alice), &[&bob]), None)
        .json(json!({"user_id": 2}))
        .send(&app)
        .await;
    assert_eq!(s, 403);
    let (s, _, _) = call(Method::POST, "/api/v1/browser/accounts/switch")
        .header("origin", "https://evil.example")
        .header("cookie", &jar(None, &[&bob]))
        .json(json!({"user_id": 2}))
        .send(&app)
        .await;
    assert_eq!(s, 403);
    // Signing Bob out of this browser revokes only Bob's session.
    let (s, h, _) = call(Method::POST, "/api/v1/browser/accounts/logout")
        .browser(&jar(Some(&alice), &[&bob]), Some(&alice))
        .json(json!({"user_id": 2}))
        .send(&app)
        .await;
    assert_eq!(s, 204);
    assert_eq!(set_cookie(&h, accounts::STASH).as_deref(), Some(""));
    assert_eq!(user_of(&app, &bob).await, None);
    assert_eq!(user_of(&app, &alice).await, Some(1));
    assert_eq!(user_of(&app, &carol).await, Some(3));
    // Signing the active account out leaves the waiting ones signed in.
    let (s, h, _) = call(Method::POST, "/api/v1/browser/accounts/logout")
        .browser(&jar(Some(&alice), &[&carol]), Some(&alice))
        .json(json!({"user_id": 1}))
        .send(&app)
        .await;
    assert_eq!(s, 204);
    assert_eq!(
        set_cookie(&h, "__Host-sticktalk_session").as_deref(),
        Some("")
    );
    assert_eq!(user_of(&app, &alice).await, None);
    assert_eq!(user_of(&app, &carol).await, Some(3));
    // Signed out, the browser can still list and switch to the waiting account.
    let (_, _, list) = call(Method::GET, "/api/v1/browser/accounts")
        .browser(&jar(None, &[&carol]), None)
        .send(&app)
        .await;
    assert_eq!(list["accounts"][0]["active"], false);
    let (s, h, _) = call(Method::POST, "/api/v1/browser/accounts/switch")
        .browser(&jar(None, &[&carol]), None)
        .json(json!({"user_id": 3}))
        .send(&app)
        .await;
    assert_eq!(s, 200);
    assert_eq!(
        set_cookie(&h, "__Host-sticktalk_session").as_deref(),
        Some(carol.as_str())
    );
}

#[tokio::test]
async fn account_switcher_drops_dead_and_duplicate_sessions_and_caps_accounts() {
    let (_d, app) = app_with(&[]).await;
    let alice = user(&app, 1, "alice").await;
    let again = session_for(&app, 1).await;
    let dead = state::token();
    let (_, h, list) = call(Method::GET, "/api/v1/browser/accounts")
        .browser(&jar(Some(&alice), &[&again, &dead]), None)
        .send(&app)
        .await;
    assert_eq!(list["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(set_cookie(&h, accounts::STASH).as_deref(), Some(""));
    // The duplicate session no longer counts toward Alice's per-account limit.
    assert_eq!(user_of(&app, &again).await, None);
    let mut tokens = vec![];
    for n in 2..=5 {
        tokens.push(user(&app, n, &format!("user{n}")).await);
    }
    let stash: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let (s, _, _) = call(Method::POST, "/api/v1/browser/accounts/add")
        .browser(&jar(Some(&alice), &stash), Some(&alice))
        .send(&app)
        .await;
    assert_eq!(s, 409, "five accounts per browser");
}

// ----- Personal data export ----------------------------------------------------

async fn seed_content(app: &App) {
    sqlx::raw_sql("INSERT INTO guilds VALUES(10,1,'Home <b>Base</b>',NULL,0); INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'member',0); INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(11,'text',10,'general',0,0); INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'hello from alice',1),(13,11,2,'bob secret',2),(14,11,1,'deleted one',3); UPDATE messages SET deleted_at=4 WHERE id=14; INSERT INTO relationships VALUES(1,2,'friend',5),(2,1,'friend',5);")
        .execute(&app.0.pool)
        .await
        .unwrap();
    for i in 0..1200_i64 {
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,11,1,?,?)",
        )
        .bind(1000 + i)
        .bind(format!("bulk {i}"))
        .bind(10 + i)
        .execute(&app.0.pool)
        .await
        .unwrap();
    }
    std::fs::write(
        app.0.config.data_dir.join("files").join("k1"),
        b"attachment bytes",
    )
    .unwrap();
    sqlx::raw_sql("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(20,1,'message',11,'../../evil name.txt','text/plain',16,'k1',0); INSERT INTO message_attachments VALUES(12,20,0);")
        .execute(&app.0.pool)
        .await
        .unwrap();
}
fn unzip(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut files = std::collections::BTreeMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let mut v = vec![];
        f.read_to_end(&mut v).unwrap();
        files.insert(f.name().to_owned(), v);
    }
    files
}

#[tokio::test]
async fn data_export_is_reauthed_bounded_and_only_for_its_owner() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    let bob = user(&app, 2, "bob").await;
    seed_content(&app).await;
    std::fs::create_dir_all(app.0.config.data_dir.join("exports")).unwrap();
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(s, 401, "re-authentication required");
    let p = proof(&app, &token, AccountOperation::DataExport).await;
    let (s, _, v) = call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["export"]["state"], "queued");
    // One job at a time per account.
    let p = proof(&app, &token, AccountOperation::DataExport).await;
    let (s, _, _) = call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!(s, 409);
    // The refused request did not burn the proof.
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM reauth_proofs)")
            .fetch_one(&app.0.pool)
            .await
            .unwrap()
    );
    let next = export::work(&app).await.unwrap().unwrap();
    let (_, _, status) = call(Method::GET, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .send(&app)
        .await;
    let e = &status["export"];
    assert_eq!(e["state"], "ready", "{status}");
    assert_eq!(e["expires_at"].as_u64().unwrap(), next);
    assert!(next >= state::now() + export::TTL - 60_000);
    assert!(status["next_request_at"].as_u64().unwrap() > state::now());
    // Then one request per day.
    let (s, h, _) = call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    assert_eq!(s, 429);
    assert!(h.contains_key("retry-after"));
    let path = format!(
        "/api/v1/users/@me/data-export/{}/download",
        e["id"].as_str().unwrap()
    );
    let (s, _, _) = call(Method::GET, &path).bearer(&bob).send(&app).await;
    assert_eq!(s, 404, "someone else's archive looks missing");
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
    assert_eq!(r.headers()["content-type"], "application/zip");
    assert!(r.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("sticktalk-data-alice.zip"));
    let bytes = axum::body::to_bytes(r.into_body(), 64 << 20).await.unwrap();
    assert_eq!(bytes.len() as u64, e["byte_size"].as_u64().unwrap());
    let files = unzip(&bytes);
    for name in [
        "README.txt",
        "index.html",
        "account.json",
        "settings.json",
        "servers.json",
        "relationships.json",
        "messages/index.json",
        "messages/11.json",
        "attachments/index.json",
        "attachments/20-_.._evil_name.txt",
    ] {
        assert!(
            files.contains_key(name),
            "{name}: {:?}",
            files.keys().collect::<Vec<_>>()
        );
    }
    let account: Value = serde_json::from_slice(&files["account.json"]).unwrap();
    assert_eq!(account["user"]["username"], "alice");
    assert_eq!(account["format"], "sticktalk-data-export");
    assert!(!String::from_utf8_lossy(&files["account.json"]).contains("password_hash"));
    let messages: Value = serde_json::from_slice(&files["messages/11.json"]).unwrap();
    let list = messages["messages"].as_array().unwrap();
    assert_eq!(
        list.len(),
        1201,
        "every message alice wrote, across pages, none deleted"
    );
    assert_eq!(list[0]["content"], "hello from alice");
    assert_eq!(list[0]["attachments"][0]["filename"], "../../evil name.txt");
    let all = files
        .values()
        .map(|v| String::from_utf8_lossy(v).to_string())
        .collect::<String>();
    assert!(!all.contains("bob secret"), "only her own messages");
    assert!(!all.contains("deleted one"));
    assert_eq!(
        files["attachments/20-_.._evil_name.txt"],
        b"attachment bytes"
    );
    let index = String::from_utf8_lossy(&files["index.html"]);
    assert!(index.contains("Home &lt;b&gt;Base&lt;/b&gt;") && !index.contains("<b>Base"));
    let relationships: Value = serde_json::from_slice(&files["relationships.json"]).unwrap();
    assert_eq!(relationships[0]["user"]["username"], "bob");
    assert!(files
        .keys()
        .all(|k| !k.contains("..\\") && !k.starts_with('/')));
}

#[tokio::test]
async fn data_exports_expire_on_their_timer_and_vanish_with_the_account() {
    let (_d, app) = app_with(&[]).await;
    let token = user(&app, 1, "alice").await;
    std::fs::create_dir_all(app.0.config.data_dir.join("exports")).unwrap();
    let p = proof(&app, &token, AccountOperation::DataExport).await;
    call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    export::work(&app).await.unwrap();
    let files = || {
        std::fs::read_dir(app.0.config.data_dir.join("exports"))
            .unwrap()
            .count()
    };
    assert_eq!(files(), 1);
    sqlx::query("UPDATE data_exports SET expires_at=1")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        export::work(&app).await.unwrap(),
        None,
        "nothing else is due: the timer is disarmed"
    );
    assert_eq!(files(), 0);
    let (_, _, status) = call(Method::GET, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .send(&app)
        .await;
    assert_eq!(status["export"]["state"], "expired");
    // Erasure removes the rows; the next wake removes the file.
    sqlx::query("DELETE FROM data_exports")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let p = proof(&app, &token, AccountOperation::DataExport).await;
    call(Method::POST, "/api/v1/users/@me/data-export")
        .bearer(&token)
        .header("x-sticktalk-reauth", &p)
        .send(&app)
        .await;
    export::work(&app).await.unwrap();
    assert_eq!(files(), 1);
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::account::erase(&mut c, Id(1)).await.unwrap();
    drop(c);
    export::work(&app).await.unwrap();
    assert_eq!(files(), 0);
    for table in ["data_exports", "passkeys", "user_emails", "email_tokens"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&app.0.pool)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}

/// The export covers the later features too.
mod integration;
