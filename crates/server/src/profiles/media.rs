//! Avatar and banner uploads: validated by really decoding every frame, with
//! caps on bytes, dimensions, frames and total decoded pixels, and a still
//! first-frame rendition stored as the upload's thumbnail. Lists show that
//! rendition; the original (which may move) plays only on hover or focus.
use crate::{
    error::{require, Error, Result},
    files::Media,
};
use image::{
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, RgbaImage,
};
use std::{
    collections::VecDeque,
    io::{BufReader, Read},
    path::Path,
    sync::Mutex,
};
use sticktalk_proto::*;

/// Behind transparent pixels in the still rendition: the avatar and banner
/// fallback color (`--color-brand-avatar`), which is also what shows through
/// a transparent original in the app.
const BACKDROP: [u8; 3] = [0x49, 0x6c, 0x63];
/// Recently prepared uploads that move, by storage file stem. `SetImage` reads
/// it to mark an avatar or banner animated. Bounded; after a restart an upload
/// not yet used falls back to its content type (GIF and APNG count as moving).
static ANIMATED: Mutex<VecDeque<(String, bool)>> = Mutex::new(VecDeque::new());
const REMEMBERED: usize = 4096;

fn bad() -> Error {
    Error::new(ErrorCode::UnsupportedMediaType)
}
fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
fn remember(path: &Path, animated: bool) {
    let mut list = ANIMATED.lock().unwrap_or_else(|e| e.into_inner());
    let key = stem(path);
    list.retain(|(k, _)| *k != key);
    if list.len() >= REMEMBERED {
        list.pop_front();
    }
    list.push_back((key, animated));
}
/// Whether the upload stored as `storage_key` moves, if this process prepared it.
pub(crate) fn animated(storage_key: &str) -> Option<bool> {
    let key = stem(Path::new(storage_key));
    ANIMATED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

/// Replaces `files::prepare` for every upload. Avatar uploads (which are also
/// used for banners) get the profile rules; everything else is unchanged.
pub(crate) fn prepare(path: &Path, claimed: &str, purpose: UploadPurpose) -> Result<Media> {
    if purpose != UploadPurpose::Avatar {
        return crate::files::prepare(path, claimed, purpose != UploadPurpose::Message);
    }
    let (media, animated) = profile_image(path)?;
    remember(path, animated);
    Ok(media)
}

fn sniff(head: &[u8]) -> Option<ImageFormat> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if head.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else {
        None
    }
}
fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(MAX_PROFILE_IMAGE_SIDE);
    l.max_image_height = Some(MAX_PROFILE_IMAGE_SIDE);
    l.max_alloc = Some(256 * 1024 * 1024);
    l
}
/// Decode every frame of an animation, checking the caps as it goes, and
/// return the first frame and the frame count.
fn frames<'a>(decoder: impl AnimationDecoder<'a>, w: u32, h: u32) -> Result<(RgbaImage, u32)> {
    let pixels = u64::from(w) * u64::from(h);
    require(pixels <= MAX_ANIMATED_FRAME_PIXELS, ErrorCode::PayloadTooLarge)?;
    let mut first = None;
    let mut count = 0u32;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|_| bad())?;
        count += 1;
        require(
            count <= MAX_ANIMATION_FRAMES && u64::from(count) * pixels <= MAX_ANIMATION_TOTAL_PIXELS,
            ErrorCode::PayloadTooLarge,
        )?;
        if first.is_none() {
            first = Some(frame.into_buffer());
        }
    }
    Ok((first.ok_or_else(bad)?, count))
}
fn reader(path: &Path) -> Result<BufReader<std::fs::File>> {
    Ok(BufReader::new(std::fs::File::open(path)?))
}
fn still(path: &Path, format: ImageFormat) -> Result<RgbaImage> {
    let mut r = image::ImageReader::with_format(reader(path)?, format);
    r.limits(limits());
    let mut decoder = r.into_decoder().map_err(|_| bad())?;
    let (w, h) = decoder.dimensions();
    require(
        w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 16_000_000,
        ErrorCode::UnsupportedMediaType,
    )?;
    let orientation = decoder.orientation().map_err(|_| bad())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| bad())?;
    img.apply_orientation(orientation);
    Ok(img.to_rgba8())
}
/// Validate an avatar or banner and make its still rendition.
pub(crate) fn profile_image(path: &Path) -> Result<(Media, bool)> {
    let size = std::fs::metadata(path)?.len();
    require(size <= MAX_PROFILE_IMAGE_BYTES, ErrorCode::PayloadTooLarge)?;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    let format = sniff(&head[..n]).ok_or_else(bad)?;
    let (first, count) = match format {
        ImageFormat::Gif => {
            let mut d = GifDecoder::new(reader(path)?).map_err(|_| bad())?;
            d.set_limits(limits()).map_err(|_| bad())?;
            let (w, h) = d.dimensions();
            frames(d, w, h)?
        }
        ImageFormat::Png => {
            let mut d = PngDecoder::new(reader(path)?).map_err(|_| bad())?;
            d.set_limits(limits()).map_err(|_| bad())?;
            if d.is_apng().map_err(|_| bad())? {
                let (w, h) = d.dimensions();
                frames(d.apng().map_err(|_| bad())?, w, h)?
            } else {
                (still(path, format)?, 1)
            }
        }
        ImageFormat::WebP => {
            let d = WebPDecoder::new(reader(path)?).map_err(|_| bad())?;
            if d.has_animation() {
                let (w, h) = d.dimensions();
                require(
                    w <= MAX_PROFILE_IMAGE_SIDE && h <= MAX_PROFILE_IMAGE_SIDE,
                    ErrorCode::PayloadTooLarge,
                )?;
                frames(d, w, h)?
            } else {
                (still(path, format)?, 1)
            }
        }
        _ => (still(path, format)?, 1),
    };
    let animated = count > 1;
    let (w, h) = first.dimensions();
    require(w > 0 && h > 0, ErrorCode::UnsupportedMediaType)?;
    let mime = match format {
        ImageFormat::Png if animated => "image/apng",
        f => f.to_mime_type(),
    };
    let thumbnail = rendition(&first)?;
    Ok((
        Media {
            mime: mime.into(),
            width: Some(w),
            height: Some(h),
            thumb: Some((
                thumbnail.1 as u16,
                thumbnail.2 as u16,
                thumbnail.0.len() as u32,
            )),
            thumbnail_bytes: thumbnail.0,
        },
        animated,
    ))
}
/// The still first frame as a JPEG: up to 256x256 for an avatar-shaped image,
/// up to 960x384 for a banner-shaped one (2:1 or wider).
fn rendition(first: &RgbaImage) -> Result<(Vec<u8>, u32, u32)> {
    let (w, h) = first.dimensions();
    let mut flat = image::RgbImage::new(w, h);
    for (x, y, p) in first.enumerate_pixels() {
        let a = u32::from(p[3]);
        let mix = |i: usize| ((u32::from(p[i]) * a + u32::from(BACKDROP[i]) * (255 - a)) / 255) as u8;
        flat.put_pixel(x, y, image::Rgb([mix(0), mix(1), mix(2)]));
    }
    let (bw, bh) = if w >= 2 * h { (960, 384) } else { (256, 256) };
    let t = DynamicImage::ImageRgb8(flat).thumbnail(w.min(bw), h.min(bh)).to_rgb8();
    let mut bytes = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82)
        .encode_image(&t)
        .map_err(|_| bad())?;
    require(bytes.len() <= 256 * 1024, ErrorCode::UnsupportedMediaType)?;
    Ok((bytes, t.width(), t.height()))
}
/// A custom badge icon: any still image, fitted into 64x64 PNG.
pub(crate) fn badge_icon(bytes: &[u8]) -> Result<Vec<u8>> {
    require(
        !bytes.is_empty() && bytes.len() <= MAX_BADGE_ICON_BYTES,
        ErrorCode::PayloadTooLarge,
    )?;
    let format = sniff(&bytes[..bytes.len().min(16)]).ok_or_else(bad)?;
    let mut r = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut l = limits();
    l.max_image_width = Some(1024);
    l.max_image_height = Some(1024);
    r.limits(l);
    let img = r.decode().map_err(|_| bad())?;
    let fitted = img.resize(64, 64, image::imageops::FilterType::Lanczos3);
    let mut canvas = RgbaImage::new(64, 64);
    let (x, y) = ((64 - fitted.width()) / 2, (64 - fitted.height()) / 2);
    image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), x.into(), y.into());
    let mut out = std::io::Cursor::new(vec![]);
    canvas
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| bad())?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::gif::GifEncoder, Frame, Rgba};

    fn gif(dir: &Path, name: &str, frames: u32, w: u32, h: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut bytes = vec![];
        {
            let mut e = GifEncoder::new(&mut bytes);
            for i in 0..frames {
                let shade = (i * 40 % 255) as u8;
                e.encode_frame(Frame::new(RgbaImage::from_pixel(w, h, Rgba([shade, 0, 0, 255]))))
                    .unwrap();
            }
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
    #[test]
    fn animations_are_decoded_capped_and_get_a_still_rendition() {
        let dir = tempfile::tempdir().unwrap();
        let moving = gif(dir.path(), "a.tmp", 3, 64, 64);
        let (media, animated) = profile_image(&moving).unwrap();
        assert!(animated);
        assert_eq!(media.mime, "image/gif");
        assert_eq!((media.width, media.height), (Some(64), Some(64)));
        assert!(media.thumbnail_bytes.starts_with(b"\xff\xd8\xff"));
        let still = gif(dir.path(), "b.tmp", 1, 640, 160);
        let (media, animated) = profile_image(&still).unwrap();
        assert!(!animated);
        // Banner-shaped: the rendition keeps its width.
        assert_eq!(media.thumb.unwrap().0, 640);
        // Too many frames: refused, after decoding at most one too many.
        let long = gif(dir.path(), "c.tmp", MAX_ANIMATION_FRAMES + 1, 2, 2);
        assert_eq!(
            profile_image(&long).err().unwrap().0.code,
            ErrorCode::PayloadTooLarge
        );
        // A frame larger than 1920x1080 cannot animate.
        let big = gif(dir.path(), "d.tmp", 2, 2000, 1100);
        assert_eq!(
            profile_image(&big).err().unwrap().0.code,
            ErrorCode::PayloadTooLarge
        );
        // Not an image at all, whatever it claims.
        let fake = dir.path().join("e.tmp");
        std::fs::write(&fake, b"GIF89a not really").unwrap();
        assert!(profile_image(&fake).is_err());
        let svg = dir.path().join("f.tmp");
        std::fs::write(&svg, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert!(prepare(&svg, "image/svg+xml", UploadPurpose::Avatar).is_err());
        // The registry remembers which uploads move, by storage stem.
        prepare(&moving, "image/gif", UploadPurpose::Avatar).unwrap();
        assert_eq!(super::animated("a.bin"), Some(true));
        prepare(&still, "image/gif", UploadPurpose::Avatar).unwrap();
        assert_eq!(super::animated("b.part"), Some(false));
    }
    #[test]
    fn apng_is_detected_and_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.tmp");
        let mut bytes = vec![];
        {
            let mut e = png::Encoder::new(&mut bytes, 4, 4);
            e.set_color(png::ColorType::Rgba);
            e.set_animated(2, 0).unwrap();
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[255; 64]).unwrap();
            w.write_image_data(&[0; 64]).unwrap();
            w.finish().unwrap();
        }
        std::fs::write(&path, bytes).unwrap();
        let (media, animated) = profile_image(&path).unwrap();
        assert!(animated);
        assert_eq!(media.mime, "image/apng");
    }
    #[test]
    fn oversized_files_and_badge_icons() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.tmp");
        std::fs::write(&path, vec![0u8; MAX_PROFILE_IMAGE_BYTES as usize + 1]).unwrap();
        assert_eq!(
            profile_image(&path).err().unwrap().0.code,
            ErrorCode::PayloadTooLarge
        );
        let mut png = std::io::Cursor::new(vec![]);
        RgbaImage::from_pixel(200, 100, Rgba([1, 2, 3, 255]))
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let icon = badge_icon(png.get_ref()).unwrap();
        let back = image::load_from_memory(&icon).unwrap();
        assert_eq!((back.width(), back.height()), (64, 64));
        assert!(badge_icon(b"<svg/>").is_err());
    }
}
