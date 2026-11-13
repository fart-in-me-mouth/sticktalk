//! Durable per-account upload journal. No bearer token is persisted and sources stream in 64 KiB reads.
use crate::{
    backend::{Config, Http},
    platform::FileSource,
};
use futures_util::{SinkExt, StreamExt};
use iced::{futures::channel::mpsc, stream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use sticktalk_client_core::{
    http::{decode_json, Body},
    upload::UploadSource,
};
use sticktalk_proto::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub path: PathBuf,
    pub channel: Id,
    #[serde(default)]
    pub forward: bool,
    pub size: u64,
    pub modified_ms: u64,
    pub upload: Option<ResumableUpload>,
    pub attachment: Option<Attachment>,
}
#[derive(Clone, Debug)]
pub enum Event {
    Progress(u64, u64, u64, f64),
    Prepared(Box<Job>),
    Complete(u64, Attachment),
    Failed(u64, String),
}
#[derive(Clone)]
pub struct Worker {
    pub config: Config,
    pub account: Id,
    pub id: u64,
    pub journal: String,
}
// The journal is startup payload, not subscription identity. Publishing a newly
// created session must not cancel/restart the PUT currently using that session.
impl PartialEq for Worker {
    fn eq(&self, other: &Self) -> bool {
        self.config == other.config && self.account == other.account && self.id == other.id
    }
}
impl Eq for Worker {}
impl std::hash::Hash for Worker {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(&self.config, self.account, self.id), state);
    }
}
fn dir(config: &Config, account: Id) -> PathBuf {
    crate::media::namespace(config, account).with_file_name(format!(
        "{}-uploads",
        crate::media::namespace(config, account)
            .file_name()
            .unwrap()
            .to_string_lossy()
    ))
}
fn journal(config: &Config, account: Id, id: u64) -> PathBuf {
    dir(config, account).join(format!("{id}.json"))
}
pub fn save(config: &Config, account: Id, job: &Job) -> Result<(), String> {
    let path = journal(config, account, job.id);
    if path.with_extension("cancelled").exists() {
        return Err("Upload cancelled".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "Could not save upload queue")?;
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec(job).map_err(|_| "Could not encode upload queue")?,
    )
    .map_err(|_| "Could not save upload queue")?;
    std::fs::rename(tmp, path).map_err(|_| "Could not commit upload queue".into())
}
pub fn remove(config: &Config, account: Id, id: u64) {
    let path = journal(config, account, id);
    let _ = std::fs::write(path.with_extension("cancelled"), []);
    let _ = std::fs::remove_file(path);
}
pub fn restore(config: &Config, account: Id) -> Vec<Job> {
    std::fs::read_dir(dir(config, account))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(32)
        .filter_map(|e| {
            if e.path().with_extension("cancelled").exists() {
                return None;
            }
            let meta = e.metadata().ok()?;
            if meta.len() > 16384 || e.path().extension()?.to_str()? != "json" {
                return None;
            }
            let job: Job = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            (job.id > 0 && job.channel.0 > 0 && job.path.is_absolute()).then_some(job)
        })
        .collect()
}
pub fn preferences(config: &Config, account: Id) -> PathBuf {
    dir(config, account).join("media-preferences.json")
}
pub fn fingerprint(path: &Path) -> Result<(u64, u64), String> {
    let m =
        std::fs::metadata(path).map_err(|_| "Upload source is missing. Choose the file again.")?;
    if !m.is_file() {
        return Err("Choose a regular file".into());
    }
    let modified = m
        .modified()
        .map_err(|_| "Could not inspect file")?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Ok((m.len(), modified))
}
pub fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
pub fn subscription(worker: &Worker) -> impl futures_util::Stream<Item = Event> {
    let worker = worker.clone();
    stream::channel(2, move |mut output| async move {
        let job = serde_json::from_str(&worker.journal);
        let result = match job {
            Ok(job) => run(&worker, job, &mut output).await,
            Err(_) => Err("Upload queue is invalid".into()),
        };
        if let Err(e) = result {
            let _ = output.send(Event::Failed(worker.id, e)).await;
        }
    })
}
async fn checkpoint(worker: &Worker, job: &Job) -> Result<(), String> {
    let w = worker.clone();
    let j = job.clone();
    tokio::task::spawn_blocking(move || save(&w.config, w.account, &j))
        .await
        .map_err(|_| "Upload queue write failed")?
}
async fn run(worker: &Worker, mut job: Job, out: &mut mpsc::Sender<Event>) -> Result<(), String> {
    let path = job.path.clone();
    let current = tokio::task::spawn_blocking(move || fingerprint(&path))
        .await
        .map_err(|_| "Could not inspect file")??;
    if current != (job.size, job.modified_ms) {
        return Err(
            "The file changed since this upload started. Remove it and choose it again.".into(),
        );
    }
    let source = FileSource(job.path.clone());
    let http = Http::new(&worker.config.server, &worker.config.token)?.core();
    if job.upload.is_none() {
        let mut hash = Sha256::new();
        let mut bytes = source.chunks(0, job.size).await?;
        while let Some(chunk) = bytes.next().await {
            hash.update(chunk?);
        }
        job.upload = Some(
            http.post(
                "/uploads/resumable",
                &CreateUploadRequest {
                    purpose: UploadPurpose::Message,
                    channel_id: Some(job.channel),
                    filename: source.filename(),
                    content_type: content_type(&job.path).into(),
                    byte_size: job.size,
                    sha256: format!("{:x}", hash.finalize()),
                },
            )
            .await?,
        );
        checkpoint(worker, &job).await?;
        let _ = out.send(Event::Prepared(Box::new(job.clone()))).await;
    }
    let route = format!(
        "/uploads/resumable/{}",
        job.upload.as_ref().unwrap().upload_id
    );
    let mut state: ResumableUpload = http.json(&route).await?;
    validate(&state, job.size)?;
    if let Some(done) = state.completed.take() {
        job.attachment = Some(done.attachment.clone());
        checkpoint(worker, &job).await?;
        let _ = out.send(Event::Complete(job.id, done.attachment)).await;
        return Ok(());
    }
    let started = std::time::Instant::now();
    let initial = state.offset;
    while state.offset < job.size {
        let offset = state.offset;
        let count = (job.size - offset).min(u64::from(state.chunk_size));
        let mut progress = out.clone();
        let mut sent = 0;
        let mut last = std::time::Instant::now();
        let id = job.id;
        let size = job.size;
        let bytes = source.chunks(offset, count).await?.map(move |chunk| {
            if let Ok(bytes) = &chunk {
                sent += bytes.len() as u64;
                if last.elapsed() >= std::time::Duration::from_millis(250) || sent == count {
                    let speed = (offset + sent - initial) as f64
                        / started.elapsed().as_secs_f64().max(0.001);
                    // Coalesce UI progress, never back up the file/network stream.
                    let _ = progress.try_send(Event::Progress(id, offset + sent, size, speed));
                    last = std::time::Instant::now();
                }
            }
            chunk
        });
        let response = http
            .execute_with_error(
                "PUT",
                &format!("/api/v1{route}"),
                Body::Stream {
                    chunks: Box::pin(bytes),
                    length: count,
                },
                vec![("Upload-Offset".into(), offset.to_string())],
                3_600_000,
                "Upload interrupted. Retry to resume.",
            )
            .await?;
        state = decode_json(response)?;
        validate(&state, job.size)?;
        if state.offset != offset + count {
            return Err("Server returned an incorrect upload offset".into());
        }
        job.upload = Some(state.clone());
        checkpoint(worker, &job).await?;
        let speed = (state.offset - initial) as f64 / started.elapsed().as_secs_f64().max(0.001);
        if out
            .send(Event::Progress(job.id, state.offset, job.size, speed))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
    let done: UploadResponse = decode_json(
        http.execute_with_error(
            "POST",
            &format!("/api/v1{route}/finish"),
            Body::Empty,
            vec![],
            3_600_000,
            "Could not finish upload. Retry to resume.",
        )
        .await?,
    )?;
    job.attachment = Some(done.attachment.clone());
    job.upload = Some(state);
    checkpoint(worker, &job).await?;
    let _ = out.send(Event::Complete(job.id, done.attachment)).await;
    Ok(())
}
pub fn validate(s: &ResumableUpload, size: u64) -> Result<(), String> {
    sticktalk_client_core::upload::validate_state(s, size)
}
pub async fn cancel(
    config: Config,
    upload: Option<ResumableUpload>,
    attachment: Option<Attachment>,
) {
    if config.mock {
        return;
    }
    if let Ok(h) = Http::new(&config.server, &config.token) {
        let path = if let Some(a) = attachment {
            Some(format!("/uploads/{}", a.id.0))
        } else {
            upload.map(|u| format!("/uploads/resumable/{}", u.upload_id))
        };
        if let Some(path) = path {
            let _ = h
                .core()
                .execute(
                    "DELETE",
                    &format!("/api/v1{path}"),
                    Body::Empty,
                    vec![],
                    30000,
                )
                .await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_keeps_account_scoped_offset_and_no_token() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("file.bin");
        std::fs::write(&path, [1, 2, 3]).unwrap();
        let (size, modified_ms) = fingerprint(&path).unwrap();
        let j = Job {
            id: 4,
            path,
            channel: Id(12),
            forward: false,
            size,
            modified_ms,
            upload: Some(ResumableUpload {
                upload_id: "abc".into(),
                offset: 2,
                byte_size: 3,
                chunk_size: 8 * 1024 * 1024,
                expires_at: 999,
                completed: None,
            }),
            attachment: None,
        };
        let bytes = serde_json::to_vec(&j).unwrap();
        let copy: Job = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(copy.upload.as_ref().unwrap().offset, 2);
        assert_eq!(
            fingerprint(&copy.path).unwrap(),
            (copy.size, copy.modified_ms)
        );
        assert!(!String::from_utf8(bytes).unwrap().contains("token"));
        validate(copy.upload.as_ref().unwrap(), 3).unwrap();
        let mut state = copy.upload.unwrap();
        state.chunk_size = 0;
        assert!(validate(&state, 3).is_err());
        state.chunk_size = u32::MAX;
        assert!(validate(&state, 3).is_err());
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::hash::{Hash, Hasher};
    #[test]
    fn checkpoints_do_not_restart_an_in_flight_worker() {
        let w = Worker {
            config: Config {
                server: "http://127.0.0.1:1".into(),
                token: "private".into(),
                mock: false,
                generation: 1,
                scenario: String::new(),
            },
            account: Id(1),
            id: 9,
            journal: "before create".into(),
        };
        let mut checkpoint = w.clone();
        checkpoint.journal = "after create, offset advanced".into();
        assert!(w == checkpoint);
        let hash = |w: &Worker| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            w.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&w), hash(&checkpoint));
        checkpoint.config.generation += 1;
        assert!(w != checkpoint);
    }
}
