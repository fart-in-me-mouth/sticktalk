//! OAuth 2.1 for the MCP connector: RFC 9728 resource metadata, RFC 8414 server
//! metadata, RFC 7591 dynamic registration (stateless signed client IDs, as in
//! T3 Code), PKCE S256 authorization codes, rotating refresh tokens, RFC 7009
//! revocation. Public clients only.
use super::{pages, sign, verify, SCOPE_READ, SCOPE_WRITE};
use crate::{
    db,
    error::{Error, Result},
    http,
    state::{self, now, App},
};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Connection, Row};
use sticktalk_proto::{ErrorCode, Event, Id};
use subtle::ConstantTimeEq;

const MAX_REDIRECTS: usize = 5;
const MAX_URI: usize = 512;
const MAX_CLIENT_ID: usize = 4096;
const MAX_STATE: usize = 1024;

pub(crate) async fn resource_metadata(State(app): State<App>) -> Response {
    Json(json!({
        "resource": super::resource(&app),
        "authorization_servers": [super::issuer(&app)],
        "scopes_supported": [SCOPE_READ, SCOPE_WRITE],
        "bearer_methods_supported": ["header"],
        "resource_name": "StickTalk",
        "resource_documentation": format!("{}/mcp", super::issuer(&app)),
    }))
    .into_response()
}
pub(crate) async fn server_metadata(State(app): State<App>) -> Response {
    let issuer = super::issuer(&app);
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "registration_endpoint": format!("{issuer}/oauth/register"),
        "revocation_endpoint": format!("{issuer}/oauth/revoke"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE_READ, SCOPE_WRITE],
        "authorization_response_iss_parameter_supported": true,
        "service_documentation": format!("{issuer}/mcp"),
    }))
    .into_response()
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    let mut r = (
        status,
        Json(json!({"error": error, "error_description": description})),
    )
        .into_response();
    r.headers_mut()
        .insert("cache-control", "no-store".parse().expect("static"));
    r
}
fn rate(e: Error) -> Response {
    let mut r = oauth_error(
        StatusCode::TOO_MANY_REQUESTS,
        "temporarily_unavailable",
        "Too many requests; retry later.",
    );
    if let Some(ms) = e.0.retry_after_ms {
        if let Ok(v) = ms.div_ceil(1000).to_string().parse() {
            r.headers_mut().insert("retry-after", v);
        }
    }
    r
}

// ---- Clients -------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ClientBody {
    v: u8,
    n: String,
    r: Vec<String>,
}
pub(crate) struct Client {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}
fn encode_client(key: &ring::hmac::Key, name: &str, uris: &[String]) -> String {
    let body = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&ClientBody {
            v: 1,
            n: name.into(),
            r: uris.to_vec(),
        })
        .expect("serializable client"),
    );
    let tag = sign(key, "mcp-client-id", body.as_bytes());
    format!("{body}.{tag}")
}
pub(crate) fn decode_client(key: &ring::hmac::Key, id: &str) -> Option<Client> {
    if id.len() > MAX_CLIENT_ID {
        return None;
    }
    let (body, tag) = id.split_once('.')?;
    if !verify(key, "mcp-client-id", body.as_bytes(), tag) {
        return None;
    }
    let c: ClientBody = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).ok()?).ok()?;
    (c.v == 1).then(|| Client {
        id: id.into(),
        name: c.n,
        redirect_uris: c.r,
    })
}
fn loopback(u: &Url) -> bool {
    u.scheme() == "http" && matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}
/// Schemes a browser could execute or that address local resources.
const DENIED_SCHEMES: &[&str] = &[
    "javascript",
    "data",
    "file",
    "vbscript",
    "about",
    "blob",
    "filesystem",
    "view-source",
    "ws",
    "wss",
    "ftp",
    "jar",
    "mailto",
    "tel",
    "sms",
    "intent",
];
/// HTTPS anywhere, HTTP only on loopback (RFC 8252 §7.3), or a private-use
/// app scheme such as `cursor://` (§7.1). Never fragments or credentials.
pub(crate) fn valid_redirect(s: &str) -> bool {
    if s.len() > MAX_URI || s.contains('#') || s.chars().any(|c| c.is_control()) {
        return false;
    }
    let Ok(u) = Url::parse(s) else {
        return false;
    };
    if !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => u.host_str().is_some(),
        "http" => loopback(&u),
        scheme => !DENIED_SCHEMES.contains(&scheme),
    }
}
/// Exact match, except loopback redirects may use any port (RFC 8252 §7.3).
fn redirect_matches(registered: &str, presented: &str) -> bool {
    if registered == presented {
        return true;
    }
    match (Url::parse(registered), Url::parse(presented)) {
        (Ok(a), Ok(b)) if loopback(&a) && loopback(&b) => {
            a.host_str() == b.host_str() && a.path() == b.path() && a.query() == b.query()
        }
        _ => false,
    }
}
fn clean_name(name: Option<&str>) -> String {
    let name: String = name
        .unwrap_or_default()
        .chars()
        // Bidi overrides could make the consent page show a different name.
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    if name.is_empty() {
        "MCP client".into()
    } else {
        name
    }
}

#[derive(Deserialize)]
struct Registration {
    redirect_uris: Option<Vec<String>>,
    client_name: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
}
pub(crate) async fn register(State(app): State<App>, r: Request<Body>) -> Response {
    let bucket = http::auth_bucket(&app, &r).replacen("auth-ip", "mcp-register", 1);
    if let Err(e) = app.0.core.lock().await.rate(bucket, 20., 10.) {
        return rate(e);
    }
    let Ok(reg) = http::parse::<Registration>(r).await else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Send a JSON client registration (RFC 7591).",
        );
    };
    let uris = reg.redirect_uris.unwrap_or_default();
    if uris.is_empty() || uris.len() > MAX_REDIRECTS || !uris.iter().all(|u| valid_redirect(u)) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "Use 1-5 redirect URIs: https, http on localhost/127.0.0.1/[::1], or an app scheme; no fragments.",
        );
    }
    let grants_ok = reg.grant_types.as_ref().is_none_or(|g| {
        g.iter().any(|v| v == "authorization_code")
            && g.iter()
                .all(|v| matches!(v.as_str(), "authorization_code" | "refresh_token"))
    });
    let responses_ok = reg
        .response_types
        .as_ref()
        .is_none_or(|t| t.iter().all(|v| v == "code"));
    if !grants_ok || !responses_ok {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Only the authorization_code and refresh_token grants with response type code are supported.",
        );
    }
    let Ok(key) = super::secret(&app).await else {
        return oauth_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "Try again later.",
        );
    };
    let name = clean_name(reg.client_name.as_deref());
    let client_id = encode_client(&key, &name, &uris);
    let mut r = (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_id_issued_at": now() / 1000,
            "client_name": name,
            "redirect_uris": uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": format!("{SCOPE_READ} {SCOPE_WRITE}"),
        })),
    )
        .into_response();
    r.headers_mut()
        .insert("cache-control", "no-store".parse().expect("static"));
    r
}

// ---- Authorization -------------------------------------------------------

#[derive(Deserialize, Default, Clone)]
pub(crate) struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
    /// Internal: one same-site reload so a SameSite=Strict session cookie is sent.
    pub st_hop: Option<String>,
    /// Internal: show the sign-in form even when a browser session exists.
    pub st_login: Option<String>,
    // Consent form fields (POST only).
    pub csrf: Option<String>,
    pub decision: Option<String>,
    pub access: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}
pub(crate) struct Valid {
    pub client: Client,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: Option<String>,
    pub default_write: bool,
}
enum Invalid {
    /// Never redirect to an unverified client/redirect pair.
    Page(&'static str),
    Redirect(String, &'static str, &'static str, Option<String>),
}
fn pkce_challenge(s: &str) -> bool {
    s.len() == 43
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn pkce_verifier(s: &str) -> bool {
    (43..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}
fn same_resource(app: &App, presented: &str) -> bool {
    presented.trim_end_matches('/') == super::resource(app)
}
fn validate(
    app: &App,
    key: &ring::hmac::Key,
    p: &AuthorizeParams,
) -> std::result::Result<Valid, Invalid> {
    let client = p
        .client_id
        .as_deref()
        .and_then(|id| decode_client(key, id))
        .ok_or(Invalid::Page(
            "This app's registration is not recognized. Remove the connector from your AI app and add it again.",
        ))?;
    let redirect_uri = p.redirect_uri.clone().ok_or(Invalid::Page(
        "The app did not say where to return after sign-in.",
    ))?;
    if !client
        .redirect_uris
        .iter()
        .any(|r| redirect_matches(r, &redirect_uri))
    {
        return Err(Invalid::Page(
            "The app asked to return to an address it did not register.",
        ));
    }
    let state = p.state.clone();
    let fail = |error, description| {
        Invalid::Redirect(redirect_uri.clone(), error, description, state.clone())
    };
    if state.as_ref().is_some_and(|s| s.len() > MAX_STATE) {
        return Err(fail("invalid_request", "state is too long"));
    }
    if p.response_type.as_deref() != Some("code") {
        return Err(fail(
            "unsupported_response_type",
            "response_type must be code",
        ));
    }
    let challenge = p.code_challenge.clone().unwrap_or_default();
    if p.code_challenge_method.as_deref() != Some("S256") || !pkce_challenge(&challenge) {
        return Err(fail("invalid_request", "PKCE with S256 is required"));
    }
    if p.resource
        .as_deref()
        .is_some_and(|r| !same_resource(app, r))
    {
        return Err(fail("invalid_target", "unknown resource"));
    }
    let scopes: Vec<&str> = p.scope.as_deref().unwrap_or_default().split(' ').collect();
    let default_write = scopes.contains(&SCOPE_WRITE) || !scopes.contains(&SCOPE_READ);
    Ok(Valid {
        client,
        redirect_uri,
        code_challenge: challenge,
        state,
        default_write,
    })
}
pub(crate) fn redirect_with(redirect_uri: &str, params: &[(&str, &str)]) -> Response {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return pages::error(StatusCode::BAD_REQUEST, "Invalid return address.");
    };
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    let mut r = StatusCode::SEE_OTHER.into_response();
    r.headers_mut().insert(
        "location",
        url.as_str()
            .parse()
            .unwrap_or_else(|_| "about:blank".parse().expect("static")),
    );
    r.headers_mut()
        .insert("cache-control", "no-store".parse().expect("static"));
    r
}
fn invalid(app: &App, e: Invalid) -> Response {
    match e {
        Invalid::Page(msg) => pages::error(StatusCode::BAD_REQUEST, msg),
        Invalid::Redirect(uri, error, description, state) => {
            let iss = super::issuer(app);
            let mut params = vec![("error", error), ("error_description", description)];
            if let Some(s) = state.as_deref() {
                params.push(("state", s));
            }
            params.push(("iss", &iss));
            redirect_with(&uri, &params)
        }
    }
}
/// Binds the consent form to the browser session (or "anon") and the request.
fn csrf_data(session: Option<&[u8]>, v: &Valid) -> Vec<u8> {
    let mut d = session
        .map(<[u8]>::to_vec)
        .unwrap_or_else(|| b"anon".to_vec());
    for part in [
        v.client.id.as_str(),
        &v.redirect_uri,
        &v.code_challenge,
        v.state.as_deref().unwrap_or_default(),
    ] {
        d.push(0);
        d.extend_from_slice(part.as_bytes());
    }
    d
}
async fn browser_user(app: &App, h: &HeaderMap) -> Option<(Vec<u8>, Id)> {
    let token = crate::browser::session_cookie(h).ok().flatten()?.to_owned();
    let mut c = app.0.pool.acquire().await.ok()?;
    let (key, u) = db::auth(&mut c, &token).await.ok()?;
    db::enabled(&mut c, u).await.ok()?;
    Some((key, u))
}
async fn render(
    app: &App,
    key: &ring::hmac::Key,
    v: &Valid,
    p: &AuthorizeParams,
    session: Option<(Vec<u8>, Id)>,
    error: Option<&str>,
) -> Response {
    let user = match &session {
        Some((_, u)) => {
            let Ok(mut c) = app.0.pool.acquire().await else {
                return pages::error(StatusCode::SERVICE_UNAVAILABLE, "Try again later.");
            };
            db::user(&mut c, *u).await.ok()
        }
        None => None,
    };
    let csrf = sign(
        key,
        "mcp-consent",
        &csrf_data(session.as_ref().map(|(k, _)| k.as_slice()), v),
    );
    let write = match p.access.as_deref() {
        Some("write") => true,
        Some("read") => false,
        _ => v.default_write,
    };
    pages::consent(pages::Consent {
        client_name: &v.client.name,
        redirect_uri: &v.redirect_uri,
        user: user.as_ref(),
        csrf: &csrf,
        params: p,
        write,
        error,
        server: &super::issuer(app),
    })
}
pub(crate) async fn authorize_page(State(app): State<App>, r: Request<Body>) -> Response {
    let Ok(axum::extract::Query(p)) =
        axum::extract::Query::<AuthorizeParams>::try_from_uri(r.uri())
    else {
        return pages::error(StatusCode::BAD_REQUEST, "This sign-in link is malformed.");
    };
    let Ok(key) = super::secret(&app).await else {
        return pages::error(StatusCode::SERVICE_UNAVAILABLE, "Try again later.");
    };
    let v = match validate(&app, &key, &p) {
        Ok(v) => v,
        Err(e) => return invalid(&app, e),
    };
    let session = if p.st_login.is_some() {
        None
    } else {
        browser_user(&app, r.headers()).await
    };
    // Arriving from another site, the browser withholds SameSite=Strict cookies.
    // One self-initiated reload is same-site, so a signed-in browser is recognized.
    if session.is_none() && p.st_hop.is_none() && p.st_login.is_none() {
        let target = format!(
            "/oauth/authorize?{}&st_hop=1",
            r.uri().query().unwrap_or_default()
        );
        return pages::hop(&target);
    }
    render(&app, &key, &v, &p, session, None).await
}
fn same_origin(app: &App, h: &HeaderMap) -> bool {
    h.get_all("origin").iter().count() == 1
        && h.get("origin").and_then(|v| v.to_str().ok()) == Some(super::issuer(app).as_str())
}
async fn password_user(app: &App, bucket: String, name: &str, password: String) -> Result<Id> {
    app.0.core.lock().await.rate(bucket, 10., 5.)?;
    let name = db::username(name).map_err(|_| Error::new(ErrorCode::InvalidCredentials))?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("auth-name:{name}"), 5., 3.)?;
    let row = sqlx::query("SELECT id,password_hash FROM users WHERE username=?")
        .bind(&name)
        .fetch_optional(&app.0.pool)
        .await?;
    let hash = row
        .as_ref()
        .map(|r| r.get::<String, _>("password_hash"))
        .filter(|h| !h.is_empty());
    let exists = hash.is_some();
    let phc = hash.unwrap_or_else(|| app.0.dummy_hash.clone());
    let permit = app
        .0
        .passwords
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let ok = (12..=128).contains(&password.len())
        && tokio::task::spawn_blocking(move || {
            let _p = permit;
            http::verify_password(&password, &phc)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    crate::error::require(ok && exists, ErrorCode::InvalidCredentials)?;
    let u = Id(row.expect("exists").get::<i64, _>("id") as u64);
    let mut c = app.0.pool.acquire().await?;
    db::enabled(&mut c, u).await?;
    Ok(u)
}
pub(crate) async fn authorize_decision(State(app): State<App>, r: Request<Body>) -> Response {
    if !same_origin(&app, r.headers()) {
        return pages::error(
            StatusCode::FORBIDDEN,
            "This approval did not come from the StickTalk sign-in page.",
        );
    }
    let bucket = http::auth_bucket(&app, &r);
    let headers = r.headers().clone();
    let Ok(body) = to_bytes(r.into_body(), 16384).await else {
        return pages::error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large.");
    };
    let Ok(p) = serde_urlencoded::from_bytes::<AuthorizeParams>(&body) else {
        return pages::error(StatusCode::BAD_REQUEST, "This approval form is malformed.");
    };
    let Ok(key) = super::secret(&app).await else {
        return pages::error(StatusCode::SERVICE_UNAVAILABLE, "Try again later.");
    };
    let v = match validate(&app, &key, &p) {
        Ok(v) => v,
        Err(e) => return invalid(&app, e),
    };
    let iss = super::issuer(&app);
    let csrf = p.csrf.as_deref().unwrap_or_default();
    let session = browser_user(&app, &headers).await;
    let anon_csrf = verify(&key, "mcp-consent", &csrf_data(None, &v), csrf);
    let session_csrf = session
        .as_ref()
        .is_some_and(|(k, _)| verify(&key, "mcp-consent", &csrf_data(Some(k), &v), csrf));
    if !anon_csrf && !session_csrf {
        return render(
            &app,
            &key,
            &v,
            &p,
            session,
            Some("This page expired. Review the request and try again."),
        )
        .await;
    }
    if p.decision.as_deref() != Some("allow") {
        let mut params = vec![
            ("error", "access_denied"),
            ("error_description", "The user declined."),
        ];
        if let Some(s) = v.state.as_deref() {
            params.push(("state", s));
        }
        params.push(("iss", &iss));
        return redirect_with(&v.redirect_uri, &params);
    }
    let name = p.username.as_deref().unwrap_or_default().trim().to_owned();
    let user = if !name.is_empty() {
        match password_user(&app, bucket, &name, p.password.clone().unwrap_or_default()).await {
            Ok(u) => u,
            Err(e) => {
                let msg = if e.0.code == ErrorCode::RateLimited {
                    "Too many sign-in attempts. Wait a minute and try again."
                } else {
                    "That username and password didn't match."
                };
                return render(&app, &key, &v, &p, None, Some(msg)).await;
            }
        }
    } else if let Some((_, u)) = session.as_ref().filter(|_| session_csrf) {
        *u
    } else {
        return render(&app, &key, &v, &p, session, Some("Sign in to continue.")).await;
    };
    let scope = if p.access.as_deref() == Some("write") {
        format!("{SCOPE_READ} {SCOPE_WRITE}")
    } else {
        SCOPE_READ.to_owned()
    };
    let code = state::token();
    let issued = async {
        let mut c = app.0.pool.acquire().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_grants g JOIN sessions s ON s.token_hash=g.session_hash WHERE g.user_id=? AND s.expires_at>?")
            .bind(db::id(user)?)
            .bind(now() as i64)
            .fetch_one(&mut *c)
            .await?;
        crate::error::require(n < super::MAX_GRANTS_PER_USER, ErrorCode::LimitExceeded)?;
        sqlx::query("INSERT INTO mcp_codes(code_hash,client_id,user_id,redirect_uri,code_challenge,scope,expires_at) VALUES(?,?,?,?,?,?,?)")
            .bind(super::token_hash(&code))
            .bind(&v.client.id)
            .bind(db::id(user)?)
            .bind(&v.redirect_uri)
            .bind(&v.code_challenge)
            .bind(&scope)
            .bind((now() + super::CODE_TTL_MS) as i64)
            .execute(&mut *c)
            .await?;
        Ok::<_, Error>(())
    }
    .await;
    match issued {
        Ok(()) => {}
        Err(e) if e.0.code == ErrorCode::LimitExceeded => {
            return pages::error(
                StatusCode::CONFLICT,
                "You already have 10 connected AI apps. Log one out under Settings › Devices, then try again.",
            )
        }
        Err(_) => return pages::error(StatusCode::SERVICE_UNAVAILABLE, "Try again later."),
    }
    let mut params = vec![("code", code.as_str())];
    if let Some(s) = v.state.as_deref() {
        params.push(("state", s));
    }
    params.push(("iss", &iss));
    redirect_with(&v.redirect_uri, &params)
}

// ---- Tokens --------------------------------------------------------------

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}
fn form_body(h: &HeaderMap) -> bool {
    h.get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';').next().map(str::trim) == Some("application/x-www-form-urlencoded")
        })
}
/// Public clients may still send `Authorization: Basic id:` out of habit.
fn basic_client(h: &HeaderMap) -> Option<String> {
    let v = h
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let raw = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(v.trim())
            .ok()?,
    )
    .ok()?;
    let id = raw.split_once(':').map_or(raw.as_str(), |(id, _)| id);
    serde_urlencoded::from_str::<Vec<(String, String)>>(&format!("v={id}"))
        .ok()?
        .pop()
        .map(|(_, v)| v)
}
/// `stkr_<family>.<secret>`: the family is fixed for a grant's lifetime.
fn new_refresh(family: &str) -> String {
    format!("stkr_{family}.{}", state::token())
}
fn refresh_family(token: &str) -> Option<&str> {
    let (family, secret) = token.strip_prefix("stkr_")?.split_once('.')?;
    (family.len() == 43 && secret.len() == 43).then_some(family)
}
fn tokens(access: &str, refresh: &str, scope: &str) -> Response {
    let mut r = Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": super::ACCESS_TTL_MS / 1000,
        "refresh_token": refresh,
        "scope": scope,
    }))
    .into_response();
    r.headers_mut()
        .insert("cache-control", "no-store".parse().expect("static"));
    r.headers_mut()
        .insert("pragma", "no-cache".parse().expect("static"));
    r
}
fn invalid_grant(description: &str) -> Box<Response> {
    Box::new(oauth_error(
        StatusCode::BAD_REQUEST,
        "invalid_grant",
        description,
    ))
}
pub(crate) async fn token(State(app): State<App>, r: Request<Body>) -> Response {
    let bucket = http::auth_bucket(&app, &r).replacen("auth-ip", "mcp-token", 1);
    if let Err(e) = app.0.core.lock().await.rate(bucket, 60., 20.) {
        return rate(e);
    }
    if !form_body(r.headers()) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Use application/x-www-form-urlencoded.",
        );
    }
    let basic = basic_client(r.headers());
    let Ok(body) = to_bytes(r.into_body(), 16384).await else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Body too large.",
        );
    };
    let Ok(t) = serde_urlencoded::from_bytes::<TokenRequest>(&body) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Malformed form body.",
        );
    };
    let client_id = t.client_id.clone().or(basic).unwrap_or_default();
    let result = match t.grant_type.as_deref() {
        Some("authorization_code") => exchange_code(&app, &t, &client_id).await,
        Some("refresh_token") => refresh(&app, &t, &client_id).await,
        _ => Err(Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Use authorization_code or refresh_token.",
        ))),
    };
    result.unwrap_or_else(|r| *r)
}
type Reply = std::result::Result<Response, Box<Response>>;
fn busy_box(e: Error) -> Box<Response> {
    Box::new(busy(e))
}
fn busy(_: Error) -> Response {
    oauth_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "temporarily_unavailable",
        "Try again later.",
    )
}
async fn exchange_code(app: &App, t: &TokenRequest, client_id: &str) -> Reply {
    let code = t.code.as_deref().unwrap_or_default();
    // Spend the code first: a failed check below must not leave it replayable.
    let row = sqlx::query("DELETE FROM mcp_codes WHERE code_hash=? RETURNING client_id,user_id,redirect_uri,code_challenge,scope,expires_at")
        .bind(super::token_hash(code))
        .fetch_optional(&app.0.pool)
        .await
        .map_err(|e| busy_box(e.into()))?
        .ok_or_else(|| invalid_grant("The authorization code is invalid or was already used."))?;
    let verifier = t.code_verifier.as_deref().unwrap_or_default();
    let challenge = URL_SAFE_NO_PAD.encode(state::hash(verifier.as_bytes()));
    let stored_challenge: String = row.get("code_challenge");
    let ok = row.get::<i64, _>("expires_at") > now() as i64
        && row.get::<String, _>("client_id") == client_id
        && t.redirect_uri
            .as_deref()
            .is_none_or(|r| r == row.get::<String, _>("redirect_uri"))
        && pkce_verifier(verifier)
        && bool::from(challenge.as_bytes().ct_eq(stored_challenge.as_bytes()))
        && t.resource.as_deref().is_none_or(|r| same_resource(app, r));
    if !ok {
        return Err(invalid_grant(
            "The authorization code, client, redirect URI or PKCE verifier did not match.",
        ));
    }
    let user = Id(row.get::<i64, _>("user_id") as u64);
    let scope: String = row.get("scope");
    let key = super::secret(app).await.map_err(busy_box)?;
    let name = decode_client(&key, client_id)
        .map(|c| c.name)
        .unwrap_or_else(|| "MCP client".into());
    let access = super::new_token("stka_");
    let family = state::token();
    let refresh = new_refresh(&family);
    let session = state::hash(&state::random(32));
    let issued = async {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        db::enabled(&mut tx, user).await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_grants g JOIN sessions s ON s.token_hash=g.session_hash WHERE g.user_id=? AND s.expires_at>?")
            .bind(db::id(user)?)
            .bind(now() as i64)
            .fetch_one(&mut *tx)
            .await?;
        crate::error::require(n < super::MAX_GRANTS_PER_USER, ErrorCode::LimitExceeded)?;
        let t = now() as i64;
        // The session's own token is never issued: it exists so the connection is
        // listed and revocable under Devices and can carry command receipts.
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(&session)
            .bind(db::id(user)?)
            .bind(t)
            .bind(t + super::GRANT_TTL_MS as i64)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE session_devices SET device_name=?,platform='AI connector (MCP)' WHERE token_hash=?")
            .bind(name)
            .bind(&session)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO mcp_grants(session_hash,user_id,client_id,scope,refresh_hash,family_hash,created_at) VALUES(?,?,?,?,?,?,?)")
            .bind(&session)
            .bind(db::id(user)?)
            .bind(client_id)
            .bind(&scope)
            .bind(super::token_hash(&refresh))
            .bind(super::token_hash(&family))
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO mcp_access_tokens VALUES(?,?,?)")
            .bind(super::token_hash(&access))
            .bind(&session)
            .bind(t + super::ACCESS_TTL_MS as i64)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        core.emit(&[user], Event::SessionsUpdated)?;
        Ok::<_, Error>(())
    }
    .await;
    match issued {
        Ok(()) => Ok(tokens(&access, &refresh, &scope)),
        Err(e)
            if matches!(
                e.0.code,
                ErrorCode::LimitExceeded | ErrorCode::InvalidCredentials
            ) =>
        {
            Err(invalid_grant(
                "This account can't connect another app right now.",
            ))
        }
        Err(e) => Err(busy_box(e)),
    }
}
async fn revoke_grant(app: &App, session: &[u8]) -> Result<()> {
    let mut core = app.0.core.lock().await;
    let user: Option<i64> =
        sqlx::query_scalar("DELETE FROM sessions WHERE token_hash=? RETURNING user_id")
            .bind(session)
            .fetch_optional(&app.0.pool)
            .await?;
    core.streams.remove(session);
    if let Some(u) = user {
        core.emit(&[Id(u as u64)], Event::SessionsUpdated)?;
    }
    Ok(())
}
async fn refresh(app: &App, t: &TokenRequest, client_id: &str) -> Reply {
    let token = t.refresh_token.as_deref().unwrap_or_default();
    let gone = || invalid_grant("The refresh token is invalid, expired or revoked. Connect again.");
    let family = refresh_family(token).ok_or_else(gone)?;
    let presented = super::token_hash(token);
    let core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await.map_err(|e| busy_box(e.into()))?;
    let row = sqlx::query("SELECT g.session_hash,g.client_id,g.scope,g.refresh_hash FROM mcp_grants g JOIN sessions s ON s.token_hash=g.session_hash WHERE g.family_hash=? AND s.expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users d WHERE d.user_id=g.user_id)")
        .bind(super::token_hash(family))
        .bind(now() as i64)
        .fetch_optional(&mut *c)
        .await
        .map_err(|e| busy_box(e.into()))?
        .ok_or_else(gone)?;
    let session: Vec<u8> = row.get("session_hash");
    let current: Vec<u8> = row.get("refresh_hash");
    if !bool::from(current.ct_eq(&presented)) {
        // Any earlier token of this family was replayed, at any age: assume
        // theft and end the grant, so neither holder keeps access.
        drop(c);
        drop(core);
        revoke_grant(app, &session).await.map_err(busy_box)?;
        return Err(gone());
    }
    let scope: String = row.get("scope");
    if row.get::<String, _>("client_id") != client_id {
        return Err(invalid_grant(
            "The refresh token belongs to another client.",
        ));
    }
    let access = super::new_token("stka_");
    let refresh = new_refresh(family);
    let rotated = async {
        let mut tx = c.begin().await?;
        let t = now() as i64;
        let n = sqlx::query("UPDATE mcp_grants SET refresh_hash=? WHERE session_hash=? AND refresh_hash=?")
            .bind(super::token_hash(&refresh))
            .bind(&session)
            .bind(&presented)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        crate::error::require(n == 1, ErrorCode::Conflict)?;
        // Keep the newest previous access token so in-flight calls finish.
        sqlx::query("DELETE FROM mcp_access_tokens WHERE session_hash=? AND token_hash NOT IN (SELECT token_hash FROM mcp_access_tokens WHERE session_hash=? AND expires_at>? ORDER BY expires_at DESC LIMIT 1)")
            .bind(&session)
            .bind(&session)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO mcp_access_tokens VALUES(?,?,?)")
            .bind(super::token_hash(&access))
            .bind(&session)
            .bind(t + super::ACCESS_TTL_MS as i64)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE sessions SET expires_at=?,last_used_at=? WHERE token_hash=?")
            .bind(t + super::GRANT_TTL_MS as i64)
            .bind(t)
            .bind(&session)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok::<_, Error>(())
    }
    .await;
    drop(core);
    match rotated {
        Ok(()) => Ok(tokens(&access, &refresh, &scope)),
        Err(e) if e.0.code == ErrorCode::Conflict => {
            Err(invalid_grant("The refresh token was already used."))
        }
        Err(e) => Err(busy_box(e)),
    }
}

#[derive(Deserialize)]
struct RevokeRequest {
    token: Option<String>,
}
pub(crate) async fn revoke(State(app): State<App>, r: Request<Body>) -> Response {
    let bucket = http::auth_bucket(&app, &r).replacen("auth-ip", "mcp-token", 1);
    if let Err(e) = app.0.core.lock().await.rate(bucket, 60., 20.) {
        return rate(e);
    }
    if !form_body(r.headers()) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Use application/x-www-form-urlencoded.",
        );
    }
    let Ok(body) = to_bytes(r.into_body(), 16384).await else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Body too large.",
        );
    };
    let Ok(RevokeRequest { token: Some(token) }) = serde_urlencoded::from_bytes(&body) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required.",
        );
    };
    let h = super::token_hash(&token);
    let done = async {
        // Any refresh token of a family (even a spent one) ends its grant.
        let session: Option<Vec<u8>> = match refresh_family(&token) {
            Some(family) => {
                sqlx::query_scalar("SELECT session_hash FROM mcp_grants WHERE family_hash=?")
                    .bind(super::token_hash(family))
                    .fetch_optional(&app.0.pool)
                    .await?
            }
            None => None,
        };
        if let Some(session) = session {
            revoke_grant(&app, &session).await?;
        } else {
            sqlx::query("DELETE FROM mcp_access_tokens WHERE token_hash=?")
                .bind(&h)
                .execute(&app.0.pool)
                .await?;
        }
        Ok::<_, Error>(())
    }
    .await;
    match done {
        // RFC 7009: unknown tokens also succeed.
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => busy(e),
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    #[test]
    fn redirect_rules() {
        for ok in [
            "https://claude.ai/api/mcp/auth_callback",
            "http://localhost:3334/callback",
            "http://127.0.0.1/cb?x=1",
            "http://[::1]:9/cb",
            "cursor://anysphere.cursor-mcp/oauth/callback",
        ] {
            assert!(valid_redirect(ok), "{ok}");
        }
        for bad in [
            "http://example.com/cb",
            "https://claude.ai/cb#frag",
            "https://user:pw@claude.ai/cb",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
            "http://localhost.evil.com/cb",
            "not a url",
        ] {
            assert!(!valid_redirect(bad), "{bad}");
        }
        assert!(redirect_matches(
            "http://127.0.0.1:1000/cb",
            "http://127.0.0.1:2000/cb"
        ));
        assert!(!redirect_matches(
            "http://127.0.0.1:1000/cb",
            "http://localhost:1000/cb"
        ));
        assert!(!redirect_matches(
            "https://a.example/cb",
            "https://a.example/cb2"
        ));
        assert!(!redirect_matches(
            "https://a.example:443/cb",
            "https://a.example:444/cb"
        ));
    }
    #[test]
    fn signed_client_ids_resist_tampering() {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &[7; 32]);
        let id = encode_client(&key, "Claude", &["https://claude.ai/cb".into()]);
        let c = decode_client(&key, &id).unwrap();
        assert_eq!(c.name, "Claude");
        let (body, tag) = id.split_once('.').unwrap();
        let forged =
            URL_SAFE_NO_PAD.encode(br#"{"v":1,"n":"Claude","r":["https://evil.example/cb"]}"#);
        assert!(decode_client(&key, &format!("{forged}.{tag}")).is_none());
        let other = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &[8; 32]);
        assert!(decode_client(&other, &format!("{body}.{tag}")).is_none());
        assert_eq!(clean_name(Some("  Evil\u{202e}\n  App ")), "Evil App");
    }
}
