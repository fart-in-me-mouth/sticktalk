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
/// How an image attachment is drawn in a message.
#[derive(Debug, PartialEq, Eq)]
pub struct ImageBox {
    /// CSS pixels. The row reserves exactly this box before the picture loads.
    pub width: u32,
    pub height: u32,
    /// Draw the file itself, not its 256-pixel thumbnail.
    pub original: bool,
}
/// The box an image attachment gets, or `None` when it has no thumbnail (so is
/// not a picture the server could read).
///
/// A picture is shown at its own size, fitted into 550x350. One that would end
/// up a thin strip (a wide screenshot) may run wider than 550 until it is 200
/// tall; the message column still limits it. Nothing is drawn above its own
/// size. Only a file light enough to decode in a message list is drawn itself:
/// a heavier one stays at its thumbnail's size, which is all a thumbnail can
/// fill without blurring.
pub fn image_box(f: &Attachment) -> Option<ImageBox> {
    const MAX_W: f64 = 550.;
    const MAX_H: f64 = 350.;
    const STRIP_H: f64 = 200.;
    const LIGHT_BYTES: u64 = 4 * 1024 * 1024;
    // A thumbnail is one still frame. A modest GIF is shown itself, so it moves.
    const LIGHT_GIF_BYTES: u64 = 8 * 1024 * 1024;
    const LIGHT_PIXELS: u64 = 3840 * 2160;
    let t = f.thumbnail.as_ref()?;
    let small = ImageBox {
        width: t.width.max(1).into(),
        height: t.height.max(1).into(),
        original: false,
    };
    let (Some(w), Some(h)) = (f.width, f.height) else {
        return Some(small);
    };
    let budget = if f.content_type == "image/gif" {
        LIGHT_GIF_BYTES
    } else {
        LIGHT_BYTES
    };
    if w == 0 || h == 0 || f.byte_size > budget || u64::from(w) * u64::from(h) > LIGHT_PIXELS {
        return Some(small);
    }
    let (w, h) = (f64::from(w), f64::from(h));
    let scale = (MAX_W / w).max(STRIP_H / h).min(MAX_H / h).min(1.);
    Some(ImageBox {
        width: ((w * scale).round() as u32).max(1),
        height: ((h * scale).round() as u32).max(1),
        original: true,
    })
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
/// MessagePack for the messages every session sends without anyone doing
/// anything (identify, heartbeats, focus, the subscriptions after Ready, the
/// device name), and a few small frequent commands, written by hand. The
/// full command encoder (every command's serializer) then downloads only
/// with a command a person sends. Byte for byte what `rmp_serde` writes;
/// None for anything else.
pub fn encode_small(nonce: Nonce, msg: Small<'_>) -> Option<Vec<u8>> {
    let mut w = Writer(Vec::with_capacity(64));
    match msg {
        Small::Message(ClientMsg::Identify(i)) => {
            w.tag("identify");
            let fields = 3 + usize::from(i.launch_features) + usize::from(i.text_features);
            w.map(fields);
            w.str("version").uint(u64::from(i.version));
            w.str("token").str(&i.token);
            w.str("resume");
            match &i.resume {
                None => {
                    w.nil();
                }
                Some(c) => {
                    w.map(2).str("stream_id").str(&c.stream_id);
                    w.str("seq").uint(c.seq);
                }
            }
            if i.launch_features {
                w.str("launch_features").bool(true);
            }
            if i.text_features {
                w.str("text_features").bool(true);
            }
        }
        Small::Message(ClientMsg::Heartbeat(h)) => {
            w.tag("heartbeat").map(2).str("nonce").uint(h.nonce);
            w.str("last_seq").uint(h.last_seq);
        }
        Small::Message(ClientMsg::Focus { focused }) => {
            w.tag("focus").map(1).str("focused").bool(*focused);
        }
        Small::Message(_) => return None,
        Small::Command(command) => {
            w.tag("command")
                .map(2)
                .str("nonce")
                .uint(nonce)
                .str("command");
            match command {
                Command::Content(ContentCommand::Subscribe) => w.unit_in("content", "subscribe"),
                Command::Threads(ThreadCommand::Subscribe) => w.unit_in("threads", "subscribe"),
                Command::Expression(ExpressionCommand::Subscribe) => {
                    w.unit_in("expression", "subscribe")
                }
                Command::Privacy(PrivacyCommand::Subscribe) => w.unit_in("privacy", "subscribe"),
                Command::Community(CommunityCommand::Subscribe) => {
                    w.unit_in("community", "subscribe")
                }
                Command::Schedule(ScheduleCommand::Subscribe) => w.unit_in("schedule", "subscribe"),
                Command::Appearance(AppearanceCommand::Subscribe) => {
                    w.unit_in("appearance", "subscribe")
                }
                Command::Apps(AppCommand::Subscribe) => w.unit_in("apps", "subscribe"),
                Command::Launch(LaunchCommand::Moderation(ModerationCommand::Subscribe)) => {
                    w.tag("launch").unit_in("moderation", "subscribe")
                }
                Command::Launch(LaunchCommand::UpdateDevice {
                    device_name,
                    platform,
                }) => {
                    w.tag("launch").tag("update_device").map(2);
                    w.str("device_name").str(device_name);
                    w.str("platform").str(platform);
                }
                Command::SubscribeGuild {
                    guild_id,
                    presence_user_ids,
                } => {
                    w.tag("subscribe_guild").map(2).str("guild_id");
                    match guild_id {
                        Some(g) => w.uint(g.0),
                        None => w.nil(),
                    };
                    w.str("presence_user_ids").array(presence_user_ids.len());
                    for u in presence_user_ids {
                        w.uint(u.0);
                    }
                }
                Command::MarkRead {
                    channel_id,
                    message_id,
                } => {
                    w.tag("mark_read")
                        .map(2)
                        .str("channel_id")
                        .uint(channel_id.0);
                    w.str("message_id").uint(message_id.0);
                }
                Command::Typing { channel_id } => {
                    w.tag("typing").map(1).str("channel_id").uint(channel_id.0);
                }
                _ => return None,
            }
        }
    }
    Some(w.0)
}
/// What `encode_small` writes.
pub enum Small<'a> {
    Message(&'a ClientMsg),
    Command(&'a Command),
}
struct Writer(Vec<u8>);
impl Writer {
    fn len(&mut self, n: usize, fix: u8, fix_max: usize, m16: u8, m32: u8) -> &mut Self {
        if n <= fix_max {
            self.0.push(fix | n as u8);
        } else if n <= 0xffff {
            self.0.push(m16);
            self.0.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            self.0.push(m32);
            self.0.extend_from_slice(&(n as u32).to_be_bytes());
        }
        self
    }
    fn map(&mut self, n: usize) -> &mut Self {
        self.len(n, 0x80, 15, 0xde, 0xdf)
    }
    fn array(&mut self, n: usize) -> &mut Self {
        self.len(n, 0x90, 15, 0xdc, 0xdd)
    }
    fn str(&mut self, s: &str) -> &mut Self {
        if s.len() <= 31 {
            self.0.push(0xa0 | s.len() as u8);
        } else if s.len() <= 0xff {
            self.0.extend_from_slice(&[0xd9, s.len() as u8]);
        } else {
            self.len(s.len(), 0, 0, 0xda, 0xdb);
        }
        self.0.extend_from_slice(s.as_bytes());
        self
    }
    fn uint(&mut self, v: u64) -> &mut Self {
        match v {
            0..=0x7f => self.0.push(v as u8),
            0x80..=0xff => self.0.extend_from_slice(&[0xcc, v as u8]),
            0x100..=0xffff => {
                self.0.push(0xcd);
                self.0.extend_from_slice(&(v as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.0.push(0xce);
                self.0.extend_from_slice(&(v as u32).to_be_bytes());
            }
            _ => {
                self.0.push(0xcf);
                self.0.extend_from_slice(&v.to_be_bytes());
            }
        }
        self
    }
    fn bool(&mut self, v: bool) -> &mut Self {
        self.0.push(if v { 0xc3 } else { 0xc2 });
        self
    }
    fn nil(&mut self) -> &mut Self {
        self.0.push(0xc0);
        self
    }
    /// The start of an adjacently tagged variant with content: `{type, data: `.
    fn tag(&mut self, tag: &str) -> &mut Self {
        self.map(2).str("type").str(tag).str("data")
    }
    /// `{type: outer, data: {type: inner}}`: a unit variant inside a variant.
    fn unit_in(&mut self, outer: &str, inner: &str) {
        self.tag(outer).map(1).str("type").str(inner);
    }
}
pub fn encode_message(msg: &ClientMsg) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(msg).map_err(|_| "Could not encode command".to_string())
}
/// `ClientMsg::Command(CommandRequest { nonce, command })`, byte for byte,
/// from a borrowed command (the app keeps each pending command only once).
pub fn encode_command(nonce: Nonce, command: &Command) -> Result<Vec<u8>, String> {
    #[derive(serde::Serialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    enum Borrowed<'a> {
        Command { nonce: Nonce, command: &'a Command },
    }
    rmp_serde::to_vec_named(&Borrowed::Command { nonce, command })
        .map_err(|_| "Could not encode command".to_string())
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

/// The voice event the server sends after every Ready when none of your
/// conversations has a call (`VoiceEvent::CallSnapshot { calls: [] }`, as the
/// SFU encodes events). A client that knows of no calls has nothing to do with
/// it, so it need not load the voice module on an idle screen.
pub const EMPTY_CALL_SNAPSHOT: &[u8] = b"\x82\xa4type\xadcall_snapshot\xa4data\x81\xa5calls\x90";
/// The answer to the events subscription after Ready when none of your
/// servers has an upcoming event (`ScheduleResult::Summary { guilds: [] }`).
pub const EMPTY_EVENT_SUMMARY: &[u8] = b"\x82\xa4type\xa7summary\xa4data\x81\xa6guilds\x90";
/// The Stage list sent after every Ready when none of your servers has a live
/// Stage (`VoiceEvent::Stage(StageEvent::Snapshot { stages: [] })`): nothing
/// to load the stage feature for while none is known.
pub const EMPTY_STAGE_SNAPSHOT: &[u8] =
    b"\x82\xa4type\xa5stage\xa4data\x82\xa4type\xa8snapshot\xa4data\x81\xa6stages\x90";
/// The answer to the threads subscription after Ready when you are in no
/// thread and none is active anywhere you can see (`ThreadResult::Snapshot`
/// of empty lists): settled in the core (`operations::settle`).
pub const EMPTY_THREAD_SNAPSHOT: &[u8] =
    b"\x82\xa4type\xa8snapshot\xa4data\x83\xa7threads\x90\xabmemberships\x90\xabread_states\x90";

#[cfg(test)]
mod tests {
    use super::*;
    fn picture(content_type: &str, bytes: u64, w: u32, h: u32) -> Attachment {
        // The server's thumbnail: the picture fitted into 256x256.
        let scale = (256. / f64::from(w)).min(256. / f64::from(h)).min(1.);
        Attachment {
            id: Id(1),
            filename: "picture".into(),
            content_type: content_type.into(),
            byte_size: bytes,
            width: Some(w),
            height: Some(h),
            thumbnail: Some(Thumbnail {
                width: ((f64::from(w) * scale) as u16).max(1),
                height: ((f64::from(h) * scale) as u16).max(1),
                byte_size: 1,
                content_type: "image/jpeg".into(),
            }),
            meta: Default::default(),
        }
    }
    #[test]
    fn pictures_get_a_box_they_can_fill_sharply() {
        let shown = |a: &Attachment| {
            let b = image_box(a).unwrap();
            (b.width, b.height, b.original)
        };
        const MB: u64 = 1024 * 1024;
        // A wide screenshot strip is shown at its own size, not as a 256x25 sliver.
        assert_eq!(
            shown(&picture("image/png", 90_000, 1296, 125)),
            (1296, 125, true)
        );
        // A wider one grows only until it is 200 tall.
        assert_eq!(
            shown(&picture("image/png", MB, 3840, 400)),
            (1920, 200, true)
        );
        // Ordinary pictures fit 550x350.
        assert_eq!(
            shown(&picture("image/png", MB, 1920, 1080)),
            (550, 309, true)
        );
        assert_eq!(
            shown(&picture("image/jpeg", MB, 1200, 1600)),
            (263, 350, true)
        );
        // Never above its own size.
        assert_eq!(shown(&picture("image/png", 900, 48, 48)), (48, 48, true));
        // Too heavy to decode in a list: the thumbnail, at the size it can fill.
        assert_eq!(
            shown(&picture("image/jpeg", 6 * MB, 4000, 3000)),
            (256, 192, false)
        );
        assert_eq!(
            shown(&picture("image/png", MB, 6000, 3000)),
            (256, 128, false)
        );
        // A GIF moves, so it is allowed to be heavier.
        assert_eq!(
            shown(&picture("image/gif", 6 * MB, 480, 270)),
            (480, 270, true)
        );
        assert_eq!(
            shown(&picture("image/gif", 9 * MB, 480, 270)),
            (256, 144, false)
        );
        // No dimensions from an older server: the thumbnail's.
        let mut old = picture("image/png", 900, 800, 600);
        (old.width, old.height) = (None, None);
        assert_eq!(shown(&old), (256, 192, false));
        // Not a picture the server could read.
        old.thumbnail = None;
        assert_eq!(image_box(&old), None);
    }
    #[test]
    fn small_messages_are_written_exactly_as_rmp_serde_writes_them() {
        let owned = |nonce, command: Command| {
            encode_message(&ClientMsg::Command(CommandRequest { nonce, command })).unwrap()
        };
        let commands = [
            Command::Content(ContentCommand::Subscribe),
            Command::Threads(ThreadCommand::Subscribe),
            Command::Expression(ExpressionCommand::Subscribe),
            Command::Privacy(PrivacyCommand::Subscribe),
            Command::Community(CommunityCommand::Subscribe),
            Command::Schedule(ScheduleCommand::Subscribe),
            Command::Appearance(AppearanceCommand::Subscribe),
            Command::Apps(AppCommand::Subscribe),
            Command::Launch(LaunchCommand::Moderation(ModerationCommand::Subscribe)),
            Command::Launch(LaunchCommand::UpdateDevice {
                device_name: "Chrome on Windows".into(),
                platform: "x".repeat(40),
            }),
            Command::SubscribeGuild {
                guild_id: Some(Id(250565579184799744)),
                presence_user_ids: (0..20).map(|i| Id(i * 70_000)).collect(),
            },
            Command::SubscribeGuild {
                guild_id: None,
                presence_user_ids: vec![],
            },
            Command::MarkRead {
                channel_id: Id(300),
                message_id: Id(u64::MAX >> 1),
            },
            Command::Typing {
                channel_id: Id(127),
            },
        ];
        for (i, command) in commands.into_iter().enumerate() {
            let nonce = [0, 5, 200, 70_000, u64::MAX >> 1][i % 5];
            assert_eq!(
                encode_small(nonce, Small::Command(&command)).as_deref(),
                Some(owned(nonce, command.clone()).as_slice()),
                "{command:?}"
            );
        }
        for msg in [
            ClientMsg::Identify(Identify {
                version: PROTOCOL_VERSION,
                token: String::new(),
                resume: None,
                launch_features: true,
                text_features: true,
            }),
            ClientMsg::Identify(Identify {
                resume: Some(ResumeCursor {
                    stream_id: "f0718bda130a48e5b8362db1c56ac7ea".into(),
                    seq: 1234,
                }),
                ..Default::default()
            }),
            ClientMsg::Heartbeat(Heartbeat {
                nonce: u64::MAX >> 1,
                last_seq: 99,
            }),
            ClientMsg::Focus { focused: false },
        ] {
            assert_eq!(
                encode_small(0, Small::Message(&msg)).unwrap(),
                encode_message(&msg).unwrap(),
                "{msg:?}"
            );
        }
        assert!(encode_small(
            1,
            Small::Command(&Command::Content(ContentCommand::EndPoll {
                channel_id: Id(1),
                message_id: Id(2)
            }))
        )
        .is_none());
    }
    #[test]
    fn a_borrowed_command_encodes_exactly_like_the_owned_message() {
        let command = Command::SendMessage(MessageDraft {
            channel_id: Id(4),
            content: "hi".into(),
            reply_to: Some(Id(2)),
            attachment_ids: vec![Id(9)],
            mention_ids: vec![],
        });
        let owned = encode_message(&ClientMsg::Command(CommandRequest {
            nonce: 77,
            command: command.clone(),
        }))
        .unwrap();
        assert_eq!(encode_command(77, &command).unwrap(), owned);
    }
    #[test]
    fn the_empty_call_snapshot_bytes_match_the_encoding() {
        let encoded = rmp_serde::to_vec_named(&VoiceEvent::CallSnapshot { calls: vec![] }).unwrap();
        assert_eq!(encoded, EMPTY_CALL_SNAPSHOT);
        let encoded =
            rmp_serde::to_vec_named(&VoiceEvent::Stage(StageEvent::Snapshot { stages: vec![] }))
                .unwrap();
        assert_eq!(encoded, EMPTY_STAGE_SNAPSHOT);
        let encoded = rmp_serde::to_vec_named(&ScheduleResult::Summary { guilds: vec![] }).unwrap();
        assert_eq!(encoded, EMPTY_EVENT_SUMMARY);
        let encoded = rmp_serde::to_vec_named(&ThreadResult::Snapshot(ThreadSnapshot {
            threads: vec![],
            memberships: vec![],
            read_states: vec![],
        }))
        .unwrap();
        assert_eq!(encoded, EMPTY_THREAD_SNAPSHOT);
    }
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
