//! Resumable upload orchestration; sources stream from native files or browser Blobs.
use crate::{
    http::{decode_json, Body, ByteStream, Http, HttpTransport},
    io::{MaybeSend, MaybeSync, Task},
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use sticktalk_proto::*;
pub trait UploadSource: MaybeSend + MaybeSync {
    fn filename(&self) -> String;
    fn byte_size(&self) -> Task<'_, Result<u64, String>>;
    /// Stream at most count bytes from offset; never load the entire source.
    fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<ByteStream, String>>;
}
/// Reject malformed offsets before seeking or allocating. Used by both desktop and web.
pub fn validate_state(state: &ResumableUpload, size: u64) -> Result<(), String> {
    if state.byte_size != size
        || state.offset > size
        || !(1..=16 * 1024 * 1024).contains(&state.chunk_size)
        || state.upload_id.is_empty()
        || state.upload_id.len() > 128
        || !state
            .upload_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid resumable upload state".into());
    }
    Ok(())
}
impl<A: HttpTransport> Http<A> {
    pub async fn upload_resumable(
        &self,
        source: &impl UploadSource,
        purpose: UploadPurpose,
        channel: Option<Id>,
    ) -> Result<UploadResponse, String> {
        let len = source.byte_size().await?;
        let mut hash = Sha256::new();
        let mut stream = source.chunks(0, len).await?;
        while let Some(chunk) = stream.next().await {
            hash.update(chunk?);
        }
        let upload = self
            .post(
                "/uploads/resumable",
                &CreateUploadRequest {
                    purpose,
                    channel_id: channel,
                    filename: source.filename(),
                    content_type: "application/octet-stream".into(),
                    byte_size: len,
                    sha256: format!("{:x}", hash.finalize()),
                },
            )
            .await?;
        self.resume_upload(source, upload).await
    }
    pub async fn resume_upload(
        &self,
        source: &impl UploadSource,
        upload: ResumableUpload,
    ) -> Result<UploadResponse, String> {
        let size = source.byte_size().await?;
        validate_state(&upload, size)?;
        let route = format!("/uploads/resumable/{}", upload.upload_id);
        let mut upload: ResumableUpload = self.json(&route).await?;
        validate_state(&upload, size)?;
        if let Some(response) = upload.completed {
            return Ok(response);
        }
        while upload.offset < upload.byte_size {
            let previous = upload.offset;
            let count = (upload.byte_size - upload.offset).min(u64::from(upload.chunk_size));
            let chunks = source.chunks(upload.offset, count).await?;
            upload = decode_json(
                self.execute_with_error(
                    "PUT",
                    &format!("/api/v1{route}"),
                    Body::Stream {
                        chunks,
                        length: count,
                    },
                    vec![
                        ("Upload-Offset".into(), upload.offset.to_string()),
                        ("Content-Length".into(), count.to_string()),
                    ],
                    3_600_000,
                    "Upload interrupted; retry to resume",
                )
                .await?,
            )?;
            validate_state(&upload, size)?;
            if upload.offset != previous + count {
                return Err("Server returned an incorrect upload offset".into());
            }
        }
        decode_json(
            self.execute_with_error(
                "POST",
                &format!("/api/v1{route}/finish"),
                Body::Empty,
                vec![],
                3_600_000,
                "Could not finish upload",
            )
            .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpRequest, HttpResponse};
    use futures_util::FutureExt;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Peer {
        offset: Arc<Mutex<u64>>,
        puts: Arc<Mutex<Vec<u64>>>,
        regress: bool,
        full: bool,
    }
    fn state(offset: u64) -> ResumableUpload {
        ResumableUpload {
            upload_id: "resume-id".into(),
            offset,
            byte_size: 8,
            chunk_size: 3,
            expires_at: 999,
            completed: None,
        }
    }
    fn response(value: &impl serde::Serialize) -> HttpResponse {
        HttpResponse {
            status: 200,
            bytes: serde_json::to_vec(value).unwrap(),
        }
    }
    impl HttpTransport for Peer {
        fn execute(&self, r: HttpRequest) -> Task<'_, Result<HttpResponse, String>> {
            Box::pin(async move {
                if r.method == "GET" {
                    return Ok(response(&state(*self.offset.lock().unwrap())));
                }
                if r.method == "PUT" {
                    if self.full {
                        return Ok(HttpResponse {
                            status: 507,
                            bytes: serde_json::to_vec(&ApiError {
                                code: ErrorCode::Unavailable,
                                message: "Server storage is full.".into(),
                                retry_after_ms: None,
                                field: None,
                            })
                            .unwrap(),
                        });
                    }
                    let offset = r
                        .headers
                        .iter()
                        .find(|(k, _)| k == "Upload-Offset")
                        .unwrap()
                        .1
                        .parse::<u64>()
                        .unwrap();
                    assert_eq!(offset, *self.offset.lock().unwrap());
                    self.puts.lock().unwrap().push(offset);
                    let Body::Stream { mut chunks, length } = r.body else {
                        panic!("stream required")
                    };
                    let mut count = 0;
                    while let Some(chunk) = chunks.next().await {
                        count += chunk?.len() as u64;
                    }
                    assert_eq!(count, length);
                    if !self.regress {
                        *self.offset.lock().unwrap() += count;
                    }
                    return Ok(response(&state(*self.offset.lock().unwrap())));
                }
                assert!(r.url.ends_with("/finish"));
                assert_eq!(*self.offset.lock().unwrap(), 8);
                Ok(response(&UploadResponse {
                    expires_at: 999,
                    attachment: Attachment {
                        id: Id(4),
                        filename: "file".into(),
                        content_type: "application/octet-stream".into(),
                        byte_size: 8,
                        width: None,
                        height: None,
                        thumbnail: None,
                        meta: Default::default(),
                    },
                }))
            })
        }
    }
    #[derive(Default)]
    struct Source {
        reads: Mutex<Vec<(u64, u64)>>,
    }
    impl UploadSource for Source {
        fn filename(&self) -> String {
            "file".into()
        }
        fn byte_size(&self) -> Task<'_, Result<u64, String>> {
            Box::pin(async { Ok(8) })
        }
        fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<ByteStream, String>> {
            self.reads.lock().unwrap().push((offset, count));
            Box::pin(async move {
                Ok(
                    Box::pin(futures_util::stream::iter([Ok(vec![7_u8; count as usize])]))
                        as ByteStream,
                )
            })
        }
    }
    #[test]
    fn restart_uses_server_offset_not_stale_checkpoint() {
        let peer = Peer::default();
        *peer.offset.lock().unwrap() = 3;
        let http = Http::new("http://127.0.0.1:1", "not-on-disk", peer.clone()).unwrap();
        let source = Source::default();
        let saved = serde_json::to_vec(&state(0)).unwrap();
        let restored = serde_json::from_slice(&saved).unwrap();
        let result = http
            .resume_upload(&source, restored)
            .now_or_never()
            .unwrap()
            .unwrap();
        assert_eq!(result.attachment.byte_size, 8);
        assert_eq!(*peer.puts.lock().unwrap(), [3, 6]);
        assert_eq!(*source.reads.lock().unwrap(), [(3, 3), (6, 2)]);
    }
    #[test]
    fn malformed_ack_cannot_spin_and_storage_full_is_preserved() {
        for (full, regress, expected) in [
            (false, true, "Server returned an incorrect upload offset"),
            (true, false, "Server storage is full."),
        ] {
            let peer = Peer {
                full,
                regress,
                ..Default::default()
            };
            let http = Http::new("http://127.0.0.1:1", "", peer.clone()).unwrap();
            let error = http
                .resume_upload(&Source::default(), state(0))
                .now_or_never()
                .unwrap()
                .unwrap_err();
            assert_eq!(error, expected);
            assert!(peer.puts.lock().unwrap().len() <= 1);
        }
    }
}
