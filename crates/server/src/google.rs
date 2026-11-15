//! Google identity verification and account lifecycle. No OAuth secrets, email
//! matching, user-controlled key URLs, or network I/O under the mutation gate.
use crate::{
    db::{auth, id, me, username},
    error::{require, Error, Result},
    http::{self, ApiResponse},
    state::{self, App},
};
use axum::{body::Body, http::Request};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
use serde::Deserialize;
use sqlx::{Connection, Row, SqliteConnection};
use std::time::{Duration, Instant};
use sticktalk_proto::*;
use tokio::sync::Mutex;

const JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: String,
    #[serde(default)]
    crit: Vec<String>,
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    exp: u64,
    iat: u64,
    email_verified: bool,
    #[serde(default)]
    nonce: Option<String>,
}
#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    n: String,
    e: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    key_ops: Option<Vec<String>>,
}
struct Key {
    kid: String,
    n: Vec<u8>,
    e: Vec<u8>,
}
#[derive(Default)]
struct Cache {
    keys: Vec<Key>,
    expires: Option<Instant>,
    attempted: Option<Instant>,
    failed: bool,
}
pub(crate) struct Verifier {
    client: reqwest::Client,
    url: String,
    issuer: Option<String>,
    cache: Mutex<Cache>,
}
impl Verifier {
    pub(crate) fn configured(config: &crate::Config) -> Result<Self> {
        match (&config.mock_google_jwks, &config.mock_google_issuer) {
            (None, None) => Self::new(),
            (Some(url), Some(issuer)) => {
                require(config.bind.ip().is_loopback(), ErrorCode::InvalidRequest)?;
                crate::state::validate_mock_url(url)?;
                crate::state::validate_mock_url(issuer)?;
                let mut verifier = Self::with_url(url.clone())?;
                verifier.issuer = Some(issuer.clone());
                Ok(verifier)
            }
            _ => Err(Error::new(ErrorCode::InvalidRequest)),
        }
    }
    pub(crate) fn new() -> Result<Self> {
        Self::with_url(JWKS_URL.into())
    }
    fn with_url(url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!cfg!(test) && !url.starts_with("http://127.0.0.1:"))
            .no_proxy()
            .build()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        Ok(Self {
            client,
            url,
            issuer: None,
            cache: Mutex::new(Cache::default()),
        })
    }
    async fn fetch(&self) -> Result<(Vec<Key>, Duration)> {
        let mut response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        require(response.status().is_success(), ErrorCode::Unavailable)?;
        let ttl = cache_ttl(response.headers());
        // Bound even chunked responses. The production URL is fixed, TLS-only,
        // and redirects are disabled; a token can never supply a JWKS location.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::new(ErrorCode::Unavailable))?
        {
            require(bytes.len() + chunk.len() <= 65536, ErrorCode::Unavailable)?;
            bytes.extend_from_slice(&chunk);
        }
        let jwks: Jwks =
            serde_json::from_slice(&bytes).map_err(|_| Error::new(ErrorCode::Unavailable))?;
        require(
            !jwks.keys.is_empty() && jwks.keys.len() <= 32,
            ErrorCode::Unavailable,
        )?;
        let mut keys = Vec::new();
        for k in jwks.keys {
            if k.kty != "RSA"
                || k.alg.as_deref().is_some_and(|a| a != "RS256")
                || k.usage.as_deref().is_some_and(|u| u != "sig")
                || k.key_ops
                    .as_ref()
                    .is_some_and(|ops| !ops.iter().any(|op| op == "verify"))
            {
                continue;
            }
            let n = URL_SAFE_NO_PAD
                .decode(k.n)
                .map_err(|_| Error::new(ErrorCode::Unavailable))?;
            let e = URL_SAFE_NO_PAD
                .decode(k.e)
                .map_err(|_| Error::new(ErrorCode::Unavailable))?;
            require(
                (256..=1024).contains(&n.len())
                    && (1..=5).contains(&e.len())
                    && !k.kid.is_empty()
                    && k.kid.len() <= 256
                    && !keys.iter().any(|old: &Key| old.kid == k.kid),
                ErrorCode::Unavailable,
            )?;
            keys.push(Key { kid: k.kid, n, e });
        }
        require(!keys.is_empty(), ErrorCode::Unavailable)?;
        Ok((keys, ttl))
    }
    #[cfg(test)]
    pub(crate) async fn verify(&self, token: &str, client_id: &str) -> Result<String> {
        self.verify_bound(token, client_id, None).await.map(|v| v.0)
    }
    pub(crate) async fn verify_bound(
        &self,
        token: &str,
        client_id: &str,
        expected: Option<&str>,
    ) -> Result<(String, Option<String>)> {
        require(
            !token.is_empty() && token.len() <= 12288,
            ErrorCode::InvalidCredentials,
        )?;
        let mut parts = token.split('.');
        let (h, c, s) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        require(parts.next().is_none(), ErrorCode::InvalidCredentials)?;
        let decode = |v: &str| {
            URL_SAFE_NO_PAD
                .decode(v)
                .map_err(|_| Error::new(ErrorCode::InvalidCredentials))
        };
        let header: Header = serde_json::from_slice(&decode(h)?)
            .map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
        require(
            header.alg == "RS256"
                && !header.kid.is_empty()
                && header.kid.len() <= 256
                && header.crit.is_empty(),
            ErrorCode::InvalidCredentials,
        )?;
        let claims: Claims = serde_json::from_slice(&decode(c)?)
            .map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
        let now = state::now() / 1000;
        require(
            self.issuer.as_ref().map_or_else(
                || {
                    matches!(
                        claims.iss.as_str(),
                        "accounts.google.com" | "https://accounts.google.com"
                    )
                },
                |issuer| claims.iss == *issuer,
            ) && claims.aud == client_id
                && claims.email_verified
                && !claims.sub.is_empty()
                && claims.sub.len() <= 255
                && claims.exp > now.saturating_sub(60)
                && claims.iat <= now.saturating_add(60)
                && claims.iat < claims.exp,
            ErrorCode::InvalidCredentials,
        )?;
        let signature = decode(s)?;
        // This separate mutex coalesces refreshes without holding the DB gate.
        // Unknown kids get at most one refresh/minute, including failed fetches.
        let mut cache = self.cache.lock().await;
        let fresh = cache.expires.is_some_and(|e| Instant::now() < e);
        let known = cache.keys.iter().any(|k| k.kid == header.kid);
        if !fresh || !known {
            let throttled = cache
                .attempted
                .is_some_and(|t| t.elapsed() < REFRESH_INTERVAL);
            if !throttled {
                cache.attempted = Some(Instant::now());
                cache.failed = true; // Also back off after cancellation of fetch.
                let (keys, ttl) = self.fetch().await?;
                cache.keys = keys;
                cache.expires = Some(Instant::now() + ttl);
                cache.failed = false;
            } else if !fresh {
                return Err(Error::new(ErrorCode::Unavailable));
            }
        }
        let key = cache
            .keys
            .iter()
            .find(|k| k.kid == header.kid)
            .ok_or_else(|| {
                Error::new(if cache.failed {
                    ErrorCode::Unavailable
                } else {
                    ErrorCode::InvalidCredentials
                })
            })?;
        let signed = format!("{h}.{c}");
        RsaPublicKeyComponents {
            n: &key.n,
            e: &key.e,
        }
        .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
        .map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
        if let Some(expected) = expected {
            require(
                claims.nonce.as_deref() == Some(expected),
                ErrorCode::InvalidCredentials,
            )?;
        }
        Ok((claims.sub, claims.nonce))
    }
}
fn cache_ttl(headers: &reqwest::header::HeaderMap) -> Duration {
    let mut age = 0;
    let mut no_cache = false;
    let mut max_age = None;
    for value in headers.get_all("cache-control") {
        if let Ok(value) = value.to_str() {
            for directive in value.split(',').map(str::trim) {
                if directive.eq_ignore_ascii_case("no-cache")
                    || directive.eq_ignore_ascii_case("no-store")
                {
                    no_cache = true;
                }
                if let Some((key, value)) = directive.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("max-age") {
                        max_age = value.trim().trim_matches('"').parse::<u64>().ok();
                    }
                }
            }
        }
    }
    if let Some(value) = headers
        .get("age")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        age = value;
    }
    Duration::from_secs(if no_cache {
        0
    } else {
        max_age.unwrap_or(300).saturating_sub(age).min(86400)
    })
}

pub(crate) fn is_route(path: &str) -> bool {
    matches!(
        path,
        "meta"
            | "auth/google"
            | "auth/google/complete"
            | "auth/google/link"
            | "auth/google/unlink"
            | "auth/methods"
            | "auth/password"
    )
}
pub(crate) async fn api(app: &App, path: &str, r: Request<Body>) -> Result<ApiResponse> {
    let mut response = dispatch(app, path, r).await?;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("static header"));
    Ok(response)
}
async fn dispatch(app: &App, path: &str, r: Request<Body>) -> Result<ApiResponse> {
    let method = r.method().clone();
    if path == "meta" && method == "GET" {
        return Ok(http::json(MetaResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            google_client_id: app.0.config.google_client_id.clone(),
            gifs: app.0.gifs.capability(),
        }));
    }
    let client_id = if path.starts_with("auth/google") {
        Some(
            app.0
                .config
                .google_client_id
                .as_deref()
                .or(app.0.config.browser.google_web_client_id.as_deref())
                .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?,
        )
    } else {
        None
    };
    require(
        (path == "auth/methods" && method == "GET")
            || (!matches!(path, "meta" | "auth/methods") && method == "POST"),
        ErrorCode::NotFound,
    )?;
    let bucket = http::auth_bucket(app, &r);
    app.0.core.lock().await.rate(bucket, 10., 5.)?;
    if path == "auth/google" {
        let request: GoogleAuthRequest = http::parse(r).await?;
        let subject = crate::reauth::verify_challenge(
            app,
            &request.id_token,
            client_id.unwrap_or_default(),
            None,
            AccountOperation::SignIn,
        )
        .await?;
        return Ok(http::json(sign_in(app, subject).await?));
    }
    if path == "auth/google/complete" {
        let request: GoogleCompleteRequest = http::parse(r).await?;
        require(request.ticket.len() == 43, ErrorCode::InvalidSignupTicket)?;
        let name = username(&request.username)?;
        let mut core = app.0.core.lock().await;
        core.rate(format!("auth-name:{name}"), 5., 3.)?;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        let hash = state::hash(request.ticket.as_bytes());
        let subject: String = sqlx::query_scalar(
            "SELECT subject FROM oauth_signup_tickets WHERE token_hash=? AND expires_at>?",
        )
        .bind(&hash)
        .bind(state::now() as i64)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::InvalidSignupTicket))?;
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM oauth_identities WHERE provider='google' AND subject=?)",
        )
        .bind(&subject)
        .fetch_one(&mut *tx)
        .await?;
        require(!linked, ErrorCode::InvalidSignupTicket)?;
        let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username=?)")
            .bind(&name)
            .fetch_one(&mut *tx)
            .await?;
        require(
            !taken && !crate::account::reserved(&mut tx, &name).await?,
            ErrorCode::UsernameTaken,
        )?;
        let user = core.id().await?;
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'','online',?)")
            .bind(id(user)?).bind(&name).bind(state::now() as i64).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO oauth_identities VALUES('google',?,?)")
            .bind(&subject)
            .bind(id(user)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM oauth_signup_tickets WHERE token_hash=?")
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        let response = session(&mut tx, user).await?;
        core.persist_id(&mut tx).await?;
        tx.commit().await?;
        return Ok(http::json(response));
    }
    let headers = r.headers().clone();
    let token = http::bearer(r.headers())?.to_owned();
    // Authenticate before external I/O / hashing, then again under the gate.
    let (_, user) = auth(&mut *app.0.pool.acquire().await?, &token).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("auth-user:{}", user.0), 10., 5.)?;
    if path == "auth/password" {
        let request: SetPasswordRequest = http::parse(r).await?;
        require(
            http::password_len_ok(&request.new_password)
                && request
                    .current_password
                    .as_ref()
                    .is_none_or(|p| p.len() <= 128),
            ErrorCode::InvalidRequest,
        )?;
        let use_proof =
            headers.contains_key("x-sticktalk-reauth") && request.current_password.is_none();
        let old: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=?")
            .bind(id(user)?)
            .fetch_one(&app.0.pool)
            .await?;
        let expected = old.clone();
        let permit = app
            .0
            .passwords
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let new = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            require(
                old.is_empty()
                    || use_proof
                    || request
                        .current_password
                        .as_ref()
                        .is_some_and(|p| http::verify_password(p, &old)),
                ErrorCode::InvalidCredentials,
            )?;
            http::password_hash(request.new_password)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))??;
        let mut gate = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        let (key, current) = auth(&mut tx, &token).await?;
        require(current == user, ErrorCode::Unauthenticated)?;
        if expected.is_empty() || use_proof {
            crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::Password).await?;
        }
        let result = sqlx::query("UPDATE users SET password_hash=? WHERE id=? AND password_hash=?")
            .bind(new)
            .bind(id(user)?)
            .bind(expected)
            .execute(&mut *tx)
            .await?;
        require(result.rows_affected() == 1, ErrorCode::Conflict)?;
        let keys: Vec<Vec<u8>> =
            sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=?")
                .bind(id(user)?)
                .fetch_all(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id=?")
            .bind(id(user)?)
            .execute(&mut *tx)
            .await?;
        let rotated = session(&mut tx, user).await?;
        let status = methods(&mut tx, user).await?;
        tx.commit().await?;
        crate::reauth::disconnect(app, &mut gate, &mut c, &keys).await?;
        if crate::browser::session_cookie(&headers)?.is_some() {
            return Ok(http::ApiResponse::from_response(
                crate::browser::browser_session(rotated),
            ));
        }
        return Ok(http::json(AccountUpdateResponse {
            methods: status,
            session: Some(rotated),
        }));
    }
    let subject = if path == "auth/google/link" {
        let request: GoogleLinkRequest = http::parse(r).await?;
        let key = state::hash(token.as_bytes());
        Some(
            crate::reauth::verify_challenge(
                app,
                &request.id_token,
                client_id.unwrap_or_default(),
                Some(&key),
                AccountOperation::Link,
            )
            .await?,
        )
    } else if path == "auth/google/unlink" {
        let _: GoogleUnlinkRequest = http::parse(r).await?;
        None
    } else {
        None
    };
    let _gate = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let (key, current) = auth(&mut tx, &token).await?;
    require(current == user, ErrorCode::Unauthenticated)?;
    if path == "auth/google/link" || path == "auth/google/unlink" {
        crate::reauth::consume(
            &mut tx,
            &headers,
            &key,
            if path.ends_with("/link") {
                AccountOperation::Link
            } else {
                AccountOperation::Unlink
            },
        )
        .await?;
    }
    if let Some(subject) = subject {
        let rows = sqlx::query("SELECT subject,user_id FROM oauth_identities WHERE provider='google' AND (subject=? OR user_id=?)")
            .bind(&subject).bind(id(user)?).fetch_all(&mut *tx).await?;
        require(
            rows.iter().all(|r| {
                r.get::<i64, _>("user_id") == user.0 as i64
                    && r.get::<String, _>("subject") == subject
            }),
            ErrorCode::IdentityAlreadyLinked,
        )?;
        if rows.is_empty() {
            sqlx::query("INSERT INTO oauth_identities VALUES('google',?,?)")
                .bind(&subject)
                .bind(id(user)?)
                .execute(&mut *tx)
                .await?;
        }
        // A sign-in ticket must not survive linking and later unlinking.
        sqlx::query("DELETE FROM oauth_signup_tickets WHERE subject=?")
            .bind(subject)
            .execute(&mut *tx)
            .await?;
    } else if path == "auth/google/unlink" {
        let status = methods(&mut tx, user).await?;
        require(
            !status.google_linked || status.has_password,
            ErrorCode::LastLoginMethod,
        )?;
        sqlx::query("DELETE FROM oauth_identities WHERE provider='google' AND user_id=?")
            .bind(id(user)?)
            .execute(&mut *tx)
            .await?;
    }
    let result = methods(&mut tx, user).await?;
    tx.commit().await?;
    Ok(http::json(result))
}
pub(crate) async fn sign_in(app: &App, subject: String) -> Result<GoogleAuthResponse> {
    let mut core = app.0.core.lock().await;
    core.rate(format!("google-sub:{subject}"), 10., 5.)?;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let u: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM oauth_identities WHERE provider='google' AND subject=?",
    )
    .bind(&subject)
    .fetch_optional(&mut *tx)
    .await?;
    let response = if let Some(u) = u {
        GoogleAuthResponse::Session(session(&mut tx, Id(u as u64)).await?)
    } else {
        sqlx::query("DELETE FROM oauth_signup_tickets WHERE expires_at<=?")
            .bind(state::now() as i64)
            .execute(&mut *tx)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM oauth_signup_tickets WHERE subject<>?")
                .bind(&subject)
                .fetch_one(&mut *tx)
                .await?;
        require(count < 4096, ErrorCode::LimitExceeded)?;
        let ticket = state::token();
        let expires_at = state::now() + 600_000;
        // Only one outstanding ticket/subject, bounded globally. A fresh
        // verified sign-in invalidates that subject's previous ticket.
        sqlx::query("INSERT INTO oauth_signup_tickets VALUES(?,?,?) ON CONFLICT(subject) DO UPDATE SET token_hash=excluded.token_hash,expires_at=excluded.expires_at")
                .bind(state::hash(ticket.as_bytes())).bind(&subject).bind(expires_at as i64).execute(&mut *tx).await?;
        GoogleAuthResponse::SignupRequired(GoogleSignupRequired { ticket, expires_at })
    };
    tx.commit().await?;
    Ok(response)
}
async fn methods(c: &mut SqliteConnection, user: Id) -> Result<AuthMethodsResponse> {
    let r = sqlx::query("SELECT password_hash<>'' AS has_password,EXISTS(SELECT 1 FROM oauth_identities WHERE provider='google' AND user_id=users.id) AS google_linked FROM users WHERE id=?")
        .bind(id(user)?).fetch_one(c).await?;
    Ok(AuthMethodsResponse {
        has_password: r.get("has_password"),
        google_linked: r.get("google_linked"),
    })
}
async fn session(c: &mut SqliteConnection, user: Id) -> Result<AuthResponse> {
    crate::db::enabled(c, user).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id=? AND expires_at>? AND token_hash NOT IN (SELECT session_hash FROM mcp_grants)")
            .bind(id(user)?)
            .bind(state::now() as i64)
            .fetch_one(&mut *c)
            .await?;
    require(
        count < MAX_SESSIONS_PER_USER as i64,
        ErrorCode::LimitExceeded,
    )?;
    let token = state::token();
    let now = state::now();
    let expires_at = now + 30 * 86_400_000;
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(state::hash(token.as_bytes()))
        .bind(id(user)?)
        .bind(now as i64)
        .bind(expires_at as i64)
        .bind(now as i64)
        .execute(&mut *c)
        .await?;
    Ok(AuthResponse {
        token,
        expires_at,
        user: me(c, user).await?,
    })
}

#[cfg(test)]
mod tests;
