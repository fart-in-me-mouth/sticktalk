//! Stage channels: one-to-many audio rooms (`docs/features/stage.md`).
//!
//! A Stage is a channel of kind `stage`. Its media is the ordinary voice path
//! (`VoiceSignal::Join` with the Stage's channel ID). What a Stage adds is a
//! role for each person in it: **speakers** publish like anyone in a voice
//! channel; the **audience** only listens. The server gives an audience
//! member's session no right to publish, and the SFU treats it as a listener:
//! it never appears in `Participant`, `Joined` or `GuildState`, and any media
//! it sends is dropped. Stage moderators (Manage Channels, Mute Members and
//! Move Members on the channel, as Discord defines them) start and end the
//! Stage and decide who speaks.
//!
//! All Stage signals and events travel in the voice envelope, like calls.
use crate::*;
use serde::{Deserialize, Serialize};

/// A Stage's topic, 1 to this many characters.
pub const MAX_STAGE_TOPIC_CHARS: usize = 120;
/// The optional note shown to everyone before and while they listen
/// ("This Stage is recorded", "Members only, please do not share").
pub const MAX_STAGE_NOTE_CHARS: usize = 200;
/// People who may speak at once. Each may be connected from more than one device.
pub const MAX_STAGE_SPEAKERS: usize = 16;
/// Raised hands a Stage keeps in its queue.
pub const MAX_STAGE_REQUESTS: usize = 250;

/// A live Stage, as the channel list shows it. Full replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageInstance {
    pub channel_id: ChannelId,
    pub guild_id: GuildId,
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_note: Option<String>,
    pub started_by: UserId,
    pub started_at: UnixMillis,
    /// People (not devices) on the Stage now.
    pub speakers: u32,
    /// People listening now.
    pub listeners: u32,
    /// The scheduled event this Stage is the session of, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageRole {
    Speaker,
    Audience,
}

/// One person connected to a Stage (one entry however many devices they use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageMember {
    pub user_id: UserId,
    /// Their name in this server (nickname, display name or username).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_id: Option<AttachmentId>,
    pub role: StageRole,
    /// A Stage moderator here.
    #[serde(default)]
    pub moderator: bool,
    /// When they raised their hand. Shown only to moderators and to themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_raised_at: Option<UnixMillis>,
    /// Asked to speak by a moderator and not answered yet. Shown only to
    /// moderators and to themselves.
    #[serde(default)]
    pub invited: bool,
}

/// Everyone on the Stage you are connected to, with the Stage itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRoster {
    pub channel_id: ChannelId,
    /// None while the Stage is not live (a moderator waiting to start it).
    pub instance: Option<StageInstance>,
    /// Speakers first, then the audience, each in the order they joined.
    pub members: Vec<StageMember>,
}

/// The record a finished Stage leaves in its channel's chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageSummary {
    pub topic: String,
    pub started_at: UnixMillis,
    pub ended_at: UnixMillis,
    /// The most people listening at once.
    pub peak_listeners: u32,
    /// Distinct people who spoke.
    pub speakers: u32,
}

impl StageSummary {
    pub fn duration_ms(&self) -> u64 {
        self.ended_at.saturating_sub(self.started_at)
    }
}

/// What a Stage or scheduled event adds to a message (`MessageExtra::stage`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StageMessage {
    /// The system message a finished Stage leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<StageSummary>,
    /// An event shared to the channel, shown as a card (`schedule.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<EventCard>,
}

/// Stage actions. Host-handled; the SFU rejects them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StageSignal {
    /// A moderator opens the Stage. If they are connected to it they become a speaker.
    Start {
        channel_id: ChannelId,
        topic: String,
        privacy_note: Option<String>,
        /// Mark this scheduled event live as the Stage's session.
        event_id: Option<Id>,
    },
    /// A moderator changes the topic or note of a live Stage.
    Update {
        channel_id: ChannelId,
        topic: String,
        privacy_note: Option<String>,
    },
    /// A moderator ends the Stage: everyone is disconnected and the channel's
    /// chat gets the record.
    End { channel_id: ChannelId },
    /// An audience member raises (or lowers) their hand: "Request to Speak".
    RaiseHand { channel_id: ChannelId, raised: bool },
    /// A moderator lets someone with a raised hand speak.
    Accept {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// A moderator lowers someone's raised hand.
    Dismiss {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// A moderator invites an audience member to speak; they must accept.
    Invite {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// The invited member's answer.
    AnswerInvite { channel_id: ChannelId, accept: bool },
    /// Back to the audience: a moderator moving a speaker, or a speaker
    /// moving themselves (`user_id` is then their own).
    ToAudience {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// A moderator in the audience moves themselves onto the Stage.
    BecomeSpeaker { channel_id: ChannelId },
}

/// Stage events, in the voice envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StageEvent {
    /// A Stage started or changed (topic, counts). Sent to everyone who can
    /// see the channel; count changes are combined, a few a second at most.
    Live(StageInstance),
    /// A Stage ended.
    Ended {
        guild_id: GuildId,
        channel_id: ChannelId,
    },
    /// Every live Stage you can see, after Ready and Resumed.
    Snapshot { stages: Vec<StageInstance> },
    /// The Stage you just connected to: everyone in it. Also sent again when
    /// your moderator status changes.
    Roster(StageRoster),
    /// Changes to the people on the Stage you are connected to, combined over
    /// a short interval. `upserts` replace whole entries.
    Members {
        channel_id: ChannelId,
        upserts: Vec<StageMember>,
        removed: Vec<UserId>,
    },
}
