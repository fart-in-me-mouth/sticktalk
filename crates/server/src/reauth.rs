//! Fresh, operation/session-bound proof of an EXISTING login method.
use crate::{
    db::{self, auth, id},
    error::{require, Error, Result},
    http,
    state::{self, App},
};
use axum::{
    body::Body,
    http::{HeaderMap, Request},
};
use sqlx::SqliteConnection;
use sticktalk_proto::*;
pub const TTL: u64 = 300_000;
pub fn operation(op: AccountOperation) -> String {
    db::enum_str(op)
}
pub async fn issue_challenge(
    app: &App,
    session: Option<&[u8]>,
    op: AccountOperation,
) -> Result<GoogleChallengeResponse> {
    let _gate = app.0.core.lock().await;
    sqlx::query("DELETE FROM google_challenges WHERE expires_at<=?")
        .bind(state::now() as i64)
        .execute(&app.0.pool)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM google_challenges")
        .fetch_one(&app.0.pool)
        .await?;
    require(count < 4096, ErrorCode::LimitExceeded)?;
    let nonce = state::token();
    let expires_at = state::now() + TTL;
    sqlx::query("INSERT INTO google_challenges VALUES(?,?,?,?)")
        .bind(state::hash(nonce.as_bytes()))
        .bind(session)
        .bind(operation(op))
        .bind(expires_at as i64)
        .execute(&app.0.pool)
        .await?;
    Ok(GoogleChallengeResponse { nonce, expires_at })
}
pub async fn verify_challenge(
    app: &App,
    token: &str,
    client_id: &str,
    session: Option<&[u8]>,
    op: AccountOperation,
) -> Result<String> {
    let (subject, nonce) = app.0.google.verify_bound(token, client_id, None).await?;
    let nonce = nonce.ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    let consumed = sqlx::query("DELETE FROM google_challenges WHERE nonce_hash=? AND session_hash IS ? AND operation=? AND expires_at>?")
        .bind(state::hash(nonce.as_bytes())).bind(session).bind(operation(op)).bind(state::now() as i64).execute(&app.0.pool).await?;
    require(consumed.rows_affected() == 1, ErrorCode::InvalidCredentials)?;
    Ok(subject)
}
pub async fn issue(
    c: &mut SqliteConnection,
    key: &[u8],
    op: AccountOperation,
) -> Result<ReauthResponse> {
    let proof = state::token();
    let expires_at = state::now() + TTL;
    // At most one proof per session/operation; the session cap bounds storage.
    sqlx::query("DELETE FROM reauth_proofs WHERE session_hash=? AND operation=?")
        .bind(key)
        .bind(operation(op))
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO reauth_proofs VALUES(?,?,?,?)")
        .bind(state::hash(proof.as_bytes()))
        .bind(key)
        .bind(operation(op))
        .bind(expires_at as i64)
        .execute(c)
        .await?;
    Ok(ReauthResponse { proof, expires_at })
}
pub async fn consume(
    c: &mut SqliteConnection,
    headers: &HeaderMap,
    key: &[u8],
    op: AccountOperation,
) -> Result<()> {
    let proof = headers
        .get("x-sticktalk-reauth")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    require(proof.len() == 43, ErrorCode::InvalidCredentials)?;
    consume_hash(c, &state::hash(proof.as_bytes()), key, op).await
}
pub async fn consume_hash(
    c: &mut SqliteConnection,
    proof_hash: &[u8],
    key: &[u8],
    op: AccountOperation,
) -> Result<()> {
    let r=sqlx::query("DELETE FROM reauth_proofs WHERE proof_hash=? AND session_hash=? AND operation=? AND expires_at>?")
        .bind(proof_hash).bind(key).bind(operation(op)).bind(state::now() as i64).execute(c).await?;
    require(r.rows_affected() == 1, ErrorCode::InvalidCredentials)
}
pub async fn api(app: &App, path: &str, r: Request<Body>) -> Result<http::ApiResponse> {
    require(r.method() == "POST", ErrorCode::NotFound)?;
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(app, &r), 10., 5.)?;
    let browser = crate::browser::session_cookie(r.headers())?.is_some();
    let token = http::bearer(r.headers()).ok().map(str::to_owned);
    if path == "auth/google/challenge" {
        require(
            app.0.config.google_client_id.is_some()
                || app.0.config.browser.google_web_client_id.is_some(),
            ErrorCode::FeatureDisabled,
        )?;
        let req: GoogleChallengeRequest = http::parse(r).await?;
        let session = if req.operation == AccountOperation::SignIn {
            None
        } else {
            Some(
                auth(
                    &mut *app.0.pool.acquire().await?,
                    token.as_deref().unwrap_or(""),
                )
                .await?
                .0,
            )
        };
        return Ok(http::json(
            issue_challenge(app, session.as_deref(), req.operation).await?,
        ));
    }
    let token = token.ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let (key, user) = auth(&mut *app.0.pool.acquire().await?, &token).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("reauth-user:{}", user.0), 10., 5.)?;
    let req: ReauthRequest = http::parse(r).await?;
    require(
        req.operation != AccountOperation::SignIn,
        ErrorCode::InvalidRequest,
    )?;
    let expected: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=?")
        .bind(id(user)?)
        .fetch_one(&app.0.pool)
        .await?;
    let mut google_subject = None;
    if let Some(password) = req.current_password {
        require(
            password.len() <= 128 && !expected.is_empty(),
            ErrorCode::InvalidCredentials,
        )?;
        let hash = expected.clone();
        let permit = app
            .0
            .passwords
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let valid = tokio::task::spawn_blocking(move || {
            let _p = permit;
            http::verify_password(&password, &hash)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))?;
        require(valid, ErrorCode::InvalidCredentials)?;
    } else {
        let client = app
            .0
            .config
            .google_client_id
            .as_deref()
            .or(app.0.config.browser.google_web_client_id.as_deref())
            .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?;
        let subject = verify_challenge(
            app,
            req.id_token.as_deref().unwrap_or(""),
            client,
            Some(&key),
            req.operation,
        )
        .await?;
        existing_google(&mut *app.0.pool.acquire().await?, user, &subject).await?;
        google_subject = Some(subject);
    }
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (current, current_user) = auth(&mut c, &token).await?;
    let unchanged: bool = sqlx::query_scalar("SELECT password_hash=? FROM users WHERE id=?")
        .bind(expected)
        .bind(id(user)?)
        .fetch_one(&mut *c)
        .await?;
    require(
        current == key && current_user == user && unchanged,
        ErrorCode::InvalidCredentials,
    )?;
    if let Some(subject) = google_subject {
        existing_google(&mut c, user, &subject).await?;
    }
    let proof = issue(&mut c, &key, req.operation).await?;
    let mut response = http::json(proof.clone());
    if browser {
        response.headers_mut().append("set-cookie",format!("__Host-sticktalk_reauth={}; Path=/; Max-Age=300; Secure; HttpOnly; SameSite=Strict",proof.proof).parse().map_err(|_|Error::new(ErrorCode::Internal))?);
    }
    Ok(response)
}
pub async fn existing_google(c: &mut SqliteConnection, user: Id, subject: &str) -> Result<()> {
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_identities WHERE provider='google' AND user_id=? AND subject=?)").bind(id(user)?).bind(subject).fetch_one(c).await?;
    require(valid, ErrorCode::InvalidCredentials)
}
/// Shared immediate live revocation path, called only after the DB commit.
pub async fn disconnect(
    app: &App,
    core: &mut state::Core,
    c: &mut SqliteConnection,
    keys: &[Vec<u8>],
) -> Result<()> {
    for key in keys {
        crate::voice::disconnect(app, core, c, key, None).await?;
        if let Some(stream) = core.streams.remove(key) {
            if let Some(connection) = stream.connection {
                let _ = connection.close.send(Some(4001));
            }
        }
    }
    Ok(())
}
