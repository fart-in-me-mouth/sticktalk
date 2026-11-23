//! Presence and custom status, friend nicknames and notes, ignoring people,
//! who can message you (message requests and spam), notification controls,
//! in-app reports and the server side of age-restricted channels.
//! See `docs/features/privacy.md`.
//!
//! Commands travel as `Command::Privacy`. Live changes travel as
//! `Event::Privacy`, which the server sends only to connections that sent
//! `PrivacyCommand::Subscribe`, so older clients never see an unknown variant.
use crate::*;
use serde::{Deserialize, Serialize};

/// A custom status emoji: one Unicode emoji, or a custom emoji reference
/// (`<:name:id>`), kept general so custom server emoji fit too.
pub const MAX_STATUS_EMOJI_CHARS: usize = 64;
/// Custom status text (the existing `status_text`).
pub const MAX_STATUS_TEXT_CHARS: usize = 128;
/// "Clear after" may be at most a year ahead.
pub const MAX_STATUS_CLEAR_MS: u64 = 366 * 86_400_000;
/// A timed status ("Do Not Disturb for 8 hours") ends at most a day ahead.
pub const MAX_STATUS_DURATION_MS: u64 = 86_400_000;
/// Without a focused window for this long, Online shows as Idle.
pub const AUTO_IDLE_MS: u64 = 10 * 60_000;
pub const MAX_FRIEND_NICKNAME_CHARS: usize = 32;
/// One per friend at most; friends are bounded by the relationship limit.
pub const MAX_FRIEND_NICKNAMES: usize = MAX_RELATIONSHIPS_PER_USER;
pub const MAX_NOTE_CHARS: usize = 256;
pub const MAX_NOTES: usize = 500;
pub const MAX_IGNORED_USERS: usize = 100;
/// First-contact DMs one account may start: per hour, and per day.
pub const FIRST_CONTACTS_PER_HOUR: u32 = 10;
pub const FIRST_CONTACTS_PER_DAY: u32 = 30;
/// Requests listed (newest first), each list.
pub const MAX_LISTED_REQUESTS: usize = 100;
pub const MAX_REPORT_NOTE_CHARS: usize = 1_000;
/// Evidence text kept with a report (the message as it was), in characters.
pub const MAX_REPORT_EVIDENCE_CHARS: usize = 4_000;
/// Reports one account may file: per hour, and open at once.
pub const REPORTS_PER_HOUR: u32 = 10;
pub const MAX_OPEN_REPORTS_PER_USER: usize = 50;
/// Reports listed in a queue page.
pub const MAX_REPORTS_PAGE: usize = 50;
/// A notification mute set for a duration ends at most a year ahead.
pub const MAX_MUTE_MS: u64 = 366 * 86_400_000;

/// The status the user chose, and when it goes back to Online (None: until changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatusChoice {
    pub status: PresenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<UnixMillis>,
}

/// Custom status: text, one emoji, and when it clears itself (None: never).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_at: Option<UnixMillis>,
}

/// A private name for a friend, shown only to the person who set it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendNickname {
    pub user_id: UserId,
    pub nickname: String,
}

/// A private note about anyone, shown only to the person who wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNote {
    pub user_id: UserId,
    pub note: String,
    pub updated_at: UnixMillis,
}

/// Who may start a direct message with you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmAllow {
    /// Only friends (the default, and StickTalk's behavior before requests).
    #[default]
    Friends,
    /// Friends, and members of servers you share (as message requests).
    ServerMembers,
}

/// A server where your choice differs from the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmServerOverride {
    pub guild_id: GuildId,
    /// Members of this server may (true) or may not (false) send you requests.
    pub allow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DmPrivacy {
    pub allow: DmAllow,
    #[serde(default)]
    pub servers: Vec<DmServerOverride>,
}

/// How the unread badge (window title and app icon) counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeMode {
    /// Mentions and direct messages (the default).
    #[default]
    Mentions,
    /// Every unread conversation that is not muted.
    All,
    Off,
}

/// Which reactions to your messages notify you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionAlerts {
    /// Reactions to your messages in direct and group messages (the default).
    #[default]
    Direct,
    All,
    Never,
}

/// Account-wide notification preferences (they follow the account).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalNotifications {
    pub desktop: bool,
    /// Master switch for every notification sound below.
    pub sounds: bool,
    pub sound_message: bool,
    pub sound_mention: bool,
    pub sound_direct: bool,
    pub sound_reaction: bool,
    pub badge: BadgeMode,
    pub reactions: ReactionAlerts,
}
impl Default for GlobalNotifications {
    fn default() -> Self {
        Self {
            desktop: true,
            sounds: true,
            sound_message: true,
            sound_mention: true,
            sound_direct: true,
            sound_reaction: false,
            badge: BadgeMode::Mentions,
            reactions: ReactionAlerts::Direct,
        }
    }
}

/// The full notification preference for one server, channel, DM or group DM:
/// the existing `NotificationSettings` plus the controls added here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationOptions {
    pub settings: NotificationSettings,
    /// Role mentions do not notify, badge or reach the Inbox.
    #[serde(default)]
    pub suppress_roles: bool,
    /// Push to phones and away devices (server-wide setting; ignored on channels).
    #[serde(default = "yes")]
    pub mobile_push: bool,
}
fn yes() -> bool {
    true
}

/// The added controls for one scope (the base settings are in Ready).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationExtra {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub suppress_roles: bool,
    pub mobile_push: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RequestCounts {
    pub pending: u32,
    pub spam: u32,
}

/// Everything here that belongs to the signed-in account.
/// `GET /api/v1/users/@me/privacy`; also opts the stream in to `Event::Privacy`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrivacySettings {
    pub status: StatusChoice,
    /// The automatic idle is in effect (no window focused for a while).
    pub auto_idle: bool,
    pub custom: CustomStatus,
    pub nicknames: Vec<FriendNickname>,
    pub ignored: Vec<UserSummary>,
    pub dm: DmPrivacy,
    pub global: GlobalNotifications,
    pub notification_extras: Vec<NotificationExtra>,
    pub requests: RequestCounts,
}

/// A first-contact DM waiting for the recipient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRequest {
    pub channel_id: ChannelId,
    pub user: UserSummary,
    pub created_at: UnixMillis,
    pub spam: bool,
    /// Servers you share with the sender.
    pub mutual_servers: Vec<Guild>,
    /// Their newest messages, newest first (at most 3).
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageRequests {
    pub requests: Vec<MessageRequest>,
    pub spam: Vec<MessageRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportCategory {
    Spam,
    Harassment,
    Hate,
    Violence,
    SelfHarm,
    Sexual,
    MinorSafety,
    Impersonation,
    Illegal,
    Other,
}
impl ReportCategory {
    pub const ALL: &'static [ReportCategory] = &[
        Self::Spam,
        Self::Harassment,
        Self::Hate,
        Self::Violence,
        Self::SelfHarm,
        Self::Sexual,
        Self::MinorSafety,
        Self::Impersonation,
        Self::Illegal,
        Self::Other,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Spam => "Spam or scam",
            Self::Harassment => "Harassment or bullying",
            Self::Hate => "Hateful content",
            Self::Violence => "Violence or threats",
            Self::SelfHarm => "Self-harm",
            Self::Sexual => "Unwanted sexual content",
            Self::MinorSafety => "A minor's safety",
            Self::Impersonation => "Impersonation",
            Self::Illegal => "Illegal activity",
            Self::Other => "Something else",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ReportTarget {
    Message {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    User {
        user_id: UserId,
    },
    Guild {
        guild_id: GuildId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDraft {
    pub target: ReportTarget,
    pub category: ReportCategory,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Open,
    Dismissed,
    Actioned,
}

/// Who handles a report: the server's moderators, or the instance administrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportQueue {
    Guild,
    Admin,
}

/// What was reported, captured when the report was filed, so later edits or
/// deletion do not erase it. Bounded (`MAX_REPORT_EVIDENCE_CHARS`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReportEvidence {
    /// The reported person (the message's author, or the reported user).
    #[serde(default)]
    pub user: Option<UserSummary>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    #[serde(default)]
    pub sent_at: Option<UnixMillis>,
    #[serde(default)]
    pub edited_at: Option<UnixMillis>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub guild_name: Option<String>,
    /// Profile text of a reported user (about me), clipped.
    #[serde(default)]
    pub about: Option<String>,
}

/// A report as its handlers see it. The reporter is never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub id: Id,
    pub target: ReportTarget,
    pub queue: ReportQueue,
    pub guild_id: Option<GuildId>,
    pub category: ReportCategory,
    pub note: Option<String>,
    pub evidence: ReportEvidence,
    pub status: ReportStatus,
    /// What was done (`dismiss`, `delete_message`, `timeout`, `kick`, `ban`, ...).
    pub action: Option<String>,
    pub created_at: UnixMillis,
    pub resolved_at: Option<UnixMillis>,
    /// The reported message is in a channel this moderator cannot read now:
    /// `evidence` carries who was reported, not what was said. Omitted when
    /// false (older servers never set it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub evidence_hidden: bool,
}

/// A report as the person who filed it sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyReport {
    pub id: Id,
    pub target: ReportTarget,
    pub category: ReportCategory,
    pub status: ReportStatus,
    pub created_at: UnixMillis,
    pub resolved_at: Option<UnixMillis>,
    /// What it was about, in a few words ("A message from @alex").
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReportPage {
    pub reports: Vec<Report>,
    pub next_before: Option<Id>,
}

/// What a server moderator does about a report. Each runs the normal command
/// (with its own permission check and audit entry), then closes the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ReportAction {
    Dismiss,
    DeleteMessage,
    Timeout { seconds: u32 },
    Kick,
    Ban,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PrivacyCommand {
    /// Opt this event stream in to `Event::Privacy`. Answers with the settings.
    Subscribe,
    /// Online, Idle, Do Not Disturb or Invisible, optionally until a time.
    SetStatus {
        status: PresenceStatus,
        #[serde(default)]
        until: Option<UnixMillis>,
    },
    /// Replace the custom status (all fields None clears it).
    SetCustomStatus(CustomStatus),
    /// None or empty removes it. Friends only.
    SetFriendNickname {
        user_id: UserId,
        nickname: Option<String>,
    },
    /// None or empty removes it.
    SetNote {
        user_id: UserId,
        note: Option<String>,
    },
    Ignore {
        user_id: UserId,
    },
    Unignore {
        user_id: UserId,
    },
    SetDmPrivacy(DmPrivacy),
    /// The request becomes a normal DM.
    AcceptRequest {
        channel_id: ChannelId,
    },
    /// Remove the request; it stays hidden and the sender is not told.
    /// "Block and report" is this, then `BlockUser`, then `Report`.
    DeclineRequest {
        channel_id: ChannelId,
    },
    /// Notification preferences for a server, channel, DM or group DM.
    SetNotifications(NotificationOptions),
    SetGlobalNotifications(GlobalNotifications),
    Report(ReportDraft),
}

/// `POST /api/v1/guilds/{g}/reports/{id}/resolve`: a server moderator handles
/// a report about that server's content. Answers with the updated `Report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportResolution {
    pub action: ReportAction,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PrivacyResult {
    Settings(Box<PrivacySettings>),
    Note(UserNote),
    Filed(MyReport),
}

/// Why the app should alert (sound and desktop notification), decided by the
/// server from the recipient's preferences. Never sent in Do Not Disturb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Message,
    Mention,
    Direct,
    Reaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub kind: AlertKind,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    /// Who wrote it (or reacted).
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// When the server decided it. A resumed stream replays events, so the
    /// app ignores alerts that are more than a few seconds old.
    pub at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PrivacyEvent {
    /// This account's privacy settings changed (another device, or a timer).
    Settings(Box<PrivacySettings>),
    Alert(Alert),
    /// New or removed message requests: a quiet badge, never a notification.
    Requests(RequestCounts),
    /// The open report count of a server's queue changed (moderators only).
    GuildReports {
        guild_id: GuildId,
        open: u32,
    },
    /// One of your reports was handled.
    ReportUpdated(MyReport),
    /// Your note about someone changed (another device).
    Note(UserNote),
}
