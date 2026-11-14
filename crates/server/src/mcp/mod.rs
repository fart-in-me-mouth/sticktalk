//! Remote MCP connector (docs/MCP.md): a Streamable HTTP MCP endpoint at `/mcp`
//! authorized by the standard MCP OAuth flow. The StickTalk server is itself the
//! public origin, so unlike T3 Code no relay or tunnel is involved.
mod oauth;
mod pages;
#[cfg(test)]
mod tests;
mod tools;
mod transport;

use crate::{error::Result, state::App};
use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

pub(crate) const SCOPE_READ: &str = "chat:read";
pub(crate) const SCOPE_WRITE: &str = "chat:write";
/// Connections a user may approve at once; they do not use the five sign-in slots.
pub(crate) const MAX_GRANTS_PER_USER: i64 = 10;
pub(crate) const ACCESS_TTL_MS: u64 = 3_600_000;
pub(crate) const GRANT_TTL_MS: u64 = 30 * 86_400_000;
pub(crate) const CODE_TTL_MS: u64 = 60_000;

pub(crate) fn routes() -> axum::Router<App> {
    let api = axum::Router::new()
        .route(
            "/mcp",
            post(transport::post)
                .get(transport::get)
                .delete(transport::delete),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::server_metadata),
        )
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .layer(axum::middleware::from_fn(cors));
    axum::Router::new()
        .merge(api)
        .route(
            "/oauth/authorize",
            get(oauth::authorize_page).post(oauth::authorize_decision),
        )
        .route("/oauth/style.css", get(pages::stylesheet))
}

/// Paths that answer in OAuth/MCP formats rather than StickTalk API errors.
pub(crate) fn is_route(path: &str) -> bool {
    path == "/mcp" || path.starts_with("/oauth/") || path.starts_with("/.well-known/")
}

/// Bearer-only endpoints never read cookies, so any origin may call them; this
/// lets browser-based MCP clients (e.g. the MCP Inspector) discover and connect.
async fn cors(r: Request<Body>, next: axum::middleware::Next) -> Response {
    let preflight = r.method() == Method::OPTIONS;
    let mut response = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(r).await
    };
    let h = response.headers_mut();
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert(
        "access-control-expose-headers",
        HeaderValue::from_static("www-authenticate, mcp-session-id, mcp-protocol-version"),
    );
    if preflight {
        h.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        h.insert(
            "access-control-allow-headers",
            HeaderValue::from_static(
                "authorization, content-type, accept, mcp-protocol-version, mcp-session-id, last-event-id",
            ),
        );
        h.insert("access-control-max-age", HeaderValue::from_static("600"));
    }
    response
}

/// Issuer (no trailing slash) from the configured public URL, never the Host header.
pub(crate) fn issuer(app: &App) -> String {
    app.0.config.public_url.trim_end_matches('/').to_owned()
}
pub(crate) fn resource(app: &App) -> String {
    format!("{}/mcp", issuer(app))
}

/// Server-wide HMAC key for stateless client IDs and consent CSRF tokens.
pub(crate) async fn secret(app: &App) -> Result<ring::hmac::Key> {
    let query = "SELECT key FROM mcp_secret WHERE singleton=1";
    let mut key: Option<Vec<u8>> = sqlx::query_scalar(query)
        .fetch_optional(&app.0.pool)
        .await?;
    if key.is_none() {
        sqlx::query("INSERT OR IGNORE INTO mcp_secret(singleton,key) VALUES(1,?)")
            .bind(crate::state::random(32))
            .execute(&app.0.pool)
            .await?;
        key = Some(sqlx::query_scalar(query).fetch_one(&app.0.pool).await?);
    }
    Ok(ring::hmac::Key::new(
        ring::hmac::HMAC_SHA256,
        &key.unwrap_or_default(),
    ))
}
pub(crate) fn sign(key: &ring::hmac::Key, purpose: &str, data: &[u8]) -> String {
    let mut ctx = ring::hmac::Context::with_key(key);
    ctx.update(purpose.as_bytes());
    ctx.update(b"\0");
    ctx.update(data);
    URL_SAFE_NO_PAD.encode(ctx.sign().as_ref())
}
pub(crate) fn verify(key: &ring::hmac::Key, purpose: &str, data: &[u8], tag: &str) -> bool {
    let Ok(tag) = URL_SAFE_NO_PAD.decode(tag) else {
        return false;
    };
    let mut msg = purpose.as_bytes().to_vec();
    msg.push(0);
    msg.extend_from_slice(data);
    ring::hmac::verify(key, &msg, &tag).is_ok()
}
/// Opaque bearer secrets carry a recognizable prefix for secret scanners.
pub(crate) fn new_token(prefix: &str) -> String {
    format!("{prefix}{}", crate::state::token())
}
pub(crate) fn token_hash(token: &str) -> Vec<u8> {
    crate::state::hash(token.as_bytes())
}
