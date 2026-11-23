//! Account security and personal data: passkeys (passwordless sign-in only),
//! QR code sign-in, optional email password reset, the browser account switcher
//! and the personal data export. Every route lives in this module tree; the
//! shared files only merge `routes()`, call `spawn()` and `erase()`, and hold one
//! `AuthSec` value. See docs/features/authsec.md.
//!
//! Nothing here polls. QR codes rotate on a timer armed for their own expiry;
//! the export worker sleeps until a job is queued or the next archive expires.
mod accounts;
mod email;
mod export;
mod passkey;
mod qr;
#[cfg(test)]
mod tests;

use crate::{
    db::{self, id},
    error::{require, Error, Result},
    http,
    state::{self, App},
};
use axum::{
    body::Body,
    http::{HeaderMap, Request},
};
use clap::Args;
use sqlx::SqliteConnection;
use sticktalk_proto::*;

#[derive(Args, Clone, Default)]
pub struct AuthSecOptions {
    /// WebAuthn relying party ID for passkeys. Defaults to the public URL's host;
    /// set it to your own domain on a self-hosted server.
    #[arg(long, env = "STICKTALK_WEBAUTHN_RP_ID")]
    pub webauthn_rp_id: Option<String>,
    /// SMTP server for optional email password reset. Unset: administrator reset only.
    #[arg(long, env = "STICKTALK_SMTP_HOST")]
    pub smtp_host: Option<String>,
    #[arg(long, env = "STICKTALK_SMTP_PORT")]
    pub smtp_port: Option<u16>,
    /// `starttls` (default), `tls`, or `none` (loopback hosts only, for local relays/tests).
    #[arg(long, env = "STICKTALK_SMTP_SECURITY", default_value = "starttls")]
    pub smtp_security: String,
    #[arg(long, env = "STICKTALK_SMTP_USERNAME")]
    pub smtp_username: Option<String>,
    #[arg(long, env = "STICKTALK_SMTP_PASSWORD", hide_env_values = true)]
    pub smtp_password: Option<String>,
    /// Sender address, for example `StickTalk <no-reply@example.com>`.
    #[arg(long, env = "STICKTALK_SMTP_FROM")]
    pub smtp_from: Option<String>,
}
impl std::fmt::Debug for AuthSecOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSecOptions")
            .field("webauthn_rp_id", &self.webauthn_rp_id)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_security", &self.smtp_security)
            .field("smtp_username", &self.smtp_username)
            .field(
                "smtp_password",
                &self.smtp_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("smtp_from", &self.smtp_from)
            .finish()
    }
}

/// Process-wide state for this module, one per `App`.
pub(crate) struct AuthSec {
    passkeys: passkey::Passkeys,
    qr: qr::Qr,
    mail: email::Mailer,
    export: export::Exports,
}
impl AuthSec {
    pub(crate) fn new(config: &crate::Config) -> Result<Self> {
        Ok(Self {
            passkeys: passkey::Passkeys::new(config)?,
            qr: Default::default(),
            mail: email::Mailer::new(config)?,
            export: Default::default(),
        })
    }
}

pub(crate) fn routes() -> axum::Router<App> {
    use axum::routing::get;
    axum::Router::new()
        .route("/api/v1/auth/options", get(options))
        .merge(passkey::routes())
        .merge(qr::routes())
        .merge(email::routes())
        .merge(accounts::routes())
        .merge(export::routes())
}

pub(crate) use export::wake;

/// Background work: the export worker and its expiry timer.
pub(crate) fn spawn(app: &App) {
    export::spawn(app);
}

/// Account erasure: remove everything this module stores for `user`. The export
/// worker deletes any archive whose row is gone the next time it wakes.
pub(crate) async fn erase(c: &mut SqliteConnection, user: Id) -> Result<()> {
    let u = id(user)?;
    for sql in [
        "DELETE FROM passkeys WHERE user_id=?",
        "DELETE FROM passkey_users WHERE user_id=?",
        "DELETE FROM email_tokens WHERE user_id=?",
        "DELETE FROM user_emails WHERE user_id=?",
        "DELETE FROM data_exports WHERE user_id=?",
    ] {
        sqlx::query(sql).bind(u).execute(&mut *c).await?;
    }
    Ok(())
}

async fn options(axum::extract::State(app): axum::extract::State<App>) -> axum::Json<AuthOptions> {
    axum::Json(AuthOptions {
        passkeys: app.0.authsec.passkeys.enabled(),
        qr_login: true,
        email_reset: app.0.authsec.mail.enabled(),
    })
}

// ----- Shared helpers --------------------------------------------------------

/// The caller's address, as the auth rate limiter sees it (trusted proxy aware).
pub(crate) fn client_ip(app: &App, r: &Request<Body>) -> String {
    http::auth_bucket(app, r)
        .trim_start_matches("auth-ip:")
        .to_owned()
}

/// Bearer (or cookie-injected bearer) authentication for a REST call.
async fn authed(app: &App, headers: &HeaderMap) -> Result<(Vec<u8>, Id)> {
    let token = http::bearer(headers)?;
    db::auth(&mut *app.0.pool.acquire().await?, token).await
}

/// A browser-only route: the exact public origin, and no native bearer mixed in.
fn same_origin(app: &App, headers: &HeaderMap) -> Result<()> {
    require(
        headers.get_all("origin").iter().count() == 1
            && headers.get("origin").and_then(|v| v.to_str().ok())
                == Some(app.0.config.public_url.trim_end_matches('/')),
        ErrorCode::Forbidden,
    )
}

/// Like the password login route: same origin, and a bearer only if the browser's
/// own session cookie put it there (the boundary injects it).
fn browser_only(app: &App, headers: &HeaderMap) -> Result<()> {
    same_origin(app, headers)?;
    require(
        !headers.contains_key("authorization")
            || crate::browser::session_cookie(headers)?.is_some(),
        ErrorCode::InvalidRequest,
    )
}

/// Expire a consumed reauth cookie, as the browser boundary does for its routes.
fn clear_reauth_cookie(response: &mut http::ApiResponse) {
    response.headers_mut().append(
        "set-cookie",
        "__Host-sticktalk_reauth=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict"
            .parse()
            .expect("static cookie"),
    );
}

/// A coarse, human description of a User-Agent: "Chrome on Windows".
pub(crate) fn device_label(user_agent: &str) -> String {
    let ua = user_agent;
    let os = if ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iPod") {
        "iOS"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    // Order matters: Edge and Opera also say Chrome; Chrome also says Safari.
    let browser = if ua.contains("StickTalk") {
        "StickTalk app"
    } else if ua.contains("Edg/") || ua.contains("EdgA/") || ua.contains("EdgiOS/") {
        "Edge"
    } else if ua.contains("OPR/") {
        "Opera"
    } else if ua.contains("Firefox/") || ua.contains("FxiOS/") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        ""
    };
    match (browser, os) {
        ("", "") => "Unknown device".into(),
        ("", os) => format!("A browser on {os}"),
        (b, "") => b.into(),
        (b, os) => format!("{b} on {os}"),
    }
}

/// Mint a normal session for `user` (same lifetime and per-user cap as password login).
async fn session(c: &mut SqliteConnection, user: Id) -> Result<AuthResponse> {
    crate::google::session(c, user).await
}

fn bad_json<T>(_: T) -> Error {
    Error::new(ErrorCode::InvalidRequest)
}
