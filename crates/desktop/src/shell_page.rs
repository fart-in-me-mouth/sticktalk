//! The built-in "connect to a server" page. It is compiled into the binary and
//! answered by the app through a WebView custom protocol: nothing here touches
//! the network, and it works when no server can be reached.
use crate::{address, address::Server, policy::Policy};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

/// Custom protocol name. WebView2 shows it as `http://<name>.localhost/`.
pub const SCHEME: &str = "sticktalk-shell";
/// The page's address as the WebView sees it.
pub const PAGE: &str = "http://sticktalk-shell.localhost/";

/// Only what this page itself loads; no inline code, no other origin.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const MAX_BODY: usize = 8192;

pub enum Route {
    Asset {
        mime: &'static str,
        body: &'static [u8],
    },
    State,
    Connect,
    NotFound,
}

pub fn route(method: &str, path: &str) -> Route {
    let asset = |mime, body| Route::Asset { mime, body };
    match (method, path) {
        ("GET", "/" | "/index.html") => asset(
            "text/html; charset=utf-8",
            include_bytes!("../assets/index.html"),
        ),
        ("GET", "/shell.css") => asset(
            "text/css; charset=utf-8",
            include_bytes!("../assets/shell.css"),
        ),
        ("GET", "/shell.js") => asset(
            "text/javascript; charset=utf-8",
            include_bytes!("../assets/shell.js"),
        ),
        // The app's own artwork and UI font (see crates/client/ASSETS.md).
        ("GET", "/icon.png") => asset(
            "image/png",
            include_bytes!("../../client/assets/sticktalk.png"),
        ),
        ("GET", "/Inter-Regular.ttf") => asset(
            "font/ttf",
            include_bytes!("../../client/assets/fonts/Inter-Regular.ttf"),
        ),
        ("GET", "/Inter-SemiBold.ttf") => asset(
            "font/ttf",
            include_bytes!("../../client/assets/fonts/Inter-SemiBold.ttf"),
        ),
        ("GET", "/state") => Route::State,
        ("POST", "/connect") => Route::Connect,
        _ => Route::NotFound,
    }
}

/// What the page shows when it opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub address: String,
    pub error: Option<String>,
}

/// Shared by the window, the WebView callbacks and the address check thread.
pub struct Shared {
    pub server: Option<Server>,
    pub policy: Policy,
    pub form: Form,
    /// The top-level address most recently allowed to load.
    pub navigating: String,
}

pub type SharedState = Arc<Mutex<Shared>>;

pub fn lock(shared: &SharedState) -> MutexGuard<'_, Shared> {
    // A panic elsewhere must not take the navigation policy down with it.
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Serialize)]
struct StateReply<'a> {
    token: &'a str,
    address: &'a str,
    error: Option<&'a str>,
    version: &'static str,
}

pub fn state_json(token: &str, form: &Form) -> Vec<u8> {
    serde_json::to_vec(&StateReply {
        token,
        address: &form.address,
        error: form.error.as_deref(),
        version: env!("CARGO_PKG_VERSION"),
    })
    .unwrap_or_default()
}

#[derive(Deserialize)]
struct ConnectRequest {
    token: String,
    address: String,
}

#[derive(Serialize)]
struct ConnectReply {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn connect_json(result: Result<&Server, &str>) -> Vec<u8> {
    serde_json::to_vec(&match result {
        Ok(server) => ConnectReply {
            ok: true,
            url: Some(server.home().into()),
            error: None,
        },
        Err(error) => ConnectReply {
            ok: false,
            url: None,
            error: Some(error.to_string()),
        },
    })
    .unwrap_or_default()
}

/// Reads a `connect` request. `token` is this run's secret, which only the
/// built-in page can learn (it is in the same-origin `state` reply): a page
/// from any other origin cannot make the app switch servers.
pub fn connect_request(body: &[u8], token: &str) -> Result<Server, &'static str> {
    const REFUSED: &str = "This page is out of date. Close StickTalk and open it again.";
    if body.len() > MAX_BODY {
        return Err(REFUSED);
    }
    let request: ConnectRequest = serde_json::from_slice(body).map_err(|_| REFUSED)?;
    if token.is_empty() || !same(request.token.as_bytes(), token.as_bytes()) {
        return Err(REFUSED);
    }
    address::parse(&request.address)
}

/// Comparison time does not depend on where the inputs differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_only_its_own_files() {
        for path in [
            "/",
            "/index.html",
            "/shell.css",
            "/shell.js",
            "/icon.png",
            "/Inter-Regular.ttf",
            "/Inter-SemiBold.ttf",
        ] {
            assert!(
                matches!(route("GET", path), Route::Asset { body, .. } if !body.is_empty()),
                "{path}"
            );
        }
        assert!(matches!(route("GET", "/state"), Route::State));
        assert!(matches!(route("POST", "/connect"), Route::Connect));
        for (method, path) in [
            ("GET", "/connect"),
            ("POST", "/state"),
            ("POST", "/"),
            ("GET", "/../Cargo.toml"),
            ("GET", "/shell.js/"),
            ("GET", "//shell.js"),
            ("GET", ""),
            ("PUT", "/connect"),
        ] {
            assert!(
                matches!(route(method, path), Route::NotFound),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn the_page_loads_nothing_inline_or_remote() {
        let Route::Asset { body, .. } = route("GET", "/") else {
            panic!("index missing")
        };
        let html = std::str::from_utf8(body).unwrap();
        // The content security policy forbids these; keep the page honest.
        for forbidden in ["src=\"http", "href=\"http", "style=", "onclick", "<script>"] {
            assert!(!html.contains(forbidden), "index.html contains {forbidden}");
        }
        assert!(html.contains("<title>StickTalk</title>"));
        assert!(PAGE.starts_with(&format!("http://{SCHEME}.")));
    }

    #[test]
    fn state_carries_the_address_error_and_token() {
        let form = Form {
            address: "https://chat.example.com".into(),
            error: Some("Couldn't find \"chat\" </script>".into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&state_json("t0ken", &form)).unwrap();
        assert_eq!(value["token"], "t0ken");
        assert_eq!(value["address"], "https://chat.example.com");
        assert_eq!(value["error"], "Couldn't find \"chat\" </script>");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        let value: serde_json::Value =
            serde_json::from_slice(&state_json("t", &Form::default())).unwrap();
        assert!(value["error"].is_null());
    }

    #[test]
    fn connect_needs_this_runs_token_and_a_valid_address() {
        let body = |token: &str, address: &str| {
            serde_json::to_vec(&serde_json::json!({ "token": token, "address": address })).unwrap()
        };
        let server = connect_request(&body("secret", " chat.example.com "), "secret").unwrap();
        assert_eq!(server.origin(), "https://chat.example.com");
        assert!(connect_request(&body("secret", "http://chat.example.com"), "secret").is_err());
        // Wrong, missing or empty token: refused before the address is looked at.
        for (sent, expected) in [
            ("guess", "secret"),
            ("", "secret"),
            ("", ""),
            ("secre", "secret"),
        ] {
            assert!(
                connect_request(&body(sent, "https://chat.example.com"), expected).is_err(),
                "{sent:?}"
            );
        }
        assert!(connect_request(b"address=https://chat.example.com", "secret").is_err());
        assert!(connect_request(br#"{"address":"https://chat.example.com"}"#, "secret").is_err());
        let huge = body("secret", &"a".repeat(MAX_BODY));
        assert!(connect_request(&huge, "secret").is_err());
    }

    #[test]
    fn connect_replies_are_what_the_page_expects() {
        let server = address::parse("http://127.0.0.1:8080").unwrap();
        let ok: serde_json::Value = serde_json::from_slice(&connect_json(Ok(&server))).unwrap();
        assert_eq!(
            ok,
            serde_json::json!({ "ok": true, "url": "http://127.0.0.1:8080/" })
        );
        let failed: serde_json::Value =
            serde_json::from_slice(&connect_json(Err("No answer."))).unwrap();
        assert_eq!(
            failed,
            serde_json::json!({ "ok": false, "error": "No answer." })
        );
    }
}
