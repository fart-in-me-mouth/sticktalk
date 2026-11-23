//! Several accounts signed in on one browser, switched without credentials.
//!
//! Each account keeps its own independent session token. The active one is the
//! normal `__Host-sticktalk_session` cookie; up to four others wait in one
//! `__Host-sticktalk_accounts` cookie. Both are HttpOnly, Secure, host-only and
//! SameSite=Strict, so page script (and therefore XSS) can never read a token.
//! Switching swaps cookies on the server; signing one account out revokes only
//! that account's session. The Windows app is WebView2 showing this same web
//! app, so the same cookies live in its WebView2 profile.
use super::*;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
};

pub(super) const STASH: &str = "__Host-sticktalk_accounts";
const SESSION: &str = "__Host-sticktalk_session";
const CSRF: &str = "__Host-sticktalk_csrf";
/// Accounts per browser, the active one included.
pub(super) const MAX_ACCOUNTS: usize = 5;

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/browser/accounts", get(list))
        .route("/api/v1/browser/accounts/add", post(add))
        .route("/api/v1/browser/accounts/switch", post(switch))
        .route("/api/v1/browser/accounts/logout", post(logout))
}

fn cookie_line(name: &str, value: &str, age: u64, http_only: bool) -> axum::http::HeaderValue {
    format!(
        "{name}={value}; Path=/; Max-Age={age}; Secure; SameSite=Strict{}",
        if http_only { "; HttpOnly" } else { "" }
    )
    .parse()
    .expect("generated cookie")
}

fn stashed(headers: &HeaderMap) -> Result<Vec<String>> {
    let mut tokens: Vec<String> = vec![];
    for t in crate::browser::cookie(headers, STASH)?
        .unwrap_or("")
        .split('.')
        .filter(|t| t.len() == 43)
    {
        if !tokens.iter().any(|v| v == t) && tokens.len() < MAX_ACCOUNTS - 1 {
            tokens.push(t.to_owned());
        }
    }
    Ok(tokens)
}

fn stash_cookie(tokens: &[String]) -> axum::http::HeaderValue {
    if tokens.is_empty() {
        cookie_line(STASH, "", 0, true)
    } else {
        cookie_line(STASH, &tokens.join("."), 30 * 86_400, true)
    }
}

struct Signed {
    token: String,
    key: Vec<u8>,
    user: Id,
}

/// The browser's sessions that are still valid, one per account. Duplicates of
/// an account (signed in twice) are revoked so they stop counting toward its
/// per-account session limit.
async fn sessions(app: &App, headers: &HeaderMap) -> Result<(Option<Signed>, Vec<Signed>, bool)> {
    let mut c = app.0.pool.acquire().await?;
    let active = match crate::browser::session_cookie(headers)? {
        Some(token) => db::auth(&mut c, token)
            .await
            .ok()
            .map(|(key, user)| Signed {
                token: token.to_owned(),
                key,
                user,
            }),
        None => None,
    };
    let tokens = stashed(headers)?;
    let mut others: Vec<Signed> = vec![];
    let mut changed = false;
    let mut redundant = vec![];
    for token in tokens {
        match db::auth(&mut c, &token).await {
            Ok((key, user)) => {
                if active.as_ref().is_some_and(|a| a.user == user)
                    || others.iter().any(|o| o.user == user)
                {
                    redundant.push(key);
                    changed = true;
                } else {
                    others.push(Signed { token, key, user });
                }
            }
            Err(_) => changed = true,
        }
    }
    drop(c);
    if !redundant.is_empty() {
        revoke(app, &redundant).await?;
    }
    Ok((active, others, changed))
}

/// Revoke sessions exactly like logout: close their sockets and calls first.
async fn revoke(app: &App, keys: &[Vec<u8>]) -> Result<()> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut users = vec![];
    for key in keys {
        if let Some(user) = sqlx::query_scalar::<_, i64>(
            "DELETE FROM sessions WHERE token_hash=? RETURNING user_id",
        )
        .bind(key)
        .fetch_optional(&mut *c)
        .await?
        {
            users.push(Id(user as u64));
        }
    }
    crate::reauth::disconnect(app, &mut core, &mut c, keys).await?;
    for user in users {
        crate::commands::broadcast_presence(&mut c, &mut core, user).await?;
    }
    Ok(())
}

async fn list(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    let (active, others, changed) = sessions(&app, &headers).await?;
    let mut c = app.0.pool.acquire().await?;
    let mut accounts = vec![];
    if let Some(a) = &active {
        accounts.push(SignedInAccount {
            user: db::me(&mut c, a.user).await?.user,
            active: true,
        });
    }
    for o in &others {
        accounts.push(SignedInAccount {
            user: db::me(&mut c, o.user).await?.user,
            active: false,
        });
    }
    let mut r = axum::Json(SignedInAccounts {
        accounts,
        max: MAX_ACCOUNTS as u32,
    })
    .into_response();
    if changed {
        let tokens: Vec<String> = others.into_iter().map(|o| o.token).collect();
        r.headers_mut().append("set-cookie", stash_cookie(&tokens));
    }
    Ok(r)
}

/// Keep the current account signed in and show the sign-in screen for another.
async fn add(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    same_origin(&app, &headers)?;
    let (active, others, _) = sessions(&app, &headers).await?;
    let active = active.ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    require(others.len() < MAX_ACCOUNTS - 1, ErrorCode::LimitExceeded)?;
    let mut tokens: Vec<String> = others.into_iter().map(|o| o.token).collect();
    tokens.push(active.token);
    let mut r = axum::http::StatusCode::NO_CONTENT.into_response();
    r.headers_mut().append("set-cookie", stash_cookie(&tokens));
    r.headers_mut()
        .append("set-cookie", cookie_line(SESSION, "", 0, true));
    r.headers_mut()
        .append("set-cookie", cookie_line(CSRF, "", 0, false));
    Ok(r)
}

async fn switch(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    let headers = r.headers().clone();
    same_origin(&app, &headers)?;
    app.0.core.lock().await.rate(
        format!("account-switch-{}", http::auth_bucket(&app, &r)),
        30.,
        10.,
    )?;
    let choice: AccountChoice = http::parse(r).await?;
    let (active, others, _) = sessions(&app, &headers).await?;
    let (mut chosen, mut rest): (Vec<Signed>, Vec<Signed>) =
        others.into_iter().partition(|o| o.user == choice.user_id);
    let target = chosen
        .pop()
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    if let Some(a) = active {
        rest.push(a);
    }
    let expires_at: i64 = sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=?")
        .bind(&target.key)
        .fetch_one(&app.0.pool)
        .await?;
    let user = db::me(&mut *app.0.pool.acquire().await?, target.user).await?;
    let mut response = crate::browser::browser_session(AuthResponse {
        token: target.token,
        expires_at: expires_at as u64,
        user,
    });
    let tokens: Vec<String> = rest.into_iter().map(|o| o.token).collect();
    response
        .headers_mut()
        .append("set-cookie", stash_cookie(&tokens));
    // Another account's reauth proof must not follow the switch.
    response.headers_mut().append(
        "set-cookie",
        cookie_line("__Host-sticktalk_reauth", "", 0, true),
    );
    Ok(response)
}

/// Sign one account out of this browser. Every other account stays signed in.
async fn logout(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    let headers = r.headers().clone();
    same_origin(&app, &headers)?;
    let choice: AccountChoice = http::parse(r).await?;
    let (active, others, _) = sessions(&app, &headers).await?;
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    if let Some(a) = active.filter(|a| a.user == choice.user_id) {
        revoke(&app, &[a.key]).await?;
        response
            .headers_mut()
            .append("set-cookie", cookie_line(SESSION, "", 0, true));
        response
            .headers_mut()
            .append("set-cookie", cookie_line(CSRF, "", 0, false));
        return Ok(response);
    }
    let (gone, kept): (Vec<Signed>, Vec<Signed>) =
        others.into_iter().partition(|o| o.user == choice.user_id);
    require(!gone.is_empty(), ErrorCode::NotFound)?;
    revoke(&app, &gone.into_iter().map(|g| g.key).collect::<Vec<_>>()).await?;
    let tokens: Vec<String> = kept.into_iter().map(|o| o.token).collect();
    response
        .headers_mut()
        .append("set-cookie", stash_cookie(&tokens));
    Ok(response)
}
