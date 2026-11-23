//! Scheduled server events (`docs/features/stage.md`, "Scheduled events").
//!
//! An event is a *series*: one row with a rule (`EventRecurrence`) computed in
//! the creator's time zone. Each date the rule produces is an *occurrence*,
//! identified by the time the rule put it at (`occurrence`), which edits never
//! change. Editing "this event" stores an exception for that occurrence;
//! "this and following" ends the series there and starts a new one.
//!
//! Commands: `Command::Schedule(ScheduleCommand)`. Events: `Event::Schedule`,
//! sent only to connections that sent a `ScheduleCommand` (the web app sends
//! `Subscribe` after every Ready). Lists and details are REST reads, fetched
//! when a panel opens.
use crate::*;
use serde::{Deserialize, Serialize};

pub const MAX_EVENT_NAME_CHARS: usize = 100;
pub const MAX_EVENT_DESCRIPTION_CHARS: usize = 1000;
pub const MAX_EVENT_PLACE_CHARS: usize = 100;
/// Scheduled, live and recurring events a server may have at once.
pub const MAX_GUILD_EVENTS: usize = 100;
/// A cover image: PNG, JPEG, WebP or GIF (first frame), at most this size.
pub const MAX_EVENT_COVER_BYTES: usize = 4 * 1024 * 1024;
/// The longest one occurrence may last.
pub const MAX_EVENT_DURATION_MS: u64 = 7 * 24 * 3_600_000;
/// How far ahead an event may be scheduled.
pub const MAX_EVENT_LEAD_MS: u64 = 5 * 365 * 24 * 3_600_000;
/// Ended and cancelled events are deleted this long after they end.
pub const EVENT_RETENTION_MS: u64 = 7 * 24 * 3_600_000;
/// A Stage or voice occurrence nobody started lapses this long after its end
/// (or, with no end time, after its start).
pub const EVENT_LAPSE_MS: u64 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventLocation {
    /// A Stage channel in the event's server.
    Stage { channel_id: ChannelId },
    /// A voice channel in the event's server.
    Voice { channel_id: ChannelId },
    /// Somewhere else: a place or a link, as free text.
    External { place: String },
}

impl EventLocation {
    pub fn channel(&self) -> Option<ChannelId> {
        match self {
            Self::Stage { channel_id } | Self::Voice { channel_id } => Some(*channel_id),
            Self::External { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRecurrence {
    #[default]
    None,
    Daily,
    Weekly,
    EveryTwoWeeks,
    Monthly,
    Yearly,
    /// Monday to Friday.
    Weekdays,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Scheduled,
    Live,
    Ended,
    Cancelled,
}

/// Which occurrences an edit or cancellation applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditScope {
    /// Only the named occurrence.
    #[default]
    This,
    /// The named occurrence and every later one.
    Following,
}

/// One occurrence of an event (for a one-off event, the event itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    /// The series.
    pub id: Id,
    pub guild_id: GuildId,
    pub creator_id: UserId,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Present when it has a cover image (`GET /api/v1/events/{id}/cover?v=`);
    /// the number changes when the image does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<u32>,
    /// Where the rule put this occurrence (its identity; edits keep it).
    pub occurrence: UnixMillis,
    pub start_at: UnixMillis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<UnixMillis>,
    pub location: EventLocation,
    pub status: EventStatus,
    pub recurrence: EventRecurrence,
    /// The IANA time zone the recurrence is computed in, e.g. "Europe/Berlin".
    pub time_zone: String,
    /// Members who marked the event Interested.
    pub interested: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewEvent {
    pub guild_id: GuildId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub start_at: UnixMillis,
    #[serde(default)]
    pub end_at: Option<UnixMillis>,
    pub location: EventLocation,
    #[serde(default)]
    pub recurrence: EventRecurrence,
    pub time_zone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EventPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<UnixMillis>,
    #[serde(default)]
    pub end_at: Patch<UnixMillis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EventLocation>,
    /// Only with `EditScope::Following` (or on a one-off event).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<EventRecurrence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

/// An event as a card in chat: what it was when shared. The card asks the
/// server (or the live summary) for its current status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCard {
    pub event_id: Id,
    pub guild_id: GuildId,
    pub name: String,
    pub start_at: UnixMillis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<UnixMillis>,
    pub location: EventLocation,
    /// The channel's name when shared, for a Stage or voice location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<u32>,
    pub recurrence: EventRecurrence,
}

/// What the channel list needs: how many events a server has coming up or
/// live, and which are live now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildEvents {
    pub guild_id: GuildId,
    pub count: u32,
    pub live: Vec<Id>,
}

/// `GET /api/v1/guilds/{id}/events`: the next occurrence of each event, live
/// ones first, then by start time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    pub events: Vec<ScheduledEvent>,
    /// The events you marked Interested.
    pub interested: Vec<Id>,
    pub can_create: bool,
    /// You may edit, start, end, cancel and delete every event here.
    pub can_manage: bool,
}

/// `GET /api/v1/events/{id}?occurrence=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDetail {
    pub event: ScheduledEvent,
    pub me_interested: bool,
    pub can_manage: bool,
    /// The location channel's name, for a Stage or voice location you can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_name: Option<String>,
    /// The next occurrences after this one (at most five).
    pub upcoming: Vec<UnixMillis>,
}

/// `GET /api/v1/events/{id}/interested?after=`: 50 a page, in user ID order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterestedPage {
    pub users: Vec<UserSummary>,
    pub next: Option<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ScheduleCommand {
    /// Opt this connection in to `Event::Schedule`; answers `Summary`.
    Subscribe,
    /// Create Events (your own) or Manage Events.
    Create(NewEvent),
    Update {
        event_id: Id,
        occurrence: UnixMillis,
        #[serde(default)]
        scope: EditScope,
        patch: EventPatch,
    },
    Cancel {
        event_id: Id,
        occurrence: UnixMillis,
        #[serde(default)]
        scope: EditScope,
    },
    Delete {
        event_id: Id,
    },
    SetInterested {
        event_id: Id,
        interested: bool,
    },
    /// Go live now (a Stage or voice event; an external one starts by itself).
    Start {
        event_id: Id,
    },
    End {
        event_id: Id,
    },
    /// Post the event's card in a channel (Send Messages there).
    Share {
        event_id: Id,
        channel_id: ChannelId,
    },
    /// An invite to the server that leads to this event (Create Invite).
    CreateInvite {
        event_id: Id,
        ttl_seconds: u32,
        max_uses: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ScheduleResult {
    Ack,
    Summary { guilds: Vec<GuildEvents> },
    Event(ScheduledEvent),
    Message(Message),
    Invite(Invite),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ScheduleUpdate {
    /// Created, edited, started, ended or cancelled: its next occurrence.
    Upsert(ScheduledEvent),
    Removed {
        guild_id: GuildId,
        event_id: Id,
    },
    Summary(GuildEvents),
    /// Your own devices: you marked it Interested, or stopped.
    Interest {
        event_id: Id,
        interested: bool,
    },
    /// An event you are interested in is starting now.
    Starting(ScheduledEvent),
}

/// `GET /api/v1/invites/{code}/event`: the event an invite leads to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteEvent {
    pub event: EventCard,
    pub status: EventStatus,
}
