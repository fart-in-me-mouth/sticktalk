//! What the window may load, what goes to the default browser, and which
//! origin gets the microphone and camera. Pure decisions; the callers act.
use crate::address::Server;
use url::{Origin, Url};

/// Where the server starts the Google sign-in round trip for a browser session.
const GOOGLE_START: &str = "/api/v1/browser/google/start";
/// Longer than any real link; keeps junk away from the shell's "open" verb.
const MAX_EXTERNAL_LEN: usize = 8192;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Load it in the window.
    Allow,
    /// Not ours: hand this exact URL to the default browser.
    External(String),
    /// Neither: drop it.
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Microphone,
    Camera,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Grant {
    Allow,
    Deny,
    /// Leave it to WebView2's own prompt.
    Ask,
}

#[derive(Debug)]
pub struct Policy {
    server: Option<Origin>,
    shell: Origin,
    /// True from the server's Google start URL until the window is back on the
    /// server (or on the built-in page). Google hosts load in-app only then.
    google: bool,
}

impl Policy {
    pub fn new(shell_page: &Url, server: Option<&Server>) -> Self {
        Self {
            server: server.map(Server::url_origin),
            shell: shell_page.origin(),
            google: false,
        }
    }

    pub fn set_server(&mut self, server: &Server) {
        self.server = Some(server.url_origin());
        self.google = false;
    }

    pub fn is_server(&self, url: &str) -> bool {
        match (&self.server, Url::parse(url)) {
            (Some(server), Ok(url)) => url.origin() == *server,
            _ => false,
        }
    }

    /// The server's app document itself (`/`, any query): the only page the
    /// window's message channel listens to. Never a frame's page, such as an
    /// activity under `/activities/`, even though it is on the server's
    /// origin: WebView2 can run the host's scripts in frames too.
    pub fn is_app_page(&self, url: &str) -> bool {
        match (&self.server, Url::parse(url)) {
            (Some(server), Ok(url)) => {
                url.origin() == *server && matches!(url.path(), "/" | "/index.html")
            }
            _ => false,
        }
    }

    pub fn is_shell(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| url.origin() == self.shell)
    }

    /// A top-level navigation of the window, including each redirect hop.
    pub fn navigate(&mut self, url: &str) -> Decision {
        if url == "about:blank" {
            return Decision::Allow;
        }
        let Ok(url) = Url::parse(url) else {
            return Decision::Block;
        };
        let origin = url.origin();
        if origin == self.shell {
            self.google = false;
            return Decision::Allow;
        }
        if self.server.as_ref() == Some(&origin) {
            self.google = url.path() == GOOGLE_START;
            return Decision::Allow;
        }
        if self.google && google_sign_in(&url) {
            return Decision::Allow;
        }
        external(url)
    }

    /// `target=_blank` and `window.open`: never a second in-app window.
    pub fn new_window(&self, url: &str) -> Decision {
        Url::parse(url).map_or(Decision::Block, external)
    }

    /// `requester` is the page asking, as reported by the WebView.
    pub fn permission(&self, requester: &str, permission: Permission) -> Grant {
        if !self.is_server(requester) {
            return Grant::Deny;
        }
        match permission {
            Permission::Microphone | Permission::Camera => Grant::Allow,
            Permission::Other => Grant::Ask,
        }
    }
}

fn external(url: Url) -> Decision {
    let web = matches!(url.scheme(), "http" | "https") && url.host().is_some();
    if (web || url.scheme() == "mailto") && url.as_str().len() <= MAX_EXTERNAL_LEN {
        Decision::External(url.into())
    } else {
        Decision::Block
    }
}

/// Google's sign-in pages, plus the sibling hosts a fresh sign-in bounces
/// through to set its cookies (`accounts.youtube.com`, `accounts.google.<cc>`).
fn google_sign_in(url: &Url) -> bool {
    if url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if matches!(host, "accounts.google.com" | "accounts.youtube.com") {
        return true;
    }
    let country = |code: &str| code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase());
    host.strip_prefix("accounts.google.").is_some_and(|rest| {
        country(rest)
            || rest
                .strip_prefix("co.")
                .or_else(|| rest.strip_prefix("com."))
                .is_some_and(country)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::parse;

    const SHELL: &str = "http://sticktalk-shell.localhost/";

    fn policy(server: Option<&str>) -> Policy {
        let server = server.map(|s| parse(s).unwrap());
        Policy::new(&Url::parse(SHELL).unwrap(), server.as_ref())
    }

    fn external(url: &str) -> Decision {
        Decision::External(url.to_string())
    }

    #[test]
    fn only_the_app_document_may_message_the_window() {
        let p = policy(Some("https://chat.example.com"));
        for url in [
            "https://chat.example.com/",
            "https://chat.example.com/?invite=abc",
            "https://chat.example.com/index.html#top",
        ] {
            assert!(p.is_app_page(url), "{url}");
        }
        for url in [
            // Activity frames and the registered-activity shell.
            "https://chat.example.com/activities/whiteboard/index.html",
            "https://chat.example.com/activities/frame/custom-1",
            "https://chat.example.com/public/activities/sdk.js",
            "https://chat.example.com/oauth/authorize",
            "https://games.example/",
            "http://chat.example.com/",
            "null",
            "about:srcdoc",
        ] {
            assert!(!p.is_app_page(url), "{url}");
        }
    }

    #[test]
    fn the_server_and_the_built_in_page_stay_in_the_window() {
        let mut p = policy(Some("https://chat.example.com"));
        for url in [
            "about:blank",
            SHELL,
            "http://sticktalk-shell.localhost/shell.css",
            "https://chat.example.com/",
            "https://chat.example.com/?channel=42",
            "https://chat.example.com:443/api/v1/attachments/1",
            // The web app's own object URLs (downloads it builds in the page).
            "blob:https://chat.example.com/0b8e7c1a",
        ] {
            assert_eq!(p.navigate(url), Decision::Allow, "{url}");
        }
    }

    #[test]
    fn other_sites_open_in_the_browser_never_in_the_window() {
        let mut p = policy(Some("https://chat.example.com"));
        for url in [
            "https://example.org/page?q=1#top",
            "http://chat.example.com/",
            "https://chat.example.com:8443/",
            "https://chat.example.com.evil.test/",
            "https://evil.test/https://chat.example.com/",
            "https://chat.example.com@evil.test/",
            "https://accounts.google.com/o/oauth2/v2/auth",
            "mailto:someone@example.org",
        ] {
            assert_eq!(p.navigate(url), external(url), "{url}");
            assert_eq!(p.new_window(url), external(url), "{url}");
        }
        // A new window is external even for the server's own pages.
        assert_eq!(
            p.new_window("https://chat.example.com/api/v1/attachments/1"),
            external("https://chat.example.com/api/v1/attachments/1")
        );
    }

    #[test]
    fn nothing_but_web_and_mail_links_leaves_the_app() {
        let mut p = policy(Some("https://chat.example.com"));
        for url in [
            "",
            "not a url",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "blob:https://example.org/1",
            "edge://settings",
            "ms-settings:privacy",
            "sticktalk://invite/abc",
            "about:srcdoc",
            "C:\\Windows\\System32\\calc.exe",
        ] {
            assert_eq!(p.navigate(url), Decision::Block, "{url}");
            assert_eq!(p.new_window(url), Decision::Block, "{url}");
        }
        let long = format!("https://example.org/{}", "a".repeat(MAX_EXTERNAL_LEN));
        assert_eq!(p.navigate(&long), Decision::Block);
    }

    #[test]
    fn without_a_server_only_the_built_in_page_loads() {
        let mut p = policy(None);
        assert_eq!(p.navigate(SHELL), Decision::Allow);
        assert_eq!(
            p.navigate("https://chat.example.com/"),
            external("https://chat.example.com/")
        );
        p.set_server(&parse("https://chat.example.com").unwrap());
        assert_eq!(p.navigate("https://chat.example.com/"), Decision::Allow);
    }

    #[test]
    fn google_sign_in_completes_in_the_window_only_when_the_server_starts_it() {
        let mut p = policy(Some("https://chat.example.com"));
        let google = "https://accounts.google.com/o/oauth2/v2/auth?client_id=x";
        assert_eq!(p.navigate(google), external(google));

        let start = "https://chat.example.com/api/v1/browser/google/start?operation=login";
        assert_eq!(p.navigate(start), Decision::Allow);
        for url in [
            google,
            "https://accounts.google.com/v3/signin/identifier",
            "https://accounts.youtube.com/accounts/SetSID?ssdc=1",
            "https://accounts.google.de/accounts/SetSID",
            "https://accounts.google.co.uk/accounts/SetSID",
            "https://accounts.google.com.au/accounts/SetSID",
        ] {
            assert_eq!(p.navigate(url), Decision::Allow, "{url}");
        }
        // Still only Google's sign-in hosts, and only over HTTPS.
        for url in [
            "http://accounts.google.com/",
            "https://accounts.google.com:8443/",
            "https://accounts.google.com.evil.test/",
            "https://accounts.google.site/",
            "https://accounts.google.co.evil/",
            "https://evil-accounts.google.com.test/",
            "https://support.google.com/accounts",
            "https://www.google.com/",
        ] {
            assert_eq!(p.navigate(url), external(url), "{url}");
        }
        // Back on the server: the round trip is over.
        let callback = "https://chat.example.com/api/v1/browser/google/callback?state=s&code=c";
        assert_eq!(p.navigate(callback), Decision::Allow);
        assert_eq!(p.navigate(google), external(google));
    }

    #[test]
    fn leaving_for_the_built_in_page_or_another_server_ends_the_google_round_trip() {
        let google = "https://accounts.google.com/signin";
        let start = "https://chat.example.com/api/v1/browser/google/start";
        let mut p = policy(Some("https://chat.example.com"));
        assert_eq!(p.navigate(start), Decision::Allow);
        assert_eq!(p.navigate(SHELL), Decision::Allow);
        assert_eq!(p.navigate(google), external(google));

        assert_eq!(p.navigate(start), Decision::Allow);
        p.set_server(&parse("https://other.example.com").unwrap());
        assert_eq!(p.navigate(google), external(google));
        assert_eq!(p.navigate(start), external(start));
    }

    #[test]
    fn only_the_server_gets_the_microphone_and_camera() {
        let p = policy(Some("https://chat.example.com"));
        let server = "https://chat.example.com/";
        assert_eq!(p.permission(server, Permission::Microphone), Grant::Allow);
        assert_eq!(p.permission(server, Permission::Camera), Grant::Allow);
        assert_eq!(p.permission(server, Permission::Other), Grant::Ask);
        for requester in [
            "https://accounts.google.com/",
            "https://chat.example.com.evil.test/",
            "http://chat.example.com/",
            SHELL,
            "about:blank",
            "",
        ] {
            for permission in [
                Permission::Microphone,
                Permission::Camera,
                Permission::Other,
            ] {
                assert_eq!(
                    p.permission(requester, permission),
                    Grant::Deny,
                    "{requester}"
                );
            }
        }
        let none = policy(None);
        assert_eq!(none.permission(server, Permission::Microphone), Grant::Deny);
    }
}
