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
        let route = format!("/uploads/resumable/{}", upload.upload_id);
        let mut upload: ResumableUpload = self.json(&route).await?;
        if let Some(response) = upload.completed {
            return Ok(response);
        }
        while upload.offset < upload.byte_size {
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
                    30_000,
                    "Upload interrupted; retry to resume",
                )
                .await?,
            )?;
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
