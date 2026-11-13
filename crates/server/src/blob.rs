//! Bounded streaming blob I/O. Keys are opaque single path components.
use crate::{
    error::{require, Error, Result},
    state::now,
};
use axum::body::Body;
use futures_util::{future::BoxFuture, StreamExt};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use sticktalk_proto::ErrorCode;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub const BUFFER: usize = 64 * 1024;
pub fn valid_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 100 {
        return false;
    }
    // Encrypted snapshots are still a single, fixed-format opaque component.
    if let Some(stamp) = key
        .strip_prefix("backup-")
        .and_then(|s| s.strip_suffix(".sqlite3.age"))
    {
        return !stamp.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit());
    }
    let mut parts = key.split('.');
    let stem = parts.next().unwrap_or_default();
    let suffix = parts.next();
    !stem.is_empty()
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && suffix.is_none_or(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        && parts.next().is_none()
}
pub fn offline() -> Error {
    let mut e = Error::new(ErrorCode::Unavailable);
    e.0.message = "Storage temporarily offline. Please retry later.".into();
    e
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BlobStat {
    pub size: u64,
    pub complete: bool,
}
/// All methods stream bodies, never materialize a file in RAM. PUT is durable
/// before acknowledgment; callers may retry from their last committed offset.
pub trait BlobStore: Send + Sync {
    fn put_chunk<'a>(
        &'a self,
        key: &'a str,
        offset: u64,
        length: u64,
        sha: &'a str,
        body: Body,
    ) -> BoxFuture<'a, Result<()>>;
    fn finalize<'a>(&'a self, key: &'a str, size: u64, sha: &'a str) -> BoxFuture<'a, Result<()>>;
    fn get<'a>(&'a self, key: &'a str, start: u64, length: u64) -> BoxFuture<'a, Result<Body>>;
    fn stat<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<BlobStat>>;
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>>;
    fn free_space(&self) -> BoxFuture<'_, Result<u64>>;
}
#[derive(Clone)]
pub struct LocalDisk {
    pub(crate) dir: PathBuf,
}
impl LocalDisk {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    pub(crate) fn path(&self, key: &str) -> Result<PathBuf> {
        require(valid_key(key), ErrorCode::InvalidRequest)?;
        Ok(self.dir.join(key))
    }
}
pub async fn hash_region(path: &Path, start: u64, length: u64) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut file = file.take(length);
    let mut sha = Sha256::new();
    let mut buffer = vec![0; BUFFER];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
    }
    Ok(hex(&sha.finalize()))
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl BlobStore for LocalDisk {
    fn put_chunk<'a>(
        &'a self,
        key: &'a str,
        offset: u64,
        length: u64,
        sha: &'a str,
        body: Body,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(self.path(key)?)
                .await?;
            require(file.metadata().await?.len() >= offset, ErrorCode::Conflict)?;
            file.set_len(offset).await?;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut stream = body.into_data_stream();
            let mut received = 0u64;
            let mut digest = Sha256::new();
            while let Some(bytes) = stream.next().await {
                let bytes = bytes.map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
                received = received.saturating_add(bytes.len() as u64);
                require(received <= length, ErrorCode::PayloadTooLarge)?;
                digest.update(&bytes);
                file.write_all(&bytes).await?;
            }
            require(
                received == length && hex(&digest.finalize()) == sha,
                ErrorCode::InvalidAttachment,
            )?;
            file.sync_all().await?;
            Ok(())
        })
    }
    fn finalize<'a>(&'a self, key: &'a str, size: u64, sha: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            require(
                self.stat(key).await?.size == size
                    && hash_region(&self.path(key)?, 0, size).await? == sha,
                ErrorCode::InvalidAttachment,
            )
        })
    }
    fn get<'a>(&'a self, key: &'a str, start: u64, length: u64) -> BoxFuture<'a, Result<Body>> {
        Box::pin(async move {
            let mut file = tokio::fs::File::open(self.path(key)?).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::new(ErrorCode::NotFound)
                } else {
                    e.into()
                }
            })?;
            file.seek(std::io::SeekFrom::Start(start)).await?;
            Ok(Body::from_stream(
                tokio_util::io::ReaderStream::with_capacity(file.take(length), BUFFER),
            ))
        })
    }
    fn stat<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<BlobStat>> {
        Box::pin(async move {
            let metadata = tokio::fs::metadata(self.path(key)?).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::new(ErrorCode::NotFound)
                } else {
                    e.into()
                }
            })?;
            Ok(BlobStat {
                size: metadata.len(),
                complete: true,
            })
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            match tokio::fs::remove_file(self.path(key)?).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            }
        })
    }
    fn free_space(&self) -> BoxFuture<'_, Result<u64>> {
        Box::pin(async move { Ok(fs2::available_space(&self.dir)?) })
    }
}
#[derive(clap::Args, Clone)]
pub struct StorageOptions {
    #[arg(long, env = "STICKTALK_STORAGE_URL")]
    pub storage_url: Option<String>,
    #[arg(
        long,
        env = "STICKTALK_STORAGE_SECRET",
        hide_env_values = true,
        hide_default_value = true
    )]
    pub storage_secret: Option<String>,
    #[arg(long, env = "STICKTALK_CACHE_MB", default_value_t = 102400)]
    pub cache_mb: u64,
    #[arg(long, env = "STICKTALK_CACHE_FILE_MB", default_value_t = 25)]
    pub cache_file_mb: u64,
}
impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            storage_url: None,
            storage_secret: None,
            cache_mb: 102400,
            cache_file_mb: 25,
        }
    }
}
impl std::fmt::Debug for StorageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageOptions")
            .field("storage_url", &self.storage_url)
            .field("secret", &"[REDACTED]")
            .field("cache_mb", &self.cache_mb)
            .field("cache_file_mb", &self.cache_file_mb)
            .finish()
    }
}
#[derive(Clone)]
pub struct RemoteStore {
    pub(crate) url: String,
    pub(crate) secret: Vec<u8>,
    pub(crate) client: reqwest::Client,
    verification_client: reqwest::Client,
}
impl RemoteStore {
    pub fn new(url: &str, secret: &str) -> Result<Self> {
        let u = reqwest::Url::parse(url).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        let ip = u
            .host_str()
            .and_then(|s| s.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok());
        require(
            u.scheme() == "http"
                && ip.is_some_and(crate::storage_node::private_address)
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
                && u.path() == "/"
                && secret.len() >= 32
                && secret.len() <= 256,
            ErrorCode::InvalidRequest,
        )?;
        Ok(Self {
            url: url.trim_end_matches('/').into(),
            secret: secret.as_bytes().to_vec(),
            verification_client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(2))
                .timeout(std::time::Duration::from_secs(3600))
                .build()
                .map_err(|_| offline())?,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(2))
                .read_timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|_| offline())?,
        })
    }
    pub(crate) fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        offset: u64,
        length: u64,
        sha: &str,
        range: &str,
    ) -> reqwest::RequestBuilder {
        let timestamp = (now() / 1000).to_string();
        let nonce = crate::state::stream_id();
        let signature = crate::storage_node::sign(
            &self.secret,
            method.as_str(),
            path,
            &timestamp,
            &nonce,
            offset,
            length,
            sha,
            range,
        );
        let client = if method == reqwest::Method::POST {
            &self.verification_client
        } else {
            &self.client
        };
        client
            .request(method, format!("{}{path}", self.url))
            .header("x-time", timestamp)
            .header("x-nonce", nonce)
            .header("x-signature", signature)
            .header("upload-offset", offset)
            .header("x-size", length)
            .header("x-sha256", sha)
            .header("range", range)
    }
    async fn checked(&self, r: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let r = r.send().await.map_err(|_| offline())?;
        match r.status().as_u16() {
            200..=299 => Ok(r),
            404 => Err(Error::new(ErrorCode::NotFound)),
            409 => Err(Error::new(ErrorCode::Conflict)),
            _ => Err(offline()),
        }
    }
}
impl BlobStore for RemoteStore {
    fn put_chunk<'a>(
        &'a self,
        key: &'a str,
        offset: u64,
        length: u64,
        sha: &'a str,
        body: Body,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            require(valid_key(key), ErrorCode::InvalidRequest)?;
            self.checked(
                self.request(
                    reqwest::Method::PUT,
                    &format!("/v1/blob/{key}"),
                    offset,
                    length,
                    sha,
                    "",
                )
                .header("content-length", length)
                .body(reqwest::Body::wrap_stream(body.into_data_stream())),
            )
            .await?;
            Ok(())
        })
    }
    fn finalize<'a>(&'a self, key: &'a str, size: u64, sha: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            require(valid_key(key), ErrorCode::InvalidRequest)?;
            self.checked(self.request(
                reqwest::Method::POST,
                &format!("/v1/blob/{key}"),
                0,
                size,
                sha,
                "",
            ))
            .await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, key: &'a str, start: u64, length: u64) -> BoxFuture<'a, Result<Body>> {
        Box::pin(async move {
            require(valid_key(key), ErrorCode::InvalidRequest)?;
            let range = if length == 0 {
                String::new()
            } else {
                format!("bytes={start}-{}", start.saturating_add(length) - 1)
            };
            let r = self
                .checked(self.request(
                    reqwest::Method::GET,
                    &format!("/v1/blob/{key}"),
                    0,
                    0,
                    "",
                    &range,
                ))
                .await?;
            Ok(Body::from_stream(r.bytes_stream()))
        })
    }
    fn stat<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<BlobStat>> {
        Box::pin(async move {
            require(valid_key(key), ErrorCode::InvalidRequest)?;
            let r = self
                .checked(self.request(
                    reqwest::Method::HEAD,
                    &format!("/v1/blob/{key}"),
                    0,
                    0,
                    "",
                    "",
                ))
                .await?;
            let size = r
                .headers()
                .get("upload-offset")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse().ok())
                .ok_or_else(offline)?;
            Ok(BlobStat {
                size,
                complete: r.headers().get("x-complete").is_some_and(|h| h == "1"),
            })
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            require(valid_key(key), ErrorCode::InvalidRequest)?;
            self.checked(self.request(
                reqwest::Method::DELETE,
                &format!("/v1/blob/{key}"),
                0,
                0,
                "",
                "",
            ))
            .await?;
            Ok(())
        })
    }
    fn free_space(&self) -> BoxFuture<'_, Result<u64>> {
        Box::pin(async move {
            let r = self
                .checked(self.request(reqwest::Method::GET, "/v1/space", 0, 0, "", ""))
                .await?;
            r.json().await.map_err(|_| offline())
        })
    }
}
