//! The server address: what a person types, what is remembered, and the one
//! origin the window is allowed to show.
use std::path::Path;
use url::{Host, Url};

/// Packaged builds can pin a server; unpackaged ones ask on first run.
pub const DEFAULT_SERVER: &str = match option_env!("STICKTALK_DEFAULT_SERVER") {
    Some(value) => value,
    None => "",
};
/// The same file the previous desktop client used, so upgrades keep the address.
const FILE: &str = "server.txt";
const MAX_LEN: usize = 2048;

/// A validated server origin such as `https://chat.example.com`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server(Url);

impl Server {
    /// `scheme://host[:port]`, no trailing slash.
    pub fn origin(&self) -> String {
        self.0.origin().ascii_serialization()
    }
    pub fn url_origin(&self) -> url::Origin {
        self.0.origin()
    }
    /// The web app's entry page.
    pub fn home(&self) -> Url {
        self.0.clone()
    }
    pub fn health(&self) -> String {
        format!("{}/api/v1/health", self.origin())
    }
    pub fn host(&self) -> String {
        self.0.host_str().unwrap_or_default().to_string()
    }
}

/// Accepts what people actually type (`chat.example.com`, a pasted origin with
/// a trailing slash) and rejects everything that is not a bare http(s) origin.
pub fn parse(input: &str) -> Result<Server, &'static str> {
    const SHAPE: &str =
        "That doesn't look like a server address. It should look like https://chat.example.com";
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter your server's address.");
    }
    if input.len() > MAX_LEN {
        return Err(SHAPE);
    }
    let typed_scheme = input.contains("://");
    let mut url = if typed_scheme {
        Url::parse(input)
    } else {
        Url::parse(&format!("https://{input}"))
    }
    .map_err(|_| SHAPE)?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(SHAPE);
    }
    let loopback = match url.host() {
        Some(Host::Domain(name)) => name == "localhost",
        Some(Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        None => false,
    };
    if !typed_scheme && loopback {
        // A server on this PC has no certificate; don't make people type http://.
        url.set_scheme("http").map_err(|_| SHAPE)?;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Leave out the user name and password. Enter only the server's address.");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(
            "Enter only the server's address, without anything after it. For example https://chat.example.com",
        );
    }
    if url.scheme() != "https" && !loopback {
        return Err(
            "This address must start with https://. Plain http:// only works for a server on this PC.",
        );
    }
    Ok(Server(url))
}

/// The remembered address exactly as stored, or the build's default. May be
/// empty or no longer valid; `parse` decides.
pub fn saved(root: &Path) -> String {
    std::fs::read_to_string(root.join(FILE))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && text.len() < MAX_LEN)
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

pub fn save(root: &Path, server: &Server) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let temporary = root.join("server.tmp");
    std::fs::write(&temporary, server.origin())?;
    std::fs::rename(temporary, root.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(input: &str) -> String {
        parse(input)
            .unwrap_or_else(|e| panic!("{input}: {e}"))
            .origin()
    }

    #[test]
    fn accepts_origins_and_normalizes_them() {
        assert_eq!(
            origin("https://chat.example.com"),
            "https://chat.example.com"
        );
        assert_eq!(
            origin("  https://Chat.Example.com/  "),
            "https://chat.example.com"
        );
        assert_eq!(
            origin("https://chat.example.com:443"),
            "https://chat.example.com"
        );
        assert_eq!(
            origin("https://chat.example.com:8443/"),
            "https://chat.example.com:8443"
        );
        assert_eq!(origin("http://127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(origin("http://localhost:8080/"), "http://localhost:8080");
        assert_eq!(origin("http://[::1]:8080"), "http://[::1]:8080");
        assert_eq!(origin("https://127.0.0.1:8443"), "https://127.0.0.1:8443");
    }

    #[test]
    fn a_bare_host_gets_the_right_scheme() {
        assert_eq!(origin("chat.example.ts.net"), "https://chat.example.ts.net");
        assert_eq!(
            origin("chat.example.com:8443"),
            "https://chat.example.com:8443"
        );
        assert_eq!(origin("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(origin("localhost:8080"), "http://localhost:8080");
        assert_eq!(origin("[::1]:8080"), "http://[::1]:8080");
    }

    #[test]
    fn rejects_everything_that_is_not_a_bare_origin() {
        for input in [
            "",
            "   ",
            "https://",
            "ftp://chat.example.com",
            "file:///C:/Windows",
            "javascript:alert(1)",
            "sticktalk://open",
            "https://user@chat.example.com",
            "https://user:secret@chat.example.com",
            "https://chat.example.com/app",
            "https://chat.example.com/?invite=abc",
            "https://chat.example.com/#x",
            "chat.example.com/app",
            "http://chat.example.com",
            "http://192.168.1.20:8080",
            "http://127.0.0.2:8080",
            "http://localhost.example.com",
            "https://exa mple.com",
        ] {
            assert!(parse(input).is_err(), "{input:?} must be rejected");
        }
        assert!(parse(&format!("https://{}.com", "a".repeat(MAX_LEN))).is_err());
    }

    #[test]
    fn derived_urls_stay_on_the_origin() {
        let server = parse("https://chat.example.com/").unwrap();
        assert_eq!(server.home().as_str(), "https://chat.example.com/");
        assert_eq!(server.health(), "https://chat.example.com/api/v1/health");
        assert_eq!(server.host(), "chat.example.com");
    }

    #[test]
    fn remembers_the_address_in_the_old_clients_file() {
        let root =
            std::env::temp_dir().join(format!("sticktalk-desktop-address-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(saved(&root), DEFAULT_SERVER);
        // Written by the previous client: raw text, possibly with a newline.
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(FILE), "https://old.example.com\r\n").unwrap();
        assert_eq!(saved(&root), "https://old.example.com");
        save(&root, &parse("chat.example.com").unwrap()).unwrap();
        assert_eq!(saved(&root), "https://chat.example.com");
        assert_eq!(
            std::fs::read_to_string(root.join(FILE)).unwrap(),
            "https://chat.example.com"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
