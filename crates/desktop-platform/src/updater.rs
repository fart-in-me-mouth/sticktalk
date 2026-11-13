//! Signed NSIS updates. Every disk artifact is untrusted until reverified.
use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use sticktalk_proto::{SignedUpdateManifest, UpdateManifest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
pub const TARGET: &str = "x86_64-pc-windows-msvc";
const MAX_INSTALLER: u64 = 1024 * 1024 * 1024;
const MAX_MANIFEST: usize = 16 * 1024;

#[derive(Clone)]
pub struct Updater {
    client: reqwest::Client,
    origin: url::Url,
    key: [u8; 32],
    installed: semver::Version,
    staging: PathBuf,
    download_gate: std::sync::Arc<tokio::sync::Mutex<()>>,
}
#[derive(Debug)]
pub struct VerifiedUpdate {
    envelope: SignedUpdateManifest,
    manifest: UpdateManifest,
}
impl VerifiedUpdate {
    pub fn manifest(&self) -> &UpdateManifest {
        &self.manifest
    }
}
#[derive(Debug)]
pub struct StagedUpdate {
    installer: PathBuf,
    version: String,
}
impl StagedUpdate {
    pub fn installer(&self) -> &Path {
        &self.installer
    }
    pub fn version(&self) -> &str {
        &self.version
    }
}

impl Updater {
    /// Release builds embed the owner's public key from the release environment.
    /// Missing/invalid configuration fails closed; no test key is ever trusted here.
    pub fn embedded(origin: &str, installed: &str, staging: PathBuf) -> Result<Self> {
        let key = option_env!("STICKTALK_UPDATE_PUBLIC_KEY")
            .ok_or(Error::Invalid("release public key not embedded"))?;
        let key: [u8; 32] = STANDARD
            .decode(key)
            .map_err(|_| Error::Invalid("public key base64"))?
            .try_into()
            .map_err(|_| Error::Invalid("public key length"))?;
        Self::new(origin, installed, staging, key)
    }
    /// Explicit key injection for tests/enterprise builds; never obtain it from HTTP.
    pub fn new(origin: &str, installed: &str, staging: PathBuf, key: [u8; 32]) -> Result<Self> {
        let origin = url::Url::parse(origin).map_err(|_| Error::Invalid("update origin"))?;
        let loopback = matches!(origin.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        if !(origin.scheme() == "https" || (origin.scheme() == "http" && loopback))
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(Error::Invalid(
                "HTTPS update origin required (loopback HTTP only for tests)",
            ));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30 * 60))
            .build()?;
        Ok(Self {
            client,
            origin,
            key,
            installed: semver::Version::parse(installed)
                .map_err(|_| Error::Invalid("installed semver"))?,
            staging,
            download_gate: Default::default(),
        })
    }
    pub fn verify(&self, envelope: SignedUpdateManifest) -> Result<VerifiedUpdate> {
        if envelope.manifest.len() > MAX_MANIFEST || envelope.signature.len() > 128 {
            return Err(Error::Invalid("manifest size"));
        }
        let signature = STANDARD
            .decode(&envelope.signature)
            .map_err(|_| Error::Invalid("signature base64"))?;
        UnparsedPublicKey::new(&ED25519, self.key)
            .verify(envelope.manifest.as_bytes(), &signature)
            .map_err(|_| Error::Invalid("manifest signature"))?;
        // Only now may fields influence URLs, paths or version ordering.
        let manifest: UpdateManifest = serde_json::from_str(&envelope.manifest)?;
        let version = semver::Version::parse(&manifest.version)
            .map_err(|_| Error::Invalid("update semver"))?;
        if version <= self.installed
            || manifest.version.len() > 64
            || manifest.target != TARGET
            || manifest.byte_size == 0
            || manifest.byte_size > MAX_INSTALLER
            || manifest.sha256.len() != 64
            || !manifest
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid("update version/target/size/hash"));
        }
        let expected = self
            .origin
            .join(&format!("api/update/artifact?sha256={}", manifest.sha256))
            .map_err(|_| Error::Invalid("artifact URL"))?;
        if manifest.url != expected.as_str() {
            return Err(Error::Invalid(
                "artifact must match configured origin and content-addressed route",
            ));
        }
        Ok(VerifiedUpdate { envelope, manifest })
    }
    pub async fn check(&self) -> Result<Option<VerifiedUpdate>> {
        let response = self
            .client
            .get(
                self.origin
                    .join("api/update/latest")
                    .expect("constant path"),
            )
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let mut response = response.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_MANIFEST * 2 {
                return Err(Error::Invalid("manifest response size"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope: SignedUpdateManifest = serde_json::from_slice(&bytes)?;
        // A current/older release is a normal no-update result, but authenticate first.
        match self.verify(envelope) {
            Ok(v) => {
                let failed = tokio::fs::read(self.staging.join("failed-update.json"))
                    .await
                    .ok()
                    .filter(|b| b.len() <= MAX_MANIFEST * 2)
                    .and_then(|b| serde_json::from_slice::<SignedUpdateManifest>(&b).ok())
                    .and_then(|e| self.verify(e).ok());
                if failed.is_some_and(|f| {
                    f.manifest.version == v.manifest.version
                        && f.manifest.sha256 == v.manifest.sha256
                }) {
                    Ok(None)
                } else {
                    Ok(Some(v))
                }
            }
            Err(Error::Invalid("update version/target/size/hash")) => {
                // Do not turn invalid target/hash/size into a silent success.
                let e: SignedUpdateManifest = serde_json::from_slice(&bytes)?;
                let m: UpdateManifest = serde_json::from_str(&e.manifest)?;
                if semver::Version::parse(&m.version).is_ok_and(|v| v <= self.installed) {
                    Ok(None)
                } else {
                    Err(Error::Invalid("update version/target/size/hash"))
                }
            }
            Err(e) => Err(e),
        }
    }
    /// One timer, immediate launch tick, then six-hour sleeps; host spawns this on
    /// its existing runtime. Dropping/cancelling the future stops all work.
    /// Backpressure is bounded: a full UI queue never causes another polling task.
    pub async fn watch(&self, events: tokio::sync::mpsc::Sender<Result<Option<VerifiedUpdate>>>) {
        let mut timer = tokio::time::interval(CHECK_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = events.closed() => break, _ = timer.tick() => {} }
            let result = tokio::select! {
                _ = events.closed() => break,
                result = self.check() => result,
            };
            if events.send(result).await.is_err() {
                break;
            }
        }
    }
    /// Streaming bounded-memory download; hash-named partials survive cancellation.
    /// The server currently may ignore Range: a 200 response safely restarts at zero.
    pub async fn download(&self, update: &VerifiedUpdate) -> Result<StagedUpdate> {
        let _guard = self.download_gate.lock().await;
        // VerifiedUpdate can only be made through verification; recheck for this key/origin.
        let update = self.verify(update.envelope.clone())?;
        let m = &update.manifest;
        tokio::fs::create_dir_all(&self.staging).await?;
        self.remove_obsolete_artifacts(&m.sha256).await?;
        let part = self.staging.join(format!("{}.part", m.sha256));
        let final_path = self.staging.join(format!("{}.exe", m.sha256));
        if verify_file(&final_path, m).await.is_ok() {
            return self.commit_stage(&update, final_path).await;
        }
        let mut offset = tokio::fs::metadata(&part)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if offset > m.byte_size {
            tokio::fs::remove_file(&part).await?;
            offset = 0;
        }
        if offset != m.byte_size {
            let mut request = self.client.get(&m.url);
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let mut response = request.send().await?.error_for_status()?;
            let append = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            if append {
                let expected = format!("bytes {offset}-{}/{}", m.byte_size - 1, m.byte_size);
                if response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(&expected)
                {
                    return Err(Error::Invalid("resume Content-Range"));
                }
            } else if response.status() == reqwest::StatusCode::OK {
                offset = 0;
            } else {
                return Err(Error::Invalid("artifact status"));
            }
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(append)
                .truncate(!append)
                .open(&part)
                .await?;
            while let Some(chunk) = response.chunk().await? {
                offset = offset
                    .checked_add(chunk.len() as u64)
                    .ok_or(Error::Invalid("artifact overflow"))?;
                if offset > m.byte_size {
                    return Err(Error::Invalid("artifact larger than signed size"));
                }
                file.write_all(&chunk).await?;
            }
            file.sync_all().await?;
        }
        if let Err(e) = verify_file(&part, m).await {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        tokio::fs::rename(&part, &final_path).await?;
        self.commit_stage(&update, final_path).await
    }
    async fn remove_obsolete_artifacts(&self, keep_hash: &str) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.staging).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((stem, extension)) = name.rsplit_once('.') else {
                continue;
            };
            let hash_file = stem.len() == 64
                && stem.bytes().all(|c| c.is_ascii_hexdigit())
                && matches!(extension, "exe" | "part")
                && stem != keep_hash;
            let runner = stem
                .strip_prefix("apply-")
                .is_some_and(|pid| pid.parse::<u32>().is_ok())
                && extension == "exe";
            if hash_file || runner {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        Ok(())
    }
    async fn commit_stage(
        &self,
        update: &VerifiedUpdate,
        installer: PathBuf,
    ) -> Result<StagedUpdate> {
        let tmp = self.staging.join("pending.tmp");
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(&serde_json::to_vec(&update.envelope)?)
            .await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(tmp, self.staging.join("pending.json")).await?;
        Ok(StagedUpdate {
            installer,
            version: update.manifest.version.clone(),
        })
    }
    /// Call before handing the stage to the restart helper, and in the helper after
    /// waiting for the parent. Never persist or trust a naked installer path.
    pub async fn pending(&self) -> Result<Option<StagedUpdate>> {
        let path = self.staging.join("pending.json");
        let mut f = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        (&mut f)
            .take((MAX_MANIFEST * 2 + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > MAX_MANIFEST * 2 {
            return Err(Error::Invalid("pending manifest size"));
        }
        let update = self.verify(serde_json::from_slice(&bytes)?)?;
        let installer = self.staging.join(format!("{}.exe", update.manifest.sha256));
        verify_file(&installer, &update.manifest).await?;
        Ok(Some(StagedUpdate {
            installer,
            version: update.manifest.version,
        }))
    }
    pub async fn clear_pending(&self) -> Result<()> {
        tokio::fs::remove_file(self.staging.join("pending.json")).await?;
        Ok(())
    }
    /// Only call after user confirmation. Copy the helper out of the install dir
    /// so NSIS can replace it. The host must then exit promptly (two-minute limit).
    /// No automatic restart or execution occurs in check/download/pending/watch.
    #[cfg(windows)]
    pub async fn restart_to_apply(&self, installed_exe: &Path) -> Result<()> {
        use std::os::windows::process::CommandExt;
        self.pending()
            .await?
            .ok_or(Error::Invalid("no staged update"))?;
        if !installed_exe.is_absolute()
            || installed_exe.file_name().and_then(|n| n.to_str()) != Some("sticktalk.exe")
        {
            return Err(Error::Invalid("installed executable path"));
        }
        let helper = installed_exe.with_file_name("sticktalk-maintenance.exe");
        let runner = self
            .staging
            .join(format!("apply-{}.exe", std::process::id()));
        tokio::fs::copy(helper, &runner).await?;
        std::process::Command::new(runner)
            .arg("apply")
            .arg(self.origin.as_str())
            .arg(self.installed.to_string())
            .arg(&self.staging)
            .arg(std::process::id().to_string())
            .arg(installed_exe)
            .arg("--confirmed")
            .creation_flags(0x08000000)
            .spawn()?;
        Ok(())
    }
}
async fn verify_file(path: &Path, manifest: &UpdateManifest) -> Result<()> {
    let mut file = tokio::fs::File::open(path).await?;
    if file.metadata().await?.len() != manifest.byte_size {
        return Err(Error::Invalid("installer size"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != manifest.sha256 {
        return Err(Error::Invalid("installer SHA-256"));
    }
    Ok(())
}
