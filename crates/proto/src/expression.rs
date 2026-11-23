//! Expression: custom emoji in every conversation, stickers, Super Reactions
//! and custom typing indicators. Everything is free for every account.
//!
//! All of it is additive. New message fields (`MessageExtra::stickers` and
//! `MessageExtra::super_reactions`) are omitted when empty. New events travel
//! inside `Event::Expression`, which the server sends only to connections that
//! sent `ExpressionCommand::Subscribe`. See `docs/features/expression.md`.
use crate::*;
use serde::{Deserialize, Serialize};

/// Per server: still and animated custom emoji are counted separately.
pub const MAX_GUILD_STATIC_EMOJI: usize = 250;
pub const MAX_GUILD_ANIMATED_EMOJI: usize = 250;
pub const MAX_GUILD_STICKERS: usize = 60;
/// Largest sticker file accepted for upload.
pub const MAX_STICKER_BYTES: usize = 512 * 1024;
/// Stickers are shown at this size (CSS pixels) and stored at twice it at most.
pub const STICKER_DISPLAY_PX: u32 = 160;
pub const STICKER_STORED_PX: u32 = 320;
pub const MAX_MESSAGE_STICKERS: usize = 3;
pub const MAX_STICKER_DESCRIPTION_CHARS: usize = 100;
pub const MAX_STICKER_FAVORITES: usize = 50;
pub const MAX_TYPING_EMOJI: usize = 3;
pub const MAX_TYPING_VERB_CHARS: usize = 20;
/// IDs one `GET /api/v1/emoji/resolve` may ask about.
pub const MAX_RESOLVE_EMOJI: usize = 100;

/// Sticker names: 2 to 30 characters of letters, digits, spaces, `_` and `-`,
/// with no leading or trailing space.
pub fn valid_sticker_name(name: &str) -> bool {
    let n = name.chars().count();
    (2..=30).contains(&n)
        && name.trim() == name
        && !name.contains("  ")
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '-')
}
pub fn valid_sticker_description(text: &str) -> bool {
    text.chars().count() <= MAX_STICKER_DESCRIPTION_CHARS
        && text.trim() == text
        && !text.chars().any(char::is_control)
}
/// A short run of non-ASCII, non-control characters (one Unicode emoji). The
/// server additionally checks it against its emoji table.
pub fn plausible_unicode_emoji(e: &str) -> bool {
    !e.is_empty()
        && e.len() <= 32
        && !e
            .chars()
            .any(|c| c.is_ascii() || c.is_control() || c.is_whitespace())
}
/// The custom verb in a typing indicator: 1 to 20 characters, letters and
/// single spaces, apostrophes or hyphens, starting and ending with a letter.
pub fn valid_typing_verb(verb: &str) -> bool {
    let n = verb.chars().count();
    let first = verb.chars().next();
    let last = verb.chars().next_back();
    (1..=MAX_TYPING_VERB_CHARS).contains(&n)
        && first.is_some_and(char::is_alphabetic)
        && last.is_some_and(char::is_alphabetic)
        && !verb.contains("  ")
        && verb
            .chars()
            .all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-')
}

/// Every `<:name:id>` token in `text`: (byte range, name, id).
pub fn emoji_tokens(text: &str) -> Vec<(std::ops::Range<usize>, &str, Id)> {
    let mut out = vec![];
    let mut at = 0;
    while let Some(start) = text[at..].find("<:").map(|i| i + at) {
        let Some(end) = text[start..].find('>').map(|i| i + start) else {
            break;
        };
        let token = &text[start..=end];
        if let Some(id) = custom_emoji_id(token) {
            let name = &token[2..token.len() - 1];
            let name = name.split(':').next().unwrap_or_default();
            out.push((start..end + 1, name, id));
            at = end + 1;
        } else {
            at = start + 2;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum StickerFormat {
    Png,
    Apng,
    Gif,
    Webp,
    /// Only the built-in pack. Uploads are never SVG.
    Svg,
}
crate::wire_names::wire_names!(StickerFormat {
    Png = "png",
    Apng = "apng",
    Gif = "gif",
    Webp = "webp",
    Svg = "svg",
});
impl StickerFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png | Self::Apng => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Apng => "apng",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Svg => "svg",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Png, Self::Apng, Self::Gif, Self::Webp, Self::Svg]
            .into_iter()
            .find(|f| f.name() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: Id,
    /// None for the built-in pack every account has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    pub name: String,
    /// One Unicode emoji; typing it in the composer suggests this sticker.
    pub emoji: String,
    /// Read by assistive technology in place of the image.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub format: StickerFormat,
    pub animated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_id: Option<UserId>,
    #[serde(default)]
    pub created_at: UnixMillis,
}
impl Sticker {
    pub fn item(&self) -> StickerItem {
        StickerItem {
            id: self.id,
            name: self.name.clone(),
            format: self.format,
            animated: self.animated,
            guild_id: self.guild_id,
            description: self.description.clone(),
            deleted: false,
        }
    }
}

/// A sticker as a message carries it: enough to show it to anyone who can
/// read the message, whether or not they are in the sticker's server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StickerItem {
    pub id: Id,
    pub name: String,
    pub format: StickerFormat,
    pub animated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The sticker has since been deleted: show its name instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}
impl StickerItem {
    /// Where the image is: the built-in pack ships with the app.
    pub fn url(&self) -> String {
        match self.guild_id {
            None => format!(
                "/public/stickers/{}.svg",
                builtin_slug(self.id).unwrap_or("")
            ),
            Some(_) => format!("/api/v1/stickers/{}", self.id.0),
        }
    }
    /// A still first frame of an animated sticker (reduced motion).
    pub fn still_url(&self) -> String {
        if self.animated && self.guild_id.is_some() {
            format!("/api/v1/stickers/{}/still", self.id.0)
        } else {
            self.url()
        }
    }
}

/// The original built-in pack: (id, slug, name, related emoji, description).
/// IDs are below every Snowflake, so they never collide with uploads.
pub const BUILTIN_STICKERS: &[(u64, &str, &str, &str, &str)] = &[
    (1, "hello", "Hello", "👋", "A stick figure waving hello"),
    (
        2,
        "thumbs-up",
        "Nice One",
        "👍",
        "A stick figure giving a thumbs up",
    ),
    (
        3,
        "heart",
        "Big Love",
        "❤️",
        "A stick figure hugging a big heart",
    ),
    (
        4,
        "party",
        "Party Time",
        "🎉",
        "A stick figure jumping under confetti",
    ),
    (
        5,
        "coffee",
        "Coffee First",
        "☕",
        "A stick figure holding a steaming mug",
    ),
    (
        6,
        "sleepy",
        "Sleepy",
        "😴",
        "A stick figure dozing with floating Zs",
    ),
    (
        7,
        "thinking",
        "Hmm",
        "🤔",
        "A stick figure thinking under a question mark",
    ),
    (
        8,
        "laugh",
        "LOL",
        "😂",
        "A stick figure laughing with tears of joy",
    ),
    (
        9,
        "high-five",
        "High Five",
        "🙌",
        "Two stick figures doing a high five",
    ),
    (
        10,
        "on-it",
        "On It",
        "🚀",
        "A stick figure riding a small rocket",
    ),
];
pub fn builtin_slug(id: Id) -> Option<&'static str> {
    BUILTIN_STICKERS.iter().find(|s| s.0 == id.0).map(|s| s.1)
}
pub fn builtin_stickers() -> Vec<Sticker> {
    BUILTIN_STICKERS
        .iter()
        .map(|(id, _, name, emoji, description)| Sticker {
            id: Id(*id),
            guild_id: None,
            name: (*name).into(),
            emoji: (*emoji).into(),
            description: (*description).into(),
            format: StickerFormat::Svg,
            animated: false,
            creator_id: None,
            created_at: 0,
        })
        .collect()
}
pub fn builtin_sticker(id: Id) -> Option<Sticker> {
    builtin_stickers().into_iter().find(|s| s.id == id)
}

/// A custom emoji with who added it and when (server settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiDetail {
    pub emoji: GuildEmoji,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_id: Option<UserId>,
    #[serde(default)]
    pub created_at: UnixMillis,
}

/// `GET /api/v1/guilds/{id}/expression`: the server's emoji and stickers for
/// their settings page (Manage Expressions).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuildExpressionAdmin {
    pub emoji: Vec<EmojiDetail>,
    pub stickers: Vec<Sticker>,
    /// Everyone named as a creator above.
    pub users: Vec<UserSummary>,
    pub max_static_emoji: u32,
    pub max_animated_emoji: u32,
    pub max_stickers: u32,
}

/// `GET /api/v1/emoji/resolve?ids=`: what an emoji seen in a message is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiInfo {
    pub id: Id,
    pub name: String,
    pub animated: bool,
    /// Its server, only when the person asking is a member there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_icon_id: Option<AttachmentId>,
    /// False once deleted, or when the asker may not see it: show `:name:`.
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildEmojiSet {
    pub guild_id: GuildId,
    pub emoji: Vec<GuildEmoji>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildStickerSet {
    /// None is the built-in pack.
    pub guild_id: Option<GuildId>,
    pub stickers: Vec<Sticker>,
}

/// `GET /api/v1/users/@me/expression`: everything the pickers need, once.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpressionLibrary {
    /// Custom emoji of every server the account is in.
    pub emoji: Vec<GuildEmojiSet>,
    /// The built-in pack first, then each server's stickers.
    pub stickers: Vec<GuildStickerSet>,
    pub settings: ExpressionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpressionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing: Option<TypingStyle>,
    #[serde(default)]
    pub sticker_favorites: Vec<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingMotion {
    #[default]
    Bounce,
    Wave,
    Pulse,
    Float,
    /// No movement at all.
    Still,
}
impl TypingMotion {
    pub const ALL: [Self; 5] = [
        Self::Bounce,
        Self::Wave,
        Self::Pulse,
        Self::Float,
        Self::Still,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Bounce => "bounce",
            Self::Wave => "wave",
            Self::Pulse => "pulse",
            Self::Float => "float",
            Self::Still => "still",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bounce => "Bounce",
            Self::Wave => "Wave",
            Self::Pulse => "Pulse",
            Self::Float => "Float",
            Self::Still => "Still",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "snake_case")]
pub enum TypingVerb {
    #[default]
    Typing,
    Writing,
    /// See `valid_typing_verb`.
    Custom(String),
}
impl TypingVerb {
    /// What follows the name: "is typing", "is writing" or "is <verb>".
    pub fn phrase(&self) -> String {
        match self {
            Self::Typing => "is typing".into(),
            Self::Writing => "is writing".into(),
            Self::Custom(v) => format!("is {v}"),
        }
    }
}

/// How others see you typing when you are the only one typing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TypingStyle {
    /// Up to three Unicode emoji or `<:name:id>` custom emoji tokens.
    pub emoji: Vec<String>,
    pub motion: TypingMotion,
    pub verb: TypingVerb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExpressionCommand {
    /// Opts this connection in to `Event::Expression`.
    Subscribe,
    /// The emoji is a Unicode emoji or a `<:name:id>` token, as for reactions.
    AddSuperReaction {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
    },
    RemoveSuperReaction {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
    },
    /// One to three stickers, with optional text. Answers `CommandResult::Message`.
    SendStickers {
        channel_id: ChannelId,
        sticker_ids: Vec<Id>,
        #[serde(default)]
        content: String,
        #[serde(default)]
        reply_to: Option<MessageId>,
        /// As `MessageDraft::mention_ids`, for mentions in the text.
        #[serde(default)]
        mention_ids: Vec<UserId>,
    },
    /// Manage Expressions. Uploads use `POST /api/v1/guilds/{id}/stickers`.
    UpdateSticker {
        guild_id: GuildId,
        sticker_id: Id,
        name: String,
        emoji: String,
        description: String,
    },
    DeleteSticker {
        guild_id: GuildId,
        sticker_id: Id,
    },
    /// None restores the standard indicator.
    SetTypingStyle {
        style: Option<TypingStyle>,
    },
    /// The complete list, most recent first (at most 50).
    SetStickerFavorites {
        sticker_ids: Vec<Id>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExpressionResult {
    Ack,
    SuperReaction(Box<ReactionUpdate>),
    Settings(Box<ExpressionSettings>),
    Sticker(Box<Sticker>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExpressionEvent {
    /// A server's complete sticker list, to its members.
    StickersUpdated {
        guild_id: GuildId,
        stickers: Vec<Sticker>,
    },
    /// `reaction` is the Super Reaction summary for that emoji. Viewers show a
    /// short burst when `added` is true.
    SuperReactionUpdated(ReactionUpdate),
    /// Sent just before a `Typing` event from someone with a custom indicator.
    TypingStyle { user_id: UserId, style: TypingStyle },
    /// Your own settings changed on another device.
    SettingsUpdated(ExpressionSettings),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_verbs_and_tokens_are_strict() {
        assert!(valid_sticker_name("Party Time"));
        assert!(valid_sticker_name("ok_2-go"));
        for bad in [
            "x",
            " lead",
            "trail ",
            "two  spaces",
            "semi;colon",
            &"a".repeat(31),
        ] {
            assert!(!valid_sticker_name(bad), "{bad}");
        }
        assert!(valid_typing_verb("writing"));
        assert!(valid_typing_verb("cooking up something"));
        assert!(valid_typing_verb("rock'n-rolling"));
        for bad in [
            "",
            " typing",
            "typing!",
            "type1",
            "a  b",
            "<b>",
            &"a".repeat(21),
        ] {
            assert!(!valid_typing_verb(bad), "{bad}");
        }
        let tokens = emoji_tokens("hi <:wave:42> <:bad:x> <:ok_2:7>!");
        assert_eq!(tokens.len(), 2);
        assert_eq!((tokens[0].1, tokens[0].2), ("wave", Id(42)));
        assert_eq!((tokens[1].1, tokens[1].2), ("ok_2", Id(7)));
        assert!(plausible_unicode_emoji("👍"));
        assert!(!plausible_unicode_emoji("ok"));
    }

    #[test]
    fn builtin_pack_is_complete_and_below_snowflakes() {
        let pack = builtin_stickers();
        assert_eq!(pack.len(), BUILTIN_STICKERS.len());
        for s in &pack {
            assert!(s.id.0 < 1000 && valid_sticker_name(&s.name), "{}", s.name);
            assert!(s.item().url().ends_with(".svg"));
        }
    }

    #[test]
    fn typing_style_roundtrips_in_messagepack() {
        let style = TypingStyle {
            emoji: vec!["✍️".into()],
            motion: TypingMotion::Wave,
            verb: TypingVerb::Custom("scribbling".into()),
        };
        let bytes = rmp_serde::to_vec_named(&style).unwrap();
        assert_eq!(rmp_serde::from_slice::<TypingStyle>(&bytes).unwrap(), style);
    }
}
