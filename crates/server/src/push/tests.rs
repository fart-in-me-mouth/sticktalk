use super::*;
use crate::{
    audit_tests::fixture,
    state::{Connection, Packet},
};
use axum::{extract::State, routing::post};
use std::{
    future::IntoFuture,
    sync::atomic::{AtomicU16, Ordering},
};
use tokio::sync::{watch, Semaphore};
fn b64(s: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(s).unwrap()
}
#[test]
fn rfc8291_appendix_a_derivation_and_ciphertext_vector() {
    let ua = b64(
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
    );
    let server = b64(
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
    );
    let (key, nonce) = derive(
        &b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"),
        &b64("BTBZMqHH6r4Tts7J_aSIgg"),
        &b64("DGv6ra1nlYgDCS1FRnbzlw"),
        &ua,
        &server,
    );
    assert_eq!(URL_SAFE_NO_PAD.encode(&key), "oIhVW04MRdy2XN9CiKLxTg");
    assert_eq!(URL_SAFE_NO_PAD.encode(&nonce), "4h_95klXJ5E_qnoN");
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &key).unwrap());
    let mut ciphertext =
        b64("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ");
    let plaintext = key
        .open_in_place(
            aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
            aead::Aad::empty(),
            &mut ciphertext,
        )
        .unwrap();
    assert_eq!(plaintext, b"When I grow up, I want to be a watermelon\x02");
}
fn receiver() -> (agreement::EphemeralPrivateKey, Vec<u8>, Vec<u8>) {
    let private = agreement::EphemeralPrivateKey::generate(
        &agreement::ECDH_P256,
        &ring::rand::SystemRandom::new(),
    )
    .unwrap();
    let public = private.compute_public_key().unwrap().as_ref().to_vec();
    (private, public, state::random(16))
}
fn decrypt(
    private: agreement::EphemeralPrivateKey,
    public: &[u8],
    auth: &[u8],
    body: &[u8],
) -> Vec<u8> {
    assert_eq!(body[20], 65);
    assert_eq!(&body[16..20], &4096_u32.to_be_bytes());
    assert!(body.len() <= 4096);
    let shared = agreement::agree_ephemeral(
        private,
        &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &body[21..86]),
        |s| s.to_vec(),
    )
    .unwrap();
    let (key, nonce) = derive(&shared, auth, &body[..16], public, &body[21..86]);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &key).unwrap());
    let mut record = body[86..].to_vec();
    let bytes = key
        .open_in_place(
            aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
            aead::Aad::empty(),
            &mut record,
        )
        .unwrap();
    assert_eq!(bytes.last(), Some(&2));
    bytes[..bytes.len() - 1].to_vec()
}
#[test]
fn fresh_ephemeral_encryption_roundtrip_and_invalid_curve() {
    let (private, public, auth) = receiver();
    let bytes = encrypt(&public, &auth, b"private notification").unwrap();
    assert_eq!(
        decrypt(private, &public, &auth, &bytes),
        b"private notification"
    );
    let b = encrypt(&public, &auth, b"private notification").unwrap();
    assert_ne!(&b[..86], &bytes[..86]);
    assert!(encrypt(&[4; 65], &auth, b"test").is_err());
    assert!(encrypt(&public, &auth, &vec![0; 3001]).is_err());
}
async fn auth(app: &App, user: u64) -> String {
    let token = state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=?")
        .bind(state::hash(token.as_bytes()))
        .bind(user as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
fn headers(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("authorization", format!("Bearer {token}").parse().unwrap());
    h
}
async fn subscription(
    app: &App,
    token: &str,
    endpoint: &str,
    public: &[u8],
    auth: &[u8],
) -> PushSubscription {
    let mut r = Request::new(Body::from(
        serde_json::to_vec(&PushSubscribeRequest {
            endpoint: endpoint.into(),
            p256dh: URL_SAFE_NO_PAD.encode(public),
            auth: URL_SAFE_NO_PAD.encode(auth),
        })
        .unwrap(),
    ));
    *r.headers_mut() = headers(token);
    r.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    let r = subscribe(State(app.clone()), r).await.unwrap();
    serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 4096).await.unwrap()).unwrap()
}
#[tokio::test]
async fn subscriptions_session_isolation_ssrf_quota_keys_and_logout_cascade() {
    let (_dir, app) = fixture().await;
    let t1 = auth(&app, 1).await;
    let t2 = auth(&app, 2).await;
    let (_, public, auth) = receiver();
    let sub = subscription(
        &app,
        &t1,
        "https://fcm.googleapis.com/first",
        &public,
        &auth,
    )
    .await;
    assert_eq!(
        list(State(app.clone()), headers(&t1))
            .await
            .unwrap()
            .0
            .len(),
        1
    );
    assert!(list(State(app.clone()), headers(&t2))
        .await
        .unwrap()
        .0
        .is_empty());
    assert_eq!(
        unsubscribe(State(app.clone()), Path(sub.id.clone()), headers(&t2))
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    let same = subscription(
        &app,
        &t1,
        "https://fcm.googleapis.com/first",
        &public,
        &auth,
    )
    .await;
    assert_eq!(same.id, sub.id);
    for endpoint in [
        "http://127.0.0.1:1/push",
        "https://127.0.0.1/push",
        "https://evil.test/push",
        "https://fcm.googleapis.com.evil.test/push",
        "https://u:p@fcm.googleapis.com/push",
        "https://fcm.googleapis.com/push#x",
    ] {
        assert!(app.0.push.endpoint(endpoint).is_err(), "{endpoint}");
    }
    for i in 0..4 {
        subscription(
            &app,
            &t1,
            &format!("https://fcm.googleapis.com/sub-{i}"),
            &public,
            &auth,
        )
        .await;
    }
    let mut req = Request::new(Body::from(
        serde_json::to_vec(&PushSubscribeRequest {
            endpoint: "https://fcm.googleapis.com/overflow".into(),
            p256dh: URL_SAFE_NO_PAD.encode(&public),
            auth: URL_SAFE_NO_PAD.encode(&auth),
        })
        .unwrap(),
    ));
    *req.headers_mut() = headers(&t1);
    req.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    assert_eq!(
        subscribe(State(app.clone()), req).await.unwrap_err().0.code,
        ErrorCode::LimitExceeded
    );
    let authorization = app
        .0
        .push
        .authorization(&reqwest::Url::parse("https://fcm.googleapis.com/first").unwrap())
        .unwrap();
    let jwt = authorization
        .strip_prefix("vapid t=")
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    let parts: Vec<_> = jwt.split('.').collect();
    let claims: serde_json::Value = serde_json::from_slice(&b64(parts[1])).unwrap();
    assert_eq!(claims["aud"], "https://fcm.googleapis.com");
    assert!(claims["exp"].as_u64().unwrap() <= state::now() / 1000 + 86400);
    ring::signature::UnparsedPublicKey::new(
        &ring::signature::ECDSA_P256_SHA256_FIXED,
        app.0.push.key.public_key(),
    )
    .verify(
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &b64(parts[2]),
    )
    .unwrap();
    let old = app.0.push.public.clone();
    let reloaded = Push::new(&app.0.config).await.unwrap();
    assert_eq!(old, reloaded.public);
    sqlx::query("DELETE FROM sessions WHERE user_id=1")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM push_subscriptions")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
}
#[derive(Clone)]
struct Sink {
    tx: mpsc::Sender<(HeaderMap, Vec<u8>)>,
    status: Arc<AtomicU16>,
}
async fn mock_sink(
    State(sink): State<Sink>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let _ = sink.tx.send((headers, body.to_vec())).await;
    StatusCode::from_u16(sink.status.load(Ordering::Acquire)).unwrap()
}
async fn mock(
    app: &mut App,
) -> (
    String,
    mpsc::Receiver<(HeaderMap, Vec<u8>)>,
    Arc<AtomicU16>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    Arc::get_mut(&mut app.0)
        .unwrap()
        .push
        .origins
        .push(origin.clone());
    let (tx, rx) = mpsc::channel(32);
    let status = Arc::new(AtomicU16::new(201));
    let sink = Sink {
        tx,
        status: status.clone(),
    };
    let task = tokio::spawn(
        axum::serve(
            listener,
            axum::Router::new()
                .route("/push", post(mock_sink))
                .with_state(sink),
        )
        .into_future(),
    );
    (format!("{origin}/push"), rx, status, task)
}
async fn notice(app: &App) -> PushNotice {
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,11,1,'private',0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO message_mentions(message_id,user_id) VALUES(100,2)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    PushNotice {
        user_id: Id(2),
        author_id: Id(1),
        channel_id: Id(11),
        message_id: Id(100),
        direct: false,
        mentioned: true,
    }
}
struct Deny;
impl NotificationPolicy for Deny {
    fn allows<'a>(&'a self, _: &'a mut SqliteConnection, _: &'a PushNotice) -> PolicyFuture<'a> {
        Box::pin(async { Ok(false) })
    }
}
#[tokio::test]
async fn delivery_encryption_expired_cleanup_focus_mute_acl_and_policy() {
    let (_dir, mut app) = fixture().await;
    let (endpoint, mut rx, status, task) = mock(&mut app).await;
    let token = auth(&app, 2).await;
    let (private, public, auth) = receiver();
    let sub = subscription(&app, &token, &endpoint, &public, &auth).await;
    let n = notice(&app).await;
    deliver(&app, &n, 4).await.unwrap();
    let (h, body) = rx.recv().await.unwrap();
    assert_eq!(h["content-encoding"], "aes128gcm");
    assert_eq!(h["ttl"], "300");
    assert!(h["authorization"].to_str().unwrap().starts_with("vapid t="));
    let plaintext = decrypt(private, &public, &auth, &body);
    let payload: PushPayload = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(payload.count, 4);
    assert_eq!(payload.url, "/?channel=11&message=100");
    assert!(!String::from_utf8(plaintext).unwrap().contains("private"));
    app.set_push_policy(Arc::new(Deny)).await;
    deliver(&app, &n, 1).await.unwrap();
    assert!(rx.try_recv().is_err());
    app.set_push_policy(Arc::new(Allow)).await;
    sqlx::query("UPDATE users SET desired_presence='dnd' WHERE id=2")
        .execute(&app.0.pool)
        .await
        .unwrap();
    deliver(&app, &n, 1).await.unwrap();
    assert!(rx.try_recv().is_err());
    sqlx::query("UPDATE users SET desired_presence='online' WHERE id=2")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let (tx, _rx) = mpsc::channel::<Packet>(128);
    let (close, _) = watch::channel(None);
    let key = state::hash(token.as_bytes());
    {
        let mut core = app.0.core.lock().await;
        core.cursor(&key, Id(2));
        core.streams.get_mut(&key).unwrap().connection = Some(Connection {
            id: "focused".into(),
            focused: true,
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: Arc::new(Semaphore::new(2097152)),
            close,
        });
    }
    deliver(&app, &n, 1).await.unwrap();
    assert!(rx.try_recv().is_err());
    app.0
        .core
        .lock()
        .await
        .streams
        .get_mut(&key)
        .unwrap()
        .connection = None;
    sqlx::query("DELETE FROM guild_members WHERE user_id=2")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert!(deliver(&app, &n, 1).await.is_err());
    assert!(rx.try_recv().is_err());
    sqlx::query("INSERT INTO guild_members VALUES(10,2,'member',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    status.store(410, Ordering::Release);
    deliver(&app, &n, 1).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM push_subscriptions WHERE id=?")
            .bind(sub.id)
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
    let _ = rx.recv().await;
    let sub = subscription(&app, &token, &endpoint, &public, &auth).await;
    status.store(404, Ordering::Release);
    deliver(&app, &n, 1).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM push_subscriptions WHERE id=?")
            .bind(sub.id)
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
    app.shutdown();
    task.abort();
}
#[tokio::test]
async fn committed_mentions_coalesce_rate_limit_and_dm_hook() {
    let (_dir, mut app) = fixture().await;
    let (endpoint, mut rx, _, task) = mock(&mut app).await;
    let token = auth(&app, 2).await;
    let (private, public, auth) = receiver();
    subscription(&app, &token, &endpoint, &public, &auth).await;
    // Keep a receiver until the first commands have queued their notices.
    for nonce in 1..=3 {
        let r = crate::commands::execute(
            &app,
            &mut *app.0.core.lock().await,
            &[1; 32],
            Id(1),
            CommandRequest {
                nonce,
                command: Command::SendMessage(MessageDraft {
                    channel_id: Id(11),
                    content: "mention".into(),
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![Id(2)],
                }),
            },
        )
        .await
        .unwrap();
        assert!(matches!(r.outcome, CommandOutcome::Ok(_)));
    }
    spawn(&app);
    let (h, body) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(h.contains_key("topic"));
    let payload: PushPayload =
        serde_json::from_slice(&decrypt(private, &public, &auth, &body)).unwrap();
    assert_eq!(payload.count, 3);
    // A DM without mentions still queues a notice; the per-user 30s limiter
    // suppresses another network send for this burst.
    sqlx::query("INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO dm_members VALUES(20,1,0,1),(20,2,0,1)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    let m = Message {
        id: Id(100),
        channel_id: Id(20),
        author_id: Id(1),
        content: String::new(),
        created_at: 0,
        edited_at: None,
        reply_to: None,
        attachments: vec![],
        mention_ids: vec![],
        reactions: vec![],
    };
    message(&app, &mut c, &[Id(1), Id(2)], &m).await;
    drop(c);
    assert!(tokio::time::timeout(Duration::from_millis(2500), rx.recv())
        .await
        .is_err());
    app.shutdown();
    task.abort();
}

#[tokio::test]
async fn delivery_rechecks_launch_preferences_and_current_mentions() {
    let (_dir, app) = fixture().await;
    let n = notice(&app).await;
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(allowed(&app, &mut c, &n).await.unwrap());
    async fn setting(
        c: &mut SqliteConnection,
        channel: bool,
        level: NotificationLevel,
        muted: bool,
        until: Option<u64>,
        suppress: bool,
    ) {
        let s = NotificationSettings {
            guild_id: Some(Id(10)),
            channel_id: channel.then_some(Id(11)),
            level,
            muted,
            mute_until: until,
            suppress_everyone: suppress,
            hide_muted_channels: false,
        };
        sqlx::query("INSERT INTO notification_settings(user_id,scope,target_id,settings) VALUES(2,?,?,?) ON CONFLICT(user_id,scope,target_id) DO UPDATE SET settings=excluded.settings")
            .bind(if channel {"channel"} else {"guild"}).bind(if channel {11_i64} else {10_i64})
            .bind(serde_json::to_string(&s).unwrap()).execute(c).await.unwrap();
    }
    setting(
        &mut c,
        false,
        NotificationLevel::Nothing,
        false,
        None,
        false,
    )
    .await;
    assert!(!allowed(&app, &mut c, &n).await.unwrap());
    setting(
        &mut c,
        true,
        NotificationLevel::Mentions,
        false,
        None,
        false,
    )
    .await;
    assert!(
        allowed(&app, &mut c, &n).await.unwrap(),
        "channel overrides guild"
    );
    setting(&mut c, true, NotificationLevel::Mentions, true, None, false).await;
    assert!(!allowed(&app, &mut c, &n).await.unwrap());
    setting(
        &mut c,
        true,
        NotificationLevel::Mentions,
        true,
        Some(1),
        false,
    )
    .await;
    assert!(allowed(&app, &mut c, &n).await.unwrap(), "expired mute");
    setting(&mut c, true, NotificationLevel::Mentions, false, None, true).await;
    sqlx::query("INSERT INTO message_broadcast_mentions VALUES(100,2,'everyone')")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(
        !allowed(&app, &mut c, &n).await.unwrap(),
        "suppress broadcast mention"
    );
    sqlx::query("INSERT INTO message_broadcast_mentions VALUES(100,2,'role')")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(
        allowed(&app, &mut c, &n).await.unwrap(),
        "role mention survives suppression"
    );
    sqlx::query("UPDATE users SET desired_presence='invisible' WHERE id=2")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(!allowed(&app, &mut c, &n).await.unwrap());
    sqlx::query("UPDATE users SET desired_presence='online' WHERE id=2")
        .execute(&mut *c)
        .await
        .unwrap();
    sqlx::query("DELETE FROM message_mentions WHERE message_id=100")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(
        !allowed(&app, &mut c, &n).await.unwrap(),
        "edited mention removed before delivery"
    );
}
