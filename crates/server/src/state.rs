use crate::{
    db,
    error::{require, Error, Result},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bytes::Bytes;
use clap::Parser;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use sticktalk_proto::*;
use tokio::sync::{mpsc, watch, Mutex, Semaphore};
#[derive(Parser, Clone, Debug)]
#[command(version, about = "StickTalk Phase 1 chat server")]
pub struct Config {
    #[command(flatten)]
    pub browser: crate::browser::BrowserOptions,
    #[command(flatten)]
    pub operations: crate::admin::OperationsConfig,
    #[arg(long, env = "STICKTALK_BIND", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
    #[arg(long, env = "STICKTALK_DATA_DIR", default_value = "./sticktalk-data")]
    pub data_dir: PathBuf,
    #[arg(
        long,
        env = "STICKTALK_PUBLIC_URL",
        default_value = "http://localhost:8080"
    )]
    pub public_url: String,
    #[arg(long, env = "STICKTALK_MAX_UPLOAD_MB", default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=8796093022207))]
    pub max_upload_mb: u64,
    /// Global disk admission budget, including thumbnails and in-flight files.
    #[arg(long, env = "STICKTALK_MAX_TOTAL_UPLOAD_MB", default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=8796093022207))]
    pub max_total_upload_mb: u64,
    #[arg(long, env = "STICKTALK_WORKER_ID", default_value_t = 0, value_parser = clap::value_parser!(u16).range(0..=1023))]
    pub worker_id: u16,
    #[arg(long, env = "STICKTALK_TRUSTED_PROXY", default_value = "127.0.0.1")]
    pub trusted_proxy: std::net::IpAddr,
    #[arg(long, env = "STICKTALK_GOOGLE_CLIENT_ID")]
    pub google_client_id: Option<String>,
    /// Local integration tests only; requires a loopback bind and loopback HTTP URL.
    #[arg(long, env = "STICKTALK_MOCK_GOOGLE_JWKS")]
    pub mock_google_jwks: Option<String>,
    #[arg(long, env = "STICKTALK_MOCK_GOOGLE_ISSUER")]
    pub mock_google_issuer: Option<String>,
    #[command(flatten)]
    pub gifs: crate::gifs::GifOptions,
    #[command(flatten)]
    pub voice: crate::voice::VoiceOptions,
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0; n];
    rand::rngs::OsRng.fill_bytes(&mut v);
    v
}
pub fn token() -> String {
    URL_SAFE_NO_PAD.encode(random(32))
}
pub fn hash(s: &[u8]) -> Vec<u8> {
    Sha256::digest(s).to_vec()
}
pub fn stream_id() -> String {
    random(16).iter().map(|b| format!("{b:02x}")).collect()
}
pub fn encode(msg: &ServerMsg) -> Result<Bytes> {
    let b = rmp_serde::to_vec_named(msg).map_err(|_| Error::new(ErrorCode::Internal))?;
    require(
        b.len() <= MAX_WS_FRAME_BYTES as usize,
        ErrorCode::PayloadTooLarge,
    )?;
    Ok(b.into())
}
#[derive(Clone)]
pub struct App(pub(crate) Arc<Inner>);
pub(crate) struct Inner {
    pub tier: crate::tier::Tier,
    pub config: Config,
    pub voice: crate::voice::Voice,
    pub notifications: tokio::sync::broadcast::Sender<crate::mentions::Notification>,
    pub browser: crate::browser::Browser,
    pub push: crate::push::Push,
    pub pool: SqlitePool,
    pub core: Mutex<Core>,
    pub passwords: Arc<Semaphore>,
    pub images: Arc<Semaphore>,
    pub sockets: Arc<Semaphore>,
    pub requests: Arc<Semaphore>,
    pub uploads: Arc<Semaphore>,
    pub transfers: Arc<Semaphore>,
    pub download_limits: std::sync::Mutex<HashMap<String, std::sync::Weak<Semaphore>>>,
    pub file_gc: Arc<Semaphore>,
    pub upload_io: Mutex<()>,
    pub dummy_hash: String,
    pub google: crate::google::Verifier,
    pub gifs: crate::gifs::GifService,
    pub(crate) content: crate::content::Content,
    pub shutdown: tokio_util::sync::CancellationToken,
    pub nav: crate::nav::Nav,
    pub authsec: crate::authsec::AuthSec,
}
pub(crate) struct Core {
    pub streams: HashMap<Vec<u8>, Stream>,
    pub voice: HashMap<VoiceSession, crate::voice::Entry>,
    pub calls: crate::calls::Calls,
    pub limits: HashMap<String, Bucket>,
    pub moderation: crate::moderation::Runtime,
    last_id: u64,
    worker: u16,
}
pub(crate) struct Bucket {
    credit: f64,
    at: u64,
}
pub(crate) struct Packet {
    pub bytes: Bytes,
    pub _permit: tokio::sync::OwnedSemaphorePermit,
}
pub(crate) struct Connection {
    pub id: String,
    pub focused: bool,
    pub alive: Arc<std::sync::atomic::AtomicBool>,
    pub tx: mpsc::Sender<Packet>,
    pub budget: Arc<Semaphore>,
    pub close: watch::Sender<Option<u16>>,
}
pub(crate) struct Stream {
    pub user: Id,
    pub id: String,
    pub seq: u64,
    pub floor: u64,
    pub ring: VecDeque<(u64, u64, Bytes)>,
    pub bytes: usize,
    pub touched: u64,
    pub ever_connected: bool,
    pub connection: Option<Connection>,
    pub guild: Option<Id>,
    pub interests: Vec<Id>,
    pub launch_features: bool,
    pub text_features: bool,
    /// Sent `ContentCommand::Subscribe`: receives `Event::Content`.
    pub content_features: bool,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
impl Stream {
    pub fn new(user: Id) -> Self {
        Self {
            user,
            id: stream_id(),
            seq: 0,
            floor: 0,
            ring: VecDeque::new(),
            bytes: 0,
            touched: now(),
            ever_connected: false,
            connection: None,
            guild: None,
            interests: vec![],
            launch_features: false,
            text_features: false,
            content_features: false,
        }
    }
    pub fn cursor(&self) -> ResumeCursor {
        ResumeCursor {
            stream_id: self.id.clone(),
            seq: self.seq,
        }
    }
    pub fn clear_replay(&mut self) {
        self.ring.clear();
        self.bytes = 0;
        self.floor = self.seq;
    }
    pub fn queue(&mut self, bytes: Bytes) {
        if let Some(c) = &self.connection {
            let permit = c.budget.clone().try_acquire_many_owned(bytes.len() as u32);
            let sent = permit
                .ok()
                .is_some_and(|p| c.tx.try_send(Packet { bytes, _permit: p }).is_ok());
            if !sent {
                let _ = c.close.send(Some(4010));
                self.connection = None;
                self.touched = now();
            }
        }
    }
}
impl Core {
    pub async fn persist_id(&self, c: &mut sqlx::SqliteConnection) -> Result<()> {
        sqlx::query("UPDATE id_highwater SET value=MAX(value,?) WHERE singleton=1")
            .bind(self.last_id as i64)
            .execute(c)
            .await?;
        Ok(())
    }
    pub async fn id(&mut self) -> Result<Id> {
        for _ in 0..3 {
            let t = now()
                .checked_sub(SNOWFLAKE_EPOCH_MS)
                .ok_or_else(|| Error::new(ErrorCode::Unavailable))?;
            require(t < (1 << 41), ErrorCode::Unavailable)?;
            let base = (t << 22) | (u64::from(self.worker) << 12);
            if base > self.last_id {
                self.last_id = base.max(1);
                return Ok(Id(self.last_id));
            }
            if self.last_id >> 22 == t
                && (self.last_id & 4095) < 4095
                && ((self.last_id >> 12) & 1023) == u64::from(self.worker)
            {
                self.last_id += 1;
                return Ok(Id(self.last_id));
            }
            if self.last_id >> 22 > t {
                return Err(Error::new(ErrorCode::Unavailable));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        Err(Error::new(ErrorCode::Unavailable))
    }
    pub fn rate(&mut self, key: String, per_minute: f64, burst: f64) -> Result<()> {
        let t = now();
        if self.limits.len() >= 20_000 {
            self.limits.retain(|_, b| t.saturating_sub(b.at) < 600_000);
            require(
                self.limits.len() < 20_000 || self.limits.contains_key(&key),
                ErrorCode::Unavailable,
            )?;
        }
        let b = self.limits.entry(key).or_insert(Bucket {
            credit: burst,
            at: t,
        });
        b.credit = (b.credit + t.saturating_sub(b.at) as f64 * per_minute / 60_000.).min(burst);
        b.at = t;
        if b.credit < 1. {
            return Err(Error::rate(
                ((1. - b.credit) * 60_000. / per_minute).ceil() as u32
            ));
        }
        b.credit -= 1.;
        Ok(())
    }
    pub fn cursor(&mut self, key: &[u8], user: Id) -> ResumeCursor {
        // REST-only sessions must not grow stream metadata without bound. Evict
        // disconnected streams first; their next WS identify safely gets Ready.
        if !self.streams.contains_key(key) && self.streams.len() >= 4096 {
            if let Some(oldest) = self
                .streams
                .iter()
                .filter(|(_, s)| s.connection.is_none())
                .min_by_key(|(_, s)| s.touched)
                .map(|(k, _)| k.clone())
            {
                self.streams.remove(&oldest);
            }
        }
        self.streams
            .entry(key.to_vec())
            .or_insert_with(|| Stream::new(user))
            .cursor()
    }
    pub fn send(&mut self, key: &[u8], msg: &ServerMsg) -> Result<()> {
        let mut projected;
        let msg = if self.streams.get(key).is_some_and(|s| !s.launch_features) {
            projected = msg.clone();
            crate::legacy::message(&mut projected);
            &projected
        } else {
            msg
        };
        let b = encode(msg)?;
        if let Some(s) = self.streams.get_mut(key) {
            s.queue(b);
        }
        Ok(())
    }
    pub fn emit(&mut self, users: &[Id], event: Event) -> Result<()> {
        self.emit_filtered(users, event, |_| true)
    }
    pub fn emit_filtered(
        &mut self,
        users: &[Id],
        event: Event,
        filter: impl Fn(&Stream) -> bool,
    ) -> Result<()> {
        let mut cache: HashMap<(u64, bool), Bytes> = HashMap::new();
        let t = now();
        for s in self
            .streams
            .values_mut()
            .filter(|s| users.contains(&s.user) && filter(s))
        {
            if matches!(&event, Event::Moderation(_)) && !self.moderation.subscribed.contains(&s.id)
            {
                continue;
            }
            if !s.text_features
                && matches!(
                    &event,
                    Event::ServerFoldersUpdated(_)
                        | Event::SlowmodeUpdated { .. }
                        | Event::GuildEmojiUpdated { .. }
                )
            {
                continue;
            }
            if !s.content_features && matches!(&event, Event::Content(_)) {
                continue;
            }
            if !s.launch_features
                && matches!(
                    &event,
                    Event::RolesUpdated { .. }
                        | Event::ProfileUpdated(_)
                        | Event::GuildProfileUpdated(_)
                        | Event::PinsUpdated { .. }
                        | Event::NotificationSettingsUpdated(_)
                        | Event::SessionsUpdated
                )
            {
                continue;
            }
            if matches!(&event, Event::MemberUpsert(m) if s.guild != Some(m.guild_id) && s.user != m.user.id)
                || matches!(&event, Event::MemberRemoved{guild_id,..} if s.guild != Some(*guild_id))
            {
                continue;
            }
            if let Event::MemberRemoved { user_id, .. } = &event {
                s.interests.retain(|u| u != user_id);
            }
            if matches!(&event,Event::GuildRemoved{guild_id,..} if s.guild==Some(*guild_id)) {
                s.guild = None;
                s.interests.clear();
            }
            s.seq += 1;
            let b = if let Some(b) = cache.get(&(s.seq, s.launch_features)) {
                b.clone()
            } else {
                let mut projected = event.clone();
                if !s.launch_features {
                    crate::legacy::event(&mut projected);
                }
                let b = encode(&ServerMsg::Event(Box::new(EventEnvelope {
                    seq: s.seq,
                    event: projected,
                })))?;
                cache.insert((s.seq, s.launch_features), b.clone());
                b
            };
            s.bytes += b.len();
            s.ring.push_back((s.seq, t, b.clone()));
            while s.ring.len() > 2048
                || s.bytes > 2 * 1024 * 1024
                || s.ring
                    .front()
                    .is_some_and(|e| t.saturating_sub(e.1) > 600_000)
            {
                if let Some((seq, _, b)) = s.ring.pop_front() {
                    s.bytes -= b.len();
                    s.floor = seq;
                }
            }
            s.queue(b);
        }
        self.trim();
        Ok(())
    }
    pub fn invalidate(&mut self, users: &[Id]) {
        for s in self
            .streams
            .values_mut()
            .filter(|s| users.contains(&s.user))
        {
            s.clear_replay();
        }
    }
    pub fn trim(&mut self) {
        let t = now();
        self.streams
            .retain(|_, s| s.connection.is_some() || t.saturating_sub(s.touched) <= 600_000);
        let mut total: usize = self.streams.values().map(|s| s.bytes).sum();
        if total > 64 * 1024 * 1024 {
            let mut keys: Vec<_> = self
                .streams
                .iter()
                .map(|(k, s)| (s.connection.is_some(), s.touched, k.clone()))
                .collect();
            keys.sort();
            for (_, _, k) in keys {
                if total <= 64 * 1024 * 1024 {
                    break;
                }
                if let Some(s) = self.streams.get_mut(&k) {
                    total -= s.bytes;
                    s.clear_replay();
                }
            }
        }
    }
}
impl App {
    pub async fn open(config: Config) -> Result<Self> {
        let verifier = crate::google::Verifier::configured(&config)?;
        Self::open_with_verifier(config, verifier).await
    }
    pub(crate) async fn open_with_verifier(
        config: Config,
        google: crate::google::Verifier,
    ) -> Result<Self> {
        require(
            config
                .google_client_id
                .as_ref()
                .is_none_or(|id| !id.trim().is_empty() && id.len() <= 512),
            ErrorCode::InvalidRequest,
        )?;
        if let Some(url) = &config.gifs.mock_gif_base {
            require(config.bind.ip().is_loopback(), ErrorCode::InvalidRequest)?;
            validate_mock_url(url)?;
        }
        tokio::fs::create_dir_all(config.data_dir.join("files")).await?;
        let options = SqliteConnectOptions::new()
            .filename(config.data_dir.join("sticktalk.sqlite3"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .min_connections(1)
            .connect_with(options)
            .await?;
        let mut migration = pool.acquire().await?;
        db::migrate(&mut migration).await?;
        drop(migration);
        let row = sqlx::query("SELECT MAX(id) AS id FROM (SELECT id FROM users UNION ALL SELECT id FROM guilds UNION ALL SELECT id FROM channels UNION ALL SELECT id FROM messages UNION ALL SELECT id FROM invites UNION ALL SELECT id FROM uploads UNION ALL SELECT id FROM guild_emoji UNION ALL SELECT value AS id FROM id_highwater)").fetch_one(&pool).await?;
        let last_id = row.try_get::<Option<i64>, _>("id")?.unwrap_or(0) as u64;
        let dummy_hash = tokio::task::spawn_blocking(|| {
            crate::http::password_hash("unused dummy credential".into())
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))??;
        let worker = config.worker_id;
        let gifs = crate::gifs::GifService::new(&config.gifs)?;
        let voice = crate::voice::Voice::new(&config.voice, config.bind)?;
        let browser = crate::browser::Browser::new(&config)?;
        let push = crate::push::Push::new(&config).await?;
        let tier = crate::tier::Tier::new(&config)?;
        let content = crate::content::Content::configured(&config.operations.content, config.bind)?;
        let authsec = crate::authsec::AuthSec::new(&config)?;
        let app = Self(Arc::new(Inner {
            tier,
            browser,
            push,
            voice,
            notifications: tokio::sync::broadcast::channel(128).0,
            config,
            pool,
            core: Mutex::new(Core {
                streams: HashMap::new(),
                voice: HashMap::new(),
                calls: Default::default(),
                limits: HashMap::new(),
                moderation: Default::default(),
                last_id,
                worker,
            }),
            passwords: Arc::new(Semaphore::new(2)),
            images: Arc::new(Semaphore::new(1)),
            sockets: Arc::new(Semaphore::new(1024)),
            requests: Arc::new(Semaphore::new(128)),
            uploads: Arc::new(Semaphore::new(16)),
            transfers: Arc::new(Semaphore::new(64)),
            download_limits: std::sync::Mutex::new(HashMap::new()),
            file_gc: Arc::new(Semaphore::new(1)),
            upload_io: Mutex::new(()),
            dummy_hash,
            google,
            gifs,
            content,
            shutdown: tokio_util::sync::CancellationToken::new(),
            nav: Default::default(),
            authsec,
        }));
        app.housekeeping().await?;
        sqlx::query("DELETE FROM upload_reservations")
            .execute(&app.0.pool)
            .await?;
        let mut dir = tokio::fs::read_dir(app.0.config.data_dir.join("files")).await?;
        while let Some(e) = dir.next_entry().await? {
            let key = e.file_name().to_string_lossy().to_string();
            let exists: i64 = sqlx::query_scalar(
                "SELECT (SELECT COUNT(*) FROM uploads WHERE storage_key=? OR thumbnail_key=?) + (SELECT COUNT(*) FROM partial_uploads WHERE storage_key=? OR thumbnail_key=?)",
            )
            .bind(&key)
            .bind(&key)
            .bind(&key)
            .bind(&key)
            .fetch_one(&app.0.pool)
            .await?;
            if exists == 0 && e.file_type().await?.is_file() {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
        Ok(app)
    }
    pub async fn housekeeping(&self) -> Result<()> {
        crate::resumable::gc(self).await?;
        // Maintenance writes obey the same writer gate as account mutations.
        let mut core = self.0.core.lock().await;
        let mut conn = self.0.pool.acquire().await?;
        sqlx::query("DELETE FROM reserved_usernames WHERE release_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM google_challenges WHERE expires_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM reauth_proofs WHERE expires_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM web_oauth_states WHERE expires_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM mcp_codes WHERE expires_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM mcp_access_tokens WHERE token_hash IN (SELECT token_hash FROM mcp_access_tokens WHERE expires_at<=? LIMIT 1024)").bind(now() as i64).execute(&mut *conn).await?;
        crate::voice::reconcile(self, &mut core, &mut conn).await?;
        crate::commands::expire_invites(&mut conn, &mut core).await?;
        let mut expired_users = vec![];
        let active: Vec<Vec<u8>> = core.streams.keys().cloned().collect();
        for key in active {
            let alive: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=sessions.user_id))").bind(&key).bind(now() as i64).fetch_one(&mut *conn).await?;
            if !alive {
                if let Some(s) = core.streams.remove(&key) {
                    expired_users.push(s.user);
                    if let Some(c) = s.connection {
                        let _ = c.close.send(Some(4001));
                    }
                }
            }
        }
        expired_users.sort();
        expired_users.dedup();
        for user in expired_users {
            if !core
                .streams
                .values()
                .any(|s| s.user == user && s.connection.is_some())
            {
                crate::commands::broadcast_presence(&mut conn, &mut core, user).await?;
            }
        }
        sqlx::query("DELETE FROM sessions WHERE token_hash IN (SELECT token_hash FROM sessions WHERE expires_at<=? LIMIT 256)").bind(now() as i64).execute(&mut *conn).await?;
        sqlx::query("DELETE FROM oauth_signup_tickets WHERE token_hash IN (SELECT token_hash FROM oauth_signup_tickets WHERE expires_at<=? LIMIT 1024)").bind(now() as i64).execute(&mut *conn).await?;
        sqlx::query("DELETE FROM command_receipts WHERE rowid IN (SELECT rowid FROM command_receipts WHERE expires_at<=? LIMIT 1024)").bind(now() as i64).execute(&mut *conn).await?;
        sqlx::query("DELETE FROM upload_daily_usage WHERE utc_day<?")
            .bind((now() / 86_400_000).saturating_sub(2) as i64)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM password_resets WHERE expires_at<=?")
            .bind(now() as i64)
            .execute(&mut *conn)
            .await?;
        // Collect file keys before deleting expired metadata.
        let mut paths: Vec<String> = vec![];
        for r in sqlx::query(
            "SELECT storage_key,thumbnail_key FROM uploads WHERE expires_at<=? LIMIT 100",
        )
        .bind(now() as i64)
        .fetch_all(&mut *conn)
        .await?
        {
            paths.push(r.get("storage_key"));
            if let Some(k) = r.get::<Option<String>, _>("thumbnail_key") {
                paths.push(k);
            }
        }
        sqlx::query("DELETE FROM uploads WHERE id IN (SELECT id FROM uploads WHERE expires_at<=? LIMIT 100)").bind(now() as i64).execute(&mut *conn).await?;
        paths.extend(db::gc_uploads(&mut conn).await?);
        core.trim();
        drop(conn);
        drop(core);
        if !paths.is_empty() {
            self.0.tier.wake.notify_one();
        }
        for key in paths {
            let _ = tokio::fs::remove_file(self.0.config.data_dir.join("files").join(key)).await;
        }
        Ok(())
    }
    /// Advertised ICE endpoint (one UDP socket in the configured range).
    pub fn voice_addr(&self) -> SocketAddr {
        self.0.voice.address
    }
    pub fn shutdown(&self) {
        self.0.voice.clear();
        self.0.shutdown.cancel();
    }
    pub async fn collect_orphan_files(&self) -> Result<()> {
        let Ok(_permit) = self.0.file_gc.clone().try_acquire_owned() else {
            return Ok(());
        };
        let mut dir = tokio::fs::read_dir(self.0.config.data_dir.join("files")).await?;
        while let Some(entry) = dir.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }
            let key = entry.file_name().to_string_lossy().to_string();
            let reservation = key.split('.').next().unwrap_or_default();
            let exists = {
                let _gate = self.0.core.lock().await;
                sqlx::query_scalar::<_,i64>("SELECT EXISTS(SELECT 1 FROM uploads WHERE storage_key=? OR thumbnail_key=?) OR EXISTS(SELECT 1 FROM upload_reservations WHERE key=?) OR EXISTS(SELECT 1 FROM partial_uploads WHERE storage_key=? OR thumbnail_key=?)")
                    .bind(&key).bind(&key).bind(reservation).bind(&key).bind(&key).fetch_one(&self.0.pool).await?
            };
            if exists == 0 {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        Ok(())
    }
    pub async fn serve(self, listener: tokio::net::TcpListener) -> std::io::Result<()> {
        let app = self.clone();
        let maintenance = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.tick().await;
            loop {
                tokio::select! { _=app.0.shutdown.cancelled()=>break, _=tick.tick()=> {
                    if let Err(error) = app.housekeeping().await {
                        tracing::warn!(?error, "housekeeping failed");
                    }
                    if let Err(error) = app.collect_orphan_files().await {
                        tracing::warn!(?error, "orphan file collection failed");
                    }
                } }
            }
        });
        let storage = self.clone();
        let storage_task = tokio::spawn(crate::tier::background(storage));
        let stop = self.clone();
        use axum::serve::ListenerExt as _;
        let result = axum::serve(
            crate::http_io::Listener(listener).tap_io(|_| {}),
            crate::router(self.clone()).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            stop.0.shutdown.cancelled().await;
            let core = stop.0.core.lock().await;
            for s in core.streams.values() {
                if let Some(c) = &s.connection {
                    let _ = c.close.send(Some(1001));
                }
            }
        })
        .await;
        self.shutdown();
        let _ = maintenance.await;
        storage_task.abort();
        let _ = storage_task.await;
        // Axum does not track HTTP-upgraded WebSockets in its graceful drain.
        // Keep the runtime alive until their close writers/disconnects finish.
        if tokio::time::timeout(
            Duration::from_secs(15),
            self.0.sockets.clone().acquire_many_owned(1024),
        )
        .await
        .is_err()
        {
            tracing::warn!("WebSocket shutdown deadline exceeded");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core() -> Core {
        Core {
            streams: HashMap::new(),
            voice: HashMap::new(),
            calls: Default::default(),
            limits: HashMap::new(),
            moderation: Default::default(),
            last_id: 0,
            worker: 0,
        }
    }
    #[tokio::test]
    async fn snowflakes_and_rollback() {
        let mut c = core();
        let mut previous = 0;
        for _ in 0..5000 {
            let id = c.id().await.unwrap().0;
            assert!(id > previous && id <= i64::MAX as u64);
            previous = id;
        }
        c.last_id = ((now() - SNOWFLAKE_EPOCH_MS + 1000) << 22) | 1;
        assert!(c.id().await.is_err());
    }
    #[test]
    fn bounded_shared_replay_and_slow_consumer() {
        let mut c = core();
        let user = Id(1);
        c.streams.insert(vec![1], Stream::new(user));
        c.streams.insert(vec![2], Stream::new(user));
        let e = Event::Typing {
            channel_id: Id(2),
            user_id: user,
            expires_at: now() + 8000,
        };
        for _ in 0..2100 {
            c.emit(&[user], e.clone()).unwrap();
        }
        assert_eq!(c.streams[&vec![1]].ring.len(), 2048);
        assert!(c.streams[&vec![1]].floor > 0);
        let a = &c.streams[&vec![1]].ring[0].2;
        let b = &c.streams[&vec![2]].ring[0].2;
        assert_eq!(a.as_ptr(), b.as_ptr());
        let (tx, _rx) = mpsc::channel(128);
        let (close, rx) = watch::channel(None);
        let s = c.streams.get_mut(&vec![1]).unwrap();
        s.connection = Some(Connection {
            focused: false,
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            id: "test".into(),
            tx,
            budget: Arc::new(Semaphore::new(1)),
            close,
        });
        c.emit(&[user], e).unwrap();
        assert_eq!(*rx.borrow(), Some(4010));
        assert!(c.streams[&vec![1]].connection.is_none());
    }
    #[test]
    fn global_replay_budget_and_limiter() {
        let mut c = core();
        for n in 1..=40 {
            c.streams.insert(vec![n as u8], Stream::new(Id(n)));
        }
        let users: Vec<_> = (1..=40).map(Id).collect();
        let e = Event::UserUpdated(UserSummary {
            id: Id(1),
            username: "x".repeat(300_000),
            display_name: None,
            avatar_id: None,
            status_text: None,
            style: Default::default(),
        });
        for _ in 0..8 {
            c.emit(&users, e.clone()).unwrap();
        }
        assert!(c.streams.values().map(|s| s.bytes).sum::<usize>() <= 64 * 1024 * 1024);
        assert!(c.rate("user".into(), 60., 1.).is_ok());
        assert!(c.rate("user".into(), 60., 1.).is_err());
    }
    #[test]
    fn maximal_bootstrap_fits_frame() {
        let user = UserSummary {
            id: Id(1),
            username: "u".repeat(32),
            display_name: Some("n".repeat(64)),
            avatar_id: Some(Id(2)),
            status_text: Some("s".repeat(128)),
            style: Default::default(),
        };
        let mut ready = Ready {
            roles: vec![],
            memberships: vec![],
            notification_settings: vec![],
            cursor: ResumeCursor {
                stream_id: stream_id(),
                seq: 0,
            },
            me: SelfUser {
                user: user.clone(),
                desired_presence: PresenceStatus::Online,
            },
            guilds: vec![],
            channels: vec![],
            relationships: vec![],
            users: vec![],
            read_states: vec![],
            presences: vec![],
            voice_states: vec![],
        };
        for g in 1..=10 {
            ready.guilds.push(GuildSummary {
                guild: Guild {
                    id: Id(g),
                    owner_id: Id(1),
                    name: "g".repeat(64),
                    icon_id: Some(Id(2)),
                    member_count: 1000,
                },
                my_role: MemberRole::Owner,
                my_permissions: Permissions::GUILD_OWNER,
            });
            for n in 1..=32 {
                let id = Id(1000 + g * 32 + n);
                ready.channels.push(ChannelSummary {
                    id,
                    kind: ChannelKind::Text,
                    guild_id: Some(Id(g)),
                    parent_id: None,
                    name: Some("c".repeat(64)),
                    position: n as u16 - 1,
                    recipient_ids: vec![],
                    owner_id: None,
                });
                ready.read_states.push(ReadState {
                    channel_id: id,
                    last_read_id: Some(Id(i64::MAX as u64)),
                    latest_message_id: Some(Id(i64::MAX as u64)),
                    unread_count: u32::MAX,
                    mention_count: u32::MAX,
                });
            }
        }
        for n in 0..64 {
            ready.channels.push(ChannelSummary {
                id: Id(2000 + n),
                kind: ChannelKind::GroupDm,
                guild_id: None,
                parent_id: None,
                name: Some("d".repeat(64)),
                position: 0,
                recipient_ids: (1..=16).map(|v| Id(i64::MAX as u64 - v)).collect(),
                owner_id: Some(Id(1)),
            });
            ready.users.push(user.clone());
            ready.read_states.push(ReadState {
                channel_id: Id(2000 + n),
                last_read_id: None,
                latest_message_id: None,
                unread_count: 0,
                mention_count: 0,
            });
        }
        for _ in 0..100 {
            ready.relationships.push(Relationship {
                user: user.clone(),
                kind: RelationshipKind::Friend,
                since: now(),
            });
            ready.presences.push(Presence {
                user_id: Id(1),
                status: PresenceStatus::Online,
            });
        }
        // Include the global maximum call population and maximal publication metadata.
        for n in 0..64 {
            ready.voice_states.push(GuildVoiceState {
                guild_id: Id(1),
                channel_id: Id(100),
                participant: VoiceParticipant {
                    session: VoiceSession {
                        user_id: Id(3000 + n),
                        connection_id: Id(4000 + n),
                    },
                    muted: false,
                    deafened: false,
                    tracks: [
                        TrackKind::Audio,
                        TrackKind::Camera,
                        TrackKind::Screen,
                        TrackKind::SystemAudio,
                    ]
                    .into_iter()
                    .map(|kind| PublishedTrack {
                        mid: "m".repeat(16),
                        kind,
                        codec: matches!(kind, TrackKind::Camera | TrackKind::Screen)
                            .then_some(VideoCodec::H264),
                        layers: vec!["r".repeat(16); 3],
                        surface: (kind == TrackKind::Screen).then_some(ShareSurface::Monitor),
                    })
                    .collect(),
                },
            });
        }
        assert!(encode(&ServerMsg::Ready(Box::new(ready))).unwrap().len() < 512 * 1024);
    }
}

/// Mock upstream configuration must never send credentials over a public HTTP origin.
pub(crate) fn validate_mock_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        ErrorCode::InvalidRequest,
    )
}

#[cfg(test)]
mod mock_configuration_tests {
    use super::*;
    #[test]
    fn local_upstreams_cannot_be_public_or_credential_bearing() {
        assert!(validate_mock_url("http://127.0.0.1:1234/jwks").is_ok());
        for value in [
            "http://example.com:1234/jwks",
            "http://localhost:1234/jwks",
            "https://127.0.0.1:1234/jwks",
            "http://u:p@127.0.0.1:1234/jwks",
            "http://127.0.0.1:1234/jwks?token=secret",
            "http://127.0.0.1:1234/jwks#fragment",
        ] {
            assert!(validate_mock_url(value).is_err(), "{value}");
        }
        let config = Config::try_parse_from([
            "server",
            "--bind",
            "0.0.0.0:8080",
            "--mock-google-jwks",
            "http://127.0.0.1:1234/jwks",
            "--mock-google-issuer",
            "http://127.0.0.1:1234",
        ])
        .unwrap();
        assert!(crate::google::Verifier::configured(&config).is_err());
        let config =
            Config::try_parse_from(["server", "--mock-google-jwks", "http://127.0.0.1:1234/jwks"])
                .unwrap();
        assert!(crate::google::Verifier::configured(&config).is_err());
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.voice.clear();
        self.shutdown.cancel();
    }
}

#[cfg(test)]
mod text_feature_compatibility_tests {
    use super::*;
    #[tokio::test]
    async fn older_launch_peers_never_receive_new_text_variants() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut core = app.0.core.lock().await;
        core.cursor(&[41; 32], Id(1));
        core.cursor(&[42; 32], Id(1));
        core.streams.get_mut(&vec![41; 32]).unwrap().launch_features = true;
        let modern = core.streams.get_mut(&vec![42; 32]).unwrap();
        modern.launch_features = true;
        modern.text_features = true;
        for event in [
            Event::GuildEmojiUpdated {
                guild_id: Id(10),
                emoji: vec![],
            },
            Event::SlowmodeUpdated {
                channel_id: Id(11),
                seconds: 5,
            },
            Event::ServerFoldersUpdated(vec![]),
        ] {
            core.emit(&[Id(1)], event).unwrap();
        }
        assert!(core.streams[&vec![41; 32]].ring.is_empty());
        assert_eq!(core.streams[&vec![42; 32]].ring.len(), 3);
        let old: Identify = serde_json::from_str(
            r#"{"version":1,"token":"legacy","resume":null,"launch_features":true}"#,
        )
        .unwrap();
        assert!(!old.text_features);
        assert!(serde_json::to_value(old)
            .unwrap()
            .get("text_features")
            .is_none());
    }
}
