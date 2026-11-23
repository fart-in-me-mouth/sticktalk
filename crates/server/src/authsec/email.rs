//! Optional self-service password reset by email.
//!
//! With SMTP configured, a user can add and verify one email address; anyone
//! can then ask for a reset link by username or address. The answer is always
//! the same and is sent before any lookup happens, so neither its content nor
//! its timing says whether an account exists. Links are single-use, short-lived
//! and stored hashed. A reset signs out every session. Without SMTP these
//! routes answer `feature_disabled` and the administrator reset code remains.
use super::*;
use axum::{
    extract::State,
    routing::{get, post},
};
use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::{authentication::Credentials, extension::ClientId},
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use sqlx::{Connection, Row};
use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

pub(super) const RESET_TTL: u64 = 30 * 60_000;
pub(super) const VERIFY_TTL: u64 = 24 * 3_600_000;
const MAX_IN_FLIGHT: usize = 64;
/// One address may belong to at most this many accounts that receive links.
const MAX_ACCOUNTS_PER_ADDRESS: i64 = 5;

pub(super) struct Mailer {
    transport: Option<(AsyncSmtpTransport<Tokio1Executor>, Mailbox)>,
    in_flight: Arc<AtomicUsize>,
}
impl Mailer {
    pub(super) fn new(config: &crate::Config) -> Result<Self> {
        let o = &config.browser.authsec;
        let Some(host) = o.smtp_host.as_deref() else {
            return Ok(Self {
                transport: None,
                in_flight: Default::default(),
            });
        };
        let invalid = || Error::new(ErrorCode::InvalidRequest);
        require(
            !host.trim().is_empty() && host.len() <= 253,
            ErrorCode::InvalidRequest,
        )?;
        let from: Mailbox = o
            .smtp_from
            .as_deref()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let mut builder = match o.smtp_security.as_str() {
            "starttls" => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host).map_err(|_| invalid())?
            }
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host).map_err(|_| invalid())?,
            "none" => {
                // Plain SMTP only to a relay on this machine (or a test server).
                let loopback = host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback());
                require(loopback, ErrorCode::InvalidRequest)?;
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
            }
            _ => return Err(invalid()),
        };
        if let Some(port) = o.smtp_port {
            builder = builder.port(port);
        }
        match (&o.smtp_username, &o.smtp_password) {
            (Some(user), Some(password)) => {
                builder = builder.credentials(Credentials::new(user.clone(), password.clone()))
            }
            (None, None) => {}
            _ => return Err(invalid()),
        }
        let hello = reqwest::Url::parse(&config.public_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "sticktalk".into());
        let transport = builder
            .hello_name(ClientId::Domain(hello))
            .timeout(Some(Duration::from_secs(15)))
            .build();
        Ok(Self {
            transport: Some((transport, from)),
            in_flight: Default::default(),
        })
    }
    pub(super) fn enabled(&self) -> bool {
        self.transport.is_some()
    }
    /// Queue one message. Bounded: past 64 unsent messages new ones are dropped.
    fn send(&self, to: &str, subject: &str, body: String) {
        let Some((transport, from)) = &self.transport else {
            return;
        };
        let Ok(to) = to.parse::<Mailbox>() else {
            return;
        };
        if self.in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            tracing::warn!("email queue full; message dropped");
            return;
        }
        let message = Message::builder()
            .from(from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body);
        let transport = transport.clone();
        let in_flight = self.in_flight.clone();
        tokio::spawn(async move {
            if let Ok(message) = message {
                if let Err(error) = transport.send(message).await {
                    // The error never contains the message body or link.
                    tracing::warn!(%error, "email delivery failed");
                }
            }
            in_flight.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route(
            "/api/v1/users/@me/email",
            get(status).put(set).delete(remove),
        )
        .route("/api/v1/auth/email/verify", post(verify))
        .route("/api/v1/auth/email-reset/request", post(request_reset))
        .route("/api/v1/auth/email-reset/complete", post(complete_reset))
}

fn enabled(app: &App) -> Result<()> {
    require(app.0.authsec.mail.enabled(), ErrorCode::FeatureDisabled)
}

/// A syntactically valid, lowercase-domain address of at most 254 bytes.
pub(super) fn address(value: &str) -> Result<String> {
    let value = value.trim();
    require(
        (3..=254).contains(&value.len())
            && value.is_ascii()
            && !value.chars().any(|c| c.is_control() || c.is_whitespace()),
        ErrorCode::InvalidRequest,
    )?;
    let parsed = Address::from_str(value).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        parsed.user().len() <= 64 && parsed.domain().contains('.'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(format!(
        "{}@{}",
        parsed.user(),
        parsed.domain().to_ascii_lowercase()
    ))
}

async fn email_status(c: &mut SqliteConnection, app: &App, user: Id) -> Result<EmailStatus> {
    let row =
        sqlx::query("SELECT email,verified_at,pending_email FROM user_emails WHERE user_id=?")
            .bind(id(user)?)
            .fetch_optional(c)
            .await?;
    Ok(EmailStatus {
        smtp_enabled: app.0.authsec.mail.enabled(),
        email: row.as_ref().and_then(|r| r.get("email")),
        verified: row
            .as_ref()
            .is_some_and(|r| r.get::<Option<i64>, _>("verified_at").is_some()),
        pending: row.as_ref().and_then(|r| r.get("pending_email")),
    })
}

async fn status(State(app): State<App>, headers: HeaderMap) -> Result<axum::Json<EmailStatus>> {
    let (_, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    Ok(axum::Json(
        email_status(&mut *app.0.pool.acquire().await?, &app, user).await?,
    ))
}

fn link(app: &App, kind: &str, token: &str) -> String {
    format!(
        "{}/#{kind}={token}",
        app.0.config.public_url.trim_end_matches('/')
    )
}

async fn set(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    enabled(&app)?;
    let headers = r.headers().clone();
    let (key, user) = authed(&app, &headers).await?;
    let request: SetEmailRequest = http::parse(r).await?;
    let email = address(&request.email)?;
    // Every change sends mail: a few per hour, per account.
    app.0
        .core
        .lock()
        .await
        .rate(format!("email-user:{}", user.0), 5. / 60., 3.)?;
    let token = state::token();
    let now = state::now();
    let gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    // An attacker holding a stolen session must not be able to point resets
    // at their own mailbox without proving the account's existing login.
    crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::Email).await?;
    sqlx::query("INSERT INTO user_emails(user_id,pending_email) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET pending_email=excluded.pending_email")
        .bind(id(user)?)
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO email_tokens VALUES(?,?,'verify',?,?,?) ON CONFLICT(user_id,kind) DO UPDATE SET token_hash=excluded.token_hash,email=excluded.email,created_at=excluded.created_at,expires_at=excluded.expires_at")
        .bind(state::hash(token.as_bytes()))
        .bind(id(user)?)
        .bind(&email)
        .bind(now as i64)
        .bind((now + VERIFY_TTL) as i64)
        .execute(&mut *tx)
        .await?;
    let name: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
        .bind(id(user)?)
        .fetch_one(&mut *tx)
        .await?;
    let status = email_status(&mut tx, &app, user).await?;
    tx.commit().await?;
    drop(gate);
    app.0.authsec.mail.send(
        &email,
        "Confirm your email for StickTalk",
        format!(
            "Confirm that this address belongs to the StickTalk account @{name}.\n\nOpen this link within 24 hours:\n\n{}\n\nOnce confirmed, you can use it to reset your password. If you didn't ask for this, ignore this email.\n",
            link(&app, "verify-email", &token)
        ),
    );
    let mut response = http::json(status);
    clear_reauth_cookie(&mut response);
    Ok(response)
}

async fn remove(State(app): State<App>, headers: HeaderMap) -> Result<http::ApiResponse> {
    let (key, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::Email).await?;
    sqlx::query("DELETE FROM user_emails WHERE user_id=?")
        .bind(id(user)?)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM email_tokens WHERE user_id=?")
        .bind(id(user)?)
        .execute(&mut *tx)
        .await?;
    let status = email_status(&mut tx, &app, user).await?;
    tx.commit().await?;
    let mut response = http::json(status);
    clear_reauth_cookie(&mut response);
    Ok(response)
}

fn token_hash(token: &str) -> Result<Vec<u8>> {
    require(token.len() == 43, ErrorCode::InvalidCredentials)?;
    Ok(state::hash(token.as_bytes()))
}

async fn verify(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    enabled(&app)?;
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(&app, &r), 10., 5.)?;
    let request: EmailTokenRequest = http::parse(r).await?;
    let hash = token_hash(&request.token)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let row = sqlx::query("DELETE FROM email_tokens WHERE token_hash=? AND kind='verify' AND expires_at>? RETURNING user_id,email")
        .bind(&hash)
        .bind(state::now() as i64)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    let user: i64 = row.get("user_id");
    let email: String = row.get("email");
    // Only the address still waiting is confirmed, never one replaced since.
    let done = sqlx::query("UPDATE user_emails SET email=pending_email,verified_at=?,pending_email=NULL WHERE user_id=? AND pending_email=?")
        .bind(state::now() as i64)
        .bind(user)
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    require(done.rows_affected() == 1, ErrorCode::InvalidCredentials)?;
    // A reset link sent to an older address stops working.
    sqlx::query("DELETE FROM email_tokens WHERE user_id=? AND kind='reset'")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(http::ApiResponse::empty())
}

async fn request_reset(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    enabled(&app)?;
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(&app, &r), 10., 5.)?;
    let request: EmailResetRequest = http::parse(r).await?;
    let login = request.login.trim().to_ascii_lowercase();
    require(
        (3..=254).contains(&login.len()) && !login.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    // Per target, whether or not it exists: the same answer either way.
    app.0
        .core
        .lock()
        .await
        .rate(format!("email-reset:{login}"), 3. / 60., 3.)?;
    // Answer first; look up and send afterwards, off the request's timing.
    let worker = app.clone();
    tokio::spawn(async move {
        if let Err(error) = send_reset(&worker, &login).await {
            tracing::warn!(?error, "password reset email not sent");
        }
    });
    use axum::response::IntoResponse;
    Ok(http::ApiResponse::from_response(
        axum::http::StatusCode::ACCEPTED.into_response(),
    ))
}

pub(super) async fn send_reset(app: &App, login: &str) -> Result<()> {
    let accounts: Vec<(i64, String, String)> = {
        let mut c = app.0.pool.acquire().await?;
        let rows = if login.contains('@') {
            let Ok(email) = address(login) else {
                return Ok(());
            };
            sqlx::query("SELECT u.id,u.username,e.email FROM user_emails e JOIN users u ON u.id=e.user_id WHERE e.email=? AND e.verified_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=u.id) ORDER BY u.id LIMIT ?")
                .bind(email)
                .bind(MAX_ACCOUNTS_PER_ADDRESS)
                .fetch_all(&mut *c)
                .await?
        } else {
            let Ok(name) = db::username(login) else {
                return Ok(());
            };
            sqlx::query("SELECT u.id,u.username,e.email FROM users u JOIN user_emails e ON e.user_id=u.id WHERE u.username=? AND e.verified_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=u.id)")
                .bind(name)
                .fetch_all(&mut *c)
                .await?
        };
        rows.into_iter()
            .map(|r| (r.get("id"), r.get("username"), r.get("email")))
            .collect()
    };
    for (user, name, email) in accounts {
        if app
            .0
            .core
            .lock()
            .await
            .rate(format!("email-reset-user:{user}"), 3. / 60., 3.)
            .is_err()
        {
            continue;
        }
        let token = state::token();
        let now = state::now();
        {
            let _gate = app.0.core.lock().await;
            // One outstanding link per account; a new request replaces it.
            sqlx::query("INSERT INTO email_tokens VALUES(?,?,'reset',?,?,?) ON CONFLICT(user_id,kind) DO UPDATE SET token_hash=excluded.token_hash,email=excluded.email,created_at=excluded.created_at,expires_at=excluded.expires_at")
                .bind(state::hash(token.as_bytes()))
                .bind(user)
                .bind(&email)
                .bind(now as i64)
                .bind((now + RESET_TTL) as i64)
                .execute(&app.0.pool)
                .await?;
        }
        app.0.authsec.mail.send(
            &email,
            "Reset your StickTalk password",
            format!(
                "Someone asked to reset the password for the StickTalk account @{name}.\n\nTo choose a new password, open this link within 30 minutes:\n\n{}\n\nThe link works once, and resetting signs you out everywhere. If you didn't ask for this, ignore this email: your password stays the same.\n",
                link(app, "reset", &token)
            ),
        );
    }
    Ok(())
}

async fn complete_reset(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    enabled(&app)?;
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(&app, &r), 10., 5.)?;
    let request: EmailResetComplete = http::parse(r).await?;
    let hash = token_hash(&request.token)?;
    require(
        http::password_len_ok(&request.new_password),
        ErrorCode::InvalidRequest,
    )?;
    // Check before spending Argon2 work on a guess.
    let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM email_tokens WHERE token_hash=? AND kind='reset' AND expires_at>?)")
        .bind(&hash)
        .bind(state::now() as i64)
        .fetch_one(&app.0.pool)
        .await?;
    require(live, ErrorCode::InvalidCredentials)?;
    let permit = app
        .0
        .passwords
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let password = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        http::password_hash(request.new_password)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Internal))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let row = sqlx::query("DELETE FROM email_tokens WHERE token_hash=? AND kind='reset' AND expires_at>? RETURNING user_id,email")
        .bind(&hash)
        .bind(state::now() as i64)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    let user: i64 = row.get("user_id");
    let email: String = row.get("email");
    // The link is only good while that address is still the account's verified one.
    let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_emails WHERE user_id=? AND email=? AND verified_at IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=?)")
        .bind(user)
        .bind(&email)
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    require(current, ErrorCode::InvalidCredentials)?;
    sqlx::query("UPDATE users SET password_hash=? WHERE id=?")
        .bind(password)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    let keys: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=?")
        .bind(user)
        .fetch_all(&mut *tx)
        .await?;
    for sql in [
        "DELETE FROM reauth_proofs WHERE session_hash IN (SELECT token_hash FROM sessions WHERE user_id=?)",
        "DELETE FROM sessions WHERE user_id=?",
        "DELETE FROM email_tokens WHERE user_id=? AND kind='reset'",
        "DELETE FROM password_resets WHERE user_id=?",
    ] {
        sqlx::query(sql).bind(user).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    crate::reauth::disconnect(&app, &mut core, &mut c, &keys).await?;
    Ok(http::ApiResponse::empty())
}
