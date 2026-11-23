//! Bounded, event-driven Web Push. No idle polling and no message content in payloads.
use crate::{
    db,
    error::{require, Error, Result},
    http,
    state::{self, App},
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{
    aead, agreement, hmac,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use sqlx::{Row, SqliteConnection};
use std::{collections::HashMap, future::Future, io::Write, pin::Pin, sync::Arc, time::Duration};
use sticktalk_proto::*;
use tokio::sync::{mpsc, RwLock};

/// Integrate per-user/server/channel mute settings here. Called again immediately
/// before delivery, outside the chat mutation gate. Errors fail closed.
#[derive(Clone, Debug)]
pub struct PushNotice {
    pub user_id: UserId,
    pub author_id: UserId,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub direct: bool,
    pub mentioned: bool,
}
pub type PolicyFuture<'a> = Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>>;
pub trait NotificationPolicy: Send + Sync {
    fn allows<'a>(
        &'a self,
        c: &'a mut SqliteConnection,
        notice: &'a PushNotice,
    ) -> PolicyFuture<'a>;
}
struct Allow;
impl NotificationPolicy for Allow {
    fn allows<'a>(&'a self, _: &'a mut SqliteConnection, _: &'a PushNotice) -> PolicyFuture<'a> {
        Box::pin(async { Ok(true) })
    }
}
impl App {
    pub async fn set_push_policy(&self, policy: Arc<dyn NotificationPolicy>) {
        *self.0.push.policy.write().await = policy;
    }
}
pub(crate) struct Push {
    key: EcdsaKeyPair,
    public: String,
    client: reqwest::Client,
    origins: Vec<String>,
    subject: String,
    tx: mpsc::Sender<PushNotice>,
    receiver: std::sync::Mutex<Option<mpsc::Receiver<PushNotice>>>,
    policy: RwLock<Arc<dyn NotificationPolicy>>,
}
fn crypto_error() -> Error {
    Error::new(ErrorCode::InvalidRequest)
}
impl Push {
    pub async fn new(config: &crate::Config) -> Result<Self> {
        let path = config.data_dir.join("vapid.pk8");
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            if !path.exists() {
                let bytes = EcdsaKeyPair::generate_pkcs8(
                    &ECDSA_P256_SHA256_FIXED_SIGNING,
                    &ring::rand::SystemRandom::new(),
                )
                .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                // create_new prevents silently rotating keys if two processes race.
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(mut file) => {
                        file.write_all(bytes.as_ref())?;
                        file.sync_all()?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let bytes = std::fs::read(path)?;
            require(bytes.len() <= 1024, ErrorCode::Unavailable)?;
            Ok(bytes)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))??;
        let key = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &bytes,
            &ring::rand::SystemRandom::new(),
        )
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let public = URL_SAFE_NO_PAD.encode(key.public_key().as_ref());
        let mut origins = if config.browser.push_origins.is_empty() {
            vec![
                "https://fcm.googleapis.com".into(),
                "https://updates.push.services.mozilla.com".into(),
                "https://web.push.apple.com".into(),
            ]
        } else {
            config.browser.push_origins.clone()
        };
        require(origins.len() <= 16, ErrorCode::InvalidRequest)?;
        for o in &mut origins {
            let url = crate::browser::origin(o)?;
            require(url.scheme() == "https", ErrorCode::InvalidRequest)?;
            *o = url.origin().ascii_serialization();
        }
        if let Some(mock) = &config.browser.mock_push_origin {
            require(config.bind.ip().is_loopback(), ErrorCode::InvalidRequest)?;
            state::validate_mock_url(mock)?;
            let url = crate::browser::origin(mock)?;
            origins.push(url.origin().ascii_serialization());
        }
        let subject = config
            .browser
            .vapid_subject
            .clone()
            .unwrap_or(config.public_url.clone());
        let url = reqwest::Url::parse(&subject).map_err(|_| crypto_error())?;
        require(
            subject.len() <= 1024
                && (url.scheme() == "mailto"
                    || (url.scheme() == "https" && url.host_str().is_some())
                    || (config.bind.ip().is_loopback() && url.scheme() == "http")),
            ErrorCode::InvalidRequest,
        )?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let (tx, receiver) = mpsc::channel(1024);
        Ok(Self {
            key,
            public,
            client,
            origins,
            subject,
            tx,
            receiver: std::sync::Mutex::new(Some(receiver)),
            policy: RwLock::new(Arc::new(Allow)),
        })
    }
    fn endpoint(&self, endpoint: &str) -> Result<reqwest::Url> {
        require(endpoint.len() <= 2048, ErrorCode::InvalidRequest)?;
        let url = reqwest::Url::parse(endpoint).map_err(|_| crypto_error())?;
        require(
            url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && self.origins.contains(&url.origin().ascii_serialization()),
            ErrorCode::InvalidRequest,
        )?;
        Ok(url)
    }
    fn authorization(&self, url: &reqwest::Url) -> Result<String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims=URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"aud":url.origin().ascii_serialization(),"exp":state::now()/1000+12*3600,"sub":self.subject})).map_err(|_|crypto_error())?);
        let message = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&ring::rand::SystemRandom::new(), message.as_bytes())
            .map_err(|_| crypto_error())?;
        Ok(format!(
            "vapid t={message}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public
        ))
    }
}
pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/push/key", get(key))
        .route("/api/v1/push/subscriptions", get(list).post(subscribe))
        .route(
            "/api/v1/push/subscriptions/{id}",
            axum::routing::delete(unsubscribe),
        )
}
async fn key(State(app): State<App>) -> axum::Json<PushKey> {
    axum::Json(PushKey {
        public_key: app.0.push.public.clone(),
    })
}
async fn subscribe(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    let token = http::bearer(r.headers())?.to_owned();
    let req: PushSubscribeRequest = http::parse(r).await?;
    app.0.push.endpoint(&req.endpoint)?;
    let p256dh = decode(&req.p256dh, 65)?;
    require(p256dh[0] == 4, ErrorCode::InvalidRequest)?;
    let auth = decode(&req.auth, 16)?;
    // Actual curve validation, not only point length/prefix.
    encrypt(&p256dh, &auth, b"")?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (session, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("push-sub:{}", u.0), 30., 10.)?;
    let existing =
        sqlx::query("SELECT id,session_hash,created_at FROM push_subscriptions WHERE endpoint=?")
            .bind(&req.endpoint)
            .fetch_optional(&mut *c)
            .await?;
    let (id, created_at) = if let Some(row) = existing {
        require(
            row.get::<Vec<u8>, _>("session_hash") == session,
            ErrorCode::Conflict,
        )?;
        (
            row.get::<String, _>("id"),
            row.get::<i64, _>("created_at") as u64,
        )
    } else {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE session_hash=?")
                .bind(&session)
                .fetch_one(&mut *c)
                .await?;
        require(count < 5, ErrorCode::LimitExceeded)?;
        (state::token(), state::now())
    };
    sqlx::query("INSERT INTO push_subscriptions VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET p256dh=excluded.p256dh,auth=excluded.auth").bind(&id).bind(session).bind(&req.endpoint).bind(&req.p256dh).bind(&req.auth).bind(created_at as i64).execute(&mut *c).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(PushSubscription {
            id,
            endpoint: req.endpoint,
            created_at,
        }),
    )
        .into_response())
}
async fn list(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<axum::Json<Vec<PushSubscription>>> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (session, u) = db::auth(&mut c, http::bearer(&headers)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let rows = sqlx::query(
        "SELECT id,endpoint,created_at FROM push_subscriptions WHERE session_hash=? ORDER BY id",
    )
    .bind(session)
    .fetch_all(&mut *c)
    .await?;
    Ok(axum::Json(
        rows.into_iter()
            .map(|r| PushSubscription {
                id: r.get("id"),
                endpoint: r.get("endpoint"),
                created_at: r.get::<i64, _>("created_at") as u64,
            })
            .collect(),
    ))
}
async fn unsubscribe(
    State(app): State<App>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    require(id.len() == 43, ErrorCode::InvalidRequest)?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (session, u) = db::auth(&mut c, http::bearer(&headers)?).await?;
    core.rate(format!("push-sub:{}", u.0), 30., 10.)?;
    let n = sqlx::query("DELETE FROM push_subscriptions WHERE id=? AND session_hash=?")
        .bind(id)
        .bind(session)
        .execute(&mut *c)
        .await?
        .rows_affected();
    require(n == 1, ErrorCode::NotFound)?;
    Ok(StatusCode::NO_CONTENT)
}
fn decode(value: &str, size: usize) -> Result<Vec<u8>> {
    require(value.len() <= 128, ErrorCode::InvalidRequest)?;
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| crypto_error())?;
    require(bytes.len() == size, ErrorCode::InvalidRequest)?;
    Ok(bytes)
}
fn extract(salt: &[u8], input: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, salt), input)
        .as_ref()
        .to_vec()
}
fn expand(key: &[u8], info: &[u8], size: usize) -> Vec<u8> {
    let mut input = info.to_vec();
    input.push(1);
    extract(key, &input)[..size].to_vec()
}
fn derive(shared: &[u8], auth: &[u8], salt: &[u8], ua: &[u8], server: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua);
    info.extend_from_slice(server);
    let ikm = expand(&extract(auth, shared), &info, 32);
    let prk = extract(salt, &ikm);
    (
        expand(&prk, b"Content-Encoding: aes128gcm\0", 16),
        expand(&prk, b"Content-Encoding: nonce\0", 12),
    )
}
/// RFC 8291 single final aes128gcm record; ephemeral ECDH key and salt per send.
fn encrypt(ua: &[u8], auth: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    require(
        payload.len() <= 3000 && ua.len() == 65 && auth.len() == 16,
        ErrorCode::InvalidRequest,
    )?;
    let rng = ring::rand::SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| crypto_error())?;
    let public = private.compute_public_key().map_err(|_| crypto_error())?;
    let shared = agreement::agree_ephemeral(
        private,
        &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua),
        |s| s.to_vec(),
    )
    .map_err(|_| crypto_error())?;
    let salt = state::random(16);
    let (key, nonce) = derive(&shared, auth, &salt, ua, public.as_ref());
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &key).map_err(|_| crypto_error())?,
    );
    let mut record = payload.to_vec();
    record.push(2);
    key.seal_in_place_append_tag(
        aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| crypto_error())?,
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| crypto_error())?;
    let mut body = salt;
    body.extend_from_slice(&4096_u32.to_be_bytes());
    body.push(65);
    body.extend_from_slice(public.as_ref());
    body.extend_from_slice(&record);
    Ok(body)
}
/// Called only after commit; nonce replay never reaches this hook.
pub(crate) async fn message(app: &App, c: &mut SqliteConnection, users: &[Id], message: &Message) {
    let Ok(channel) = db::channel(c, message.channel_id).await else {
        return;
    };
    let direct = matches!(
        channel.summary.kind,
        ChannelKind::Direct | ChannelKind::GroupDm
    );
    for u in users.iter().filter(|u| **u != message.author_id) {
        let mentioned = message.mention_ids.contains(u);
        if direct || mentioned {
            let _ = app.0.push.tx.try_send(PushNotice {
                user_id: *u,
                author_id: message.author_id,
                channel_id: message.channel_id,
                message_id: message.id,
                direct,
                mentioned,
            });
        }
    }
}
/// A direct or group call is ringing `user`, who has no live connection. One
/// notification per ring, to their subscribed devices; never to someone in Do
/// Not Disturb, disabled, or blocked by or blocking the caller.
pub(crate) fn call(app: &App, user: Id, channel: Id, caller: Id) {
    let app = app.clone();
    tokio::spawn(async move {
        let stop = app.0.shutdown.clone();
        tokio::select! {_=stop.cancelled()=>{},_=deliver_call(&app,user,channel,caller)=>{}}
    });
}
async fn call_allowed(c: &mut SqliteConnection, user: Id, channel: Id, caller: Id) -> Result<bool> {
    db::enabled(c, user).await?;
    db::access(c, user, channel, Permissions::VIEW_CHANNEL).await?;
    let dnd: bool = sqlx::query_scalar("SELECT desired_presence='dnd' FROM users WHERE id=?")
        .bind(db::id(user)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(!dnd && !db::blocked(c, user, caller).await?)
}
async fn deliver_call(app: &App, user: Id, channel: Id, caller: Id) -> Result<()> {
    let mut c = app.0.pool.acquire().await?;
    if !call_allowed(&mut c, user, channel, caller).await? {
        return Ok(());
    }
    let rows=sqlx::query("SELECT p.endpoint,p.p256dh,p.auth FROM push_subscriptions p JOIN sessions s ON s.token_hash=p.session_hash WHERE s.user_id=? AND s.expires_at>? LIMIT 25").bind(db::id(user)?).bind(state::now() as i64).fetch_all(&mut *c).await?;
    drop(c);
    let payload = serde_json::to_vec(&PushPayload {
        url: format!("/?channel={}", channel.0),
        channel_id: channel,
        message_id: channel,
        count: 1,
        title: "StickTalk".into(),
        body: "Incoming call".into(),
    })
    .map_err(|_| crypto_error())?;
    let topic = URL_SAFE_NO_PAD
        .encode(&state::hash(format!("call:{}:{}", user.0, channel.0).as_bytes())[..16]);
    for row in rows {
        let endpoint = app.0.push.endpoint(&row.get::<String, _>("endpoint"))?;
        let body = encrypt(
            &decode(&row.get::<String, _>("p256dh"), 65)?,
            &decode(&row.get::<String, _>("auth"), 16)?,
            &payload,
        )?;
        // A ring is stale after its timeout; "high" wakes a sleeping phone.
        let _ = app
            .0
            .push
            .client
            .post(endpoint.clone())
            .header("authorization", app.0.push.authorization(&endpoint)?)
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header("ttl", (CALL_RING_TIMEOUT_MS / 1000).to_string())
            .header("urgency", "high")
            .header("topic", topic.clone())
            .body(body)
            .send()
            .await;
    }
    Ok(())
}
fn focused(core: &crate::state::Core, u: Id) -> bool {
    core.streams.values().any(|s| {
        s.user == u
            && s.connection
                .as_ref()
                .is_some_and(|c| c.focused && c.alive.load(std::sync::atomic::Ordering::Acquire))
    })
}
pub(crate) fn spawn(app: &App) {
    let Some(mut rx) = app
        .0
        .push
        .receiver
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    else {
        return;
    };
    let app = app.clone();
    tokio::spawn(async move {
        let mut last: HashMap<Id, u64> = HashMap::new();
        loop {
            let first = tokio::select! {_=app.0.shutdown.cancelled()=>break,n=rx.recv()=>match n{Some(n)=>n,None=>break}};
            let mut batch = HashMap::new();
            batch.insert((first.user_id, first.channel_id), (first, 1_u32));
            let delay = tokio::time::sleep(Duration::from_secs(2));
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    _=app.0.shutdown.cancelled()=>return,
                    _=&mut delay=>break,
                    n=rx.recv()=>if let Some(n)=n {
                        if let Some((latest,count))=batch.get_mut(&(n.user_id,n.channel_id)) {*latest=n;*count=count.saturating_add(1);} else if batch.len()<1024 {batch.insert((n.user_id,n.channel_id),(n,1));}
                    } else {break}
                }
            }
            last.retain(|_, t| state::now().saturating_sub(*t) < 600_000);
            for ((u, _), (notice, count)) in batch {
                if last
                    .get(&u)
                    .is_some_and(|t| state::now().saturating_sub(*t) < 30_000)
                {
                    continue;
                }
                if last.len() >= 4096 && !last.contains_key(&u) {
                    continue;
                }
                last.insert(u, state::now());
                tokio::select! {_=app.0.shutdown.cancelled()=>return,_=deliver(&app,&notice,count)=>{}}
            }
        }
    });
}
async fn allowed(app: &App, c: &mut SqliteConnection, n: &PushNotice) -> Result<bool> {
    db::enabled(c, n.user_id).await?;
    db::text(
        c,
        n.user_id,
        n.channel_id,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await?;
    let alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL)",
    )
    .bind(db::id(n.message_id)?)
    .bind(db::id(n.channel_id)?)
    .fetch_one(&mut *c)
    .await?;
    if !alive || db::blocked(c, n.user_id, n.author_id).await? {
        return Ok(false);
    }
    let muted: bool = sqlx::query_scalar("SELECT desired_presence='dnd' FROM users WHERE id=?")
        .bind(db::id(n.user_id)?)
        .fetch_one(&mut *c)
        .await?;
    let read:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM read_states WHERE user_id=? AND channel_id=? AND last_read_id>=?)").bind(db::id(n.user_id)?).bind(db::id(n.channel_id)?).bind(db::id(n.message_id)?).fetch_one(&mut *c).await?;
    if muted || read {
        return Ok(false);
    }
    let channel = db::channel(c, n.channel_id).await?;
    let message = db::message(c, n.user_id, n.channel_id, n.message_id, false).await?;
    if crate::mentions::notification_preference(c, n.user_id, &channel, &message)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let policy = app.0.push.policy.read().await.clone();
    policy.allows(c, n).await
}
async fn deliver(app: &App, n: &PushNotice, count: u32) -> Result<()> {
    if focused(&*app.0.core.lock().await, n.user_id) {
        return Ok(());
    }
    let mut c = app.0.pool.acquire().await?;
    if !allowed(app, &mut c, n).await? {
        return Ok(());
    }
    let rows=sqlx::query("SELECT p.id,p.endpoint,p.p256dh,p.auth FROM push_subscriptions p JOIN sessions s ON s.token_hash=p.session_hash WHERE s.user_id=? AND s.expires_at>? LIMIT 25").bind(db::id(n.user_id)?).bind(state::now() as i64).fetch_all(&mut *c).await?;
    drop(c);
    let payload = serde_json::to_vec(&PushPayload {
        url: format!("/?channel={}&message={}", n.channel_id.0, n.message_id.0),
        channel_id: n.channel_id,
        message_id: n.message_id,
        count,
        title: "StickTalk".into(),
        body: if n.mentioned {
            "You were mentioned"
        } else {
            "New direct message"
        }
        .into(),
    })
    .map_err(|_| crypto_error())?;
    for row in rows {
        let mut c = app.0.pool.acquire().await?;
        // Recheck permissions, preferences and focus between network sends.
        if !allowed(app, &mut c, n).await? {
            break;
        }
        let id: String = row.get("id");
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM push_subscriptions p JOIN sessions s ON s.token_hash=p.session_hash WHERE p.id=? AND s.expires_at>?)").bind(&id).bind(state::now() as i64).fetch_one(&mut *c).await?;
        drop(c);
        if !exists {
            continue;
        }
        if focused(&*app.0.core.lock().await, n.user_id) {
            break;
        }
        let endpoint = app.0.push.endpoint(&row.get::<String, _>("endpoint"))?;
        let body = encrypt(
            &decode(&row.get::<String, _>("p256dh"), 65)?,
            &decode(&row.get::<String, _>("auth"), 16)?,
            &payload,
        )?;
        let topic = URL_SAFE_NO_PAD
            .encode(&state::hash(format!("{}:{}", n.user_id.0, n.channel_id.0).as_bytes())[..16]);
        let response = app
            .0
            .push
            .client
            .post(endpoint.clone())
            .header("authorization", app.0.push.authorization(&endpoint)?)
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header("ttl", "300")
            .header("urgency", "normal")
            .header("topic", topic)
            .body(body)
            .send()
            .await;
        if response.is_ok_and(|r| matches!(r.status().as_u16(), 404 | 410)) {
            sqlx::query("DELETE FROM push_subscriptions WHERE id=?")
                .bind(id)
                .execute(&app.0.pool)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
