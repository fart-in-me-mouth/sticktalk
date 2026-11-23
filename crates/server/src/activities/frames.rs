//! The frames activities run in. The web app loads each activity in an
//! `<iframe sandbox="allow-scripts">` (an opaque origin: no cookies, storage
//! or DOM of the app) and talks to it only by `postMessage` (`docs/ACTIVITIES.md`).
//!
//! - Built-in activities are served from `<web root>/public/activities/` at
//!   `/activities/...` with their own policy: scripts, styles and images from
//!   this server only, no network access at all (`connect-src 'none'`), no
//!   media, and framing only by the app itself.
//! - A registered activity loads in a small shell page served here
//!   (`/activities/frame/custom-<n>`), whose policy allows framing exactly
//!   that activity's origin. The app's own policy never lists third-party
//!   origins: they are allowed in the activity's frame only.
use super::*;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use tower::ServiceExt;

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/activities/frame/{id}", get(shell))
        .route("/activities/{*path}", get(file))
}

fn origin(app: &App) -> String {
    reqwest::Url::parse(&app.0.config.public_url)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

/// The policy of a built-in activity's files. `sandbox allow-scripts`: the
/// page runs in an opaque origin however it is opened (top-level too), not
/// only inside the app's sandboxed frame.
pub(crate) fn builtin_policy(app_origin: &str) -> String {
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'self' {o}; style-src 'self' {o}; style-src-attr 'unsafe-inline'; img-src 'self' {o} data: blob:; font-src 'self' {o}; connect-src 'none'; media-src 'none'; frame-src 'none'; worker-src 'none'; frame-ancestors {o}; base-uri 'none'; form-action 'none'",
        o = app_origin
    )
}

/// The policy of a registered activity's shell: only its origin may be
/// framed, and the shell itself is sandboxed like a built-in activity.
pub(crate) fn shell_policy(app_origin: &str, activity_origin: &str) -> String {
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'self' {o}; style-src 'self' {o}; frame-src {a}; connect-src 'none'; frame-ancestors {o}; base-uri 'none'; form-action 'none'",
        o = app_origin,
        a = activity_origin
    )
}

fn headers(r: &mut Response, policy: &str) {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(policy) {
        h.insert("content-security-policy", v);
    }
    h.insert("cache-control", HeaderValue::from_static("no-cache"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn shell(State(app): State<App>, Path(id): Path<String>) -> Response {
    let found = async {
        let mut c = app.0.pool.acquire().await?;
        catalog::custom(&mut c, &id).await
    }
    .await;
    let Ok((url, activity_origin)) = found else {
        return Error::new(ErrorCode::NotFound).into_response();
    };
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Activity</title><link rel=\"stylesheet\" href=\"/activities/shell.css\"><script src=\"/activities/shell.js\" defer></script></head><body><iframe id=\"activity\" title=\"Activity\" src=\"{}\" data-origin=\"{}\" sandbox=\"allow-scripts\" allow=\"autoplay; fullscreen\" referrerpolicy=\"no-referrer\"></iframe></body></html>",
        escape(&url),
        escape(&activity_origin)
    );
    let mut r = (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        page,
    )
        .into_response();
    headers(&mut r, &shell_policy(&origin(&app), &activity_origin));
    r
}

async fn file(State(app): State<App>, Path(path): Path<String>, r: Request<Body>) -> Response {
    let Some(root) = app.0.config.browser.web_root.clone() else {
        return Error::new(ErrorCode::NotFound).into_response();
    };
    // ServeDir resolves the path safely under the root (no `..`).
    if path.split('/').any(|p| p == ".." || p.starts_with('.')) {
        return Error::new(ErrorCode::NotFound).into_response();
    }
    let dir = root.join("public").join("activities");
    let mut request = Request::builder()
        .uri(format!("/{path}"))
        .body(Body::empty())
        .expect("request");
    if let Some(v) = r.headers().get("accept-encoding") {
        request.headers_mut().insert("accept-encoding", v.clone());
    }
    let mut response = tower_http::services::ServeDir::new(dir)
        .precompressed_br()
        .oneshot(request)
        .await
        .expect("infallible ServeDir")
        .map(Body::new);
    if response.status() == StatusCode::NOT_FOUND {
        return Error::new(ErrorCode::NotFound).into_response();
    }
    response
        .headers_mut()
        .insert("vary", HeaderValue::from_static("Accept-Encoding"));
    headers(&mut response, &builtin_policy(&origin(&app)));
    response
}
