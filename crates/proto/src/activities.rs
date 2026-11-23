//! Shared Activities (`docs/features/activities.md`, `docs/ACTIVITIES.md`).
//!
//! An activity is a small web app that people in a call run together: Watch
//! Together, a whiteboard, board games. Each runs in a sandboxed frame and
//! talks to the StickTalk page only through the embedded app SDK. The server
//! relays a bounded key-value document per session and ephemeral messages
//! between the participants. Everything travels in the voice envelope, like
//! calls and Stages: `VoiceSignal::Activities` and `VoiceEvent::Activities`.
//!
//! A session is bound to one call (a guild voice channel, a DM or a group DM
//! call) and ends when its last participant leaves it or the call ends.
use crate::*;
use serde::{Deserialize, Serialize};

/// The version of the embedded app SDK's message protocol.
pub const ACTIVITY_SDK_VERSION: u16 = 1;
/// A session's shared document: keys plus values, in bytes.
pub const MAX_ACTIVITY_STATE_BYTES: usize = 256 * 1024;
/// Keys in one session's document.
pub const MAX_ACTIVITY_KEYS: usize = 2048;
/// One key, in bytes (ASCII letters, digits and `/ _ - . :`).
pub const MAX_ACTIVITY_KEY_BYTES: usize = 64;
/// One value (JSON text), in bytes.
pub const MAX_ACTIVITY_VALUE_BYTES: usize = 16 * 1024;
/// Operations in one `Write`.
pub const MAX_ACTIVITY_OPS: usize = 32;
/// One ephemeral message or action (JSON text), in bytes.
pub const MAX_ACTIVITY_MESSAGE_BYTES: usize = 2048;
/// People (devices) in one session: a call's own limit.
pub const MAX_ACTIVITY_PARTICIPANTS: usize = 16;
/// A saved result ("Ana won against Ben"), in characters.
pub const MAX_ACTIVITY_RESULT_CHARS: usize = 160;
/// Invite cards one session may post.
pub const MAX_ACTIVITY_CARDS: usize = 4;
/// Files one session may share (Watch Together's queue).
pub const MAX_ACTIVITY_MEDIA: usize = 32;
/// Bytes of entries in one `State` event; a larger document is sent in parts.
pub const MAX_ACTIVITY_STATE_CHUNK: usize = 48 * 1024;
/// Activities the instance administrator may register.
pub const MAX_CUSTOM_ACTIVITIES: usize = 32;
/// Results kept per conversation (oldest removed first).
pub const MAX_ACTIVITY_RESULTS: usize = 50;

/// An activity that can be launched: built in, or registered by the instance
/// administrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityInfo {
    /// `watch-together`, `whiteboard`, `four-in-a-row`, `checkers`,
    /// `sketch-guess`, or `custom-<n>` for a registered one.
    pub id: String,
    pub name: String,
    pub description: String,
    /// An icon name of the app's set (built-in activities) or one emoji.
    pub icon: String,
    pub min_players: u8,
    pub max_players: u8,
    /// Where its frame loads: `/activities/<id>/` for built-in activities,
    /// `/activities/frame/<id>` (the shell that holds the registered origin)
    /// for registered ones.
    pub url: String,
    #[serde(default)]
    pub builtin: bool,
    /// The registered activity's origin (`https://games.example`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// One device taking part in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityParticipant {
    /// Small number naming this participant inside the session. The frame
    /// sees only this, the name and the picture, never account IDs.
    pub pid: u16,
    pub session: VoiceSession,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_id: Option<AttachmentId>,
    pub joined_at: UnixMillis,
}

/// A running session, as everyone in its call sees it. Full replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySession {
    pub session_id: Id,
    /// The call: a guild voice channel, a DM or a group DM.
    pub channel_id: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    pub activity: ActivityInfo,
    pub started_by: UserId,
    pub started_at: UnixMillis,
    /// The participant whose writes to `h/` keys count (0 while nobody is in it).
    pub host: u16,
    /// The host opened control to everyone: anyone may write `h/` keys.
    #[serde(default)]
    pub shared_control: bool,
    pub participants: Vec<ActivityParticipant>,
    /// The receiving connection's own participant number, when it takes part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub you: Option<u16>,
}

/// What a finished session leaves: on its invite cards and in the
/// conversation's results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySummary {
    pub activity_id: String,
    pub name: String,
    pub started_at: UnixMillis,
    pub ended_at: UnixMillis,
    /// Distinct people who took part.
    pub people: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// One key of a session's document. `value: None` means it was deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The document's version when this key was last written.
    pub version: u64,
    /// Who wrote it (0: the server).
    pub by: u16,
}

/// One change in a `Write`. `value: None` deletes the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOp {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
    /// Only if the key's version is this (0: only if it does not exist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<u64>,
}

/// Activity actions, in the voice envelope. Host-handled; the SFU rejects them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ActivitySignal {
    /// Start an activity in the call you are in (you join it at once).
    Launch {
        channel_id: ChannelId,
        activity_id: String,
    },
    Join {
        session_id: Id,
    },
    Leave {
        session_id: Id,
    },
    /// The host, or a moderator of the channel (Move Members), ends it for everyone.
    End {
        session_id: Id,
    },
    /// The host hands hosting to another participant.
    SetHost {
        session_id: Id,
        pid: u16,
    },
    /// The host lets everyone (or again only the host) write `h/` keys.
    SetShared {
        session_id: Id,
        shared: bool,
    },
    /// Change the shared document: all operations apply, or none.
    Write {
        session_id: Id,
        ops: Vec<ActivityOp>,
    },
    /// An ephemeral message (JSON text) to these participants (empty: everyone else).
    Send {
        session_id: Id,
        to: Vec<u16>,
        data: String,
    },
    /// A move for a built-in game; the server's rules decide (JSON text).
    Action {
        session_id: Id,
        data: String,
    },
    /// Share a file you can read into the session (Watch Together).
    ShareMedia {
        session_id: Id,
        attachment_id: AttachmentId,
    },
    /// The host keeps a short result for when the session ends.
    SaveResult {
        session_id: Id,
        text: String,
    },
    /// Post an invite card for this session into a conversation.
    Invite {
        session_id: Id,
        channel_id: ChannelId,
    },
}

/// Activity state, in the voice envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ActivityEvent {
    /// Started or changed. To everyone in the call.
    Session(ActivitySession),
    /// Over. To everyone in the call.
    Ended {
        session_id: Id,
        channel_id: ChannelId,
        summary: ActivitySummary,
    },
    /// The document, to participants: the whole of it after joining
    /// (`reset`, in parts until `done`), then each change.
    State {
        session_id: Id,
        reset: bool,
        done: bool,
        version: u64,
        server_time: UnixMillis,
        entries: Vec<ActivityEntry>,
    },
    /// An ephemeral message from a participant (from 0: the server's game).
    Message {
        session_id: Id,
        from: u16,
        data: String,
    },
}

/// `extra.activity` on a message: an invite card. Kept current by the server
/// while the session runs; shows the summary once it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityCard {
    pub session_id: Id,
    /// The call to join.
    pub call_channel_id: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    pub activity_id: String,
    pub name: String,
    pub icon: String,
    /// Names of the people in it now (at most eight).
    pub people: Vec<String>,
    /// How many people are in it now.
    pub count: u16,
    pub live: bool,
    pub started_at: UnixMillis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ActivitySummary>,
}

/// `GET /api/v1/activities?channel_id=`: what can be launched in that call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityCatalog {
    pub activities: Vec<ActivityInfo>,
    /// Use Activities in this channel (always in direct and group calls).
    pub can_launch: bool,
    /// Results of recent sessions in this conversation, newest first.
    #[serde(default)]
    pub results: Vec<ActivityResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityResult {
    pub name: String,
    pub text: String,
    pub ended_at: UnixMillis,
}

/// `GET /api/v1/guilds/{id}/activities` (Manage Server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildActivities {
    pub activities: Vec<GuildActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildActivity {
    pub activity: ActivityInfo,
    pub enabled: bool,
}

/// `PUT /api/v1/guilds/{id}/activities/{activity_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityToggle {
    pub enabled: bool,
}

/// `GET /api/v1/activities/media?channel_id=`: audio and video you can share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityMediaList {
    pub items: Vec<ActivityMedia>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityMedia {
    pub attachment_id: AttachmentId,
    pub filename: String,
    pub content_type: String,
    pub byte_size: u64,
    pub channel_id: ChannelId,
    pub created_at: UnixMillis,
}

/// Built-in activity icons and registered ones' emoji must be short and plain.
pub fn valid_activity_icon(icon: &str) -> bool {
    let n = icon.chars().count();
    (1..=16).contains(&n)
        && (icon
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !icon
                .chars()
                .any(|c| c.is_ascii() || c.is_control() || c.is_whitespace()))
}

/// Keys of a session's document: 1 to 64 bytes of `A-Z a-z 0-9 / _ - . :`.
pub fn valid_activity_key(key: &str) -> bool {
    (1..=MAX_ACTIVITY_KEY_BYTES).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.:".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_icons_are_checked() {
        assert!(valid_activity_key("h/media"));
        assert!(valid_activity_key("s:1/a-b.c_d"));
        assert!(!valid_activity_key(""));
        assert!(!valid_activity_key("a b"));
        assert!(!valid_activity_key(&"k".repeat(65)));
        assert!(valid_activity_icon("pencil"));
        assert!(valid_activity_icon("🎲"));
        assert!(!valid_activity_icon("a 🎲"));
        assert!(!valid_activity_icon("<b>"));
    }

    #[test]
    fn signals_round_trip_in_the_voice_envelope() {
        let s = VoiceSignal::Activities(ActivitySignal::Write {
            session_id: Id(5),
            ops: vec![ActivityOp {
                key: "h/x".into(),
                value: Some("1".into()),
                expect: None,
            }],
        });
        let bytes = rmp_serde::to_vec_named(&s).unwrap();
        assert_eq!(rmp_serde::from_slice::<VoiceSignal>(&bytes).unwrap(), s);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["type"], "activities");
        assert_eq!(json["data"]["type"], "write");
    }
}
