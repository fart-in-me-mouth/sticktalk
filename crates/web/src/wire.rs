//! Small host-boundary rules, tested natively as well as used by WASM.
use sticktalk_proto::*;
/// Older servers deliberately store non-image uploads as octet-stream. A video
/// filename is only a browser decoder hint, never a trusted MIME assertion.
pub fn video_hint(content_type: &str, filename: &str) -> bool {
    content_type.starts_with("video/")
        || (content_type == "application/octet-stream"
            && filename.rsplit_once('.').is_some_and(|(_, ext)| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "mp4" | "m4v" | "mov" | "webm"
                )
            }))
}
/// Accept a pasted invite link without navigating to a user-supplied origin.
/// What this browser is, for the Devices list: ("Chrome on Windows", "Web").
pub fn device(user_agent: &str) -> (String, String) {
    let has = |s: &str| user_agent.contains(s);
    let browser = if has("Edg/") {
        "Edge"
    } else if has("OPR/") {
        "Opera"
    } else if has("Firefox/") || has("FxiOS/") {
        "Firefox"
    } else if has("Chrome/") || has("CriOS/") {
        "Chrome"
    } else if has("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let system = if has("Windows") {
        "Windows"
    } else if has("Android") {
        "Android"
    } else if has("iPhone") {
        "iPhone"
    } else if has("iPad") {
        "iPad"
    } else if has("Mac OS X") {
        "Mac"
    } else if has("CrOS") {
        "Chromebook"
    } else if has("Linux") {
        "Linux"
    } else {
        ""
    };
    let name = if system.is_empty() {
        browser.to_owned()
    } else {
        format!("{browser} on {system}")
    };
    (name, "Web".into())
}
/// A file size as people say it: "812 B", "34 KB", "2.4 MB".
pub fn file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    // Rounded to the nearest tenth.
    let tenths = |unit: u64| {
        let t = (bytes.saturating_mul(10) + unit / 2) / unit;
        format!("{}.{}", t / 10, t % 10)
    };
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else if bytes < GB {
        format!("{} MB", tenths(MB))
    } else {
        format!("{} GB", tenths(GB))
    }
}
/// One line of a message for a reply preview. Spoilers stay hidden.
pub fn snippet(text: &str) -> String {
    let parts = text.split("||").collect::<Vec<_>>();
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        // An odd part is a spoiler only when a closing marker follows it.
        if i % 2 == 1 && i + 1 < parts.len() {
            out.push_str("(spoiler)");
        } else {
            if i % 2 == 1 {
                out.push_str("||");
            }
            out.push_str(part);
        }
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}
pub fn invite_code(input: &str) -> String {
    let input = input.trim();
    let code = input
        .split_once("?invite=")
        .map(|(_, value)| value.split(['&', '#']).next().unwrap_or(value))
        .or_else(|| {
            input
                .split_once("/invite/")
                .map(|(_, value)| value.trim_end_matches('/'))
        })
        .unwrap_or(input);
    code.to_owned()
}
pub fn stringify_ids(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if (k == "id" || k.ends_with("_id") || k == "grant") && v.is_u64() {
                    *v = v.to_string().into()
                } else {
                    stringify_ids(v)
                }
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                stringify_ids(v)
            }
        }
        _ => {}
    }
}
pub fn decode_signal(raw: &str) -> Result<VoiceSignal, String> {
    fn numbers(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    if k.ends_with("_id") {
                        if let Some(s) = v.as_str() {
                            if let Ok(n) = s.parse::<u64>() {
                                *v = n.into();
                            }
                        }
                    } else {
                        numbers(v)
                    }
                }
            }
            serde_json::Value::Array(a) => {
                for v in a {
                    numbers(v)
                }
            }
            _ => {}
        }
    }
    if raw.len() > 80 * 1024 {
        return Err("Media signal exceeds limit".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "Invalid media signal")?;
    numbers(&mut value);
    serde_json::from_value(value).map_err(|_| "Invalid media signal".into())
}
pub fn page_rank(count: u64, len: usize, anchor: Option<(u64, usize)>) -> u64 {
    anchor.map_or_else(
        || count.saturating_sub(len as u64),
        |(rank, index)| rank.saturating_sub(index as u64),
    )
}
/// Shift a bounded window by one fifth while retaining enough overlap for the
/// visible anchor. Sparse global extent anchors are only for distant seeks.
pub fn adjacent_pivot(len: usize, older: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let shift = (len / 5).max(1);
    Some(if older {
        (len / 2).saturating_sub(shift)
    } else {
        (len / 2 + shift).min(len - 1)
    })
}
pub fn typing_label(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [name] => format!("{name} is typing…"),
        [a, b] => format!("{a} and {b} are typing…"),
        _ => "Several people are typing…".into(),
    }
}
/// Preserve the role hue, but raise dark custom colors to AA on the darkest
/// applicable selected surface. The role swatch still shows the exact saved RGB.
pub fn legible_role_color(rgb: u32) -> u32 {
    fn luminance(rgb: u32) -> f64 {
        let channel = |n: u32| {
            let v = f64::from(n & 255) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                let value = (v + 0.055) / 1.055;
                // Browser-native IEEE-754 pow avoids shipping a duplicate
                // software libm implementation just for sRGB contrast checks.
                #[cfg(target_arch = "wasm32")]
                {
                    js_sys::Math::pow(value, 2.4)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    value.powf(2.4)
                }
            }
        };
        0.2126 * channel(rgb >> 16) + 0.7152 * channel(rgb >> 8) + 0.0722 * channel(rgb)
    }
    let background = luminance(0x2e2e34);
    for step in 0..=20 {
        let mix = |n: u32| {
            let c = n & 255;
            c + (255 - c) * step / 20
        };
        let value = (mix(rgb >> 16) << 16) | (mix(rgb >> 8) << 8) | mix(rgb);
        if (luminance(value) + 0.05) / (background + 0.05) >= 4.5 {
            return value;
        }
    }
    0xffffff
}
/// A virtual window always starts with a group head; subsequent rows use the
/// adjacent message, not the original group head, for the seven-minute rule.
pub fn continues_group(author: u64, at: u64, previous: Option<(u64, u64)>) -> bool {
    previous.is_some_and(|(who, when)| who == author && at >= when && at - when <= 420_000)
}

#[test]
fn message_group_boundaries() {
    assert!(!continues_group(1, 1, None));
    assert!(continues_group(1, 420_001, Some((1, 1))));
    assert!(!continues_group(1, 420_002, Some((1, 1))));
    assert!(!continues_group(2, 2, Some((1, 1))));
    assert!(!continues_group(1, 1, Some((1, 2))));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invite_links_are_parsed_as_data_not_navigation() {
        let code = "AbC_def-0123456789abcd";
        assert_eq!(invite_code(&format!("  {code}  ")), code);
        assert_eq!(
            invite_code(&format!(
                "https://example.invalid/?invite={code}&ignored=1#hash"
            )),
            code
        );
        assert_eq!(
            invite_code(&format!("https://example.invalid/invite/{code}/")),
            code
        );
        assert_eq!(invite_code("javascript:alert(1)"), "javascript:alert(1)"); // server rejects it
    }
    /// What `public/media.js` `packControl` produces (fixtures printed by it):
    /// the server's decoder must read them as the inputs they are.
    #[test]
    fn the_pages_control_messages_are_the_servers() {
        use sticktalk_proto::{ControlData, ControlInput, Id, VoiceSession};
        fn bytes(hex: &str) -> Vec<u8> {
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect()
        }
        let message = |sharer: (u64, u64), grant, sequence, input| ControlData::Input {
            sharer: VoiceSession {
                user_id: Id(sharer.0),
                connection_id: Id(sharer.1),
            },
            grant,
            sequence,
            input,
        };
        for (hex, expected) in [
            (
                "82a474797065a5696e707574a46461746184a673686172657282a7757365725f6964cf0377f879e6800000ad636f6e6e656374696f6e5f696407a56772616e74cfffffffffffffffffa873657175656e6365cd012ca5696e70757482a474797065a46d6f7665a46461746182a178cdffffa17900",
                message(
                    (249941506784231424, 7),
                    u64::MAX,
                    300,
                    ControlInput::Move { x: 65535, y: 0 },
                ),
            ),
            (
                "82a474797065a5696e707574a46461746184a673686172657282a7757365725f696401ad636f6e6e656374696f6e5f696402a56772616e7403a873657175656e636504a5696e70757482a474797065a5776865656ca46461746182a178d1f000a17978",
                message((1, 2), 3, 4, ControlInput::Wheel { x: -4096, y: 120 }),
            ),
            (
                "82a474797065a5696e707574a46461746184a673686172657282a7757365725f696401ad636f6e6e656374696f6e5f696402a56772616e7403a873657175656e636505a5696e70757482a474797065a36b6579a46461746182a57573616765cce5a4646f776ec2",
                message(
                    (1, 2),
                    3,
                    5,
                    ControlInput::Key {
                        usage: 229,
                        down: false,
                    },
                ),
            ),
            (
                "82a474797065a5696e707574a46461746184a673686172657282a7757365725f696401ad636f6e6e656374696f6e5f696402a56772616e7403a873657175656e636506a5696e70757482a474797065a6627574746f6ea46461746182a6627574746f6e01a4646f776ec3",
                message(
                    (1, 2),
                    3,
                    6,
                    ControlInput::Button {
                        button: 1,
                        down: true,
                    },
                ),
            ),
        ] {
            assert_eq!(
                rmp_serde::from_slice::<ControlData>(&bytes(hex)).unwrap(),
                expected
            );
        }
        // And the other way: what the server sends is what the page's reader expects.
        assert_eq!(
            rmp_serde::to_vec_named(&ControlData::ReleaseAll { grant: 5 }).unwrap(),
            bytes("82a474797065ab72656c656173655f616c6ca46461746181a56772616e7405")
        );
    }
    #[test]
    fn devices_are_named_for_the_devices_list() {
        let edge = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";
        assert_eq!(device(edge), ("Edge on Windows".into(), "Web".into()));
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
        assert_eq!(device(iphone).0, "Safari on iPhone");
        assert_eq!(device("").0, "Browser");
    }
    #[test]
    fn file_sizes_read_as_people_say_them() {
        assert_eq!(file_size(812), "812 B");
        assert_eq!(file_size(1024), "1 KB");
        assert_eq!(file_size(34_100), "34 KB");
        assert_eq!(file_size(2_516_582), "2.4 MB");
        assert_eq!(file_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
    #[test]
    fn reply_previews_are_one_line_and_keep_spoilers_hidden() {
        assert_eq!(
            snippet("the ending is ||everyone lives||"),
            "the ending is (spoiler)"
        );
        assert_eq!(snippet("a ||b|| c ||d||"), "a (spoiler) c (spoiler)");
        // An unclosed marker is not a spoiler: nothing is hidden, nothing is lost.
        assert_eq!(snippet("pipes || in text"), "pipes || in text");
        assert_eq!(
            snippet(
                "one
two   three"
            ),
            "one two three"
        );
        assert_eq!(snippet(&"x".repeat(300)).chars().count(), 100);
    }
    #[test]
    fn custom_role_colors_cannot_hide_names() {
        assert_eq!(legible_role_color(0xe8eaed), 0xe8eaed);
        assert_eq!(legible_role_color(0x3ddc97), 0x3ddc97);
        assert_ne!(legible_role_color(0x010101), 0x010101);
    }
    #[test]
    fn media_ids_remain_exact_above_javascript_integer_limit() {
        let n = 9_223_372_036_854_775_000u64;
        let event = VoiceEvent::Left {
            session: VoiceSession {
                user_id: Id(n),
                connection_id: Id(n - 1),
            },
        };
        let mut v = serde_json::to_value(event).unwrap();
        stringify_ids(&mut v);
        assert_eq!(v["data"]["session"]["user_id"], n.to_string());
        let signal = format!(
            r#"{{"type":"subscribe","data":{{"publisher":{{"user_id":"{n}","connection_id":"{}"}},"source_mid":"0","receiver_mid":"2","rid":null}}}}"#,
            n - 1
        );
        assert!(
            matches!(decode_signal(&signal).unwrap(),VoiceSignal::Subscribe{publisher,..}if publisher.user_id==Id(n)&&publisher.connection_id==Id(n-1))
        );
    }
    #[test]
    fn signal_boundary_rejects_malformed_and_oversized_data() {
        assert!(decode_signal("bad").is_err());
        assert!(decode_signal(&" ".repeat(81 * 1024)).is_err());
        assert!(decode_signal(r#"{"type":"subscribe","data":{"publisher":{"user_id":"rounded","connection_id":"1"},"source_mid":"0","receiver_mid":"2","rid":null}}"#).is_err());
    }
    #[test]
    fn scrollbar_rank_is_bounded_at_empty_and_oldest_extents() {
        assert_eq!(page_rank(0, 0, None), 0);
        assert_eq!(page_rank(2, 50, None), 0);
        assert_eq!(page_rank(10_000, 50, None), 9950);
        assert_eq!(page_rank(10_000, 50, Some((5000, 25))), 4975);
        assert_eq!(page_rank(10, 10, Some((0, 5))), 0);
    }
    #[test]
    fn adjacent_windows_overlap_and_never_skip_extent_anchor_gaps() {
        assert_eq!(adjacent_pivot(0, true), None);
        assert_eq!(adjacent_pivot(1, false), Some(0));
        assert_eq!(adjacent_pivot(50, true), Some(15));
        assert_eq!(adjacent_pivot(50, false), Some(35));
        for len in 1..=50 {
            for older in [true, false] {
                assert!(adjacent_pivot(len, older).unwrap() < len);
            }
        }
    }
    #[test]
    fn typing_copy_handles_plural_and_empty_states() {
        assert_eq!(typing_label(&[]), "");
        assert_eq!(typing_label(&["Alex".into()]), "Alex is typing…");
        assert_eq!(
            typing_label(&["Alex".into(), "Sam".into()]),
            "Alex and Sam are typing…"
        );
        assert_eq!(
            typing_label(&["A".into(), "B".into(), "C".into()]),
            "Several people are typing…"
        );
    }
    #[test]
    fn legacy_video_hints_are_narrow_and_keep_other_files_downloadable() {
        assert!(video_hint("video/mp4", "clip"));
        assert!(video_hint("application/octet-stream", "Camera.MOV"));
        assert!(video_hint("application/octet-stream", "clip.webm"));
        assert!(!video_hint("text/html", "clip.mp4"));
        assert!(!video_hint("application/octet-stream", "clip.mp4.html"));
        assert!(!video_hint("application/octet-stream", "archive.zip"));
    }
}
