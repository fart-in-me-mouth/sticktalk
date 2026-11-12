//! Bounded discovery metadata proxy. Media always loads from the provider CDN.
//! KLIPY proxy mode requires prior approval; direct mode never calls its API here.
use crate::{
    db,
    error::{require, Error, Result},
    http::{self, ApiResponse},
    state::App,
};
use axum::{body::Body, extract::Query, http::Request};
use clap::{Args, ValueEnum};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};
use sticktalk_proto::*;
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(600);
const CACHE_ENTRIES: usize = 128;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const RESPONSE_BYTES: usize = 1024 * 1024;
const MEDIA_FILTER: &str = "nanowebm,nanomp4,tinywebm,tinymp4,mediumgif";

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ProviderOption {
    Klipy,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum GifMode {
    #[default]
    Direct,
    Proxy,
}
#[derive(Clone, Default, Args)]
pub struct GifOptions {
    #[arg(long, env = "STICKTALK_GIF_PROVIDER", value_enum)]
    pub gif_provider: Option<ProviderOption>,
    #[arg(long, env = "STICKTALK_GIF_MODE", value_enum, default_value = "direct")]
    pub gif_mode: GifMode,
    /// Server-only key. Never used as the direct public key.
    #[arg(long, env = "STICKTALK_GIF_API_KEY", hide_env_values = true)]
    pub gif_api_key: Option<String>,
    /// Deliberately public, separate client-distributed key; returned by /meta.
    #[arg(long, env = "STICKTALK_GIF_PUBLIC_CLIENT_KEY", hide_env_values = true)]
    pub gif_public_client_key: Option<String>,
    /// Operator attests prior provider approval for API proxy + metadata caching.
    #[arg(long, env = "STICKTALK_GIF_PROXY_APPROVED")]
    pub gif_proxy_approved: bool,
    /// Loopback-only integration-test provider, never inferred from user input.
    #[arg(long, env = "STICKTALK_MOCK_GIF_BASE")]
    pub mock_gif_base: Option<String>,
}
impl std::fmt::Debug for GifOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GifOptions")
            .field("provider", &self.gif_provider)
            .field("mode", &self.gif_mode)
            .field("api_key_configured", &self.gif_api_key.is_some())
            .field(
                "public_key_configured",
                &self.gif_public_client_key.is_some(),
            )
            .field("proxy_approved", &self.gif_proxy_approved)
            .finish()
    }
}

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
/// Implementations return bounded, normalized DTOs and never receive user IPs,
/// tokens, or headers. This interface is independent of any provider HTTP schema.
pub trait GifProvider: Send + Sync {
    fn trending<'a>(&'a self, cursor: Option<&'a str>) -> ProviderFuture<'a, GifPage>;
    fn search<'a>(&'a self, q: &'a str, cursor: Option<&'a str>) -> ProviderFuture<'a, GifPage>;
    fn categories(&self) -> ProviderFuture<'_, GifCategoriesResponse>;
    fn items<'a>(&'a self, ids: &'a [String]) -> ProviderFuture<'a, GifPage>;
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum CacheKey {
    Trending(Option<String>),
    Search(String, Option<String>),
    Categories,
    Items(Vec<String>),
}
#[derive(Clone)]
enum Value {
    Page(GifPage),
    Categories(GifCategoriesResponse),
}
struct Entry {
    at: Instant,
    bytes: usize,
    value: Value,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<CacheKey, Entry>,
    bytes: usize,
}
impl Cache {
    fn prune(&mut self) {
        self.entries.retain(|_, e| e.at.elapsed() < TTL);
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }
    fn insert(&mut self, key: CacheKey, value: Value, bytes: usize) {
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.at)
                .map(|(k, _)| k.clone())
            else {
                return;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= entry.bytes;
            }
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                at: Instant::now(),
                bytes,
                value,
            },
        );
    }
}
pub(crate) struct GifService {
    capability: GifCapability,
    provider: Option<Box<dyn GifProvider>>,
    // One bounded, coalesced upstream refresh at a time, independent of chat's
    // mutation gate. No refresh timers, polling, or idle allocations.
    cache: Mutex<Cache>,
}
impl GifService {
    pub(crate) fn new(options: &GifOptions) -> Result<Self> {
        for key in [&options.gif_api_key, &options.gif_public_client_key]
            .into_iter()
            .flatten()
        {
            require(
                !key.trim().is_empty() && key.len() <= 512 && !key.chars().any(char::is_control),
                ErrorCode::InvalidRequest,
            )?;
        }
        let mut service = Self {
            capability: GifCapability::Disabled,
            provider: None,
            cache: Mutex::new(Cache::default()),
        };
        if options.gif_provider.is_none() {
            return Ok(service);
        }
        if options.gif_mode == GifMode::Direct {
            if let Some(key) = &options.gif_public_client_key {
                service.capability = GifCapability::Enabled(GifConfig::Direct {
                    provider: GifProviderName::Klipy,
                    public_client_key: key.clone(),
                });
            }
        } else if let Some(key) = &options.gif_api_key {
            // A production key alone is NOT permission to proxy/cache.
            if options.gif_proxy_approved {
                service.provider = Some(Box::new(match &options.mock_gif_base {
                    Some(base) => KlipyProvider::with_base(key.clone(), base.clone())?,
                    None => KlipyProvider::new(key.clone())?,
                }));
                service.capability = GifCapability::Enabled(GifConfig::Proxy {
                    provider: GifProviderName::Klipy,
                });
            }
        }
        Ok(service)
    }
    pub(crate) fn capability(&self) -> GifCapability {
        self.capability.clone()
    }
    async fn fetch(&self, key: CacheKey) -> Result<Value> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?;
        let mut cache = self.cache.lock().await;
        cache.prune();
        if let Some(entry) = cache.entries.get(&key) {
            return Ok(entry.value.clone());
        }
        let value = tokio::time::timeout(Duration::from_secs(10), async {
            Ok::<_, Error>(match &key {
                CacheKey::Trending(cursor) => {
                    Value::Page(provider.trending(cursor.as_deref()).await?)
                }
                CacheKey::Search(q, cursor) => {
                    Value::Page(provider.search(q, cursor.as_deref()).await?)
                }
                CacheKey::Categories => Value::Categories(provider.categories().await?),
                CacheKey::Items(ids) => Value::Page(provider.items(ids).await?),
            })
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))??;
        let bytes = match &value {
            Value::Page(page) => {
                require(page.items.len() <= GIF_PAGE_SIZE, ErrorCode::Unavailable)?;
                for item in &page.items {
                    validate_item(item).map_err(|_| Error::new(ErrorCode::Unavailable))?;
                }
                validate_cursor(page.next_cursor.as_deref())
                    .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                serde_json::to_vec(page)
            }
            Value::Categories(categories) => {
                require(categories.items.len() <= 8, ErrorCode::Unavailable)?;
                for category in &categories.items {
                    validate_text(&category.title, 128)
                        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                    validate_text(&category.query, 200)
                        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                    validate_media(&category.preview, true)
                        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                }
                serde_json::to_vec(categories)
            }
        }
        .map_err(|_| Error::new(ErrorCode::Unavailable))?
        .len();
        require(bytes <= RESPONSE_BYTES, ErrorCode::Unavailable)?;
        cache.insert(key, value.clone(), bytes);
        Ok(value)
    }
}

fn validate_text(text: &str, limit: usize) -> Result<()> {
    require(
        !text.trim().is_empty() && text.len() <= limit && !text.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )
}
fn validate_cursor(cursor: Option<&str>) -> Result<()> {
    if let Some(cursor) = cursor {
        validate_text(cursor, 256)?;
    }
    Ok(())
}
fn validate_id(id: &str) -> Result<()> {
    let raw = id
        .strip_prefix("klipy:")
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    require(
        !raw.is_empty()
            && raw.len() <= 128
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        ErrorCode::InvalidRequest,
    )
}
fn validate_media(media: &GifMedia, preview: bool) -> Result<()> {
    let url = reqwest::Url::parse(&media.url).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        media.url.len() <= 2048
            && url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.port().is_none()
            && matches!(
                url.host_str(),
                Some("static.klipy.com" | "static1.klipy.com" | "static2.klipy.com")
            )
            && media.width > 0
            && media.height > 0
            && media.width <= if preview { 320 } else { 4096 }
            && media.height <= if preview { 320 } else { 4096 }
            && (matches!(
                media.content_type.as_str(),
                "video/webm" | "video/mp4" | "image/webp"
            ) || (!preview && media.content_type == "image/gif")),
        ErrorCode::InvalidRequest,
    )
}
fn validate_item(item: &GifItem) -> Result<()> {
    validate_id(&item.id)?;
    validate_text(&item.title, 512)?;
    validate_media(&item.preview, true)?;
    validate_media(&item.send, false)
}

pub(crate) async fn api(app: &App, path: &str, r: Request<Body>) -> Result<ApiResponse> {
    let mut response = dispatch(app, path, r).await?;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("static header"));
    Ok(response)
}
async fn dispatch(app: &App, path: &str, r: Request<Body>) -> Result<ApiResponse> {
    let token = http::bearer(r.headers())?.to_owned();
    let (_, user) = db::auth(&mut *app.0.pool.acquire().await?, &token).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("gifs:{}", user.0), 30., 10.)?;
    let method = r.method().as_str();
    if path == "gifs/favorites" && method == "GET" {
        let _gate = app.0.core.lock().await;
        let mut conn = app.0.pool.acquire().await?;
        db::auth(&mut conn, &token).await?;
        let ids = sqlx::query_scalar("SELECT gif_id FROM gif_favorites WHERE user_id=? ORDER BY created_at DESC,gif_id ASC LIMIT 200")
            .bind(db::id(user)?).fetch_all(&mut *conn).await?;
        return Ok(http::json(GifFavoritesResponse { ids }));
    }
    if let Some(gif_id) = path.strip_prefix("gifs/favorites/") {
        validate_id(gif_id)?;
        require(matches!(method, "PUT" | "DELETE"), ErrorCode::NotFound)?;
        // Star state is private to the account and shared by all its sessions.
        // Only opaque IDs persist: no provider content or cached media URLs.
        let _gate = app.0.core.lock().await;
        let mut conn = app.0.pool.acquire().await?;
        let (_, current) = db::auth(&mut conn, &token).await?;
        require(current == user, ErrorCode::Unauthenticated)?;
        if method == "DELETE" {
            sqlx::query("DELETE FROM gif_favorites WHERE user_id=? AND gif_id=?")
                .bind(db::id(user)?)
                .bind(gif_id)
                .execute(&mut *conn)
                .await?;
        } else {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM gif_favorites WHERE user_id=? AND gif_id=?)",
            )
            .bind(db::id(user)?)
            .bind(gif_id)
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                let count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM gif_favorites WHERE user_id=?")
                        .bind(db::id(user)?)
                        .fetch_one(&mut *conn)
                        .await?;
                require(count < MAX_GIF_FAVORITES as i64, ErrorCode::LimitExceeded)?;
                sqlx::query("INSERT INTO gif_favorites(user_id,gif_id,created_at) VALUES(?,?,?)")
                    .bind(db::id(user)?)
                    .bind(gif_id)
                    .bind(crate::state::now() as i64)
                    .execute(&mut *conn)
                    .await?;
            }
        }
        return Ok(ApiResponse::empty());
    }
    require(method == "GET", ErrorCode::NotFound)?;
    let key = if path == "gifs/items" {
        let Query(query) = Query::<GifItemsQuery>::try_from_uri(r.uri())
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        require(query.ids.len() <= 3300, ErrorCode::InvalidRequest)?;
        let mut ids: Vec<_> = query.ids.split(',').map(str::to_owned).collect();
        require(
            !ids.is_empty() && ids.len() <= GIF_PAGE_SIZE,
            ErrorCode::InvalidRequest,
        )?;
        for id in &ids {
            validate_id(id)?;
        }
        ids.sort();
        ids.dedup();
        CacheKey::Items(ids)
    } else {
        let Query(query) = Query::<GifQuery>::try_from_uri(r.uri())
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        validate_cursor(query.cursor.as_deref())?;
        match path {
            "gifs/categories" => CacheKey::Categories,
            "gifs/trending" => CacheKey::Trending(query.cursor),
            "gifs/search" => {
                let q = query.q.unwrap_or_default().trim().to_owned();
                validate_text(&q, 200)?;
                CacheKey::Search(q, query.cursor)
            }
            _ => return Err(Error::new(ErrorCode::NotFound)),
        }
    };
    match app.0.gifs.fetch(key).await? {
        Value::Page(page) => Ok(http::json(page)),
        Value::Categories(categories) => Ok(http::json(categories)),
    }
}

/// KLIPY's documented Tenor-v2 compatibility API. Production origin is fixed;
/// test builds alone can inject a loopback HTTP origin.
pub struct KlipyProvider {
    client: reqwest::Client,
    base: String,
    key: String,
    calls: Mutex<VecDeque<Instant>>,
}
impl KlipyProvider {
    pub fn new(key: String) -> Result<Self> {
        Self::with_base(key, "https://api.klipy.com/v2".into())
    }
    fn with_base(key: String, base: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!cfg!(test) && !base.starts_with("http://127.0.0.1:"))
            .no_proxy()
            .build()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        Ok(Self {
            client,
            base,
            key,
            calls: Mutex::new(VecDeque::new()),
        })
    }
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        // Fixed conservative test-key budget across ALL accounts and endpoints,
        // including category preview subrequests. Errors consume quota too.
        {
            let mut calls = self.calls.lock().await;
            while calls
                .front()
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(3600))
            {
                calls.pop_front();
            }
            if calls.len() >= 100 {
                let remaining = Duration::from_secs(3600).saturating_sub(calls[0].elapsed());
                return Err(Error::rate(
                    remaining.as_millis().min(u32::MAX as u128) as u32
                ));
            }
            calls.push_back(Instant::now());
        }
        let mut response = self
            .client
            .get(format!("{}/{endpoint}", self.base))
            .query(&[
                ("key", self.key.as_str()),
                ("client_key", "sticktalk"),
                ("media_filter", MEDIA_FILTER),
                ("contentfilter", "high"),
            ])
            .query(query)
            .send()
            .await
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(60)
                .clamp(1, 3600);
            return Err(Error::rate(seconds * 1000));
        }
        require(response.status().is_success(), ErrorCode::Unavailable)?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::new(ErrorCode::Unavailable))?
        {
            require(
                bytes.len() + chunk.len() <= RESPONSE_BYTES,
                ErrorCode::Unavailable,
            )?;
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::new(ErrorCode::Unavailable))
    }
    async fn page(&self, endpoint: &str, query: &[(&str, String)]) -> Result<GifPage> {
        let response: ProviderPage = self.get(endpoint, query).await?;
        require(
            response.results.len() <= GIF_PAGE_SIZE,
            ErrorCode::Unavailable,
        )?;
        let items = response
            .results
            .into_iter()
            .map(normalize)
            .collect::<Result<_>>()?;
        let next_cursor = response.next.filter(|next| !next.is_empty() && next != "0");
        validate_cursor(next_cursor.as_deref()).map_err(|_| Error::new(ErrorCode::Unavailable))?;
        Ok(GifPage { items, next_cursor })
    }
}
impl GifProvider for KlipyProvider {
    fn trending<'a>(&'a self, cursor: Option<&'a str>) -> ProviderFuture<'a, GifPage> {
        Box::pin(async move {
            self.page("featured", &page_query(cursor, GIF_PAGE_SIZE))
                .await
        })
    }
    fn search<'a>(&'a self, q: &'a str, cursor: Option<&'a str>) -> ProviderFuture<'a, GifPage> {
        Box::pin(async move {
            let mut query = page_query(cursor, GIF_PAGE_SIZE);
            query.push(("q", q.to_owned()));
            self.page("search", &query).await
        })
    }
    fn items<'a>(&'a self, ids: &'a [String]) -> ProviderFuture<'a, GifPage> {
        Box::pin(async move {
            let mut raw = Vec::new();
            for id in ids {
                validate_id(id)?;
                raw.push(id.trim_start_matches("klipy:"));
            }
            self.page("posts", &[("ids", raw.join(","))]).await
        })
    }
    fn categories(&self) -> ProviderFuture<'_, GifCategoriesResponse> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Categories {
                tags: Vec<Category>,
            }
            #[derive(Deserialize)]
            struct Category {
                name: String,
                searchterm: String,
            }
            let categories: Categories = self
                .get(
                    "categories",
                    &[("type", "featured".into()), ("locale", "en_US".into())],
                )
                .await?;
            require(categories.tags.len() <= 100, ErrorCode::Unavailable)?;
            // Category image fields are full GIFs with no dimensions. Obtain a
            // real nano rendition instead; never hand those full GIFs to UI.
            let items = stream::iter(categories.tags.into_iter().take(8).map(
                |category| async move {
                    validate_text(&category.name, 128)
                        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                    validate_text(&category.searchterm, 200)
                        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
                    let mut query = page_query(None, 1);
                    query.push(("q", category.searchterm.clone()));
                    let page = self.page("search", &query).await?;
                    Ok::<_, Error>(page.items.into_iter().next().map(|item| GifCategory {
                        title: category.name,
                        query: category.searchterm,
                        preview: item.preview,
                    }))
                },
            ))
            .buffered(4)
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .flatten()
            .collect();
            Ok(GifCategoriesResponse { items })
        })
    }
}
fn page_query(cursor: Option<&str>, limit: usize) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(cursor) = cursor {
        query.push(("pos", cursor.to_owned()));
    }
    query
}
#[derive(Deserialize)]
struct ProviderPage {
    results: Vec<ProviderItem>,
    next: Option<String>,
}
#[derive(Deserialize)]
struct ProviderItem {
    id: String,
    title: String,
    #[serde(default)]
    content_description: String,
    media_formats: HashMap<String, ProviderMedia>,
}
#[derive(Deserialize)]
struct ProviderMedia {
    url: String,
    dims: [u16; 2],
}
fn normalize(mut item: ProviderItem) -> Result<GifItem> {
    fn take(
        formats: &mut HashMap<String, ProviderMedia>,
        choices: &[(&str, &str)],
    ) -> Result<GifMedia> {
        for (format, content_type) in choices {
            if let Some(media) = formats.remove(*format) {
                return Ok(GifMedia {
                    url: media.url,
                    width: media.dims[0],
                    height: media.dims[1],
                    content_type: (*content_type).into(),
                });
            }
        }
        Err(Error::new(ErrorCode::Unavailable))
    }
    let preview = take(
        &mut item.media_formats,
        &[("nanowebm", "video/webm"), ("nanomp4", "video/mp4")],
    )?;
    let send = take(
        &mut item.media_formats,
        &[
            ("mediumgif", "image/gif"),
            ("tinywebm", "video/webm"),
            ("tinymp4", "video/mp4"),
        ],
    )?;
    let item = GifItem {
        id: format!("klipy:{}", item.id),
        title: if item.content_description.trim().is_empty() {
            item.title
        } else {
            item.content_description
        },
        preview,
        send,
    };
    validate_item(&item).map_err(|_| Error::new(ErrorCode::Unavailable))?;
    Ok(item)
}

#[cfg(test)]
mod tests;
