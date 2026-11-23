//! How images served under `/api` are cached by the browser.
//!
//! Every `/api` response is `no-store` unless its handler chose a policy
//! (`browser::boundary`). The images below choose one of two:
//!
//! - [`Policy::Checked`]: the server decides per request who may see it
//!   (custom emoji, stickers, attachments and their thumbnails, link preview
//!   images, event covers). `private, no-cache` with a strong validator: the
//!   browser keeps the bytes and asks before every use (once per page; Blink
//!   reuses an image it validated in the same document), the server runs the
//!   permission check and answers `304 Not Modified` without the body. Someone
//!   who may no longer see it gets 404 instead, and so does another account
//!   signed in to the same browser, which shares its HTTP cache.
//! - [`Policy::Immutable`]: art any signed-in account may see whose URL never
//!   points at other bytes (badge icons). Cached for a year, never asked again.
//!
//! The ETag names the asset and its version, never its owner or its viewer.
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Policy {
    Checked,
    Immutable,
}
impl Policy {
    pub(crate) fn header(self) -> &'static str {
        match self {
            Policy::Checked => "private, no-cache",
            Policy::Immutable => "private, max-age=31536000, immutable",
        }
    }
}

/// A strong validator for an immutable asset: `"<kind>-<id>-<version>"`.
pub(crate) fn etag(kind: &str, id: u64, version: impl std::fmt::Display) -> String {
    format!("\"{kind}-{id}-{version}\"")
}

/// The request already holds this version (`If-None-Match`).
pub(crate) fn fresh(h: &HeaderMap, etag: &str) -> bool {
    h.get_all("if-none-match").iter().any(|v| {
        v.to_str().is_ok_and(|v| {
            v.split(',')
                .any(|v| matches!(v.trim(), t if t == etag || t == "*"))
        })
    })
}

/// `304 Not Modified` for a request whose copy is current. Call only after
/// the permission check.
pub(crate) fn not_modified(policy: Policy, etag: &str) -> Response {
    let mut r = StatusCode::NOT_MODIFIED.into_response();
    insert(&mut r, policy, etag);
    r
}

/// Image bytes this server checked: never sniffed or run, cached per `policy`.
pub(crate) fn image(policy: Policy, etag: &str, kind: &str, bytes: Vec<u8>) -> Response {
    let mut r = Response::new(Body::from(bytes));
    let h = r.headers_mut();
    if let Ok(v) = kind.parse() {
        h.insert("content-type", v);
    }
    h.insert("x-content-type-options", "nosniff".parse().expect("static"));
    h.insert(
        "content-security-policy",
        "default-src 'none'; sandbox".parse().expect("static"),
    );
    insert(&mut r, policy, etag);
    r
}

fn insert(r: &mut Response, policy: Policy, etag: &str) {
    let h = r.headers_mut();
    h.insert("cache-control", policy.header().parse().expect("static"));
    if let Ok(v) = etag.parse() {
        h.insert("etag", v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validators_match_exactly_and_in_lists() {
        let tag = etag("emoji", 7, 0);
        assert_eq!(tag, "\"emoji-7-0\"");
        let mut h = HeaderMap::new();
        assert!(!fresh(&h, &tag));
        h.insert("if-none-match", "\"emoji-70-0\"".parse().unwrap());
        assert!(!fresh(&h, &tag));
        h.insert("if-none-match", "\"x\", \"emoji-7-0\"".parse().unwrap());
        assert!(fresh(&h, &tag));
        let r = not_modified(Policy::Checked, &tag);
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(r.headers()["cache-control"], "private, no-cache");
        assert_eq!(r.headers()["etag"], tag.as_str());
    }
}
