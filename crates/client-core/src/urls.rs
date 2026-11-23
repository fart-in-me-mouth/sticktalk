//! The parts of a URL that StickTalk checks (server address, message links,
//! GIF media). Both implementations follow the WHATWG URL standard: the
//! browser's own parser on the web (so the web app carries no URL or IDNA
//! tables of its own, docs/features/perf.md), the `url` crate elsewhere.

/// A parsed absolute URL, reduced to what the checks look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Lowercase, without the colon: `https`.
    pub scheme: String,
    /// Lowercase and ASCII (IDNA applied); IPv6 in brackets; no port.
    pub host: String,
    pub port: Option<u16>,
    pub credentials: bool,
    /// A `?` or `#` is present, even an empty one.
    pub query: bool,
    pub fragment: bool,
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn parse(s: &str) -> Option<Parsed> {
    let u = web_sys::Url::new(s).ok()?;
    let scheme = u.protocol().trim_end_matches(':').to_owned();
    Some(Parsed {
        port: match u.port().as_str() {
            "" => None,
            p => Some(p.parse().ok()?),
        },
        credentials: !u.username().is_empty() || !u.password().is_empty(),
        query: !u.search().is_empty() || before_fragment(&u.href()).contains('?'),
        fragment: !u.hash().is_empty() || u.href().contains('#'),
        host: u.hostname(),
        scheme,
    })
}
/// The href without its fragment.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
fn before_fragment(href: &str) -> &str {
    href.split_once('#').map_or(href, |(head, _)| head)
}

#[cfg(not(all(target_arch = "wasm32", feature = "web")))]
pub fn parse(s: &str) -> Option<Parsed> {
    let u = url::Url::parse(s).ok()?;
    Some(Parsed {
        scheme: u.scheme().to_owned(),
        host: u.host_str().unwrap_or_default().to_owned(),
        port: u.port(),
        credentials: !u.username().is_empty() || u.password().is_some(),
        query: u.query().is_some(),
        fragment: u.fragment().is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_parts_the_checks_use() {
        let p = parse("HTTPS://User:pw@Example.COM:8443/a?b#c").unwrap();
        assert_eq!(p.scheme, "https");
        assert_eq!(p.host, "example.com");
        assert_eq!(p.port, Some(8443));
        assert!(p.credentials && p.query && p.fragment);
        let p = parse("http://[::1]/").unwrap();
        assert_eq!(p.host, "[::1]");
        assert!(!p.credentials && !p.query && !p.fragment && p.port.is_none());
        assert!(parse("https://a/?").unwrap().query);
        assert!(parse("https://a/#").unwrap().fragment);
        assert_eq!(
            parse("https://bücher.example/").unwrap().host,
            "xn--bcher-kva.example"
        );
        assert!(parse("not a url").is_none());
        assert!(parse("https://exa mple.com/").is_none());
    }
}
