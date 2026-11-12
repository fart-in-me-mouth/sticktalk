//! Same-origin browser boundary. Native bearer requests are never rewritten.
use crate::{
    db,
    error::{require, Error, Result},
    http,
    state::{self, App},
};
use axum::{
    body::{to_bytes, Body},
    extract::{Query, State},
    http::{HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::Args;
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use sticktalk_proto::*;
use subtle::ConstantTimeEq;
use tower::ServiceExt;

const SESSION: &str = "__Host-sticktalk_session";
const CSRF: &str = "__Host-sticktalk_csrf";
const OAUTH: &str = "__Host-sticktalk_oauth";
const SIGNUP: &str = "__Host-sticktalk_signup";

#[derive(Args, Clone, Default)]
pub struct BrowserOptions {
    #[arg(long, env = "STICKTALK_WEB_ROOT")]
    pub web_root: Option<PathBuf>,
    /// HTTPS origins only; no wildcard sources are accepted.
    #[arg(long, env = "STICKTALK_GIF_MEDIA_ORIGINS", value_delimiter = ',')]
    pub gif_media_origins: Vec<String>,
    #[arg(long, env = "STICKTALK_GOOGLE_WEB_CLIENT_ID")]
    pub google_web_client_id: Option<String>,
    #[arg(
        long,
        env = "STICKTALK_GOOGLE_WEB_CLIENT_SECRET",
        hide_env_values = true
    )]
    pub google_web_client_secret: Option<String>,
    #[arg(long, env = "STICKTALK_MOCK_GOOGLE_TOKEN_URL")]
    pub mock_google_token_url: Option<String>,
    #[arg(long, env = "STICKTALK_PUSH_ORIGINS", value_delimiter = ',')]
    pub push_origins: Vec<String>,
    #[arg(long, env = "STICKTALK_VAPID_SUBJECT")]
    pub vapid_subject: Option<String>,
    /// Loopback-only mock push service; never enable on a public bind.
    #[arg(long, env = "STICKTALK_MOCK_PUSH_ORIGIN")]
    pub mock_push_origin: Option<String>,
}
impl std::fmt::Debug for BrowserOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserOptions")
            .field("web_root", &self.web_root)
            .field("google_web_client_id", &self.google_web_client_id)
            .field(
                "google_web_secret_configured",
                &self.google_web_client_secret.is_some(),
            )
            .finish_non_exhaustive()
    }
}
pub(crate) struct Browser {
    csp: axum::http::HeaderValue,
    client: reqwest::Client,
}
pub(crate) fn origin(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        u.host_str().is_some_and(|h| !h.contains('*'))
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && u.path() == "/",
        ErrorCode::InvalidRequest,
    )?;
    Ok(u)
}
impl Browser {
    pub fn new(config: &crate::Config) -> Result<Self> {
        let u = origin(&config.public_url)?;
        require(
            u.scheme() == "https" || u.scheme() == "http",
            ErrorCode::InvalidRequest,
        )?;
        let b = &config.browser;
        require(
            b.google_web_client_id.is_some() == b.google_web_client_secret.is_some(),
            ErrorCode::InvalidRequest,
        )?;
        for s in [&b.google_web_client_id, &b.google_web_client_secret]
            .into_iter()
            .flatten()
        {
            require(
                !s.trim().is_empty() && s.len() <= 1024,
                ErrorCode::InvalidRequest,
            )?;
        }
        if let Some(url) = &b.mock_google_token_url {
            require(config.bind.ip().is_loopback(), ErrorCode::InvalidRequest)?;
            state::validate_mock_url(url)?;
        }
        let mut media = String::new();
        require(b.gif_media_origins.len() <= 16, ErrorCode::InvalidRequest)?;
        for value in &b.gif_media_origins {
            let u = origin(value)?;
            require(u.scheme() == "https", ErrorCode::InvalidRequest)?;
            media.push(' ');
            media.push_str(&u.origin().ascii_serialization());
        }
        // Single-threaded wasm needs no cross-origin isolation. Do not enable
        // COEP: it would block provider GIF/video assets lacking CORP headers.
        let ws_origin = config
            .public_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        // Dynamic avatar colors/virtual spacer heights are style attributes only.
        // Keep inline script and style ELEMENTS blocked; the web packager externalizes
        // Trunk's bootstrap and generated UI tokens into same-origin hashed assets.
        let csp=format!("default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'unsafe-inline'; font-src 'self'; img-src 'self'{media}; media-src 'self'{media}; connect-src 'self' {}; worker-src 'self'; manifest-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'",ws_origin.trim_end_matches('/')).parse().map_err(|_|Error::new(ErrorCode::InvalidRequest))?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        Ok(Self { csp, client })
    }
}
/// Reject duplicate cookie names rather than permitting path-shadowing ambiguity.
pub(crate) fn cookie<'a>(h: &'a HeaderMap, name: &str) -> Result<Option<&'a str>> {
    let mut result = None;
    for line in h.get_all("cookie") {
        for item in line
            .to_str()
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?
            .split(';')
        {
            if let Some((k, v)) = item.trim().split_once('=') {
                if k == name {
                    require(result.is_none(), ErrorCode::InvalidRequest)?;
                    result = Some(v);
                }
            }
        }
    }
    Ok(result)
}
pub(crate) fn session_cookie(h: &HeaderMap) -> Result<Option<&str>> {
    cookie(h, SESSION)
}
fn same_origin(app: &App, h: &HeaderMap) -> bool {
    h.get_all("origin").iter().count() == 1
        && h.get("origin").and_then(|v| v.to_str().ok())
            == Some(app.0.config.public_url.trim_end_matches('/'))
}
fn csrf(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(
        ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, token.as_bytes()),
            b"sticktalk-browser-csrf-v1",
        )
        .as_ref(),
    )
}
fn equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}
fn set_cookie(r: &mut Response, name: &str, value: &str, age: u64, http_only: bool, lax: bool) {
    let s = format!(
        "{name}={value}; Path=/; Max-Age={age}; Secure; SameSite={}{}",
        if lax { "Lax" } else { "Strict" },
        if http_only { "; HttpOnly" } else { "" }
    );
    r.headers_mut()
        .append("set-cookie", s.parse().expect("generated cookie"));
}
fn no_store(r: &mut Response) {
    r.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
}
fn browser_session(auth: AuthResponse) -> Response {
    let token = csrf(&auth.token);
    let age = auth.expires_at.saturating_sub(state::now()) / 1000;
    let credential = auth.token;
    let mut r = axum::Json(BrowserSession {
        user: auth.user,
        expires_at: auth.expires_at,
        csrf_token: token.clone(),
    })
    .into_response();
    set_cookie(&mut r, SESSION, &credential, age, true, false);
    set_cookie(&mut r, CSRF, &token, age, false, false);
    no_store(&mut r);
    r
}
/// Outermost middleware protects *all* REST upload/auth routes, not just JSON.
pub(crate) async fn boundary(
    State(app): State<App>,
    mut r: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let result = async {
        let api = r.uri().path().starts_with("/api/");
        let cookie_auth = api && !r.headers().contains_key("authorization");
        if cookie_auth {
            if let Some(token) = session_cookie(r.headers())?.map(str::to_owned) {
                let ws = r.uri().path() == "/api/v1/ws";
                if ws || !matches!(*r.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
                    require(same_origin(&app, r.headers()), ErrorCode::Forbidden)?;
                    if !ws {
                        let provided = r
                            .headers()
                            .get("x-sticktalk-csrf")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        require(
                            equal(provided, &csrf(&token))
                                && cookie(r.headers(), CSRF)?.is_some_and(|c| equal(c, provided)),
                            ErrorCode::Forbidden,
                        )?;
                    }
                }
                // Leave the WS cookie visible so upgrade can bind Identify to it.
                if !ws {
                    r.headers_mut().insert(
                        "authorization",
                        format!("Bearer {token}")
                            .parse()
                            .map_err(|_| Error::new(ErrorCode::Unauthenticated))?,
                    );
                }
            }
        }
        let logout = r.uri().path().ends_with("/auth/logout");
        let mut response = next.run(r).await;
        if cookie_auth && logout && response.status().is_success() {
            set_cookie(&mut response, SESSION, "", 0, true, false);
            set_cookie(&mut response, CSRF, "", 0, false, false);
        }
        if api {
            no_store(&mut response);
        }
        Ok::<_, Error>(response)
    }
    .await;
    let mut response = result.unwrap_or_else(IntoResponse::into_response);
    response
        .headers_mut()
        .insert("content-security-policy", app.0.browser.csp.clone());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert("permissions-policy","microphone=(self), camera=(self), display-capture=(self), geolocation=(), payment=(), usb=()".parse().unwrap());
    response
        .headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    response
}
pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/browser/capabilities", get(capabilities))
        .route("/api/v1/browser/session", get(current))
        .route("/api/v1/browser/session/login", post(login))
        .route("/api/v1/browser/session/register", post(register))
        .route("/api/v1/browser/session/google/complete", post(complete))
        .route("/api/v1/browser/google/start", get(start))
        .route("/api/v1/browser/google/callback", get(callback))
}
async fn capabilities(State(app): State<App>) -> axum::Json<BrowserCapabilities> {
    axum::Json(BrowserCapabilities {
        cookie_sessions: true,
        google_redirect: web_client(&app).is_ok(),
        web_push: true,
    })
}
fn require_browser(app: &App, r: &Request<Body>) -> Result<()> {
    require(same_origin(app, r.headers()), ErrorCode::Forbidden)?;
    require(
        !r.headers().contains_key("authorization") || session_cookie(r.headers())?.is_some(),
        ErrorCode::InvalidRequest,
    )?;
    Ok(())
}
async fn password(app: App, mut r: Request<Body>, register: bool) -> Result<Response> {
    require_browser(&app, &r)?;
    *r.uri_mut() = if register {
        "/api/v1/auth/register"
    } else {
        "/api/v1/auth/login"
    }
    .parse()
    .unwrap();
    let auth = http::api(State(app.clone()), r).await?.into_response();
    require(auth.status().is_success(), ErrorCode::InvalidCredentials)?;
    let auth: AuthResponse = serde_json::from_slice(
        &to_bytes(auth.into_body(), 16384)
            .await
            .map_err(|_| Error::new(ErrorCode::Internal))?,
    )
    .map_err(|_| Error::new(ErrorCode::Internal))?;
    Ok(browser_session(auth))
}
async fn login(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    password(app, r, false).await
}
async fn register(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    password(app, r, true).await
}
async fn current(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let token = http::bearer(&headers)?;
    let (key, u) = db::auth(&mut c, token).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let expires: i64 = sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=?")
        .bind(key)
        .fetch_one(&mut *c)
        .await?;
    let mut r = axum::Json(BrowserSession {
        user: db::me(&mut c, u).await?,
        expires_at: expires as u64,
        csrf_token: csrf(token),
    })
    .into_response();
    set_cookie(
        &mut r,
        CSRF,
        &csrf(token),
        expires.saturating_sub(state::now() as i64).max(0) as u64 / 1000,
        false,
        false,
    );
    no_store(&mut r);
    Ok(r)
}
async fn complete(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    require_browser(&app, &r)?;
    let ticket = cookie(r.headers(), SIGNUP)?
        .ok_or_else(|| Error::new(ErrorCode::InvalidSignupTicket))?
        .to_owned();
    require(
        cookie(r.headers(), CSRF)?.is_some_and(|c| {
            r.headers()
                .get("x-sticktalk-csrf")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|h| equal(h, c) && equal(h, &csrf(&ticket)))
        }),
        ErrorCode::Forbidden,
    )?;
    let mut request: GoogleCompleteRequest = http::parse(r).await?;
    require(request.ticket.is_empty(), ErrorCode::InvalidRequest)?;
    request.ticket = ticket;
    let r = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/google/complete")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&request).map_err(|_| Error::new(ErrorCode::Internal))?,
        ))
        .unwrap();
    let auth = crate::google::api(&app, "auth/google/complete", r)
        .await?
        .into_response();
    let auth = serde_json::from_slice(
        &to_bytes(auth.into_body(), 16384)
            .await
            .map_err(|_| Error::new(ErrorCode::Internal))?,
    )
    .map_err(|_| Error::new(ErrorCode::Internal))?;
    let mut response = browser_session(auth);
    set_cookie(&mut response, SIGNUP, "", 0, true, false);
    Ok(response)
}
fn web_client(app: &App) -> Result<(&str, &str)> {
    require(
        app.0.config.public_url.starts_with("https://"),
        ErrorCode::FeatureDisabled,
    )?;
    Ok((
        app.0
            .config
            .browser
            .google_web_client_id
            .as_deref()
            .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?,
        app.0
            .config
            .browser
            .google_web_client_secret
            .as_deref()
            .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?,
    ))
}
fn callback_uri(app: &App) -> String {
    format!(
        "{}/api/v1/browser/google/callback",
        app.0.config.public_url.trim_end_matches('/')
    )
}
async fn start(State(app): State<App>, r: Request<Body>) -> Result<Response> {
    let (client_id, _) = web_client(&app)?;
    if r.headers().contains_key("origin") {
        require(same_origin(&app, r.headers()), ErrorCode::Forbidden)?;
    }
    app.0
        .core
        .lock()
        .await
        .rate(http::auth_bucket(&app, &r), 10., 5.)?;
    let state = state::token();
    let verifier = state::token();
    let mut core = app.0.core.lock().await;
    core.rate("web-oauth-global".into(), 100., 2.)?;
    let mut c = app.0.pool.acquire().await?;
    sqlx::query("DELETE FROM web_oauth_states WHERE expires_at<=?")
        .bind(state::now() as i64)
        .execute(&mut *c)
        .await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM web_oauth_states")
        .fetch_one(&mut *c)
        .await?;
    require(n < 4096, ErrorCode::LimitExceeded)?;
    sqlx::query("INSERT INTO web_oauth_states VALUES(?,?,?)")
        .bind(state::hash(state.as_bytes()))
        .bind(&verifier)
        .bind((state::now() + 600_000) as i64)
        .execute(&mut *c)
        .await?;
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", &callback_uri(&app)),
        ("response_type", "code"),
        ("scope", "openid email profile"),
        ("state", &state),
        (
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(state::hash(verifier.as_bytes())),
        ),
        ("code_challenge_method", "S256"),
    ]);
    let mut r = (StatusCode::SEE_OTHER, [("location", url.as_str())]).into_response();
    // The transient state cookie must survive Google's cross-site top-level GET.
    set_cookie(&mut r, OAUTH, &state, 600, true, true);
    no_store(&mut r);
    Ok(r)
}
#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<Callback>,
) -> Result<Response> {
    let (client_id, secret) = web_client(&app)?;
    require(
        q.state.len() == 43 && cookie(&headers, OAUTH)?.is_some_and(|s| equal(s, &q.state)),
        ErrorCode::InvalidCredentials,
    )?;
    let verifier: Option<String> = sqlx::query_scalar(
        "DELETE FROM web_oauth_states WHERE state_hash=? AND expires_at>? RETURNING verifier",
    )
    .bind(state::hash(q.state.as_bytes()))
    .bind(state::now() as i64)
    .fetch_optional(&app.0.pool)
    .await?;
    let verifier = verifier.ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    require(q.error.is_none(), ErrorCode::InvalidCredentials)?;
    let code = q
        .code
        .filter(|c| !c.is_empty() && c.len() <= 4096)
        .ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    let mut upstream = app
        .0
        .browser
        .client
        .post(
            app.0
                .config
                .browser
                .mock_google_token_url
                .as_deref()
                .unwrap_or("https://oauth2.googleapis.com/token"),
        )
        .form(&[
            ("client_id", client_id),
            ("client_secret", secret),
            ("code", &code),
            ("code_verifier", &verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", &callback_uri(&app)),
        ])
        .send()
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    require(
        upstream.status().is_success(),
        ErrorCode::InvalidCredentials,
    )?;
    let mut body = Vec::new();
    while let Some(chunk) = upstream
        .chunk()
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))?
    {
        require(body.len() + chunk.len() <= 16384, ErrorCode::Unavailable)?;
        body.extend_from_slice(&chunk);
    }
    let token: GoogleAuthRequest =
        serde_json::from_slice(&body).map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
    let subject = app.0.google.verify(&token.id_token, client_id).await?;
    let result = crate::google::sign_in(&app, subject).await?;
    let (mut response, location) = match result {
        GoogleAuthResponse::Session(auth) => (browser_session(auth), "/"),
        GoogleAuthResponse::SignupRequired(ticket) => {
            let mut r = Response::new(Body::empty());
            set_cookie(&mut r, SIGNUP, &ticket.ticket, 600, true, false);
            set_cookie(&mut r, SESSION, "", 0, true, false);
            set_cookie(&mut r, CSRF, &csrf(&ticket.ticket), 600, false, false);
            (r, "/?google=signup")
        }
    };
    *response.status_mut() = StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert("location", location.parse().unwrap());
    set_cookie(&mut response, OAUTH, "", 0, true, true);
    no_store(&mut response);
    Ok(response)
}
fn hashed_asset(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    !name.ends_with(".html")
        && !name.starts_with("sw.")
        && !name.starts_with("sw-")
        && !name.starts_with("service-worker")
        && !name.ends_with(".webmanifest")
        && !name.contains("manifest")
        && name
            .split(['-', '.', '_'])
            .any(|s| s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(crate) async fn static_file(State(app): State<App>, r: Request<Body>) -> Response {
    let Some(root) = &app.0.config.browser.web_root else {
        return Error::new(ErrorCode::NotFound).into_response();
    };
    if r.uri().path() == "/api" || r.uri().path().starts_with("/api/") {
        return Error::new(ErrorCode::NotFound).into_response();
    }
    let immutable = hashed_asset(r.uri().path())
        && r.headers()
            .get("service-worker")
            .and_then(|h| h.to_str().ok())
            != Some("script");
    let service = tower_http::services::ServeDir::new(root)
        .precompressed_br()
        .precompressed_gzip();
    let mut response = service
        .oneshot(r)
        .await
        .expect("infallible ServeDir")
        .map(Body::new);
    response
        .headers_mut()
        .insert("vary", "Accept-Encoding".parse().unwrap());
    let cache_forever = immutable
        && (response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED);
    response.headers_mut().insert(
        "cache-control",
        if cache_forever {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }
        .parse()
        .unwrap(),
    );
    response
}

#[cfg(test)]
mod tests;
