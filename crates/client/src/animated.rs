//! Bounded small-image animation. Video renditions are not software-decoded.
use iced::widget::image::Handle;
use std::sync::Arc;
#[derive(Debug, Clone)]
pub struct Frames {
    pub images: Vec<(Handle, u64)>,
    pub bytes: usize,
}
#[derive(Default)]
pub struct Animation {
    pub frames: Option<Arc<Frames>>,
    pub index: usize,
    pub active: bool,
    pub failed: bool,
    pub task: Option<iced::task::Handle>,
}
impl Animation {
    pub fn frame(&self) -> Option<&Handle> {
        self.frames
            .as_ref()
            .and_then(|f| f.images.get(self.index))
            .map(|v| &v.0)
    }
    pub fn delay(&self) -> u64 {
        self.frames
            .as_ref()
            .and_then(|f| f.images.get(self.index))
            .map_or(100, |v| v.1)
    }
}
pub async fn load(url: String) -> Result<Arc<Frames>, String> {
    if !crate::gifs::inline_url(&url) {
        return Err("Unsupported GIF origin".into());
    }
    if url.ends_with(".mp4") || url.ends_with(".webm") {
        return Err("Video GIF preview requires the hardware container player".into());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "GIF download unavailable")?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|_| "Could not load GIF")?;
    if !response.status().is_success() {
        return Err("GIF is no longer available".into());
    }
    let bytes = crate::backend::limited(response, 8 * 1024 * 1024).await?;
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|_| "GIF decoder stopped")?
}
pub fn decode(bytes: &[u8]) -> Result<Arc<Frames>, String> {
    use image::{AnimationDecoder, ImageDecoder};
    let reader = std::io::Cursor::new(bytes);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    let frames = if bytes.starts_with(b"GIF8") {
        let mut d = image::codecs::gif::GifDecoder::new(reader).map_err(|_| "Invalid GIF")?;
        d.set_limits(limits)
            .map_err(|_| "GIF exceeds decoder budget")?;
        d.into_frames()
    } else {
        let mut d = image::codecs::webp::WebPDecoder::new(reader).map_err(|_| "Invalid WebP")?;
        d.set_limits(limits)
            .map_err(|_| "WebP exceeds decoder budget")?;
        d.into_frames()
    };
    let mut images = Vec::new();
    let mut retained = 0;
    for f in frames.take(120) {
        let f = f.map_err(|_| "Invalid animation frame")?;
        let (num, den) = f.delay().numer_denom_ms();
        let delay = (num as u64 / den.max(1) as u64).clamp(20, 10000);
        let image = image::DynamicImage::ImageRgba8(f.into_buffer())
            .thumbnail(320, 240)
            .into_rgba8();
        let n = image.as_raw().len();
        if retained + n > 4 * 1024 * 1024 {
            break;
        }
        retained += n;
        images.push((
            Handle::from_rgba(image.width(), image.height(), image.into_raw()),
            delay,
        ));
    }
    if images.is_empty() {
        return Err("Animation is empty".into());
    }
    Ok(Arc::new(Frames {
        images,
        bytes: retained,
    }))
}
