//! Transport boundary: one iced/Tokio executor, one WebSocket, bounded queues.
use crate::{mock::Mock, model::ConnectionState};
use futures_util::{SinkExt, Stream, StreamExt};
use iced::{futures::channel::mpsc, stream};
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::{path::PathBuf, sync::Arc, time::Duration};
pub use sticktalk_client_core::backend::{decode, AccountAction, Config, HistoryAnchor, Session};
use sticktalk_proto::*;
use tokio::sync::{mpsc as channel, OwnedSemaphorePermit, Semaphore};
pub type Request = sticktalk_client_core::backend::Request<PathBuf>;
pub type Data = sticktalk_client_core::backend::Data<Handle>;
#[derive(Debug)]
struct Outbound {
    request: Request,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug, Clone)]
pub struct Handle {
    sender: channel::Sender<Outbound>,
    budget: Arc<Semaphore>,
}
impl Handle {
    pub fn try_send(&self, request: Request) -> Result<(), String> {
        let bytes = match &request {
            Request::Voice(v) => rmp_serde::to_vec_named(&ClientMsg::Voice(v.clone()))
                .map_err(|_| "Could not encode voice signal".to_string())?
                .len(),
            Request::Command(v) => rmp_serde::to_vec_named(&ClientMsg::Command(v.clone()))
                .map_err(|_| "Could not encode action".to_string())?
                .len(),
            Request::Upload { path, .. } | Request::SaveOriginal { path, .. } => {
                path.to_string_lossy().len() + 512
            }
            _ => 1024,
        };
        if bytes > MAX_WS_FRAME_BYTES as usize {
            return Err("Action exceeds the protocol frame limit".into());
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(bytes.max(1) as u32)
            .map_err(|_| "The 2 MiB network queue is full. Please wait.".to_string())?;
        self.sender
            .try_send(Outbound {
                request,
                _permit: permit,
            })
            .map_err(|_| "Connection busy or unavailable. Please retry.".into())
    }
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub data: Arc<Data>,
    _permit: Arc<OwnedSemaphorePermit>,
}
async fn emit(output: &mut mpsc::Sender<Delivery>, budget: &Arc<Semaphore>, data: Data) -> bool {
    // Every payload-bearing variant contributes its actual named-map size;
    // small-control estimates must not undercount bulk users or error strings.
    let encoded =
        |v: Result<Vec<u8>, rmp_serde::encode::Error>| v.map_or(2 * 1024 * 1024 + 1, |v| v.len());
    let bytes = match &data {
        Data::Launch { data, .. } => encoded(match data {
            sticktalk_client_core::launch::SnapshotData::Profile(v) => rmp_serde::to_vec_named(v),
            sticktalk_client_core::launch::SnapshotData::Pins(_, v) => rmp_serde::to_vec_named(v),
            sticktalk_client_core::launch::SnapshotData::Overwrites(_, v) => {
                rmp_serde::to_vec_named(v)
            }
            sticktalk_client_core::launch::SnapshotData::Extent(v) => rmp_serde::to_vec_named(v),
        }),
        Data::Frame(v) => encoded(rmp_serde::to_vec_named(v)),
        Data::History { page, .. }
        | Data::AroundHistory { page, .. }
        | Data::NewerHistory { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Members { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Invites { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Bans { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Channel { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Users { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Message { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Uploaded { response, .. } => encoded(rmp_serde::to_vec_named(response)),
        Data::Error(v)
        | Data::Rejected { reason: v, .. }
        | Data::State(ConnectionState::Stopped(v)) => v.len(),
        Data::Uncertain(v) => v.len() * std::mem::size_of::<Nonce>(),
        _ => 128,
    }
    .saturating_add(128);
    if bytes > 2 * 1024 * 1024 {
        return false;
    }
    let Ok(permit) = budget.clone().acquire_many_owned(bytes as u32).await else {
        return false;
    };
    output
        .send(Delivery {
            data: Arc::new(data),
            _permit: Arc::new(permit),
        })
        .await
        .is_ok()
}
pub fn subscription(config: &Config) -> impl Stream<Item = Delivery> {
    let config = config.clone();
    stream::channel(128, move |mut output| async move {
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        let (tx, mut rx) = channel::channel::<Outbound>(128);
        if !emit(
            &mut output,
            &budget,
            Data::Connected(Handle {
                sender: tx,
                budget: Arc::new(Semaphore::new(2 * 1024 * 1024)),
            }),
        )
        .await
        {
            return;
        }
        if config.mock {
            let mut mock = Mock::new();
            let frames = mock.startup(&config);
            let dir = crate::media::namespace(&config, Id(1));
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("900001.thumb"), sticktalk_client_core::mock::IMAGE);
            if config.scenario == "image-heavy" || config.scenario == "stress-scroll" {
                for n in 0..100u64 {
                    let _ = std::fs::write(
                        dir.join(format!("{}.thumb", 910_000 + n)),
                        sticktalk_client_core::mock::IMAGE,
                    );
                }
            }
            for (index, frame) in frames.into_iter().enumerate() {
                if !emit(&mut output, &budget, Data::Frame(frame)).await {
                    return;
                }
                if index == 0 && config.scenario == "disconnected" {
                    emit(
                        &mut output,
                        &budget,
                        Data::State(ConnectionState::Backoff(4)),
                    )
                    .await;
                }
            }
            while let Some(outbound) = rx.recv().await {
                let request = outbound.request;
                if config.scenario == "loading" && matches!(request, Request::History { .. }) {
                    continue;
                }
                let data = match request {
                    Request::Command(v) => {
                        for frame in mock.command(v) {
                            if !emit(&mut output, &budget, Data::Frame(frame)).await {
                                return;
                            }
                        }
                        continue;
                    }
                    Request::Applied(_) | Request::View => continue,
                    Request::Upload {
                        form_revision,
                        path,
                        purpose,
                        channel,
                        generation,
                    } => match mock.upload(&path, purpose, &config) {
                        Ok(response) => Data::Uploaded {
                            form_revision,
                            response,
                            purpose,
                            channel,
                            generation,
                        },
                        Err(v) => Data::Error(v),
                    },
                    Request::DiscardUpload(_) => continue,
                    v if !matches!(v, Request::SaveOriginal { .. }) => {
                        let Some(data) = mock.request(v) else {
                            continue;
                        };
                        data
                    }
                    Request::SaveOriginal { attachment, path } => {
                        if attachment == Id(900001) {
                            let saved = tokio::task::spawn_blocking(move || {
                                use std::io::Write;
                                std::fs::OpenOptions::new()
                                    .write(true)
                                    .create_new(true)
                                    .open(path)?
                                    .write_all(sticktalk_client_core::mock::IMAGE)
                            })
                            .await;
                            match saved {
                                Ok(Ok(())) => Data::Saved,
                                _ => Data::Error(
                                    "Could not save original. Choose a new filename.".into(),
                                ),
                            }
                        } else if let Some(source) = mock.originals.get(&attachment) {
                            let source = source.clone();
                            let saved = tokio::task::spawn_blocking(move || {
                                use std::io::Write;
                                let mut dest = std::fs::OpenOptions::new()
                                    .create_new(true)
                                    .write(true)
                                    .open(path)?;
                                let mut file = std::fs::File::open(source)?;
                                std::io::copy(&mut file, &mut dest)?;
                                dest.flush()
                            })
                            .await;
                            match saved {
                                Ok(Ok(())) => Data::Saved,
                                _ => Data::Error(
                                    "Could not save mock original; destination must not exist"
                                        .into(),
                                ),
                            }
                        } else {
                            Data::Error("Mock original is unavailable".into())
                        }
                    }
                    _ => continue,
                };
                if !emit(&mut output, &budget, data).await {
                    return;
                }
            }
            return;
        }
        let http = match Http::new(&config.server, &config.token) {
            Ok(v) => v,
            Err(v) => {
                emit(&mut output, &budget, Data::Error(v)).await;
                return;
            }
        };
        struct Output<'a> {
            output: &'a mut mpsc::Sender<Delivery>,
            budget: &'a Arc<Semaphore>,
        }
        impl sticktalk_client_core::protocol::EventSink<Handle> for Output<'_> {
            fn emit(&mut self, data: Data) -> sticktalk_client_core::io::Task<'_, bool> {
                Box::pin(emit(self.output, self.budget, data))
            }
        }
        let requests = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|v| ((v.request, v._permit), rx))
        });
        futures_util::pin_mut!(requests);
        sticktalk_client_core::protocol::run(
            &config,
            &crate::platform::Native,
            &crate::platform::Native,
            &crate::platform::Native,
            http,
            requests,
            &mut Output {
                output: &mut output,
                budget: &budget,
            },
        )
        .await;
    })
}
impl sticktalk_client_core::protocol::RestService<PathBuf, Handle> for Http {
    fn request(
        &self,
        request: Request,
    ) -> sticktalk_client_core::io::Task<'static, Result<Data, String>> {
        let http = self.clone();
        Box::pin(async move { http.request(request).await })
    }
}
pub fn nonce() -> u64 {
    sticktalk_client_core::io::nonce(&crate::platform::Native)
}

#[derive(Clone)]
pub struct Http {
    pub base: String,
    token: String,
    pub client: Client,
}
impl Http {
    pub fn new(server: &str, token: &str) -> Result<Self, String> {
        sticktalk_client_core::http::validate_server(server)?;
        Ok(Self {
            base: server.trim_end_matches('/').to_string(),
            token: token.into(),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not initialize HTTPS".to_string())?,
        })
    }
    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}/api/v1{}", self.base, path))
            .bearer_auth(&self.token)
    }
    pub fn core(&self) -> sticktalk_client_core::http::Http<crate::platform::NativeHttp> {
        sticktalk_client_core::http::Http {
            base: self.base.clone(),
            token: self.token.clone(),
            adapter: crate::platform::NativeHttp(self.client.clone()),
        }
    }
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, String> {
        self.core().post(path, body).await
    }
    pub async fn json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.core().json(path).await
    }
    pub async fn request(&self, request: Request) -> Result<Data, String> {
        match request {
            Request::Upload {
                form_revision,
                path,
                purpose,
                channel,
                generation,
            } => {
                let response = self.upload_resumable(&path, purpose, channel).await?;
                Ok(Data::Uploaded {
                    form_revision,
                    response,
                    purpose,
                    channel,
                    generation,
                })
            }
            Request::SaveOriginal { attachment, path } => {
                self.download(attachment, path).await?;
                Ok(Data::Saved)
            }
            v => self.core().request(v).await,
        }
    }
    pub async fn upload_resumable(
        &self,
        path: &std::path::Path,
        purpose: UploadPurpose,
        channel: Option<Id>,
    ) -> Result<UploadResponse, String> {
        self.core()
            .upload_resumable(
                &crate::platform::FileSource(path.to_path_buf()),
                purpose,
                channel,
            )
            .await
    }
    pub async fn resume_upload(
        &self,
        path: &std::path::Path,
        upload: ResumableUpload,
    ) -> Result<UploadResponse, String> {
        self.core()
            .resume_upload(&crate::platform::FileSource(path.to_path_buf()), upload)
            .await
    }
    /// Compatibility path for older servers; bounded by their legacy multipart limit.
    pub async fn upload_legacy(
        &self,
        path: &std::path::Path,
        purpose: UploadPurpose,
        channel: Option<Id>,
    ) -> Result<UploadResponse, String> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| "Could not open selected file".to_string())?;
        let len = file
            .metadata()
            .await
            .map_err(|_| "Could not read file size".to_string())?
            .len();
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        let purpose = match purpose {
            UploadPurpose::Message => "message",
            UploadPurpose::Avatar => "avatar",
            UploadPurpose::GuildIcon => "guild_icon",
        };
        let mut form = reqwest::multipart::Form::new().text("purpose", purpose.to_string());
        if let Some(id) = channel {
            form = form.text("channel_id", id.0.to_string());
        }
        form = form.part(
            "file",
            reqwest::multipart::Part::stream_with_length(body, len).file_name(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        json(
            self.client
                .post(format!("{}/api/v1/uploads", self.base))
                .bearer_auth(&self.token)
                .multipart(form)
                .send()
                .await
                .map_err(|_| "Upload failed".to_string())?,
        )
        .await
    }
    pub async fn logout(&self) {
        self.core().logout().await;
    }
    pub async fn download(&self, id: Id, path: PathBuf) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let response = self
            .get(&format!("/files/{}", id.0))
            .send()
            .await
            .map_err(|_| "Download failed".to_string())?;
        if !response.status().is_success() {
            return Err("No longer allowed to download this file".into());
        }
        let mut out = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|_| "Destination already exists or cannot be written".to_string())?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Download interrupted".to_string())?;
            out.write_all(&chunk)
                .await
                .map_err(|_| "Could not save file".to_string())?;
        }
        Ok(())
    }
}
async fn json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status().as_u16();
    let bytes = limited(response, 2 * 1024 * 1024).await?;
    sticktalk_client_core::http::decode_json(sticktalk_client_core::http::HttpResponse {
        status,
        bytes,
    })
}
pub async fn limited(response: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err("Server response exceeds limit".into());
    }
    let chunks = response.bytes_stream().map(|v| {
        v.map(|v| v.to_vec())
            .map_err(|_| "Network interrupted".into())
    });
    sticktalk_client_core::http::limited(Box::pin(chunks), max).await
}
pub async fn authenticate(
    server: String,
    username: String,
    password: String,
    register: bool,
    display: String,
) -> Result<Session, String> {
    let http = Http::new(&server, "")?;
    let auth = authenticate_unstored(&server, username, password, register, display).await?;
    let token = auth.token.clone();
    let saved_server = http.base.clone();
    tokio::task::spawn_blocking(move || {
        crate::storage::save_server(&saved_server);
        crate::storage::save_token(&saved_server, &token)
    })
    .await
    .map_err(|_| "Credential storage failed".to_string())??;
    Ok(Session {
        config: Config {
            server: http.base,
            token: auth.token,
            mock: false,
            generation: nonce(),
            scenario: String::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outgoing_queue_obeys_byte_budget_and_accepts_maximum_message() {
        let (sender, mut receiver) = channel::channel(128);
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        let handle = Handle {
            sender,
            budget: budget.clone(),
        };
        let draft = MessageDraft {
            channel_id: Id(1),
            content: "🌱".repeat(4000),
            reply_to: None,
            attachment_ids: vec![Id(2), Id(3), Id(4), Id(5)],
            mention_ids: (1..=100).map(Id).collect(),
        };
        assert!(handle
            .try_send(Request::Command(CommandRequest {
                nonce: 1,
                command: Command::SendMessage(draft)
            }))
            .is_ok());
        drop(receiver.try_recv().unwrap());
        let mut sent = 0;
        for nonce in 1..=16 {
            let request = Request::Command(CommandRequest {
                nonce,
                command: Command::SendMessage(MessageDraft {
                    channel_id: Id(1),
                    content: "x".repeat(200_000),
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![],
                }),
            });
            if handle.try_send(request).is_ok() {
                sent += 1;
            } else {
                break;
            }
        }
        assert!((2..=10).contains(&sent));
        while let Ok(packet) = receiver.try_recv() {
            drop(packet);
        }
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024);
    }
    #[test]
    fn requires_secure_server() {
        assert!(Http::new("http://example.com", "").is_err());
        assert!(Http::new("http://127.0.0.1:8080", "").is_ok());
        assert!(Http::new("https://example.com", "").is_ok());
        assert!(Http::new("https://user:pass@example.com", "").is_err());
    }
    #[tokio::test]
    async fn inbound_budget_counts_large_non_frame_payloads_and_releases_on_drop() {
        use futures_util::StreamExt;
        let (mut tx, mut rx) = mpsc::channel(128);
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        assert!(emit(&mut tx, &budget, Data::Error("x".repeat(32768))).await);
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024 - 32768 - 128);
        let packet = rx.next().await.unwrap();
        drop(packet);
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024);
        assert!(!emit(&mut tx, &budget, Data::Error("x".repeat(2 * 1024 * 1024))).await);
    }
}

pub type MockAccount = Arc<std::sync::Mutex<crate::mock::MockAuth>>;
pub async fn meta(server: String, mock: bool) -> Result<MetaResponse, String> {
    if mock {
        return Ok(MetaResponse {
            version: "0.1".into(),
            protocol_version: PROTOCOL_VERSION,
            google_client_id: Some("mock-desktop".into()),
            gifs: Default::default(),
        });
    }
    Http::new(&server, "")?.core().meta().await
}
pub async fn account(
    config: Config,
    action: AccountAction,
    mock: MockAccount,
) -> Result<AuthMethodsResponse, String> {
    if config.mock {
        return mock
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .action(&action);
    }
    Http::new(&config.server, &config.token)?
        .core()
        .account(action)
        .await
}
#[derive(Clone)]
pub enum GoogleOutcome {
    Session(Session),
    Signup(GoogleSignupRequired),
}
impl std::fmt::Debug for GoogleOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GoogleOutcome(<redacted>)")
    }
}
pub async fn google(
    server: String,
    id_token: String,
    mock: bool,
    fixture: MockAccount,
) -> Result<GoogleOutcome, String> {
    let response = if mock {
        fixture
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .google(&id_token)?
    } else {
        Http::new(&server, "")?.core().google(id_token).await?
    };
    match response {
        GoogleAuthResponse::Session(auth) => save_auth(server, auth, mock)
            .await
            .map(GoogleOutcome::Session),
        GoogleAuthResponse::SignupRequired(ticket) => Ok(GoogleOutcome::Signup(ticket)),
    }
}
pub async fn google_complete(
    server: String,
    ticket: String,
    username: String,
    mock: bool,
    fixture: MockAccount,
) -> Result<Session, String> {
    let auth = if mock {
        fixture
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .complete(&ticket, &username)?
    } else {
        Http::new(&server, "")?
            .core()
            .google_complete(ticket, username)
            .await?
    };
    save_auth(server, auth, mock).await
}
async fn save_auth(server: String, auth: AuthResponse, mock: bool) -> Result<Session, String> {
    let server = server.trim().trim_end_matches('/').to_owned();
    if !mock {
        let saved_server = server.clone();
        let token = auth.token.clone();
        tokio::task::spawn_blocking(move || {
            crate::storage::save_server(&saved_server);
            crate::storage::save_token(&saved_server, &token)
        })
        .await
        .map_err(|_| "Could not save session".to_string())??;
    }
    Ok(Session {
        config: Config {
            server,
            token: auth.token,
            mock,
            generation: nonce(),
            scenario: String::new(),
        },
    })
}

/// Authenticate without writing OS credentials/preferences (headless clients and tests).
pub async fn authenticate_unstored(
    server: &str,
    username: String,
    password: String,
    register: bool,
    display: String,
) -> Result<AuthResponse, String> {
    Http::new(server, "")?
        .core()
        .authenticate(username, password, register, display)
        .await
}
