//! Passkeys as an optional passwordless way to sign in. Never a second factor.
//!
//! Verification is webauthn-rs (origin, RP ID hash, challenge, user
//! verification, signature and signature counter). Ceremonies live in memory,
//! one use each, for five minutes. This file, its routes line in `mod.rs` and
//! the `passkeys`/`passkey_users` tables are the whole feature.
use super::*;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
};
use sqlx::{Connection, Row};
use std::{collections::HashMap, sync::Mutex};
use webauthn_rs::prelude::*;
use webauthn_rs_proto::ResidentKeyRequirement;

const TTL: u64 = 300_000;
const MAX_PENDING: usize = 4096;
pub(super) const MAX_PER_USER: i64 = 10;

enum Ceremony {
    Register {
        user: Id,
        session: Vec<u8>,
        name: String,
        state: Box<PasskeyRegistration>,
    },
    SignIn(Box<DiscoverableAuthentication>),
}
struct Pending {
    ceremony: Ceremony,
    expires_at: u64,
}

pub(super) struct Passkeys {
    webauthn: Option<Webauthn>,
    pending: Mutex<HashMap<String, Pending>>,
}
impl Passkeys {
    pub(super) fn new(config: &crate::Config) -> Result<Self> {
        let origin = Url::parse(config.public_url.trim_end_matches('/')).map_err(bad_json)?;
        let host = origin.host_str().unwrap_or_default().to_owned();
        let rp_id = config
            .browser
            .authsec
            .webauthn_rp_id
            .clone()
            .unwrap_or(host.clone());
        // Browsers only offer WebAuthn on HTTPS, or plain HTTP on "localhost".
        // An IP address can never be an RP ID.
        let usable = (origin.scheme() == "https" || host == "localhost")
            && rp_id.parse::<std::net::IpAddr>().is_err()
            && !rp_id.is_empty()
            && (host == rp_id || host.ends_with(&format!(".{rp_id}")));
        let webauthn = if usable {
            Some(
                WebauthnBuilder::new(&rp_id, &origin)
                    .and_then(|b| b.rp_name("StickTalk").build())
                    .map_err(|_| Error::new(ErrorCode::InvalidRequest))?,
            )
        } else {
            if config.browser.authsec.webauthn_rp_id.is_some() {
                // An explicit but unusable RP ID is a configuration mistake.
                return Err(Error::new(ErrorCode::InvalidRequest));
            }
            None
        };
        Ok(Self {
            webauthn,
            pending: Mutex::new(HashMap::new()),
        })
    }
    pub(super) fn enabled(&self) -> bool {
        self.webauthn.is_some()
    }
    fn webauthn(&self) -> Result<&Webauthn> {
        self.webauthn
            .as_ref()
            .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))
    }
    fn put(&self, ceremony: Ceremony) -> Result<(String, u64)> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let now = state::now();
        pending.retain(|_, p| p.expires_at > now);
        require(pending.len() < MAX_PENDING, ErrorCode::Unavailable)?;
        let id = state::token();
        let expires_at = now + TTL;
        pending.insert(
            id.clone(),
            Pending {
                ceremony,
                expires_at,
            },
        );
        Ok((id, expires_at))
    }
    /// One use: a challenge is gone after its first answer, right or wrong.
    fn take(&self, id: &str) -> Result<Ceremony> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let p = pending
            .remove(id)
            .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
        require(p.expires_at > state::now(), ErrorCode::InvalidCredentials)?;
        Ok(p.ceremony)
    }
    #[cfg(test)]
    pub(super) fn expire_all(&self) {
        for p in self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values_mut()
        {
            p.expires_at = 0;
        }
    }
}

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/users/@me/passkeys", get(list))
        .route(
            "/api/v1/users/@me/passkeys/register/start",
            post(register_start),
        )
        .route(
            "/api/v1/users/@me/passkeys/register/finish",
            post(register_finish),
        )
        .route(
            "/api/v1/users/@me/passkeys/{id}",
            delete(remove).patch(rename),
        )
        .route("/api/v1/auth/passkey/start", post(sign_in_start))
        .route("/api/v1/auth/passkey/finish", post(sign_in_finish))
        .route("/api/v1/browser/session/passkey", post(browser_sign_in))
}

fn label(name: &str) -> Result<String> {
    let name = name.trim();
    require(
        !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(name.to_owned())
}

pub(super) async fn summaries(c: &mut SqliteConnection, user: Id) -> Result<Vec<PasskeySummary>> {
    Ok(sqlx::query("SELECT id,name,created_at,last_used_at,backed_up FROM passkeys WHERE user_id=? ORDER BY created_at,id")
        .bind(id(user)?)
        .fetch_all(c)
        .await?
        .into_iter()
        .map(|r| PasskeySummary {
            id: r.get::<i64, _>("id").to_string(),
            name: r.get("name"),
            created_at: r.get::<i64, _>("created_at") as u64,
            last_used_at: r.get::<Option<i64>, _>("last_used_at").map(|v| v as u64),
            backed_up: r.get("backed_up"),
        })
        .collect())
}

async fn list(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<axum::Json<Vec<PasskeySummary>>> {
    let (_, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    Ok(axum::Json(
        summaries(&mut *app.0.pool.acquire().await?, user).await?,
    ))
}

async fn register_start(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let webauthn = app.0.authsec.passkeys.webauthn()?;
    let headers = r.headers().clone();
    let (key, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-user:{}", user.0), 10., 5.)?;
    let request: PasskeyRegisterStart = http::parse(r).await?;
    let name = label(&request.name)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let (current, _) = db::auth(&mut tx, http::bearer(&headers)?).await?;
    require(current == key, ErrorCode::Unauthenticated)?;
    // Adding a way to sign in is as sensitive as changing the password.
    crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::Passkeys).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM passkeys WHERE user_id=?")
        .bind(id(user)?)
        .fetch_one(&mut *tx)
        .await?;
    require(count < MAX_PER_USER, ErrorCode::LimitExceeded)?;
    let handle: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT handle FROM passkey_users WHERE user_id=?")
            .bind(id(user)?)
            .fetch_optional(&mut *tx)
            .await?;
    let handle = match handle {
        Some(h) => h,
        None => {
            let h = state::random(16);
            sqlx::query("INSERT INTO passkey_users VALUES(?,?)")
                .bind(id(user)?)
                .bind(&h)
                .execute(&mut *tx)
                .await?;
            h
        }
    };
    let me = db::me(&mut tx, user).await?;
    let exclude: Vec<CredentialID> =
        sqlx::query_scalar::<_, Vec<u8>>("SELECT credential_id FROM passkeys WHERE user_id=?")
            .bind(id(user)?)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let uuid = Uuid::from_slice(&handle).map_err(|_| Error::new(ErrorCode::Internal))?;
    let display = me
        .user
        .display_name
        .clone()
        .unwrap_or_else(|| me.user.username.clone());
    let (mut options, state) = webauthn
        .start_passkey_registration(uuid, &me.user.username, &display, Some(exclude))
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    // Discoverable, so "Sign in with a passkey" needs no username first.
    let selection = options
        .public_key
        .authenticator_selection
        .get_or_insert_with(Default::default);
    selection.resident_key = Some(ResidentKeyRequirement::Required);
    selection.require_resident_key = true;
    let (challenge_id, expires_at) = app.0.authsec.passkeys.put(Ceremony::Register {
        user,
        session: key,
        name,
        state: Box::new(state),
    })?;
    let mut response = http::json(PasskeyChallenge {
        challenge_id,
        public_key: serde_json::to_string(&options).map_err(|_| Error::new(ErrorCode::Internal))?,
        expires_at,
    });
    clear_reauth_cookie(&mut response);
    Ok(response)
}

async fn register_finish(
    State(app): State<App>,
    r: Request<Body>,
) -> Result<axum::Json<PasskeySummary>> {
    let webauthn = app.0.authsec.passkeys.webauthn()?;
    let headers = r.headers().clone();
    let (key, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-user:{}", user.0), 10., 5.)?;
    let request: PasskeyFinish = http::parse(r).await?;
    let Ceremony::Register {
        user: owner,
        session,
        name,
        state,
    } = app.0.authsec.passkeys.take(&request.challenge_id)?
    else {
        return Err(Error::new(ErrorCode::InvalidCredentials));
    };
    // The ceremony belongs to the session that passed re-authentication.
    require(
        owner == user && session == key,
        ErrorCode::InvalidCredentials,
    )?;
    require(
        request.credential.len() <= 16384,
        ErrorCode::PayloadTooLarge,
    )?;
    let credential: RegisterPublicKeyCredential =
        serde_json::from_str(&request.credential).map_err(bad_json)?;
    let passkey = webauthn
        .finish_passkey_registration(&credential, &state)
        .map_err(|error| {
            tracing::info!(?error, "passkey registration refused");
            Error::new(ErrorCode::InvalidCredentials)
        })?;
    let stored = serde_json::to_string(&passkey).map_err(|_| Error::new(ErrorCode::Internal))?;
    let credential_id: &[u8] = passkey.cred_id().as_ref();
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let (current, _) = db::auth(&mut tx, http::bearer(&headers)?).await?;
    require(current == key, ErrorCode::Unauthenticated)?;
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM passkeys WHERE credential_id=?)")
            .bind(credential_id)
            .fetch_one(&mut *tx)
            .await?;
    require(!taken, ErrorCode::AlreadyExists)?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM passkeys WHERE user_id=?")
        .bind(id(user)?)
        .fetch_one(&mut *tx)
        .await?;
    require(count < MAX_PER_USER, ErrorCode::LimitExceeded)?;
    let row = (rand::random::<u64>() >> 1).max(1) as i64;
    let now = state::now();
    sqlx::query("INSERT INTO passkeys(id,user_id,credential_id,name,passkey,backed_up,created_at) VALUES(?,?,?,?,?,0,?)")
        .bind(row)
        .bind(id(user)?)
        .bind(credential_id)
        .bind(&name)
        .bind(stored)
        .bind(now as i64)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(axum::Json(PasskeySummary {
        id: row.to_string(),
        name,
        created_at: now,
        last_used_at: None,
        backed_up: false,
    }))
}

fn row_id(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

async fn rename(
    State(app): State<App>,
    Path(passkey): Path<String>,
    r: Request<Body>,
) -> Result<axum::Json<Vec<PasskeySummary>>> {
    let (_, user) = authed(&app, r.headers()).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    let passkey = row_id(&passkey)?;
    let request: PasskeyRename = http::parse(r).await?;
    let name = label(&request.name)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let done = sqlx::query("UPDATE passkeys SET name=? WHERE id=? AND user_id=?")
        .bind(name)
        .bind(passkey)
        .bind(id(user)?)
        .execute(&mut *c)
        .await?;
    // Someone else's passkey looks exactly like a missing one.
    require(done.rows_affected() == 1, ErrorCode::NotFound)?;
    Ok(axum::Json(summaries(&mut c, user).await?))
}

async fn remove(
    State(app): State<App>,
    Path(passkey): Path<String>,
    headers: HeaderMap,
) -> Result<http::ApiResponse> {
    let (key, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-user:{}", user.0), 10., 5.)?;
    let passkey = row_id(&passkey)?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM passkeys WHERE id=? AND user_id=?)")
            .bind(passkey)
            .bind(id(user)?)
            .fetch_one(&mut *tx)
            .await?;
    require(exists, ErrorCode::NotFound)?;
    crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::Passkeys).await?;
    sqlx::query("DELETE FROM passkeys WHERE id=? AND user_id=?")
        .bind(passkey)
        .bind(id(user)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut response = http::json(summaries(&mut c, user).await?);
    clear_reauth_cookie(&mut response);
    Ok(response)
}

async fn sign_in_start(
    State(app): State<App>,
    r: Request<Body>,
) -> Result<axum::Json<PasskeyChallenge>> {
    let webauthn = app.0.authsec.passkeys.webauthn()?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-ip:{}", client_ip(&app, &r)), 20., 10.)?;
    let (mut options, state) = webauthn
        .start_discoverable_authentication()
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    // A button press, not the autofill ("conditional") prompt.
    options.mediation = None;
    let (challenge_id, expires_at) = app
        .0
        .authsec
        .passkeys
        .put(Ceremony::SignIn(Box::new(state)))?;
    Ok(axum::Json(PasskeyChallenge {
        challenge_id,
        public_key: serde_json::to_string(&options).map_err(|_| Error::new(ErrorCode::Internal))?,
        expires_at,
    }))
}

/// Verify a discoverable assertion and mint a brand-new session for its owner.
pub(super) async fn sign_in(app: &App, r: Request<Body>) -> Result<AuthResponse> {
    let webauthn = app.0.authsec.passkeys.webauthn()?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-ip:{}", client_ip(app, &r)), 20., 10.)?;
    let request: PasskeyFinish = http::parse(r).await?;
    let Ceremony::SignIn(state) = app.0.authsec.passkeys.take(&request.challenge_id)? else {
        return Err(Error::new(ErrorCode::InvalidCredentials));
    };
    require(
        request.credential.len() <= 16384,
        ErrorCode::PayloadTooLarge,
    )?;
    let credential: PublicKeyCredential = serde_json::from_str(&request.credential)
        .map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
    let (handle, credential_id) = webauthn
        .identify_discoverable_authentication(&credential)
        .map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
    let row = sqlx::query("SELECT p.id,p.user_id,p.passkey FROM passkeys p JOIN passkey_users u ON u.user_id=p.user_id WHERE p.credential_id=? AND u.handle=?")
        .bind(credential_id)
        .bind(handle.as_bytes().as_slice())
        .fetch_optional(&app.0.pool)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    let user = Id(row.get::<i64, _>("user_id") as u64);
    let stored: String = row.get("passkey");
    app.0
        .core
        .lock()
        .await
        .rate(format!("passkey-user:{}", user.0), 10., 5.)?;
    let mut passkey: Passkey =
        serde_json::from_str(&stored).map_err(|_| Error::new(ErrorCode::Internal))?;
    let result = webauthn
        .finish_discoverable_authentication_v2(&credential, *state, &passkey)
        .map_err(|error| {
            tracing::info!(?error, "passkey sign-in refused");
            Error::new(ErrorCode::InvalidCredentials)
        })?;
    // Passkeys require user verification; webauthn-rs enforced it. Check anyway.
    require(result.user_verified(), ErrorCode::InvalidCredentials)?;
    passkey.update_credential(&result);
    let updated = serde_json::to_string(&passkey).map_err(|_| Error::new(ErrorCode::Internal))?;
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    // Another sign-in with this passkey may have advanced its counter meanwhile:
    // only the answer verified against the latest stored state counts.
    let changed = sqlx::query(
        "UPDATE passkeys SET passkey=?,last_used_at=?,backed_up=? WHERE id=? AND passkey=?",
    )
    .bind(updated)
    .bind(state::now() as i64)
    .bind(result.backup_state())
    .bind(row.get::<i64, _>("id"))
    .bind(stored)
    .execute(&mut *tx)
    .await?;
    require(changed.rows_affected() == 1, ErrorCode::InvalidCredentials)?;
    let auth = session(&mut tx, user).await?;
    tx.commit().await?;
    Ok(auth)
}

async fn sign_in_finish(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let mut response = http::json(sign_in(&app, r).await?);
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("static"));
    Ok(response)
}

async fn browser_sign_in(
    State(app): State<App>,
    r: Request<Body>,
) -> Result<axum::response::Response> {
    browser_only(&app, r.headers())?;
    Ok(crate::browser::browser_session(sign_in(&app, r).await?))
}
