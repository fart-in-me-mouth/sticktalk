//! Desktop adapters only: Tokio clock/tasks, tungstenite, reqwest and streaming files.
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};
use sticktalk_client_core::{
    http::{Body, ByteStream, HttpRequest, HttpResponse, HttpTransport},
    io::{Clock, Entropy, MaybeSend, Spawner, Task},
    protocol::{ConnectError, Frame, Incoming, Outgoing, Transport},
    upload::UploadSource,
};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};
#[derive(Clone, Copy)]
pub struct Native;
impl Clock for Native {
    fn monotonic_ms(&self) -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
    fn unix_ms(&self) -> u64 {
        if let Ok(value) = std::env::var("STICKTALK_TEST_UNIX_MS") {
            if std::env::args().any(|arg| matches!(arg.as_str(), "--screenshots" | "--capture")) {
                if let Ok(value) = value.parse() {
                    return value;
                }
            }
        }

        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
    fn sleep(&self, ms: u64) -> Task<'static, ()> {
        Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
    }
}
impl Entropy for Native {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), String> {
        getrandom::fill(bytes).map_err(|_| "Could not start secure sign-in".into())
    }
}
struct AbortTask<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl Spawner for Native {
    fn spawn<T: MaybeSend + 'static>(
        &self,
        task: Task<'static, T>,
    ) -> Task<'static, Result<T, String>> {
        let mut handle = AbortTask(tokio::spawn(task));
        Box::pin(async move {
            (&mut handle.0)
                .await
                .map_err(|_| "Network task cancelled".into())
        })
    }
}
impl Transport for Native {
    fn connect(&self, url: String) -> Task<'_, Result<(Outgoing, Incoming), ConnectError>> {
        Box::pin(async move {
            let mut request = url
                .into_client_request()
                .map_err(|_| "Invalid socket URL".to_string())?;
            request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", "sticktalk.v1".parse().unwrap());
            let (socket, response) = tokio_tungstenite::connect_async_with_config(
                request,
                Some(
                    tungstenite::protocol::WebSocketConfig::default()
                        .max_message_size(Some(sticktalk_proto::MAX_WS_FRAME_BYTES as usize))
                        .max_frame_size(Some(sticktalk_proto::MAX_WS_FRAME_BYTES as usize)),
                ),
                false,
            )
            .await
            .map_err(|_| "Connection failed".to_string())?;
            if response
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|v| v.to_str().ok())
                != Some("sticktalk.v1")
            {
                return Err(ConnectError::Stop(
                    "Server did not negotiate sticktalk.v1".into(),
                ));
            }
            let (write, read) = socket.split();
            let write = write
                .sink_map_err(|_| "Socket send failed".to_string())
                .with(|frame| async move {
                    Ok(match frame {
                        Frame::Binary(v) => Message::Binary(v.into()),
                        Frame::Pong(v) => Message::Pong(v.into()),
                        Frame::Ping(v) => Message::Ping(v.into()),
                        Frame::Close(code) => {
                            Message::Close(code.map(|code| tungstenite::protocol::CloseFrame {
                                code: code.into(),
                                reason: "".into(),
                            }))
                        }
                        _ => return Err("Invalid outbound frame".into()),
                    })
                });
            let read = read.map(|frame| {
                frame
                    .map(|frame| match frame {
                        Message::Binary(v) => Frame::Binary(v.to_vec()),
                        Message::Close(v) => Frame::Close(v.map(|v| u16::from(v.code))),
                        Message::Ping(v) => Frame::Ping(v.to_vec()),
                        Message::Pong(v) => Frame::Pong(v.to_vec()),
                        Message::Text(_) => Frame::Text,
                        _ => Frame::Other,
                    })
                    .map_err(|_| "Socket receive failed".into())
            });
            Ok((Box::pin(write) as Outgoing, Box::pin(read) as Incoming))
        })
    }
}
#[derive(Clone)]
pub struct NativeHttp(pub reqwest::Client);
impl HttpTransport for NativeHttp {
    fn execute(&self, request: HttpRequest) -> Task<'_, Result<HttpResponse, String>> {
        Box::pin(async move {
            let safe_read = request.method == "GET" && matches!(&request.body, Body::Empty);
            let mut retry = sticktalk_client_core::http::ReadRetry::new(safe_read);
            let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
            let mut builder = self
                .0
                .request(
                    reqwest::Method::from_bytes(request.method.as_bytes())
                        .map_err(|_| "Invalid HTTP method".to_string())?,
                    request.url,
                )
                .timeout(Duration::from_millis(request.timeout_ms));
            for (key, value) in request.headers {
                builder = builder.header(key, value);
            }
            builder =
                match request.body {
                    Body::Empty => builder,
                    Body::Bytes(v) => builder.body(v),
                    Body::Stream { chunks, length } => builder
                        .header("Content-Length", length)
                        .body(reqwest::Body::wrap_stream(
                            chunks.map_err(std::io::Error::other),
                        )),
                };
            loop {
                let next = if safe_read { builder.try_clone() } else { None };
                let response = builder
                    .timeout(deadline.saturating_duration_since(Instant::now()))
                    .send()
                    .await
                    .map_err(|_| request.network_error.to_string())?;
                let status = response.status().as_u16();
                let bytes = crate::backend::limited(response, request.max_response).await?;
                let response = HttpResponse { status, bytes };
                if let (Some(delay), Some(next)) = (retry.delay(&response), next) {
                    if deadline.saturating_duration_since(Instant::now())
                        > Duration::from_millis(delay)
                    {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        builder = next;
                        continue;
                    }
                }
                return Ok(response);
            }
        })
    }
}
pub struct FileSource(pub PathBuf);
impl UploadSource for FileSource {
    fn filename(&self) -> String {
        self.0
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
    fn byte_size(&self) -> Task<'_, Result<u64, String>> {
        Box::pin(async move {
            tokio::fs::metadata(&self.0)
                .await
                .map(|v| v.len())
                .map_err(|_| "Could not read file size".into())
        })
    }
    fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<ByteStream, String>> {
        Box::pin(async move {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            let mut file = tokio::fs::File::open(&self.0)
                .await
                .map_err(|_| "Could not open selected file".to_string())?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|_| "Could not seek file".to_string())?;
            Ok(Box::pin(
                tokio_util::io::ReaderStream::with_capacity(file.take(count), 65536).map(|v| {
                    v.map(|v| v.to_vec())
                        .map_err(|_| "Could not read file".into())
                }),
            ) as ByteStream)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dropping_a_spawned_task_cancels_its_work() {
        struct Done(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Done {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let (started, ready) = tokio::sync::oneshot::channel();
        let (done, ended) = tokio::sync::oneshot::channel();
        let task = Native.spawn(Box::pin(async move {
            let _done = Done(Some(done));
            let _ = started.send(());
            std::future::pending::<()>().await;
        }));
        ready.await.unwrap();
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), ended)
            .await
            .unwrap()
            .unwrap();
    }
}
