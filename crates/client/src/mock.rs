//! Native fixture facade: only wall clock, file reads and thumbnail writes live here.
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};
pub use sticktalk_client_core::mock::{user, PageKey};
use sticktalk_proto::*;
pub fn now() -> u64 {
    use sticktalk_client_core::io::Clock;
    crate::platform::Native.unix_ms()
}
pub struct Mock {
    inner: sticktalk_client_core::mock::Mock,
    pub originals: BTreeMap<Id, PathBuf>,
}
impl Default for Mock {
    fn default() -> Self {
        Self::new()
    }
}
impl Mock {
    pub fn new() -> Self {
        Self {
            inner: sticktalk_client_core::mock::Mock::with_clock(Arc::new(crate::platform::Native)),
            originals: BTreeMap::new(),
        }
    }
    pub fn upload(
        &mut self,
        path: &std::path::Path,
        purpose: UploadPurpose,
        config: &crate::backend::Config,
    ) -> Result<UploadResponse, String> {
        let meta = std::fs::metadata(path).map_err(|_| "Could not read file".to_string())?;
        if meta.len() > MAX_UPLOAD_BYTES {
            return Err("The file exceeds 25 MB".into());
        }
        let id = self.inner.id();
        let mut a = Attachment {
            id,
            filename: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            content_type: "application/octet-stream".into(),
            byte_size: meta.len(),
            width: None,
            height: None,
            thumbnail: None,
            meta: Default::default(),
        };
        if let Ok(reader) = image::ImageReader::open(path).and_then(|v| v.with_guessed_format()) {
            if matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
            ) {
                let mut reader = reader;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(16384);
                limits.max_image_height = Some(16384);
                limits.max_alloc = Some(64 * 1024 * 1024);
                reader.limits(limits);
                let decoded = reader
                    .decode()
                    .map_err(|_| "Image cannot be decoded safely".to_string())?;
                if decoded.width() as u64 * decoded.height() as u64 > 16_000_000 {
                    return Err("Image exceeds 16 million pixels".into());
                }
                let thumb = decoded.thumbnail(256, 256).to_rgb8();
                let mut bytes = vec![];
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
                    .encode_image(&thumb)
                    .map_err(|_| "Could not create thumbnail".to_string())?;
                let dir = crate::media::namespace(config, Id(1));
                std::fs::create_dir_all(&dir)
                    .map_err(|_| "Could not create mock cache".to_string())?;
                std::fs::write(dir.join(format!("{}.thumb", id.0)), &bytes)
                    .map_err(|_| "Could not cache thumbnail".to_string())?;
                a.width = Some(decoded.width());
                a.height = Some(decoded.height());
                a.content_type = "image/jpeg".into();
                a.thumbnail = Some(Thumbnail {
                    width: thumb.width() as u16,
                    height: thumb.height() as u16,
                    byte_size: bytes.len() as u32,
                    content_type: "image/jpeg".into(),
                });
            }
        }
        if purpose != UploadPurpose::Message && a.thumbnail.is_none() {
            return Err("Avatars and icons must be PNG, JPEG or WebP images".into());
        }
        self.originals.insert(id, path.to_path_buf());
        self.inner.upload(a, purpose)
    }
}
impl Deref for Mock {
    type Target = sticktalk_client_core::mock::Mock;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for Mock {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
#[derive(Clone)]
pub struct MockAuth(sticktalk_client_core::mock::MockAuth);
impl Default for MockAuth {
    fn default() -> Self {
        Self(sticktalk_client_core::mock::MockAuth::with_clock(Arc::new(
            crate::platform::Native,
        )))
    }
}
impl Deref for MockAuth {
    type Target = sticktalk_client_core::mock::MockAuth;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for MockAuth {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
