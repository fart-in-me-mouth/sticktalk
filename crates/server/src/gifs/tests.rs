use super::*;
use axum::{extract::State, http::StatusCode, response::IntoResponse};
use serde_json::{json, Value as JsonValue};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Default)]
struct Mock {
    calls: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    queries: Arc<Mutex<Vec<HashMap<String, String>>>>,
}
fn provider_item() -> JsonValue {
    json!({"id":"123", "title":"Wave", "content_description":"A friendly wave",
    "media_formats": {
        "nanowebm":{"url":"https://static.klipy.com/preview.webm?unchanged=yes","dims":[150,100]},
        "nanomp4":{"url":"https://static1.klipy.com/preview.mp4","dims":[150,100]},
        "mediumgif":{"url":"https://static2.klipy.com/medium.gif","dims":[498,332]},
        "gif":{"url":"https://static.klipy.com/full.gif","dims":[1920,1080]}
    }})
}
async fn mock_handler(State(mock): State<Mock>, r: Request<Body>) -> axum::response::Response {
    assert!(r.headers().get("authorization").is_none());
    assert!(r.headers().get("x-forwarded-for").is_none());
    assert!(r.headers().get("cookie").is_none());
    mock.calls.fetch_add(1, Ordering::SeqCst);
    let Query(query) = Query::<HashMap<String, String>>::try_from_uri(r.uri()).unwrap();
    assert_eq!(query["key"], "server-secret");
    assert_eq!(query["contentfilter"], "high");
    assert!(!query["media_filter"]
        .split(',')
        .any(|format| format == "gif"));
    mock.queries.lock().await.push(query);
    match mock.mode.load(Ordering::SeqCst) {
        1 => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "7")],
                "secret upstream error",
            )
                .into_response()
        }
        2 => {
            return (
                StatusCode::FOUND,
                [("location", "https://example.invalid/secret")],
                "redirect",
            )
                .into_response()
        }
        3 => return "not json server-secret".into_response(),
        4 => return "x".repeat(RESPONSE_BYTES + 1).into_response(),
        5 => return axum::Json(json!({"results":[{"id":"bad"}],"next":null})).into_response(),
        6 => {
            let mut item = provider_item();
            item["media_formats"]["nanowebm"]["url"] = json!("http://127.0.0.1/private");
            return axum::Json(json!({"results":[item],"next":null})).into_response();
        }
        _ => {}
    }
    if r.uri().path().ends_with("/categories") {
        axum::Json(json!({"tags":[{"name":"Happy","searchterm":"happy", "image":"https://static.klipy.com/large.gif", "path":"https://api.klipy.com/key-leak"}]})).into_response()
    } else {
        axum::Json(json!({"results":[provider_item()],"next":"opaque+cursor/="})).into_response()
    }
}
struct Fixture {
    app: App,
    url: String,
    mock: Mock,
    tokens: [String; 3],
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    upstream_task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.app.shutdown();
        self.task.abort();
        self.upstream_task.abort();
    }
}
impl Fixture {
    async fn new(options: GifOptions, proxy_mock: bool) -> Self {
        let mock = Mock::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v2", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .fallback(mock_handler)
            .with_state(mock.clone());
        let upstream_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::open(crate::Config {
            operations: Default::default(),
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: dir.path().into(),
            public_url: "http://localhost".into(),
            max_upload_mb: 25,
            max_total_upload_mb: 10240,
            worker_id: 0,
            trusted_proxy: "127.0.0.1".parse().unwrap(),
            google_client_id: None,
            mock_google_jwks: None,
            mock_google_issuer: None,
            gifs: options,
        })
        .await
        .unwrap();
        if proxy_mock {
            let inner = Arc::get_mut(&mut app.0).unwrap();
            inner.gifs = GifService {
                capability: GifCapability::Enabled(GifConfig::Proxy {
                    provider: GifProviderName::Klipy,
                }),
                provider: Some(Box::new(
                    KlipyProvider::with_base("server-secret".into(), base).unwrap(),
                )),
                cache: Mutex::new(Cache::default()),
            };
        }
        let tokens = [
            crate::state::token(),
            crate::state::token(),
            crate::state::token(),
        ];
        for user in 1..=2_i64 {
            sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'','online',0)")
                .bind(user).bind(format!("gif_user{user}")).execute(&app.0.pool).await.unwrap();
        }
        for (token, user) in tokens.iter().zip([1_i64, 2, 1]) {
            sqlx::query("INSERT INTO sessions VALUES(?,?,0,?,0)")
                .bind(crate::state::hash(token.as_bytes()))
                .bind(user)
                .bind((crate::state::now() + 600_000) as i64)
                .execute(&app.0.pool)
                .await
                .unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving = app.clone();
        let task = tokio::spawn(async move { serving.serve(listener).await });
        Self {
            app,
            url,
            mock,
            tokens,
            task,
            upstream_task,
            _dir: dir,
        }
    }
    async fn proxy() -> Self {
        Self::new(GifOptions::default(), true).await
    }
    async fn request(&self, method: &str, path: &str, user: Option<usize>) -> reqwest::Response {
        let mut request = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .request(method.parse().unwrap(), format!("{}{path}", self.url));
        if let Some(user) = user {
            request = request.bearer_auth(&self.tokens[user]);
        }
        request.send().await.unwrap()
    }
    async fn page(&self, path: &str) -> GifPage {
        let response = self.request("GET", path, Some(0)).await;
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        response.json().await.unwrap()
    }
}

#[tokio::test]
async fn config_is_explicit_disabled_by_default_and_secrets_are_not_public() {
    let options = GifOptions {
        gif_api_key: Some("private-secret".into()),
        ..Default::default()
    };
    assert_eq!(
        GifService::new(&options).unwrap().capability(),
        GifCapability::Disabled
    );
    let options = GifOptions {
        gif_provider: Some(ProviderOption::Klipy),
        gif_mode: GifMode::Proxy,
        ..options
    };
    assert_eq!(
        GifService::new(&options).unwrap().capability(),
        GifCapability::Disabled
    );
    let approved = GifOptions {
        gif_proxy_approved: true,
        ..options.clone()
    };
    let service = GifService::new(&approved).unwrap();
    let wire = serde_json::to_string(&service.capability()).unwrap();
    assert_eq!(wire, r#"{"mode":"proxy","provider":"klipy"}"#);
    assert!(!format!("{approved:?}").contains("private-secret"));
    let fixture = Fixture::new(options, false).await;
    let meta: MetaResponse = fixture
        .request("GET", "/api/meta", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(meta.gifs, GifCapability::Disabled);
    assert_eq!(
        fixture
            .request("GET", "/api/gifs/trending", Some(0))
            .await
            .status(),
        404
    );
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 0);
    let direct = GifOptions {
        gif_provider: Some(ProviderOption::Klipy),
        gif_public_client_key: Some("public-key".into()),
        gif_api_key: Some("private-secret".into()),
        ..Default::default()
    };
    let fixture = Fixture::new(direct, false).await;
    let response = fixture.request("GET", "/api/v1/meta", None).await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let meta = response.text().await.unwrap();
    assert!(!meta.contains("private-secret"));
    assert_eq!(
        serde_json::from_str::<JsonValue>(&meta).unwrap()["gifs"],
        json!({"mode":"direct","provider":"klipy","public_client_key":"public-key"})
    );
    assert_eq!(
        fixture
            .request("GET", "/api/gifs/search?q=happy", Some(0))
            .await
            .status(),
        404
    );
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn discovery_uses_light_renditions_caches_and_preserves_opaque_cursors() {
    let fixture = Fixture::proxy().await;
    let page = fixture.page("/api/gifs/trending").await;
    assert_eq!(page.items[0].id, "klipy:123");
    assert_eq!(page.items[0].title, "A friendly wave");
    assert_eq!(page.items[0].preview.width, 150);
    assert_eq!(page.items[0].preview.content_type, "video/webm");
    assert!(page.items[0].preview.url.ends_with("?unchanged=yes"));
    assert!(page.items[0].send.url.ends_with("medium.gif"));
    assert_eq!(page.next_cursor.as_deref(), Some("opaque+cursor/="));
    assert_eq!(fixture.page("/api/v1/gifs/trending").await, page);
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 1);
    fixture
        .page("/api/gifs/trending?cursor=opaque%2Bcursor%2F%3D")
        .await;
    assert_eq!(
        fixture.mock.queries.lock().await.last().unwrap()["pos"],
        "opaque+cursor/="
    );
    fixture.page("/api/gifs/search?q=happy%26safe").await;
    fixture.page("/api/gifs/search?q=%20happy%26safe%20").await;
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        fixture.mock.queries.lock().await.last().unwrap()["q"],
        "happy&safe"
    );
    let response = fixture
        .request("GET", "/api/gifs/categories", Some(0))
        .await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let categories: GifCategoriesResponse = response.json().await.unwrap();
    assert_eq!(categories.items[0].title, "Happy");
    assert_eq!(categories.items[0].query, "happy");
    assert_eq!(categories.items[0].preview.content_type, "video/webm");
    fixture
        .request("GET", "/api/gifs/categories", Some(0))
        .await
        .error_for_status()
        .unwrap();
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 5);
    fixture.page("/api/gifs/items?ids=klipy:123").await;
    assert_eq!(
        fixture.mock.queries.lock().await.last().unwrap()["ids"],
        "123"
    );
    for entry in fixture.app.0.gifs.cache.lock().await.entries.values_mut() {
        entry.at -= TTL;
    }
    fixture.page("/api/gifs/trending").await;
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn authentication_validation_account_rate_limits_and_upstream_errors() {
    let fixture = Fixture::proxy().await;
    assert_eq!(
        fixture
            .request("GET", "/api/gifs/trending", None)
            .await
            .status(),
        401
    );
    for path in [
        "/api/gifs/search",
        "/api/gifs/search?q=%20",
        "/api/gifs/search?q=%00",
        "/api/gifs/trending?cursor=",
        "/api/gifs/items?ids=bad",
        "/api/gifs/unknown",
    ] {
        assert!(fixture
            .request("GET", path, Some(0))
            .await
            .status()
            .is_client_error());
    }
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 0);
    // The burst belongs to an account, not an individual login session.
    for _ in 0..4 {
        fixture.request("GET", "/api/gifs/favorites", Some(2)).await;
    }
    let response = fixture.request("GET", "/api/gifs/favorites", Some(0)).await;
    assert_eq!(response.status(), 429);
    assert!(response.headers().contains_key("retry-after"));
    assert_eq!(
        response.json::<ApiError>().await.unwrap().code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        fixture
            .request("GET", "/api/gifs/favorites", Some(1))
            .await
            .status(),
        200
    );
    fixture.app.0.core.lock().await.limits.clear();
    fixture.mock.mode.store(1, Ordering::SeqCst);
    let response = fixture.request("GET", "/api/gifs/trending", Some(0)).await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "7");
    let text = response.text().await.unwrap();
    assert!(!text.contains("secret"));
    for mode in 2..=6 {
        fixture.mock.mode.store(mode, Ordering::SeqCst);
        let response = fixture.request("GET", "/api/gifs/trending", Some(0)).await;
        assert_eq!(response.status(), 503);
        assert!(!response.text().await.unwrap().contains("server-secret"));
    }
    fixture.mock.mode.store(0, Ordering::SeqCst);
    fixture.page("/api/gifs/trending").await;
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 7); // failures never cached, redirects never followed
}

#[tokio::test]
async fn favorites_are_idempotent_private_bounded_and_survive_reopen() {
    let fixture = Fixture::new(Default::default(), false).await;
    for _ in 0..2 {
        assert_eq!(
            fixture
                .request("PUT", "/api/gifs/favorites/klipy:123", Some(0))
                .await
                .status(),
            204
        );
    }
    let favorites: GifFavoritesResponse = fixture
        .request("GET", "/api/gifs/favorites", Some(2))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(favorites.ids, ["klipy:123"]);
    assert_eq!(
        fixture
            .request("DELETE", "/api/gifs/favorites/klipy:123", Some(1))
            .await
            .status(),
        204
    );
    let favorites: GifFavoritesResponse = fixture
        .request("GET", "/api/gifs/favorites", Some(1))
        .await
        .json()
        .await
        .unwrap();
    assert!(favorites.ids.is_empty());
    assert_eq!(
        fixture
            .request("PUT", "/api/gifs/favorites/klipy:bad!", Some(0))
            .await
            .status(),
        400
    );
    for i in 1..MAX_GIF_FAVORITES {
        sqlx::query("INSERT INTO gif_favorites VALUES(?,?,0)")
            .bind(1_i64)
            .bind(format!("klipy:id{i}"))
            .execute(&fixture.app.0.pool)
            .await
            .unwrap();
    }
    assert_eq!(
        fixture
            .request("PUT", "/api/gifs/favorites/klipy:123", Some(0))
            .await
            .status(),
        204
    );
    let response = fixture
        .request("PUT", "/api/gifs/favorites/klipy:overflow", Some(0))
        .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<ApiError>().await.unwrap().code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        fixture
            .request("DELETE", "/api/gifs/favorites/klipy:123", Some(0))
            .await
            .status(),
        204
    );
    assert_eq!(
        fixture
            .request("DELETE", "/api/gifs/favorites/klipy:123", Some(0))
            .await
            .status(),
        204
    );
    fixture.app.shutdown();
    fixture.app.0.pool.close().await;
    let reopened = App::open(fixture.app.0.config.clone()).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gif_favorites WHERE user_id=1")
        .fetch_one(&reopened.0.pool)
        .await
        .unwrap();
    assert_eq!(count, 199);
    sqlx::query("DELETE FROM users WHERE id=1")
        .execute(&reopened.0.pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gif_favorites")
        .fetch_one(&reopened.0.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cache_coalesces_and_stays_bounded_without_idle_work() {
    let fixture = Fixture::proxy().await;
    let futures = (0..12).map(|_| fixture.app.0.gifs.fetch(CacheKey::Trending(None)));
    for result in futures_util::future::join_all(futures).await {
        assert!(result.is_ok());
    }
    assert_eq!(fixture.mock.calls.load(Ordering::SeqCst), 1);
    let mut cache = Cache::default();
    let value = Value::Page(GifPage {
        items: vec![],
        next_cursor: None,
    });
    for i in 0..200 {
        cache.insert(
            CacheKey::Search(i.to_string(), None),
            value.clone(),
            100_000,
        );
    }
    assert!(cache.entries.len() <= CACHE_ENTRIES);
    assert!(cache.bytes <= CACHE_BYTES);
    assert!(!cache
        .entries
        .contains_key(&CacheKey::Search("0".into(), None)));
    for entry in cache.entries.values_mut() {
        entry.at -= TTL;
    }
    cache.prune();
    assert!(cache.entries.is_empty());
    assert_eq!(cache.bytes, 0);
    let provider =
        KlipyProvider::with_base("server-secret".into(), "http://127.0.0.1:1".into()).unwrap();
    *provider.calls.lock().await = VecDeque::from(vec![Instant::now(); 100]);
    let error = provider.trending(None).await.unwrap_err();
    assert_eq!(error.0.code, ErrorCode::RateLimited);
    assert!(error.0.retry_after_ms.unwrap() > 3_500_000);
}

#[test]
fn normalized_renditions_and_inputs_fail_closed() {
    let mut item = provider_item();
    item["media_formats"]
        .as_object_mut()
        .unwrap()
        .remove("nanowebm");
    item["media_formats"]
        .as_object_mut()
        .unwrap()
        .remove("mediumgif");
    item["media_formats"]["tinymp4"] =
        json!({"url":"https://static.klipy.com/small.mp4","dims":[320,200]});
    let normalized = normalize(serde_json::from_value(item.clone()).unwrap()).unwrap();
    assert_eq!(normalized.preview.content_type, "video/mp4");
    assert_eq!(normalized.send.content_type, "video/mp4");
    item["media_formats"]
        .as_object_mut()
        .unwrap()
        .remove("nanomp4");
    assert_eq!(
        normalize(serde_json::from_value(item).unwrap())
            .unwrap_err()
            .0
            .code,
        ErrorCode::Unavailable
    );
    for bad in [
        "https://static.klipy.com.evil.invalid/image.webm",
        "https://user@static.klipy.com/image.webm",
        "https://static.klipy.com:8443/image.webm",
        "https://static.klipy.com/image.webm#fragment",
        "file:///secret",
    ] {
        let mut item: ProviderItem = serde_json::from_value(provider_item()).unwrap();
        item.media_formats.get_mut("nanowebm").unwrap().url = bad.into();
        assert!(normalize(item).is_err());
    }
    for id in [
        "123",
        "giphy:123",
        "klipy:",
        "klipy:a/b",
        "klipy:a?key=x",
        "klipy:💚",
    ] {
        assert!(validate_id(id).is_err());
    }
    assert!(validate_text(&"é".repeat(101), 200).is_err());
    assert!(validate_cursor(Some(&"x".repeat(257))).is_err());
    assert!(validate_cursor(Some("\n")).is_err());
    for options in [
        GifOptions {
            gif_public_client_key: Some("public".into()),
            ..Default::default()
        },
        GifOptions {
            gif_provider: Some(ProviderOption::Klipy),
            gif_mode: GifMode::Proxy,
            gif_proxy_approved: true,
            ..Default::default()
        },
    ] {
        assert_eq!(
            GifService::new(&options).unwrap().capability(),
            GifCapability::Disabled
        );
    }
    assert!(GifService::new(&GifOptions {
        gif_api_key: Some("\n".into()),
        ..Default::default()
    })
    .is_err());
}
