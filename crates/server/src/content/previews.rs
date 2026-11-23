//! Link previews. After a message is sent (or edited), the server fetches each
//! link's page and builds a preview card from its Open Graph / Twitter tags.
//!
//! These are requests to URLs that anyone can choose, so the fetcher is strict:
//! - http and https only, no credentials in the URL, at most 4 redirects;
//! - every hop's host is resolved by our own resolver, and the request fails
//!   unless every address is public (no loopback, private, link-local, CGNAT,
//!   multicast, documentation or reserved ranges, IPv4 or IPv6). The client
//!   connects only to the addresses that were checked, so a second DNS answer
//!   cannot redirect it (no rebinding); IP-literal hosts are checked directly;
//! - short timeouts, a response size cap, content-type checks, no proxies;
//! - at most 4 fetches at once and 64 waiting, a bounded cache, and a
//!   per-author rate limit.
//!
//! The preview image is downloaded once, re-encoded small and stored with the
//! message; viewers load it from this server, never from the linked site.
use super::*;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};

const MAX_PAGE_BYTES: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_STORED_IMAGE_BYTES: usize = 256 * 1024;
const MAX_REDIRECTS: usize = 4;
const CONCURRENT_FETCHES: usize = 4;
const MAX_WAITING: usize = 64;
const CACHE_ENTRIES: usize = 256;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_TTL_MS: u64 = 30 * 60_000;

pub(crate) type Lookup = Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send>>;
/// Name resolution, injectable so tests never touch real DNS.
pub(crate) trait HostResolver: Send + Sync {
    fn lookup(&self, host: String) -> Lookup;
}
struct SystemResolver;
impl HostResolver for SystemResolver {
    fn lookup(&self, host: String) -> Lookup {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .map(|a| a.ip())
                .collect())
        })
    }
}
/// Unit tests resolve nothing unless a test installs its own resolver.
#[cfg(test)]
struct NoResolver;
#[cfg(test)]
impl HostResolver for NoResolver {
    fn lookup(&self, _: String) -> Lookup {
        Box::pin(async { Err(std::io::Error::other("no DNS in tests")) })
    }
}

#[derive(Clone)]
pub(crate) struct Net {
    pub resolver: Arc<dyn HostResolver>,
    pub allow_ip: fn(IpAddr) -> bool,
    pub allow_name: fn(&str) -> bool,
}
impl Default for Net {
    fn default() -> Self {
        #[cfg(test)]
        let resolver: Arc<dyn HostResolver> = Arc::new(NoResolver);
        #[cfg(not(test))]
        let resolver: Arc<dyn HostResolver> = Arc::new(SystemResolver);
        let _ = SystemResolver;
        Self {
            resolver,
            allow_ip: public_ip,
            allow_name: public_name,
        }
    }
}

/// Only globally routable unicast addresses.
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => public_v4(v),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v.segments();
            // NAT64 and 6to4 embed an IPv4 address; check that one.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                let o = v.octets();
                return public_v4(std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            if s[0] == 0x2002 {
                let o = v.octets();
                return public_v4(std::net::Ipv4Addr::new(o[2], o[3], o[4], o[5]));
            }
            // Global unicast is 2000::/3, minus documentation, Teredo, ORCHID
            // and benchmarking blocks.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] == 0x0db8)
                && !(s[0] == 0x2001 && s[1] == 0)
                && !(s[0] == 0x2001 && (0x10..=0x2f).contains(&s[1]))
                && !(s[0] == 0x2001 && s[1] == 0x0002 && s[2] == 0)
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}
fn public_v4(v: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = v.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..128).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (18..20).contains(&b))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}
/// Host names that never name a public site (no DNS query is made for them).
pub(crate) fn public_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    const RESERVED: &[&str] = &[
        "localhost",
        "local",
        "invalid",
        "test",
        "example",
        "internal",
        "intranet",
        "lan",
        "home",
        "corp",
        "onion",
        "home.arpa",
        "in-addr.arpa",
        "ip6.arpa",
    ];
    host.contains('.')
        && host.len() <= 253
        && !RESERVED
            .iter()
            .any(|r| host == *r || host.ends_with(&format!(".{r}")))
}

/// Connects only to addresses this resolver checked.
struct Guard(Net);
impl reqwest::dns::Resolve for Guard {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let net = self.0.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            if !(net.allow_name)(&host) {
                return Err("host is not public".into());
            }
            let ips = net.resolver.lookup(host).await?;
            // Any private answer means the whole name is refused.
            if ips.is_empty() || ips.iter().any(|ip| !(net.allow_ip)(*ip)) {
                return Err("host is not public".into());
            }
            let addrs: Vec<SocketAddr> = ips.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect();
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Page {
    pub site_name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub video: bool,
    pub image: Option<Image>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Image {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub width: u16,
    pub height: u16,
}

struct Bucket {
    credit: f64,
    at: u64,
}

pub(crate) struct Previews {
    enabled: bool,
    net: RwLock<(Net, Option<reqwest::Client>)>,
    fetches: Arc<tokio::sync::Semaphore>,
    waiting: AtomicUsize,
    in_flight: Mutex<HashSet<Id>>,
    cache: Mutex<HashMap<String, (u64, Option<Arc<Page>>)>>,
    limits: Mutex<HashMap<Id, Bucket>>,
}
impl Previews {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            net: RwLock::new((Net::default(), None)),
            fetches: Arc::new(tokio::sync::Semaphore::new(CONCURRENT_FETCHES)),
            waiting: AtomicUsize::new(0),
            in_flight: Mutex::new(HashSet::new()),
            cache: Mutex::new(HashMap::new()),
            limits: Mutex::new(HashMap::new()),
        }
    }
    /// `--mock-link-preview-host`: one name, on loopback, and no real DNS.
    pub(crate) fn mock(&self, host: String) {
        struct Mock(String);
        impl HostResolver for Mock {
            fn lookup(&self, host: String) -> Lookup {
                let ok = host.eq_ignore_ascii_case(&self.0);
                Box::pin(async move {
                    if ok {
                        Ok(vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)])
                    } else {
                        Err(std::io::Error::other("only the mock host resolves"))
                    }
                })
            }
        }
        *self.net.write().unwrap() = (
            Net {
                resolver: Arc::new(Mock(host)),
                allow_ip: |ip| ip == IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allow_name: public_name,
            },
            None,
        );
    }
    #[cfg(test)]
    pub(crate) fn set_net(&self, net: Net) {
        *self.net.write().unwrap() = (net, None);
        self.cache.lock().unwrap().clear();
    }
    /// The shared client, built on first use: idle servers hold no sockets.
    fn client(&self) -> Result<reqwest::Client> {
        if let Some(c) = &self.net.read().unwrap().1 {
            return Ok(c.clone());
        }
        let mut net = self.net.write().unwrap();
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(Guard(net.0.clone())))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(0)
            .user_agent(concat!(
                "Mozilla/5.0 (compatible; StickTalkBot/",
                env!("CARGO_PKG_VERSION"),
                "; link preview)"
            ))
            .build()
            .map_err(|_| Error::new(ErrorCode::Internal))?;
        net.1 = Some(client.clone());
        Ok(client)
    }
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
    /// Ten previews a minute per author, bursting to five.
    pub(crate) fn allow(&self, u: Id) -> bool {
        let t = now();
        let mut limits = self.limits.lock().unwrap();
        if limits.len() >= 4096 {
            limits.retain(|_, b| t.saturating_sub(b.at) < 120_000);
            if limits.len() >= 4096 {
                return false;
            }
        }
        let b = limits.entry(u).or_insert(Bucket { credit: 5., at: t });
        b.credit = (b.credit + t.saturating_sub(b.at) as f64 * 10. / 60_000.).min(5.);
        b.at = t;
        if b.credit < 1. {
            return false;
        }
        b.credit -= 1.;
        true
    }
    fn cached(&self, url: &str) -> Option<Option<Arc<Page>>> {
        let cache = self.cache.lock().unwrap();
        cache
            .get(url)
            .filter(|(at, _)| now().saturating_sub(*at) < CACHE_TTL_MS)
            .map(|(_, v)| v.clone())
    }
    fn remember(&self, url: String, page: Option<Arc<Page>>) {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(url, (now(), page));
        let size = |c: &HashMap<String, (u64, Option<Arc<Page>>)>| -> usize {
            c.values()
                .map(|(_, p)| {
                    p.as_ref()
                        .and_then(|p| p.image.as_ref())
                        .map_or(0, |i| i.bytes.len())
                })
                .sum()
        };
        while cache.len() > CACHE_ENTRIES || size(&cache) > CACHE_BYTES {
            let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            cache.remove(&oldest);
        }
    }

    /// One page's preview (cached), or None when it has nothing to show.
    pub(crate) async fn page(&self, url: &str) -> Option<Arc<Page>> {
        if let Some(hit) = self.cached(url) {
            return hit;
        }
        let page = self.fetch_page(url).await.ok().flatten().map(Arc::new);
        self.remember(url.to_owned(), page.clone());
        page
    }

    async fn fetch_page(&self, url: &str) -> Result<Option<Page>> {
        let (final_url, body) = self
            .get(
                url,
                &["text/html", "application/xhtml+xml"],
                MAX_PAGE_BYTES,
                true,
            )
            .await?;
        let html = String::from_utf8_lossy(&body);
        let meta = parse_html(&html);
        let mut page = Page {
            site_name: meta.get("og:site_name").map(|s| plain(s, 100)),
            title: meta
                .get("og:title")
                .or_else(|| meta.get("twitter:title"))
                .or_else(|| meta.get("<title>"))
                .map(|s| plain(s, 256)),
            description: meta
                .get("og:description")
                .or_else(|| meta.get("twitter:description"))
                .or_else(|| meta.get("description"))
                .map(|s| plain(s, 350)),
            video: meta.contains_key("og:video")
                || meta.contains_key("og:video:url")
                || meta.contains_key("og:video:secure_url")
                || meta.get("og:type").is_some_and(|t| t.starts_with("video"))
                || meta.get("twitter:card").is_some_and(|t| t == "player"),
            image: None,
        };
        page.site_name = page.site_name.filter(|s| !s.is_empty());
        page.title = page.title.filter(|s| !s.is_empty());
        page.description = page.description.filter(|s| !s.is_empty());
        let image = [
            "og:image:secure_url",
            "og:image:url",
            "og:image",
            "twitter:image",
            "twitter:image:src",
        ]
        .iter()
        .find_map(|k| meta.get(*k))
        .and_then(|src| final_url.join(src.trim()).ok());
        if let Some(image) = image {
            if let Ok((_, bytes)) = self
                .get(
                    image.as_str(),
                    &["image/png", "image/jpeg", "image/gif", "image/webp"],
                    MAX_IMAGE_BYTES,
                    false,
                )
                .await
            {
                page.image = tokio::task::spawn_blocking(move || shrink(&bytes))
                    .await
                    .ok()
                    .flatten();
            }
        }
        Ok(
            (page.title.is_some() || page.description.is_some() || page.image.is_some())
                .then_some(page),
        )
    }

    /// GET with manual, re-checked redirects. Returns the final URL and body.
    async fn get(
        &self,
        url: &str,
        types: &[&str],
        cap: usize,
        stop_at_head: bool,
    ) -> Result<(reqwest::Url, Vec<u8>)> {
        let bad = || Error::new(ErrorCode::InvalidRequest);
        let client = self.client()?;
        let net = self.net.read().unwrap().0.clone();
        let mut url = reqwest::Url::parse(url).map_err(|_| bad())?;
        for _ in 0..=MAX_REDIRECTS {
            check_url(&url, &net)?;
            let response = client
                .get(url.clone())
                .header("accept", types.join(","))
                .send()
                .await
                .map_err(|_| Error::new(ErrorCode::Unavailable))?;
            if response.status().is_redirection() {
                let next = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .ok_or_else(bad)?;
                url = next;
                continue;
            }
            require(response.status() == 200, ErrorCode::NotFound)?;
            let kind = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            require(
                types.contains(&kind.as_str()),
                ErrorCode::UnsupportedMediaType,
            )?;
            if let Some(len) = response.content_length().filter(|_| !stop_at_head) {
                require(len as usize <= cap, ErrorCode::PayloadTooLarge)?;
            }
            let mut response = response;
            let mut body = vec![];
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| Error::new(ErrorCode::Unavailable))?
            {
                if body.len() + chunk.len() > cap {
                    // A page's head is enough; an oversized image is refused.
                    require(stop_at_head, ErrorCode::PayloadTooLarge)?;
                    body.extend_from_slice(&chunk[..cap - body.len()]);
                    break;
                }
                body.extend_from_slice(&chunk);
                if stop_at_head
                    && find_ci(
                        &String::from_utf8_lossy(
                            &body[body.len().saturating_sub(chunk.len() + 8)..],
                        ),
                        "</head",
                    )
                    .is_some()
                {
                    break;
                }
            }
            return Ok((url, body));
        }
        Err(bad())
    }
}

fn check_url(url: &reqwest::Url, net: &Net) -> Result<()> {
    let bad = || Error::new(ErrorCode::InvalidRequest);
    require(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        ErrorCode::InvalidRequest,
    )?;
    let host = url.host_str().ok_or_else(bad)?;
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    {
        Ok(ip) => require((net.allow_ip)(ip), ErrorCode::Forbidden),
        Err(_) => require((net.allow_name)(host), ErrorCode::Forbidden),
    }
}

/// Decoded once, resized to fit 480 px and re-encoded (PNG if transparent,
/// otherwise JPEG), so viewers get a small, sanitized image.
fn shrink(bytes: &[u8]) -> Option<Image> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    require(
        matches!(
            reader.format(),
            Some(
                image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::Gif
                    | image::ImageFormat::WebP
            )
        ),
        ErrorCode::UnsupportedMediaType,
    )
    .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let img = img.thumbnail(480, 480);
    let (width, height) = (img.width() as u16, img.height() as u16);
    let mut out = vec![];
    let content_type = if img.color().has_alpha() {
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                img.to_rgba8().as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
        "image/png"
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
            .encode_image(&img.to_rgb8())
            .ok()?;
        "image/jpeg"
    };
    (out.len() <= MAX_STORED_IMAGE_BYTES && width > 0 && height > 0).then_some(Image {
        bytes: out,
        content_type,
        width,
        height,
    })
}
use image::ImageEncoder as _;

/// Plain text: entities decoded, whitespace collapsed, no control characters.
fn plain(s: &str, max: usize) -> String {
    let decoded = entities(s);
    let mut out = String::new();
    for word in decoded.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(word.chars().filter(|c| !c.is_control()));
    }
    if out.chars().count() > max {
        out = out.chars().take(max - 1).collect::<String>() + "…";
    }
    out
}
fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let name = &tail[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase().find(needle)
}

/// `<meta property|name=... content=...>` values (first one wins) and the
/// `<title>` (as "<title>"), from the document head only.
pub(crate) fn parse_html(html: &str) -> HashMap<String, String> {
    let lower = html.to_ascii_lowercase();
    let end = lower.find("</head").unwrap_or(lower.len());
    let mut out = HashMap::new();
    let mut at = 0;
    while let Some(open) = lower[at..end].find('<') {
        let start = at + open + 1;
        let close = lower[start..].find('>').map_or(lower.len(), |c| start + c);
        let tag = &html[start..close.min(html.len())];
        let lower_tag = &lower[start..close.min(lower.len())];
        if lower_tag.starts_with("title") {
            let body_start = close + 1;
            if let Some(stop) = lower.get(body_start..).and_then(|s| s.find("</title")) {
                out.entry("<title>".into())
                    .or_insert_with(|| html[body_start..body_start + stop].to_owned());
            }
        } else if lower_tag.starts_with("meta") {
            let attrs = attributes(&tag[4..]);
            let key = attrs
                .get("property")
                .or_else(|| attrs.get("name"))
                .map(|k| k.to_ascii_lowercase());
            if let (Some(key), Some(value)) = (key, attrs.get("content")) {
                out.entry(key).or_insert_with(|| value.clone());
            }
        }
        if out.len() > 64 {
            break;
        }
        at = close.min(end);
        if at >= end {
            break;
        }
    }
    out
}
fn attributes(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut s = tag.trim_start_matches('/');
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let Some(eq) = s.find('=') else { break };
        let name = s[..eq].trim().to_ascii_lowercase();
        if name.is_empty() || name.contains(char::is_whitespace) {
            // A bare attribute before this one: skip a word.
            let Some(space) = s.find(char::is_whitespace) else {
                break;
            };
            s = &s[space..];
            continue;
        }
        let rest = s[eq + 1..].trim_start();
        let (value, next) = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => match rest[1..].find(q) {
                Some(e) => (&rest[1..1 + e], &rest[e + 2..]),
                None => (&rest[1..], ""),
            },
            _ => {
                let e = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..e], &rest[e..])
            }
        };
        out.entry(name).or_insert_with(|| value.to_owned());
        s = next;
    }
    out
}

/// The preview cards stored for a message.
pub(super) async fn load(c: &mut SqliteConnection, ch: Id, m: Id) -> Result<Vec<LinkEmbed>> {
    Ok(sqlx::query("SELECT idx,url,site_name,title,description,video,image_width,image_height,image IS NOT NULL AS has_image FROM link_embeds WHERE message_id=? ORDER BY idx")
        .bind(id(m)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(|r| LinkEmbed {
            url: r.get("url"),
            site_name: r.get("site_name"),
            title: r.get("title"),
            description: r.get("description"),
            video: r.get::<i64, _>("video") != 0,
            image: (r.get::<i64, _>("has_image") != 0).then(|| EmbedImage {
                url: format!(
                    "/api/v1/channels/{}/messages/{}/embeds/{}/image",
                    ch.0,
                    m.0,
                    r.get::<i64, _>("idx")
                ),
                width: r.get::<Option<i64>, _>("image_width").unwrap_or(0) as u16,
                height: r.get::<Option<i64>, _>("image_height").unwrap_or(0) as u16,
            }),
        })
        .collect())
}

/// Starts (in the background) building previews for a sent or edited message.
pub(super) fn queue(app: &App, message: &Message) {
    let previews = &app.0.content.previews;
    if !previews.enabled || message.extra.forwarded.is_some() || message.extra.gif.is_some() {
        return;
    }
    // GIF provider media is shown inline, not as a page preview.
    let links: Vec<String> = preview_links(&message.content)
        .into_iter()
        .filter(|l| !gif_media(l))
        .collect();
    let current: Vec<&str> = message
        .extra
        .embeds
        .iter()
        .map(|e| e.url.as_str())
        .collect();
    if links.iter().map(String::as_str).eq(current.iter().copied()) {
        return;
    }
    // An edit is delivered once per viewer: only the first starts work.
    if previews.waiting.load(Ordering::Relaxed) >= MAX_WAITING
        || !previews.in_flight.lock().unwrap().insert(message.id)
    {
        return;
    }
    if !links.is_empty() && !previews.allow(message.author_id) {
        previews.in_flight.lock().unwrap().remove(&message.id);
        return;
    }
    previews.waiting.fetch_add(1, Ordering::Relaxed);
    let app = app.clone();
    let (ch, m) = (message.channel_id, message.id);
    tokio::spawn(async move {
        let previews = &app.0.content.previews;
        let permit = previews.fetches.clone().acquire_owned().await;
        let mut pages = vec![];
        if permit.is_ok() {
            for url in &links {
                tokio::select! {
                    _ = app.0.shutdown.cancelled() => break,
                    page = previews.page(url) => pages.push((url.clone(), page)),
                }
            }
        }
        drop(permit);
        let _ = store(&app, ch, m, &links, pages).await;
        previews.in_flight.lock().unwrap().remove(&m);
        previews.waiting.fetch_sub(1, Ordering::Relaxed);
    });
}

/// Replaces a message's cards, if it still has these links, and tells everyone.
async fn store(
    app: &App,
    ch: Id,
    m: Id,
    links: &[String],
    pages: Vec<(String, Option<Arc<Page>>)>,
) -> Result<()> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let Some(r) =
        sqlx::query("SELECT content,deleted_at FROM messages WHERE id=? AND channel_id=?")
            .bind(id(m)?)
            .bind(id(ch)?)
            .fetch_optional(&mut *c)
            .await?
    else {
        return Ok(());
    };
    let suppressed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM link_embeds_removed WHERE message_id=?)")
            .bind(id(m)?)
            .fetch_one(&mut *c)
            .await?;
    // Edited again meanwhile, deleted, or previews removed: leave it.
    if r.get::<Option<i64>, _>("deleted_at").is_some()
        || suppressed
        || preview_links(&r.get::<String, _>("content"))
            .into_iter()
            .filter(|l| !gif_media(l))
            .ne(links.iter().cloned())
    {
        return Ok(());
    }
    let before = load(&mut c, ch, m).await?;
    sqlx::query("DELETE FROM link_embeds WHERE message_id=?")
        .bind(id(m)?)
        .execute(&mut *c)
        .await?;
    for (i, (url, page)) in pages
        .iter()
        .filter_map(|(u, p)| p.as_ref().map(|p| (u, p)))
        .enumerate()
    {
        sqlx::query("INSERT INTO link_embeds(message_id,idx,url,site_name,title,description,video,image,image_type,image_width,image_height) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(id(m)?)
            .bind(i as i64)
            .bind(url)
            .bind(&page.site_name)
            .bind(&page.title)
            .bind(&page.description)
            .bind(page.video)
            .bind(page.image.as_ref().map(|i| i.bytes.as_slice()))
            .bind(page.image.as_ref().map(|i| i.content_type))
            .bind(page.image.as_ref().map(|i| i64::from(i.width)))
            .bind(page.image.as_ref().map(|i| i64::from(i.height)))
            .execute(&mut *c)
            .await?;
    }
    let embeds = load(&mut c, ch, m).await?;
    if embeds != before {
        let out = vec![(
            audience(&mut c, ch).await?,
            Event::Content(ContentEvent::EmbedsUpdated {
                channel_id: ch,
                message_id: m,
                embeds,
            }),
        )];
        deliver(app, &mut core, &mut c, out).await?;
    }
    Ok(())
}

pub(super) async fn remove(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    let channel = text(c, u, ch, Permissions::VIEW_CHANNEL).await?;
    let old = message(c, u, ch, m, false).await?;
    require(
        old.author_id == u || moderator(c, u, &channel).await?,
        ErrorCode::Forbidden,
    )?;
    sqlx::query("INSERT OR IGNORE INTO link_embeds_removed(message_id) VALUES(?)")
        .bind(id(m)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM link_embeds WHERE message_id=?")
        .bind(id(m)?)
        .execute(&mut *c)
        .await?;
    if !old.extra.embeds.is_empty() {
        out.push((
            audience(c, ch).await?,
            Event::Content(ContentEvent::EmbedsUpdated {
                channel_id: ch,
                message_id: m,
                embeds: vec![],
            }),
        ));
    }
    Ok(ContentResult::Ack)
}

/// The stored preview image, for someone who may read the message.
pub(super) async fn image(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    idx: u8,
) -> Result<(String, Vec<u8>)> {
    text(c, u, ch, Permissions::READ_HISTORY).await?;
    let r = sqlx::query("SELECT e.image,e.image_type FROM link_embeds e JOIN messages m ON m.id=e.message_id WHERE e.message_id=? AND m.channel_id=? AND m.deleted_at IS NULL AND e.idx=? AND e.image IS NOT NULL")
        .bind(id(m)?)
        .bind(id(ch)?)
        .bind(i64::from(idx))
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((r.get("image_type"), r.get("image")))
}
