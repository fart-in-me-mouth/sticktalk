//! KLIPY direct requests never carry StickTalk credentials. Proxy requests stay on the authenticated origin.
use crate::backend::{limited, Config, Http};
use serde::de::DeserializeOwned;
use sticktalk_client_core::gifs::{normalize, ProviderPage};
use sticktalk_proto::*;
#[derive(Clone, Debug)]
pub enum Query {
    Categories,
    Page { q: String, cursor: Option<String> },
    Favorites,
    Items(Vec<String>),
    Favorite(String, bool),
}
#[derive(Clone, Debug)]
pub enum ResultData {
    Categories(Vec<GifCategory>),
    Page(GifPage, bool),
    Favorites(Vec<String>),
    Favorite(String, bool),
}
fn query_path(route: &str, pairs: &[(&str, &str)]) -> String {
    let mut u = reqwest::Url::parse("https://unused.invalid").unwrap();
    u.set_path(route);
    u.query_pairs_mut().extend_pairs(pairs.iter().copied());
    format!(
        "{}{}",
        u.path(),
        u.query().map(|q| format!("?{q}")).unwrap_or_default()
    )
}
async fn direct<T: DeserializeOwned>(
    key: &str,
    endpoint: &str,
    pairs: &[(&str, &str)],
) -> Result<T, String> {
    let c = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not initialize GIF search")?;
    let r = c
        .get(format!("https://api.klipy.com/v2/{endpoint}"))
        .query(&[
            ("key", key),
            ("client_key", "sticktalk"),
            (
                "media_filter",
                "nanowebm,nanomp4,tinywebp,tinywebm,tinymp4,mediumgif",
            ),
            ("contentfilter", "high"),
            ("limit", "24"),
        ])
        .query(pairs)
        .send()
        .await
        .map_err(|_| "GIF provider is unavailable")?;
    if !r.status().is_success() {
        return Err(if r.status().as_u16() == 429 {
            "Too many GIF requests. Try again shortly."
        } else {
            "GIF provider is unavailable"
        }
        .into());
    }
    serde_json::from_slice(&limited(r, 1024 * 1024).await?)
        .map_err(|_| "Invalid GIF provider response".into())
}
pub async fn request(config: Config, cap: GifConfig, query: Query) -> Result<ResultData, String> {
    let http = Http::new(&config.server, &config.token)?;
    match query {
        Query::Favorites => {
            let f: GifFavoritesResponse = http.json("/gifs/favorites").await?;
            Ok(ResultData::Favorites(
                f.ids
                    .into_iter()
                    .filter(|s| valid_id(s))
                    .take(MAX_GIF_FAVORITES)
                    .collect(),
            ))
        }
        Query::Favorite(id, add) => {
            if !valid_id(&id) {
                return Err("Invalid GIF ID".into());
            }
            let response = http
                .core()
                .execute(
                    if add { "PUT" } else { "DELETE" },
                    &format!("/api/v1/gifs/favorites/{id}"),
                    sticktalk_client_core::http::Body::Empty,
                    vec![],
                    30000,
                )
                .await?;
            if !(200..300).contains(&response.status) {
                return Err("Could not update favorites".into());
            }
            Ok(ResultData::Favorite(id, add))
        }
        Query::Categories => {
            let items = match cap {
                GifConfig::Proxy { .. } => {
                    http.json::<GifCategoriesResponse>("/gifs/categories")
                        .await?
                        .items
                }
                GifConfig::Direct {
                    public_client_key: key,
                    ..
                } => {
                    #[derive(serde::Deserialize)]
                    struct Tags {
                        tags: Vec<Tag>,
                    }
                    #[derive(serde::Deserialize)]
                    struct Tag {
                        name: String,
                        searchterm: String,
                        #[serde(default)]
                        image: String,
                    }
                    let tags: Tags = direct(&key, "categories", &[("type", "featured")]).await?;
                    tags.tags
                        .into_iter()
                        .take(8)
                        .map(|t| GifCategory {
                            title: t.name,
                            query: t.searchterm,
                            preview: GifMedia {
                                url: t.image,
                                width: 160,
                                height: 96,
                                content_type: "image/gif".into(),
                            },
                        })
                        .collect()
                }
            };
            Ok(ResultData::Categories(items.into_iter().take(8).collect()))
        }
        Query::Page { q, cursor } => {
            let page = match cap {
                GifConfig::Proxy { .. } => {
                    http.json(&query_path(
                        if q.is_empty() {
                            "/gifs/trending"
                        } else {
                            "/gifs/search"
                        },
                        &[
                            ("q", q.as_str()),
                            ("cursor", cursor.as_deref().unwrap_or_default()),
                        ],
                    ))
                    .await?
                }
                GifConfig::Direct {
                    public_client_key: key,
                    ..
                } => normalize(
                    direct::<ProviderPage>(
                        &key,
                        if q.is_empty() { "featured" } else { "search" },
                        &[
                            ("q", q.as_str()),
                            ("pos", cursor.as_deref().unwrap_or_default()),
                        ],
                    )
                    .await?,
                )?,
            };
            validate_page(&page)?;
            Ok(ResultData::Page(page, false))
        }
        Query::Items(ids) => {
            let mut items = Vec::new();
            for chunk in ids.chunks(GIF_PAGE_SIZE) {
                if chunk.iter().any(|id| !valid_id(id)) {
                    return Err("Invalid GIF ID".into());
                }
                let page: GifPage = match &cap {
                    GifConfig::Proxy { .. } => {
                        http.json(&query_path("/gifs/items", &[("ids", &chunk.join(","))]))
                            .await?
                    }
                    GifConfig::Direct {
                        public_client_key: key,
                        ..
                    } => normalize(
                        direct::<ProviderPage>(
                            key,
                            "posts",
                            &[(
                                "ids",
                                &chunk
                                    .iter()
                                    .map(|s| s.trim_start_matches("klipy:"))
                                    .collect::<Vec<_>>()
                                    .join(","),
                            )],
                        )
                        .await?,
                    )?,
                };
                items.extend(page.items);
                if items.len() >= MAX_GIF_FAVORITES {
                    break;
                }
            }
            Ok(ResultData::Page(
                GifPage {
                    items,
                    next_cursor: None,
                },
                true,
            ))
        }
    }
}
pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("klipy:").is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}
pub fn inline_url(text: &str) -> bool {
    let text = text.trim();
    if text.len() > 2048
        || !(text.starts_with("https://static.klipy.com/")
            || text.starts_with("https://static1.klipy.com/")
            || text.starts_with("https://static2.klipy.com/"))
    {
        return false;
    }
    reqwest::Url::parse(text).is_ok_and(|u| {
        u.scheme() == "https"
            && matches!(
                u.host_str(),
                Some("static.klipy.com" | "static1.klipy.com" | "static2.klipy.com")
            )
            && u.username().is_empty()
            && u.password().is_none()
            && [".gif", ".webp", ".mp4", ".webm"]
                .iter()
                .any(|ext| u.path().ends_with(ext))
    })
}

/// Explicit Save action. Stream to a sibling temporary file, then atomically replace the chosen destination.
pub async fn save_original(url: String, path: std::path::PathBuf) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    if !inline_url(&url) {
        return Err("Unsupported media origin".into());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create download")?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Download failed")?;
    if !response.status().is_success() {
        return Err("Image is no longer available".into());
    }
    let tmp = path.with_file_name(format!(".sticktalk-{}.part", crate::backend::nonce()));
    struct Partial(std::path::PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _partial = Partial(tmp.clone());
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .await
        .map_err(|_| "Could not save image")?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk.map_err(|_| "Download interrupted")?)
            .await
            .map_err(|_| "Could not save image")?;
    }
    file.sync_all().await.map_err(|_| "Could not save image")?;
    drop(file);
    tokio::fs::rename(tmp, path)
        .await
        .map_err(|_| "Could not replace destination".into())
}

pub fn validate_page(page: &GifPage) -> Result<(), String> {
    if page.items.len() > GIF_PAGE_SIZE
        || page
            .next_cursor
            .as_ref()
            .is_some_and(|c| c.len() > 256 || c.chars().any(char::is_control))
    {
        return Err("Invalid GIF page".into());
    }
    for item in &page.items {
        if !valid_id(&item.id)
            || item.title.len() > 512
            || item.preview.width > 320
            || item.preview.height > 320
        {
            return Err("Invalid GIF result".into());
        }
        sticktalk_client_core::gifs::validate_media(&item.preview)?;
        sticktalk_client_core::gifs::validate_media(&item.send)?;
    }
    Ok(())
}
