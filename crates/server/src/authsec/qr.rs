//! Sign in by scanning a QR code with a device that is already signed in.
//!
//! The signed-out browser holds a WebSocket. The server pushes it a code (a
//! one-time token in a URL fragment) and replaces the code when it expires, on
//! a timer armed for that expiry. The token grants nothing by itself: a
//! signed-in device must open it, is shown which device and network is asking,
//! and must confirm. Only then does the server push a one-time, one-minute
//! ticket down that same socket, and only that browser can redeem it for its
//! own new session. Nothing is stored in the database.
use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::mpsc;

pub(super) const CODE_TTL: u64 = if cfg!(test) { 2_000 } else { 120_000 };
/// Codes per socket before it asks the person to reload (20 minutes).
pub(super) const MAX_CODES: u32 = if cfg!(test) { 3 } else { 10 };
const TICKET_TTL: u64 = 60_000;
const MAX_SOCKETS: usize = 256;

struct Entry {
    token: String,
    expires_at: u64,
    device: String,
    ip: String,
    created_at: u64,
    /// The signed-in session that opened the current code.
    scanned_by: Option<(Vec<u8>, Id)>,
    tx: mpsc::Sender<Event>,
}
#[derive(Debug)]
enum Event {
    Scanned,
    Rejected,
    Approved(String),
}
#[derive(Default)]
struct Inner {
    entries: HashMap<u64, Entry>,
    /// Current token -> entry.
    tokens: HashMap<String, u64>,
    /// SHA-256(ticket) -> (user, expiry).
    tickets: HashMap<Vec<u8>, (Id, u64)>,
}
#[derive(Default)]
pub(super) struct Qr {
    inner: Mutex<Inner>,
    next: AtomicU64,
}
impl Qr {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/auth/qr/socket", get(socket))
        .route("/api/v1/auth/qr/inspect", post(inspect))
        .route("/api/v1/auth/qr/approve", post(approve))
        .route("/api/v1/auth/qr/reject", post(reject))
        .route("/api/v1/auth/qr/redeem", post(redeem))
        .route("/api/v1/browser/session/qr", post(browser_redeem))
}

/// The QR matrix as '1'/'0' characters, row-major.
pub(super) fn matrix(text: &str) -> Result<(u32, String)> {
    let code = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium)
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    let size = code.size();
    let mut modules = String::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            modules.push(if code.get_module(x, y) { '1' } else { '0' });
        }
    }
    Ok((size as u32, modules))
}

async fn socket(
    State(app): State<App>,
    ws: WebSocketUpgrade,
    r: Request<Body>,
) -> Result<Response> {
    // Browsers always send Origin on a WebSocket; a page elsewhere cannot open one.
    same_origin(&app, r.headers())?;
    let ip = client_ip(&app, &r);
    app.0
        .core
        .lock()
        .await
        // Its own bucket: opening login pages must never use up approvals.
        .rate(format!("qr-socket-ip:{ip}"), 30., 10.)?;
    let device = device_label(
        r.headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    );
    let (tx, rx) = mpsc::channel(8);
    let entry = app.0.authsec.qr.next.fetch_add(1, Ordering::Relaxed);
    {
        let mut qr = app.0.authsec.qr.lock();
        require(qr.entries.len() < MAX_SOCKETS, ErrorCode::Unavailable)?;
        qr.entries.insert(
            entry,
            Entry {
                token: String::new(),
                expires_at: 0,
                device,
                ip,
                created_at: state::now(),
                scanned_by: None,
                tx,
            },
        );
    }
    let guard = Remove(app.clone(), entry);
    Ok(ws
        .max_message_size(1024)
        .on_upgrade(move |socket| run(app, entry, socket, rx, guard))
        .into_response())
}

/// Forget a socket's code and pending state however its task ends.
struct Remove(App, u64);
impl Drop for Remove {
    fn drop(&mut self) {
        let mut qr = self.0 .0.authsec.qr.lock();
        if let Some(e) = qr.entries.remove(&self.1) {
            qr.tokens.remove(&e.token);
        }
    }
}

/// Give the entry a fresh code; the old token stops working at once.
fn rotate(app: &App, entry: u64) -> Option<(String, u64)> {
    let mut qr = app.0.authsec.qr.lock();
    let token = state::token();
    let expires_at = state::now() + CODE_TTL;
    let old = {
        let e = qr.entries.get_mut(&entry)?;
        e.scanned_by = None;
        e.expires_at = expires_at;
        std::mem::replace(&mut e.token, token.clone())
    };
    qr.tokens.remove(&old);
    qr.tokens.insert(token.clone(), entry);
    Some((token, expires_at))
}

async fn run(
    app: App,
    entry: u64,
    socket: WebSocket,
    mut rx: mpsc::Receiver<Event>,
    _guard: Remove,
) {
    let (mut sink, mut incoming) = socket.split();
    let send =
        |m: &QrSocketMessage| Message::Text(serde_json::to_string(m).unwrap_or_default().into());
    let base = app.0.config.public_url.trim_end_matches('/').to_owned();
    for _ in 0..MAX_CODES {
        let Some((token, expires_at)) = rotate(&app, entry) else {
            return;
        };
        let url = format!("{base}/#qr={token}");
        let Ok((size, modules)) = matrix(&url) else {
            return;
        };
        if sink
            .send(send(&QrSocketMessage::Code {
                url,
                size,
                modules,
                expires_at,
            }))
            .await
            .is_err()
        {
            return;
        }
        let deadline = tokio::time::sleep(Duration::from_millis(CODE_TTL));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = app.0.shutdown.cancelled() => return,
                _ = &mut deadline => break,
                frame = incoming.next() => match frame {
                    // The page never needs to talk; anything but a ping ends it.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => return,
                },
                event = rx.recv() => match event {
                    Some(Event::Scanned) => {
                        if sink.send(send(&QrSocketMessage::Scanned)).await.is_err() { return; }
                    }
                    Some(Event::Rejected) => {
                        if sink.send(send(&QrSocketMessage::Rejected)).await.is_err() { return; }
                        break;
                    }
                    Some(Event::Approved(ticket)) => {
                        let _ = sink.send(send(&QrSocketMessage::Approved { ticket })).await;
                        let _ = sink.close().await;
                        return;
                    }
                    None => return,
                },
            }
        }
    }
    let _ = sink.send(send(&QrSocketMessage::Expired)).await;
    let _ = sink.close().await;
}

fn token_of(request: &QrTokenRequest) -> Result<&str> {
    require(request.token.len() == 43, ErrorCode::NotFound)?;
    Ok(&request.token)
}

async fn signed_in(app: &App, headers: &HeaderMap, ip: &str) -> Result<(Vec<u8>, Id)> {
    let (key, user) = authed(app, headers).await?;
    let mut core = app.0.core.lock().await;
    core.rate(format!("qr-user:{}", user.0), 10., 5.)?;
    core.rate(format!("qr-ip:{ip}"), 10., 5.)?;
    Ok((key, user))
}

async fn inspect(State(app): State<App>, r: Request<Body>) -> Result<axum::Json<QrLoginRequest>> {
    let my_ip = client_ip(&app, &r);
    let (key, user) = signed_in(&app, &r.headers().clone(), &my_ip).await?;
    let request: QrTokenRequest = http::parse(r).await?;
    let token = token_of(&request)?;
    let mut qr = app.0.authsec.qr.lock();
    let entry = *qr
        .tokens
        .get(token)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let e = qr
        .entries
        .get_mut(&entry)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(e.expires_at > state::now(), ErrorCode::NotFound)?;
    // First device to open a code owns it; another device sees nothing there.
    match &e.scanned_by {
        Some((k, u)) => require(*k == key && *u == user, ErrorCode::NotFound)?,
        None => {
            e.scanned_by = Some((key, user));
            let _ = e.tx.try_send(Event::Scanned);
        }
    }
    Ok(axum::Json(QrLoginRequest {
        device: e.device.clone(),
        same_network: e.ip == my_ip,
        ip: e.ip.clone(),
        created_at: e.created_at,
        expires_at: e.expires_at,
    }))
}

/// The entry this session opened with `token`, still unexpired.
fn opened(qr: &mut Inner, token: &str, key: &[u8], user: Id) -> Result<u64> {
    let entry = *qr
        .tokens
        .get(token)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let e = qr
        .entries
        .get(&entry)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(
        e.expires_at > state::now()
            && e.scanned_by
                .as_ref()
                .is_some_and(|(k, u)| k == key && *u == user),
        ErrorCode::NotFound,
    )?;
    Ok(entry)
}

async fn approve(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (key, user) = signed_in(&app, &r.headers().clone(), &client_ip(&app, &r)).await?;
    let request: QrTokenRequest = http::parse(r).await?;
    let token = token_of(&request)?;
    // A disabled or expired account cannot hand out sessions.
    crate::db::enabled(&mut *app.0.pool.acquire().await?, user).await?;
    let mut qr = app.0.authsec.qr.lock();
    let entry = opened(&mut qr, token, &key, user)?;
    let now = state::now();
    qr.tickets.retain(|_, (_, expires)| *expires > now);
    require(qr.tickets.len() < MAX_SOCKETS, ErrorCode::Unavailable)?;
    let ticket = state::token();
    let sent = qr
        .entries
        .get(&entry)
        .is_some_and(|e| e.tx.try_send(Event::Approved(ticket.clone())).is_ok());
    require(sent, ErrorCode::NotFound)?;
    qr.tickets
        .insert(state::hash(ticket.as_bytes()), (user, now + TICKET_TTL));
    // The code is spent: nobody can open or approve it again.
    qr.tokens.remove(token);
    Ok(http::ApiResponse::empty())
}

async fn reject(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (key, user) = signed_in(&app, &r.headers().clone(), &client_ip(&app, &r)).await?;
    let request: QrTokenRequest = http::parse(r).await?;
    let token = token_of(&request)?;
    let mut qr = app.0.authsec.qr.lock();
    let entry = opened(&mut qr, token, &key, user)?;
    qr.tokens.remove(token);
    if let Some(e) = qr.entries.get(&entry) {
        let _ = e.tx.try_send(Event::Rejected);
    }
    Ok(http::ApiResponse::empty())
}

/// One use, one minute, and only with the ticket the approving device caused
/// the server to push to the waiting socket.
pub(super) async fn redeem_ticket(app: &App, r: Request<Body>) -> Result<AuthResponse> {
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(app, &r), 10., 5.)?;
    let request: QrRedeemRequest = http::parse(r).await?;
    require(request.ticket.len() == 43, ErrorCode::InvalidCredentials)?;
    let (user, expires) = app
        .0
        .authsec
        .qr
        .lock()
        .tickets
        .remove(&state::hash(request.ticket.as_bytes()))
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    require(expires > state::now(), ErrorCode::InvalidCredentials)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    let auth = session(&mut tx, user).await?;
    tx.commit().await?;
    Ok(auth)
}

async fn redeem(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    Ok(http::json(redeem_ticket(&app, r).await?))
}

async fn browser_redeem(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    browser_only(&app, r.headers())?;
    Ok(crate::browser::browser_session(
        redeem_ticket(&app, r).await?,
    ))
}

#[cfg(test)]
pub(super) fn token_for_test(app: &App) -> Option<String> {
    app.0.authsec.qr.lock().tokens.keys().next().cloned()
}
#[cfg(test)]
pub(super) fn expire_tickets(app: &App) {
    for (_, expires) in app.0.authsec.qr.lock().tickets.values_mut() {
        *expires = 0;
    }
}
