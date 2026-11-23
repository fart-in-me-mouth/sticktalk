//! The page's theme and app icon (docs/features/themes.md). The title bar
//! follows the theme (dark mode, and the caption color on Windows 11), and the
//! window and taskbar button take the chosen app icon at once: no restart, no
//! administrator rights, no new window. The page sends the icon's pixels
//! (32x32 RGBA), so the shell needs no image decoder. Only the server's own
//! pages are listened to (`webview.rs`), and every message is checked here.
use serde::Deserialize;

/// The largest icon the page may send, per side.
pub const MAX_ICON: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Theme { dark: bool, caption: (u8, u8, u8) },
    Icon { size: u32, rgba: Vec<u8> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    shell: String,
    #[serde(default)]
    dark: Option<bool>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    size: Option<u32>,
    #[serde(default)]
    rgba: Option<Vec<u8>>,
}

fn color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// A theme or icon message from the page, or None for anything else.
pub fn parse(raw: &str) -> Option<Message> {
    // 64x64x4 numbers of up to 4 characters each, with room to spare.
    if raw.len() > 70_000 || !raw.contains("\"shell\"") {
        return None;
    }
    let raw: Raw = serde_json::from_str(raw).ok()?;
    match raw.shell.as_str() {
        "theme" if raw.size.is_none() && raw.rgba.is_none() => Some(Message::Theme {
            dark: raw.dark?,
            caption: color(raw.caption.as_deref()?)?,
        }),
        "icon" if raw.dark.is_none() && raw.caption.is_none() => {
            let size = raw.size?;
            let rgba = raw.rgba?;
            ((16..=MAX_ICON).contains(&size) && rgba.len() == (size * size * 4) as usize)
                .then_some(Message::Icon { size, rgba })
        }
        _ => None,
    }
}

/// Dark or light title bar, and its caption and text colors (Windows 11;
/// older Windows ignores what it does not know).
#[cfg(windows)]
pub fn apply_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool, caption: (u8, u8, u8)) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    let set = |attribute, value: u32| unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            attribute,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    };
    // COLORREF is 0x00BBGGRR.
    let colorref = |(r, g, b): (u8, u8, u8)| u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16);
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, u32::from(dark));
    set(DWMWA_CAPTION_COLOR, colorref(caption));
    set(
        DWMWA_TEXT_COLOR,
        colorref(if dark { (0xE8, 0xEA, 0xED) } else { (0x1F, 0x20, 0x23) }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_messages() {
        assert_eq!(
            parse(r##"{"shell":"theme","dark":true,"caption":"#111113"}"##),
            Some(Message::Theme { dark: true, caption: (0x11, 0x11, 0x13) })
        );
        assert_eq!(
            parse(r##"{"shell":"theme","dark":false,"caption":"#E3E5E8"}"##),
            Some(Message::Theme { dark: false, caption: (0xE3, 0xE5, 0xE8) })
        );
        for bad in [
            r##"{"shell":"theme","dark":true}"##,
            r##"{"shell":"theme","dark":true,"caption":"red"}"##,
            r##"{"shell":"theme","dark":true,"caption":"#12345"}"##,
            r##"{"shell":"theme","dark":true,"caption":"#111113","extra":1}"##,
            r##"{"shell":"other","dark":true,"caption":"#111113"}"##,
            r##"{"ptt":"stop"}"##,
            "not json \"shell\"",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn icon_messages() {
        let pixels: Vec<String> = (0..32 * 32 * 4).map(|i| (i % 256).to_string()).collect();
        let ok = format!(r#"{{"shell":"icon","size":32,"rgba":[{}]}}"#, pixels.join(","));
        let Some(Message::Icon { size, rgba }) = parse(&ok) else { panic!() };
        assert_eq!((size, rgba.len()), (32, 4096));
        // Wrong length, too big, a value out of range, and too long a message.
        let short = format!(r#"{{"shell":"icon","size":32,"rgba":[{}]}}"#, pixels[..100].join(","));
        assert_eq!(parse(&short), None);
        assert_eq!(parse(r#"{"shell":"icon","size":512,"rgba":[]}"#), None);
        let bad = ok.replacen("[0,", "[256,", 1);
        assert_eq!(parse(&bad), None);
        assert_eq!(parse(&" ".repeat(80_000)), None);
    }
}
