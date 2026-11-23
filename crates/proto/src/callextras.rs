//! Call extras: the soundboard, entrance sounds, camera backgrounds, stream and
//! voice quality, and clips. Everything here is free for every user and server.
//!
//! Soundboard sounds are never mixed into anybody's microphone. Playing one is a
//! host-handled `VoiceSignal::PlaySound`; the host checks it and tells everyone
//! in that call with `VoiceEvent::SoundPlayed`, and each listener plays its own
//! cached copy of the clip through the call's speaker. See
//! `docs/features/callextras.md`.
use crate::*;
use serde::{Deserialize, Serialize};

/// Sounds one server's soundboard can hold.
pub const MAX_GUILD_SOUNDS: usize = 48;
/// The largest clip file a server accepts.
pub const MAX_SOUND_BYTES: usize = 512 * 1024;
/// The longest clip a server accepts, measured from the decoded stream.
pub const MAX_SOUND_MS: u32 = 5_200;
/// Characters in a sound's name.
pub const MAX_SOUND_NAME_CHARS: usize = 32;
/// Characters in a sound's emoji (one Unicode emoji, possibly with modifiers).
pub const MAX_SOUND_EMOJI_CHARS: usize = 16;
/// Favorites one account can keep.
pub const MAX_SOUND_FAVORITES: usize = 100;
/// One entrance sound per user at most this often, however often they rejoin.
pub const ENTRANCE_SOUND_COOLDOWN_MS: u64 = 60_000;

/// Custom camera backgrounds one account can keep.
pub const MAX_CAMERA_BACKGROUNDS: usize = 10;
/// The largest custom camera background file (still image or short video).
pub const MAX_CAMERA_BACKGROUND_BYTES: usize = 10 * 1024 * 1024;

/// A voice channel's Opus bitrate range, in bits per second.
pub const MIN_VOICE_BITRATE: u32 = 8_000;
pub const MAX_VOICE_BITRATE: u32 = 384_000;
/// A voice channel's bitrate until someone changes it.
pub const DEFAULT_VOICE_BITRATE: u32 = 64_000;
/// Direct and group calls always use this.
pub const CALL_VOICE_BITRATE: u32 = 128_000;
/// The most a publisher may ask the SFU to reserve for one video track.
pub const MAX_TRACK_BITRATE: u64 = 64_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundFormat {
    Mp3,
    Ogg,
    Wav,
}

impl SoundFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Ogg => "audio/ogg",
            Self::Wav => "audio/wav",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundboardSound {
    pub id: Id,
    pub guild_id: GuildId,
    pub name: String,
    pub emoji: Option<String>,
    /// Default playback volume, 0 to 100 percent.
    pub volume: u8,
    pub duration_ms: u32,
    pub format: SoundFormat,
    pub created_by: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundboardGuild {
    pub guild_id: GuildId,
    pub name: String,
    /// Upload, rename and delete: Manage Soundboard or Manage Server.
    pub can_manage: bool,
    pub sounds: Vec<SoundboardSound>,
}

/// Every sound the user can play: their servers' soundboards. `current_guild_id`
/// is the server of the call asked about (none for a direct or group call).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundboardLibrary {
    pub current_guild_id: Option<GuildId>,
    /// Use Soundboard in that call: always true in a direct or group call.
    pub can_play: bool,
    pub guilds: Vec<SoundboardGuild>,
    pub favorites: Vec<Id>,
}

/// Rename a sound, change its emoji or default volume. Absent fields are kept;
/// an empty `emoji` removes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub volume: Option<u8>,
}

/// A per-server choice of entrance sound. `sound_id: None` means none there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntranceOverride {
    pub guild_id: GuildId,
    pub sound_id: Option<Id>,
}

/// The sound played when this user joins a voice channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntranceSettings {
    pub enabled: bool,
    pub default_sound_id: Option<Id>,
    #[serde(default)]
    pub overrides: Vec<EntranceOverride>,
}

/// A guild voice channel's Opus bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceQuality {
    pub channel_id: ChannelId,
    pub bitrate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    Image,
    Video,
}

/// A custom camera background stored on the server for its owner only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraBackground {
    pub id: Id,
    pub kind: BackgroundKind,
    pub mime: String,
    pub bytes: u64,
    pub created_at: UnixMillis,
}

/// What `VoiceEvent::SoundPlayed` names: enough to fetch, cache and play the clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundPlay {
    pub sound_id: Id,
    pub guild_id: GuildId,
    pub name: String,
    pub emoji: Option<String>,
    pub volume: u8,
    pub duration_ms: u32,
}

/// A valid sound name: 1 to 32 characters after trimming, no control characters.
pub fn valid_sound_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.len() == name.len()
        && n.chars().count() <= MAX_SOUND_NAME_CHARS
        && !n.chars().any(char::is_control)
}

/// A sound's emoji: short, no ASCII letters/digits/markup, no control characters.
pub fn valid_sound_emoji(emoji: &str) -> bool {
    let count = emoji.chars().count();
    (1..=MAX_SOUND_EMOJI_CHARS).contains(&count)
        && !emoji
            .chars()
            .any(|c| c.is_control() || c.is_ascii_alphanumeric() || "<>&\"' ".contains(c))
        && !emoji.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_names_and_emoji_are_bounded_text() {
        assert!(valid_sound_name("airhorn"));
        assert!(valid_sound_name("Ba dum tss"));
        assert!(!valid_sound_name(""));
        assert!(!valid_sound_name(" padded"));
        assert!(!valid_sound_name(&"x".repeat(33)));
        assert!(!valid_sound_name("bell\u{7}"));
        assert!(valid_sound_emoji("🎺"));
        assert!(valid_sound_emoji("👍🏽"));
        assert!(!valid_sound_emoji(""));
        assert!(!valid_sound_emoji("<b>"));
        assert!(!valid_sound_emoji(":tada:"));
        assert!(!valid_sound_emoji(&"🎺".repeat(17)));
    }

    #[test]
    fn new_signals_and_events_round_trip_as_named_messagepack() {
        for signal in [
            VoiceSignal::PlaySound { sound_id: Id(7) },
            VoiceSignal::SetClipping { enabled: true },
            VoiceSignal::ClipTaken,
            VoiceSignal::TrackQuality {
                mid: "1".into(),
                max_bps: 22_000_000,
            },
        ] {
            let bytes = rmp_serde::to_vec_named(&signal).unwrap();
            assert_eq!(
                rmp_serde::from_slice::<VoiceSignal>(&bytes).unwrap(),
                signal
            );
        }
        let session = VoiceSession {
            user_id: Id(1),
            connection_id: Id(2),
        };
        for event in [
            VoiceEvent::SoundPlayed {
                channel_id: Id(3),
                sound: SoundPlay {
                    sound_id: Id(7),
                    guild_id: Id(10),
                    name: "airhorn".into(),
                    emoji: Some("🎺".into()),
                    volume: 80,
                    duration_ms: 1200,
                },
                by: session,
                entrance: false,
            },
            VoiceEvent::Clipping {
                session,
                enabled: true,
            },
            VoiceEvent::ClipTaken { session },
            VoiceEvent::AudioQuality {
                channel_id: Id(3),
                bitrate: 96_000,
            },
        ] {
            let bytes = rmp_serde::to_vec_named(&event).unwrap();
            assert_eq!(rmp_serde::from_slice::<VoiceEvent>(&bytes).unwrap(), event);
        }
    }

    #[test]
    fn patch_fields_are_optional_on_the_wire() {
        let p: SoundPatch = serde_json::from_str("{\"volume\":40}").unwrap();
        assert_eq!(
            p,
            SoundPatch {
                volume: Some(40),
                ..Default::default()
            }
        );
        let e: EntranceSettings =
            serde_json::from_str("{\"enabled\":true,\"default_sound_id\":5}").unwrap();
        assert!(e.overrides.is_empty());
    }
}
