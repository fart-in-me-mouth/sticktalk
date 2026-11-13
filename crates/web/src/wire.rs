//! Small host-boundary rules, tested natively as well as used by WASM.
use sticktalk_proto::*;
pub fn stringify_ids(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if (k == "id" || k.ends_with("_id")) && v.is_u64() {
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
/// Preserve the role hue, but raise dark custom colors to AA on the darkest
/// applicable selected surface. The role swatch still shows the exact saved RGB.
pub fn legible_role_color(rgb: u32) -> u32 {
    fn luminance(rgb: u32) -> f64 {
        let channel = |n: u32| {
            let v = f64::from(n & 255) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
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
}
