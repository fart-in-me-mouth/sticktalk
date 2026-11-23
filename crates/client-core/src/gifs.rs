//! GIF normalization and timer-free playback eligibility shared by clients.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sticktalk_proto::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Autoplay {
    #[default]
    Always,
    OnHover,
    Never,
}
impl Autoplay {
    pub fn animates(self, visible: bool, focused: bool, hovered: bool, manual: bool) -> bool {
        visible && focused && (manual || self == Self::Always || self == Self::OnHover && hovered)
    }
}
pub fn validate_media(m: &GifMedia) -> Result<(), String> {
    let url = crate::urls::parse(&m.url).ok_or("Invalid GIF URL")?;
    if !matches!(
        url.host.as_str(),
        "static.klipy.com" | "static1.klipy.com" | "static2.klipy.com"
    ) || url.port.is_some()
        || url.fragment
        || !matches!(
            m.content_type.as_str(),
            "video/mp4" | "video/webm" | "image/webp" | "image/gif"
        )
        || url.scheme != "https"
        || url.credentials
        || m.url.len() > 2048
        || m.width == 0
        || m.height == 0
        || m.width > 4096
        || m.height > 4096
    {
        return Err("Invalid GIF media".into());
    }
    Ok(())
}
#[derive(Deserialize)]
pub struct ProviderPage {
    pub results: Vec<ProviderItem>,
    pub next: Option<String>,
}
#[derive(Deserialize)]
pub struct ProviderItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub content_description: String,
    pub media_formats: BTreeMap<String, ProviderMedia>,
}
#[derive(Deserialize)]
pub struct ProviderMedia {
    pub url: String,
    pub dims: [u16; 2],
}
pub fn normalize(page: ProviderPage) -> Result<GifPage, String> {
    fn take(
        formats: &mut BTreeMap<String, ProviderMedia>,
        choices: &[(&str, &str)],
    ) -> Result<GifMedia, String> {
        for (key, mime) in choices {
            if let Some(m) = formats.remove(*key) {
                let m = GifMedia {
                    url: m.url,
                    width: m.dims[0],
                    height: m.dims[1],
                    content_type: (*mime).into(),
                };
                validate_media(&m)?;
                return Ok(m);
            }
        }
        Err("GIF rendition unavailable".into())
    }
    if page.results.len() > GIF_PAGE_SIZE {
        return Err("Too many GIF results".into());
    }
    let items = page
        .results
        .into_iter()
        .map(|mut p| {
            if p.id.is_empty()
                || p.id.len() > 128
                || !p
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return Err("Invalid GIF ID".into());
            }
            Ok(GifItem {
                id: format!("klipy:{}", p.id),
                title: if p.content_description.trim().is_empty() {
                    p.title
                } else {
                    p.content_description
                }
                .chars()
                .take(256)
                .collect(),
                preview: take(
                    &mut p.media_formats,
                    &[
                        ("nanowebm", "video/webm"),
                        ("nanomp4", "video/mp4"),
                        ("tinywebp", "image/webp"),
                    ],
                )?,
                send: take(
                    &mut p.media_formats,
                    &[
                        ("mediumgif", "image/gif"),
                        ("tinywebm", "video/webm"),
                        ("tinymp4", "video/mp4"),
                    ],
                )?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let next_cursor = page.next.filter(|s| !s.is_empty() && s != "0");
    if next_cursor
        .as_ref()
        .is_some_and(|s| s.len() > 256 || s.chars().any(char::is_control))
    {
        return Err("Invalid GIF cursor".into());
    }
    Ok(GifPage { items, next_cursor })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_has_no_offscreen_or_background_exception() {
        for mode in [Autoplay::Always, Autoplay::OnHover, Autoplay::Never] {
            assert!(!mode.animates(false, true, true, true));
            assert!(!mode.animates(true, false, true, true));
        }
        assert!(Autoplay::Always.animates(true, true, false, false));
        assert!(!Autoplay::OnHover.animates(true, true, false, false));
        assert!(!Autoplay::Never.animates(true, true, true, false));
    }
}
