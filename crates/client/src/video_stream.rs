//! On-demand authenticated attachment playback for Media Foundation's URL source.
//! A clip-scoped loopback capability keeps the bearer token out of decoder URLs.
//! Bodies flow with backpressure; no original-file cache or whole-file allocation.
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use futures_util::StreamExt;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use sticktalk_proto::Id;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Retain until the decoder has shut down. Dropping cancels active transfers and
/// closes the listener/idle connections, including navigation and logout paths.
pub struct Source {
    pub path: PathBuf,
    stop: CancellationToken,
    received: Arc<AtomicU64>,
}
impl Drop for Source {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[derive(Clone)]
struct Proxy {
    client: reqwest::Client,
    origin: String,
    token: String,
    stop: CancellationToken,
    slots: Arc<Semaphore>,
    received: Arc<AtomicU64>,
}
impl Source {
    /// Diagnostic count of body bytes delivered to the decoder, not a cache.
    pub fn bytes_received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
    pub async fn attachment(server: &str, token: &str, attachment: Id) -> Result<Self, String> {
        let server = sticktalk_client_core::http::validate_server(server)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "Could not start video transport")?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Could not start local video stream")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Video stream unavailable")?;
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).map_err(|_| "Could not secure video stream")?;
        use base64::Engine;
        let capability = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
        let route = format!("/{capability}/clip");
        let stop = CancellationToken::new();
        let received = Arc::new(AtomicU64::new(0));
        let proxy = Proxy {
            client,
            origin: format!("{server}/api/v1/files/{}", attachment.0),
            token: token.into(),
            stop: stop.clone(),
            slots: Arc::new(Semaphore::new(4)),
            received: received.clone(),
        };
        let router = Router::new().route(&route, get(forward)).with_state(proxy);
        let shutdown = stop.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self {
            path: PathBuf::from(format!("http://{address}{route}")),
            stop,
            received,
        })
    }
}
async fn forward(
    State(proxy): State<Proxy>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let permit = proxy
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut request = proxy
        .client
        .request(method.clone(), &proxy.origin)
        .bearer_auth(&proxy.token);
    // Never forward cookies, authorization, URLs, or arbitrary decoder headers.
    for name in ["range", "if-range", "if-none-match"] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    let upstream = tokio::select! {
        _ = proxy.stop.cancelled() => return Err(StatusCode::GONE),
        response = request.send() => response.map_err(|_| StatusCode::BAD_GATEWAY)?,
    };
    let mut response = Response::builder().status(upstream.status());
    // Only range/representation metadata, not upstream cookies/location/credentials.
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
        "etag",
    ] {
        if let Some(value) = upstream.headers().get(name) {
            response = response.header(name, value);
        }
    }
    response = response.header("cache-control", "no-store");
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        let stream = upstream
            .bytes_stream()
            .take_until(proxy.stop.cancelled_owned());
        // The permit lives through the body, not just receipt of the headers.
        Body::from_stream(stream.map(move |chunk| {
            let _keep_alive = &permit;
            if let Ok(bytes) = &chunk {
                proxy
                    .received
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            chunk
        }))
    };
    response.body(body).map_err(|_| StatusCode::BAD_GATEWAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Request, routing::any};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn decoder_capability_is_lazy_scoped_bounded_and_stops_on_drop() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let upstream = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/v1/files/42",
                    any(move |request: Request| {
                        let count = count.clone();
                        async move {
                            assert_eq!(request.headers()["authorization"], "Bearer test-secret");
                            assert!(!request.headers().contains_key("cookie"));
                            count.fetch_add(1, Ordering::SeqCst);
                            if request.method() == Method::HEAD {
                                return Response::builder()
                                    .header("content-length", "2147483649")
                                    .header("accept-ranges", "bytes")
                                    .body(Body::empty())
                                    .unwrap();
                            }
                            assert_eq!(request.headers()["range"], "bytes=2147483640-2147483648");
                            Response::builder()
                                .status(206)
                                .header("content-length", "9")
                                .header("content-range", "bytes 2147483640-2147483648/2147483649")
                                .header("set-cookie", "must-not-escape")
                                .body(Body::from("123456789"))
                                .unwrap()
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let source = Source::attachment(&format!("http://{address}"), "test-secret", Id(42))
            .await
            .unwrap();
        let url = source.path.to_str().unwrap().to_owned();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "opening does not prefetch the clip"
        );
        let client = reqwest::Client::new();
        let head = client.head(&url).send().await.unwrap();
        assert_eq!(head.headers()["content-length"], "2147483649");
        let response = client
            .get(&url)
            .header("Range", "bytes=2147483640-2147483648")
            .header("Authorization", "attacker")
            .header("Cookie", "must-not-leak")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            response.headers()["content-range"],
            "bytes 2147483640-2147483648/2147483649"
        );
        assert!(!response.headers().contains_key("set-cookie"));
        assert_eq!(response.text().await.unwrap(), "123456789");
        let local = reqwest::Url::parse(&url).unwrap();
        let wrong = format!("http://{}/wrong/clip", local.authority());
        assert_eq!(client.get(wrong).send().await.unwrap().status(), 404);
        assert_eq!(client.post(&url).send().await.unwrap().status(), 405);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        drop(source);
        // A fresh connection must fail once graceful shutdown closes the listener.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if tokio::net::TcpStream::connect(local.authority())
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        upstream.abort();
    }
}
