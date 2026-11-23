//! Moderation contracts: audit log, timeouts, prune, AutoMod, raid response,
//! verification levels and age-restricted channels. See `docs/features/moderation.md`.
//!
//! Every type here is additive. Commands travel as `LaunchCommand::Moderation`;
//! live changes travel as `Event::Moderation`, which the server sends only to
//! event streams that opted in with `ModerationCommand::Subscribe`.
use crate::*;
use serde::{Deserialize, Serialize};

/// Newest entries kept per server; older ones are dropped as new ones arrive.
pub const MAX_AUDIT_ENTRIES_PER_GUILD: usize = 5_000;
pub const MAX_AUDIT_PAGE: u16 = 100;
/// Before/after summaries are short, readable strings, never full payloads.
pub const MAX_AUDIT_CHANGES: usize = 16;
pub const MAX_AUDIT_VALUE_CHARS: usize = 200;
pub const MAX_REASON_CHARS: usize = 512;
/// The longest timeout, 28 days (the same as Discord).
pub const MAX_TIMEOUT_SECONDS: u32 = 28 * 86_400;
/// The durations the timeout dialog offers; any value up to the maximum is valid.
pub const TIMEOUT_PRESETS: &[u32] = &[60, 300, 600, 3_600, 86_400, 604_800];
/// Inactivity thresholds for pruning members.
pub const PRUNE_DAYS: &[u32] = &[1, 7, 30];
/// Pauses (invites, DMs) last from one hour up to one day.
pub const MIN_PAUSE_SECONDS: u32 = 3_600;
pub const MAX_PAUSE_SECONDS: u32 = 86_400;
pub const MAX_AUTOMOD_RULES: usize = 10;
pub const MAX_AUTOMOD_KEYWORDS: usize = 1_000;
pub const MAX_AUTOMOD_ALLOW: usize = 100;
pub const MAX_KEYWORD_CHARS: usize = 60;
pub const MAX_AUTOMOD_EXEMPT_ROLES: usize = 20;
pub const MAX_AUTOMOD_EXEMPT_CHANNELS: usize = 50;
pub const MAX_AUTOMOD_MESSAGE_CHARS: usize = 150;
/// A raid alert is raised when this many members join within the window.
pub const RAID_JOIN_THRESHOLD: u32 = 10;
pub const RAID_WINDOW_SECONDS: u32 = 60;
/// Verification level thresholds.
pub const ACCOUNT_AGE_MS: u64 = 5 * 60_000;
pub const MEMBERSHIP_AGE_MS: u64 = 10 * 60_000;
/// Initial-snapshot frames (Ready) may exceed the client command frame limit:
/// at the raised capacity limits one account's bootstrap metadata is large.
pub const MAX_SERVER_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// What a member must satisfy before sending messages in a server (or
/// direct-messaging its members). StickTalk has no verified email or phone,
/// so Discord's email/phone levels do not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationLevel {
    #[default]
    None,
    /// The account is older than five minutes.
    AccountAge,
    /// The account is older than five minutes and has been a member for ten.
    MemberAge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    GuildUpdate,
    OwnershipTransfer,
    VerificationUpdate,
    InvitesPause,
    DmsPause,
    RoleCreate,
    RoleUpdate,
    RoleDelete,
    RolesReorder,
    MemberRolesUpdate,
    NicknameUpdate,
    ChannelCreate,
    ChannelUpdate,
    ChannelMove,
    ChannelDelete,
    OverwritesUpdate,
    MemberKick,
    MemberBan,
    MemberUnban,
    MemberTimeout,
    MemberTimeoutRemove,
    MemberPrune,
    InviteCreate,
    InviteDelete,
    EmojiCreate,
    EmojiUpdate,
    EmojiDelete,
    MessageDelete,
    MessagePin,
    MessageUnpin,
    AutomodRuleCreate,
    AutomodRuleUpdate,
    AutomodRuleDelete,
    AutomodBlock,
    AutomodAlert,
    AutomodTimeout,
    StickerCreate,
    StickerUpdate,
    StickerDelete,
    /// A moderator removed a member's server avatar or banner (`profiles.rs`).
    MemberProfileReset,
    /// A moderator handled a member's report (`privacy.rs`).
    ReportResolve,
    // Added with the integration of threads, announcements, soundboard and
    // polls (`docs/features/integration.md`). Appended: wire-compatible.
    ThreadUpdate,
    ThreadDelete,
    ThreadMemberRemove,
    ForumUpdate,
    MessagePublish,
    ChannelFollow,
    ChannelUnfollow,
    SoundCreate,
    SoundUpdate,
    SoundDelete,
    PollEnd,
    EmbedsRemove,
    ApplicationApprove,
    ApplicationReject,
    AdmissionUpdate,
    OnboardingUpdate,
    ListingUpdate,
    TemplateCreate,
    TemplateUpdate,
    TemplateDelete,
    // Scheduled events and Stages (`schedule.rs`, `stage.rs`).
    EventCreate,
    EventUpdate,
    EventCancel,
    EventDelete,
    StageStart,
    StageUpdate,
    StageEnd,
    // Apps, bots and webhooks (`apps.rs`).
    AppInstall,
    AppRemove,
    CommandPermissionsUpdate,
    WebhookCreate,
    WebhookUpdate,
    WebhookDelete,
}
impl AuditAction {
    pub const ALL: &'static [AuditAction] = &[
        Self::GuildUpdate,
        Self::OwnershipTransfer,
        Self::VerificationUpdate,
        Self::InvitesPause,
        Self::DmsPause,
        Self::RoleCreate,
        Self::RoleUpdate,
        Self::RoleDelete,
        Self::RolesReorder,
        Self::MemberRolesUpdate,
        Self::NicknameUpdate,
        Self::ChannelCreate,
        Self::ChannelUpdate,
        Self::ChannelMove,
        Self::ChannelDelete,
        Self::OverwritesUpdate,
        Self::MemberKick,
        Self::MemberBan,
        Self::MemberUnban,
        Self::MemberTimeout,
        Self::MemberTimeoutRemove,
        Self::MemberPrune,
        Self::InviteCreate,
        Self::InviteDelete,
        Self::EmojiCreate,
        Self::EmojiUpdate,
        Self::EmojiDelete,
        Self::MessageDelete,
        Self::MessagePin,
        Self::MessageUnpin,
        Self::AutomodRuleCreate,
        Self::AutomodRuleUpdate,
        Self::AutomodRuleDelete,
        Self::AutomodBlock,
        Self::AutomodAlert,
        Self::AutomodTimeout,
        Self::StickerCreate,
        Self::StickerUpdate,
        Self::StickerDelete,
        Self::MemberProfileReset,
        Self::ReportResolve,
        Self::ThreadUpdate,
        Self::ThreadDelete,
        Self::ThreadMemberRemove,
        Self::ForumUpdate,
        Self::MessagePublish,
        Self::ChannelFollow,
        Self::ChannelUnfollow,
        Self::SoundCreate,
        Self::SoundUpdate,
        Self::SoundDelete,
        Self::PollEnd,
        Self::EmbedsRemove,
        Self::ApplicationApprove,
        Self::ApplicationReject,
        Self::AdmissionUpdate,
        Self::OnboardingUpdate,
        Self::ListingUpdate,
        Self::TemplateCreate,
        Self::TemplateUpdate,
        Self::TemplateDelete,
        Self::EventCreate,
        Self::EventUpdate,
        Self::EventCancel,
        Self::EventDelete,
        Self::StageStart,
        Self::StageUpdate,
        Self::StageEnd,
        Self::AppInstall,
        Self::AppRemove,
        Self::CommandPermissionsUpdate,
        Self::WebhookCreate,
        Self::WebhookUpdate,
        Self::WebhookDelete,
    ];
    /// Stable snake_case name, as used on the wire and in the `action` filter.
    pub fn name(self) -> &'static str {
        match self {
            Self::GuildUpdate => "guild_update",
            Self::OwnershipTransfer => "ownership_transfer",
            Self::VerificationUpdate => "verification_update",
            Self::InvitesPause => "invites_pause",
            Self::DmsPause => "dms_pause",
            Self::RoleCreate => "role_create",
            Self::RoleUpdate => "role_update",
            Self::RoleDelete => "role_delete",
            Self::RolesReorder => "roles_reorder",
            Self::MemberRolesUpdate => "member_roles_update",
            Self::NicknameUpdate => "nickname_update",
            Self::ChannelCreate => "channel_create",
            Self::ChannelUpdate => "channel_update",
            Self::ChannelMove => "channel_move",
            Self::ChannelDelete => "channel_delete",
            Self::OverwritesUpdate => "overwrites_update",
            Self::MemberKick => "member_kick",
            Self::MemberBan => "member_ban",
            Self::MemberUnban => "member_unban",
            Self::MemberTimeout => "member_timeout",
            Self::MemberTimeoutRemove => "member_timeout_remove",
            Self::MemberPrune => "member_prune",
            Self::InviteCreate => "invite_create",
            Self::InviteDelete => "invite_delete",
            Self::EmojiCreate => "emoji_create",
            Self::EmojiUpdate => "emoji_update",
            Self::EmojiDelete => "emoji_delete",
            Self::MessageDelete => "message_delete",
            Self::MessagePin => "message_pin",
            Self::MessageUnpin => "message_unpin",
            Self::AutomodRuleCreate => "automod_rule_create",
            Self::AutomodRuleUpdate => "automod_rule_update",
            Self::AutomodRuleDelete => "automod_rule_delete",
            Self::AutomodBlock => "automod_block",
            Self::AutomodAlert => "automod_alert",
            Self::AutomodTimeout => "automod_timeout",
            Self::StickerCreate => "sticker_create",
            Self::StickerUpdate => "sticker_update",
            Self::StickerDelete => "sticker_delete",
            Self::MemberProfileReset => "member_profile_reset",
            Self::ReportResolve => "report_resolve",
            Self::ThreadUpdate => "thread_update",
            Self::ThreadDelete => "thread_delete",
            Self::ThreadMemberRemove => "thread_member_remove",
            Self::ForumUpdate => "forum_update",
            Self::MessagePublish => "message_publish",
            Self::ChannelFollow => "channel_follow",
            Self::ChannelUnfollow => "channel_unfollow",
            Self::SoundCreate => "sound_create",
            Self::SoundUpdate => "sound_update",
            Self::SoundDelete => "sound_delete",
            Self::PollEnd => "poll_end",
            Self::EmbedsRemove => "embeds_remove",
            Self::ApplicationApprove => "application_approve",
            Self::ApplicationReject => "application_reject",
            Self::AdmissionUpdate => "admission_update",
            Self::OnboardingUpdate => "onboarding_update",
            Self::ListingUpdate => "listing_update",
            Self::TemplateCreate => "template_create",
            Self::TemplateUpdate => "template_update",
            Self::TemplateDelete => "template_delete",
            Self::EventCreate => "event_create",
            Self::EventUpdate => "event_update",
            Self::EventCancel => "event_cancel",
            Self::EventDelete => "event_delete",
            Self::StageStart => "stage_start",
            Self::StageUpdate => "stage_update",
            Self::StageEnd => "stage_end",
            Self::AppInstall => "app_install",
            Self::AppRemove => "app_remove",
            Self::CommandPermissionsUpdate => "command_permissions_update",
            Self::WebhookCreate => "webhook_create",
            Self::WebhookUpdate => "webhook_update",
            Self::WebhookDelete => "webhook_delete",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.name() == s)
    }
    /// A short English label for the audit log viewer.
    pub fn label(self) -> &'static str {
        match self {
            Self::GuildUpdate => "Updated the server",
            Self::OwnershipTransfer => "Transferred ownership",
            Self::VerificationUpdate => "Changed the verification level",
            Self::InvitesPause => "Changed invite pausing",
            Self::DmsPause => "Changed DM pausing",
            Self::RoleCreate => "Created a role",
            Self::RoleUpdate => "Updated a role",
            Self::RoleDelete => "Deleted a role",
            Self::RolesReorder => "Reordered roles",
            Self::MemberRolesUpdate => "Changed a member's roles",
            Self::NicknameUpdate => "Changed a nickname",
            Self::ChannelCreate => "Created a channel",
            Self::ChannelUpdate => "Updated a channel",
            Self::ChannelMove => "Moved channels",
            Self::ChannelDelete => "Deleted a channel",
            Self::OverwritesUpdate => "Changed channel permissions",
            Self::MemberKick => "Kicked a member",
            Self::MemberBan => "Banned a member",
            Self::MemberUnban => "Revoked a ban",
            Self::MemberTimeout => "Timed out a member",
            Self::MemberTimeoutRemove => "Removed a timeout",
            Self::MemberPrune => "Pruned members",
            Self::InviteCreate => "Created an invite",
            Self::InviteDelete => "Deleted an invite",
            Self::EmojiCreate => "Added an emoji",
            Self::EmojiUpdate => "Renamed an emoji",
            Self::EmojiDelete => "Deleted an emoji",
            Self::MessageDelete => "Deleted a message",
            Self::MessagePin => "Pinned a message",
            Self::MessageUnpin => "Unpinned a message",
            Self::AutomodRuleCreate => "Created an AutoMod rule",
            Self::AutomodRuleUpdate => "Updated an AutoMod rule",
            Self::AutomodRuleDelete => "Deleted an AutoMod rule",
            Self::AutomodBlock => "AutoMod blocked a message",
            Self::AutomodAlert => "AutoMod sent an alert",
            Self::AutomodTimeout => "AutoMod timed out a member",
            Self::StickerCreate => "Added a sticker",
            Self::StickerUpdate => "Updated a sticker",
            Self::StickerDelete => "Deleted a sticker",
            Self::MemberProfileReset => "Reset a server profile",
            Self::ReportResolve => "Handled a report",
            Self::ThreadUpdate => "Updated a thread",
            Self::ThreadDelete => "Deleted a thread",
            Self::ThreadMemberRemove => "Removed someone from a thread",
            Self::ForumUpdate => "Changed forum settings",
            Self::MessagePublish => "Published an announcement",
            Self::ChannelFollow => "Followed an announcement channel",
            Self::ChannelUnfollow => "Removed an announcement following",
            Self::SoundCreate => "Added a sound",
            Self::SoundUpdate => "Changed a sound",
            Self::SoundDelete => "Deleted a sound",
            Self::PollEnd => "Ended a poll",
            Self::EmbedsRemove => "Removed link previews",
            Self::ApplicationApprove => "Approved an application",
            Self::ApplicationReject => "Rejected an application",
            Self::AdmissionUpdate => "Changed rules screening or applications",
            Self::OnboardingUpdate => "Changed onboarding",
            Self::ListingUpdate => "Changed the directory listing",
            Self::TemplateCreate => "Created a server template",
            Self::TemplateUpdate => "Synced a server template",
            Self::TemplateDelete => "Deleted a server template",
            Self::EventCreate => "Created an event",
            Self::EventUpdate => "Updated an event",
            Self::EventCancel => "Cancelled an event",
            Self::EventDelete => "Deleted an event",
            Self::StageStart => "Started a Stage",
            Self::StageUpdate => "Updated a Stage",
            Self::StageEnd => "Ended a Stage",
            Self::AppInstall => "Added an app",
            Self::AppRemove => "Removed an app",
            Self::CommandPermissionsUpdate => "Changed app command permissions",
            Self::WebhookCreate => "Created a webhook",
            Self::WebhookUpdate => "Updated a webhook",
            Self::WebhookDelete => "Deleted a webhook",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTarget {
    Guild,
    User,
    Role,
    Channel,
    Message,
    Invite,
    Emoji,
    AutomodRule,
    Sticker,
    /// A soundboard sound (appended).
    Sound,
    /// A scheduled event (`schedule.rs`).
    Event,
    /// An app (`apps.rs`).
    App,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditChange {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Id,
    pub guild_id: GuildId,
    pub actor_id: UserId,
    pub action: AuditAction,
    pub target_kind: AuditTarget,
    pub target_id: Option<Id>,
    /// What the target was called when the action happened (it may be gone now).
    pub target_name: Option<String>,
    pub changes: Vec<AuditChange>,
    pub reason: Option<String>,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogPage {
    pub items: Vec<AuditEntry>,
    pub users: Vec<UserSummary>,
    pub next_before: Option<Id>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaidAlert {
    pub detected_at: UnixMillis,
    pub joins: u32,
    pub window_seconds: u32,
}

/// Server-wide safety settings. `raid` is only filled in for moderators.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuildSafety {
    pub guild_id: GuildId,
    pub verification_level: VerificationLevel,
    pub invites_paused_until: Option<UnixMillis>,
    pub dms_paused_until: Option<UnixMillis>,
    #[serde(default)]
    pub raid: Option<RaidAlert>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeoutInfo {
    pub until: UnixMillis,
    pub reason: Option<String>,
}

/// One viewer's moderation state in one server (`GET /guilds/{id}/moderation/me`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModerationState {
    pub safety: GuildSafety,
    pub timeout: Option<TimeoutInfo>,
    /// The verification level stops this viewer sending until this time.
    pub send_blocked_until: Option<UnixMillis>,
    /// Age-restricted channels in this server the viewer can see.
    pub age_restricted: Vec<ChannelId>,
    /// Of those, the ones this viewer has confirmed they are an adult for.
    pub age_confirmed: Vec<ChannelId>,
    pub server_time: UnixMillis,
    /// Channels in this server the viewer can see that have slowmode, with its
    /// interval in seconds (appended; kept current by `SlowmodeUpdated`). A
    /// client only asks a channel's slowmode status when it is listed here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slowmode: Vec<ChannelSlowmode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSlowmode {
    pub channel_id: ChannelId,
    pub seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunePreview {
    pub days: u32,
    pub count: u32,
}

/// Who can see a channel, for the permission editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAccess {
    pub channel_id: ChannelId,
    pub members: u32,
    pub viewers: u32,
    /// A member with no roles beyond @everyone can see it.
    pub everyone: bool,
    /// Roles whose members can see it (with only that role).
    pub roles: Vec<RoleId>,
    /// A few of the members who can see it.
    pub sample: Vec<UserSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AutoModTrigger {
    /// Whole words by default; `*` at the start or end of a keyword matches
    /// any word ending or starting with it. Matching ignores letter case.
    Keyword {
        keywords: Vec<String>,
        allow: Vec<String>,
    },
    /// More than `limit` unique user and role mentions in one message.
    MentionSpam { limit: u8 },
    /// The same message `duplicates` times within 30 seconds, or more than
    /// `links` links in one message.
    Spam { duplicates: u8, links: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutoModActions {
    /// Stop the message and tell the author why.
    pub block: bool,
    /// Shown to the author when blocked; a default is used when empty.
    pub message: Option<String>,
    /// Post an alert in this channel.
    pub alert_channel_id: Option<ChannelId>,
    /// Time the member out for this long; zero for no timeout.
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoModRule {
    /// Ignored when creating.
    pub id: Id,
    pub guild_id: GuildId,
    pub name: String,
    pub enabled: bool,
    pub trigger: AutoModTrigger,
    pub actions: AutoModActions,
    pub exempt_roles: Vec<RoleId>,
    pub exempt_channels: Vec<ChannelId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ModerationCommand {
    /// Opt this event stream in to `Event::Moderation`.
    Subscribe,
    /// Kick with an optional reason (Kick Members).
    Kick {
        guild_id: GuildId,
        user_id: UserId,
        reason: Option<String>,
    },
    /// Time a member out for `seconds` (Time Out Members); zero removes it.
    Timeout {
        guild_id: GuildId,
        user_id: UserId,
        seconds: u32,
        reason: Option<String>,
    },
    /// Remove members inactive for `days` (Kick Members). With `role_ids`,
    /// only members who have one of those roles; otherwise only members
    /// without roles.
    Prune {
        guild_id: GuildId,
        days: u32,
        role_ids: Vec<RoleId>,
        reason: Option<String>,
    },
    SetVerificationLevel {
        guild_id: GuildId,
        level: VerificationLevel,
    },
    /// Zero resumes invites.
    PauseInvites {
        guild_id: GuildId,
        seconds: u32,
    },
    /// Zero resumes DMs.
    PauseDms {
        guild_id: GuildId,
        seconds: u32,
    },
    DismissRaidAlert {
        guild_id: GuildId,
    },
    CreateAutoModRule(AutoModRule),
    UpdateAutoModRule(AutoModRule),
    DeleteAutoModRule {
        guild_id: GuildId,
        rule_id: Id,
    },
    SetAgeRestricted {
        channel_id: ChannelId,
        age_restricted: bool,
    },
    /// The viewer confirms they are an adult, for this channel only.
    ConfirmAge {
        channel_id: ChannelId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ModerationResult {
    Pruned { count: u32 },
    AutoModRule(AutoModRule),
    Safety(GuildSafety),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ModerationEvent {
    SafetyUpdated(GuildSafety),
    ChannelAgeRestricted {
        channel_id: ChannelId,
        age_restricted: bool,
    },
    /// AutoMod rules changed; managers refetch them.
    AutoModUpdated {
        guild_id: GuildId,
    },
    /// This member's own timeout changed (with its reason, which only they see).
    TimeoutUpdated {
        guild_id: GuildId,
        timeout: Option<TimeoutInfo>,
    },
    /// A channel's slowmode changed; viewers refetch their own cooldown.
    SlowmodeUpdated {
        channel_id: ChannelId,
        seconds: u32,
    },
}

/// Slowmode intervals offered by the channel editor, in seconds.
pub const SLOWMODE_CHOICES: &[(u32, &str)] = &[
    (0, "Off"),
    (5, "5s"),
    (10, "10s"),
    (15, "15s"),
    (30, "30s"),
    (60, "1m"),
    (120, "2m"),
    (300, "5m"),
    (600, "10m"),
    (900, "15m"),
    (1_800, "30m"),
    (3_600, "1h"),
    (7_200, "2h"),
    (21_600, "6h"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_round_trip_by_name() {
        for a in AuditAction::ALL {
            assert_eq!(AuditAction::parse(a.name()), Some(*a));
            assert!(!a.label().is_empty());
            assert_eq!(serde_json::to_value(a).unwrap(), a.name());
        }
        assert_eq!(AuditAction::parse("nonsense"), None);
    }
    #[test]
    fn slowmode_choices_match_the_server_list() {
        let seconds: Vec<u32> = SLOWMODE_CHOICES.iter().map(|c| c.0).collect();
        assert_eq!(seconds, SLOWMODE_SECONDS);
    }
    #[test]
    fn commands_are_named_maps() {
        let c = LaunchCommand::Moderation(ModerationCommand::Timeout {
            guild_id: Id(1),
            user_id: Id(2),
            seconds: 60,
            reason: None,
        });
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "moderation");
        assert_eq!(v["data"]["type"], "timeout");
        let bytes = rmp_serde::to_vec_named(&c).unwrap();
        assert_eq!(rmp_serde::from_slice::<LaunchCommand>(&bytes).unwrap(), c);
    }
}
