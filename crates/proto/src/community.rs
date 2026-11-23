//! Community tools (task `community`): rules screening, member applications,
//! onboarding, channel opt-in ("Browse Channels"), the server directory of
//! this instance, server templates and server insights.
//! Behavior, routes and limits: `docs/features/community.md`.
//!
//! Everything here is additive: new commands go in `Command::Community`, new
//! results in `CommandResult::Community`, and `Event::Community` is sent only
//! to connections that sent `CommunityCommand::Subscribe`.
use crate::*;
use serde::{Deserialize, Serialize};

pub const MAX_RULES: usize = 16;
pub const MAX_RULE_CHARS: usize = 300;
pub const MAX_SCREENING_DESCRIPTION_CHARS: usize = 300;
pub const MAX_APPLICATION_QUESTIONS: usize = 5;
pub const MAX_QUESTION_CHARS: usize = 150;
pub const MAX_CHOICES: usize = 10;
pub const MAX_CHOICE_CHARS: usize = 80;
pub const MAX_SHORT_ANSWER_CHARS: usize = 150;
pub const MAX_PARAGRAPH_ANSWER_CHARS: usize = 1_000;
/// Waiting applications per server; more are refused until some are decided.
pub const MAX_PENDING_APPLICATIONS: usize = 500;
pub const MAX_PRE_JOIN_QUESTIONS: usize = 5;
pub const MAX_ONBOARDING_QUESTIONS: usize = 15;
pub const MAX_ONBOARDING_ANSWERS: usize = 12;
pub const MAX_ONBOARDING_TITLE_CHARS: usize = 100;
pub const MAX_ANSWER_LABEL_CHARS: usize = 50;
pub const MAX_ANSWER_ROLES: usize = 5;
pub const MAX_ANSWER_CHANNELS: usize = 10;
pub const MAX_WELCOME_DESCRIPTION_CHARS: usize = 300;
pub const MAX_HIGHLIGHTS: usize = 5;
pub const MAX_HIGHLIGHT_CHARS: usize = 60;
pub const MAX_GUIDE_TASKS: usize = 5;
pub const MAX_GUIDE_RESOURCES: usize = 5;
pub const MAX_GUIDE_TITLE_CHARS: usize = 60;
pub const MAX_DIRECTORY_DESCRIPTION_CHARS: usize = 300;
pub const MAX_DIRECTORY_TAGS: usize = 5;
pub const MAX_DIRECTORY_TAG_CHARS: usize = 20;
pub const MAX_FEATURED_EMOJI: usize = 5;
/// Listings one directory page returns per section.
pub const DIRECTORY_PAGE: usize = 24;
pub const MAX_TEMPLATES_PER_GUILD: usize = 5;
pub const MAX_TEMPLATE_DESCRIPTION_CHARS: usize = 120;
/// Days of daily insight rows kept per server.
pub const INSIGHTS_DAYS: u32 = 120;

/// One question of a membership application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    #[default]
    Short,
    Paragraph,
    Choice,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplicationQuestion {
    pub kind: QuestionKind,
    pub prompt: String,
    #[serde(default)]
    pub required: bool,
    /// Only for `Choice`: 2 to 10 options.
    #[serde(default)]
    pub choices: Vec<String>,
}

/// How people get into a server: rules they accept, and whether they apply.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Admission {
    pub guild_id: GuildId,
    /// New members accept the rules before they can talk.
    pub screening: bool,
    pub description: String,
    pub rules: Vec<String>,
    /// Raised by the server when the rules change and re-acceptance is asked.
    pub rules_version: u32,
    /// People apply instead of joining at once.
    pub applications: bool,
    pub questions: Vec<ApplicationQuestion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}
/// Where someone came from when they joined or applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinSource {
    Invite,
    Discovery,
    Template,
    Created,
    Other,
}
impl JoinSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Invite => "Invite",
            Self::Discovery => "Server directory",
            Self::Template => "Template link",
            Self::Created => "Created the server",
            Self::Other => "Other",
        }
    }
}
/// How an applicant reached the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum JoinVia {
    Invite { code: String },
    Discovery { guild_id: GuildId },
}

/// An application, as reviewers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub guild_id: GuildId,
    pub user: UserSummary,
    /// The account's creation time, to help reviewers.
    pub account_created_at: UnixMillis,
    /// One answer per question, in order (empty when skipped).
    pub answers: Vec<String>,
    /// The questions as they were when the person applied.
    pub questions: Vec<ApplicationQuestion>,
    pub status: ApplicationStatus,
    pub reason: Option<String>,
    pub reviewer_id: Option<UserId>,
    pub interview_channel_id: Option<ChannelId>,
    pub source: JoinSource,
    pub created_at: UnixMillis,
    pub decided_at: Option<UnixMillis>,
}
/// An application, as the applicant sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyApplication {
    pub guild_id: GuildId,
    pub guild_name: String,
    pub status: ApplicationStatus,
    pub reason: Option<String>,
    pub interview_channel_id: Option<ChannelId>,
    pub created_at: UnixMillis,
    pub decided_at: Option<UnixMillis>,
}
/// `GET /api/v1/guilds/{g}/applications`: the review queue (Kick Members).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplicationQueue {
    pub pending: Vec<Application>,
    /// The latest decided ones (at most 50).
    pub decided: Vec<Application>,
}

/// One onboarding answer: what choosing it grants.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnboardingAnswer {
    /// Stable within its question; the server assigns new ones.
    pub id: u32,
    pub label: String,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub role_ids: Vec<RoleId>,
    #[serde(default)]
    pub channel_ids: Vec<ChannelId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnboardingQuestion {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub required: bool,
    /// Asked before the member enters the server (at most 5).
    #[serde(default)]
    pub pre_join: bool,
    pub answers: Vec<OnboardingAnswer>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Highlight {
    pub channel_id: ChannelId,
    pub text: String,
    #[serde(default)]
    pub emoji: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WelcomeScreen {
    pub description: String,
    pub highlights: Vec<Highlight>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideTaskKind {
    /// Open a channel and read it.
    #[default]
    Read,
    /// Send a message in a channel (checked by the server).
    Talk,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuideTask {
    pub id: u32,
    pub kind: GuideTaskKind,
    pub channel_id: ChannelId,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuideResource {
    pub channel_id: ChannelId,
    pub title: String,
    #[serde(default)]
    pub description: String,
}
/// The new-member flow a server's managers configure.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Onboarding {
    pub guild_id: GuildId,
    pub enabled: bool,
    pub questions: Vec<OnboardingQuestion>,
    /// Channels shown only to members who opted in. Every other channel is a
    /// default channel that everyone gets.
    pub opt_in_channel_ids: Vec<ChannelId>,
    pub welcome: WelcomeScreen,
    pub tasks: Vec<GuideTask>,
    pub resources: Vec<GuideResource>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnboardingResponse {
    pub question_id: u32,
    pub answer_ids: Vec<u32>,
}
/// One member's own community state in a server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemberCommunity {
    pub guild_id: GuildId,
    /// The member must accept the rules before talking.
    pub rules_pending: bool,
    /// The member must answer required pre-join questions before talking.
    pub questions_pending: bool,
    pub rules_version: u32,
    pub responses: Vec<OnboardingResponse>,
    /// The member finished (or skipped) the onboarding flow.
    pub onboarded: bool,
    /// Opt-in channels this member chose to see.
    pub opted_in: Vec<ChannelId>,
    pub show_all: bool,
    /// Server guide tasks done.
    pub guide_done: Vec<u32>,
}
/// `GET /api/v1/guilds/{g}/community`: everything a member's app needs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommunityState {
    pub admission: Admission,
    pub onboarding: Onboarding,
    pub me: MemberCommunity,
    /// Waiting applications, for reviewers only.
    pub pending_applications: u32,
}
/// What joining a server asks of someone who is not a member yet
/// (`GET /api/v1/invites/{code}/admission` and the directory preview).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinGate {
    pub guild_id: GuildId,
    pub name: String,
    pub description: String,
    pub member_count: u32,
    pub joined: bool,
    pub screening: bool,
    pub screening_description: String,
    pub rules: Vec<String>,
    pub applications: bool,
    pub questions: Vec<ApplicationQuestion>,
    pub application: Option<MyApplication>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryCategory {
    Gaming,
    Music,
    Entertainment,
    Tech,
    Education,
    Art,
    Hobbies,
    Sports,
    #[default]
    Community,
    Other,
}
impl DirectoryCategory {
    pub const ALL: &'static [Self] = &[
        Self::Gaming,
        Self::Music,
        Self::Entertainment,
        Self::Tech,
        Self::Education,
        Self::Art,
        Self::Hobbies,
        Self::Sports,
        Self::Community,
        Self::Other,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Gaming => "gaming",
            Self::Music => "music",
            Self::Entertainment => "entertainment",
            Self::Tech => "tech",
            Self::Education => "education",
            Self::Art => "art",
            Self::Hobbies => "hobbies",
            Self::Sports => "sports",
            Self::Community => "community",
            Self::Other => "other",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Gaming => "Gaming",
            Self::Music => "Music",
            Self::Entertainment => "Entertainment",
            Self::Tech => "Science & Tech",
            Self::Education => "Education",
            Self::Art => "Creative Arts",
            Self::Hobbies => "Hobbies",
            Self::Sports => "Sports",
            Self::Community => "Community",
            Self::Other => "Other",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == s)
    }
}
/// What a server owner writes for the directory.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListingDraft {
    pub category: DirectoryCategory,
    pub description: String,
    pub tags: Vec<String>,
    /// A short language tag such as `en` or `pt-BR`.
    pub language: String,
    pub featured_emoji: Vec<Id>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Listing {
    pub guild_id: GuildId,
    pub draft: ListingDraft,
    /// Set by the instance administrator.
    pub featured: bool,
    pub hidden: bool,
    pub delisted: bool,
    pub listed_at: UnixMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingRequirement {
    pub key: String,
    pub label: String,
    pub met: bool,
}
/// `GET /api/v1/guilds/{g}/listing` (Manage Server).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListingAdmin {
    pub directory_enabled: bool,
    pub listing: Option<Listing>,
    pub requirements: Vec<ListingRequirement>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeaturedEmoji {
    pub id: Id,
    pub name: String,
    pub animated: bool,
}
/// One server's public preview in the directory. Counts are cached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub guild_id: GuildId,
    pub name: String,
    pub icon_id: Option<AttachmentId>,
    pub banner_color: u8,
    pub description: String,
    pub category: DirectoryCategory,
    pub tags: Vec<String>,
    pub language: String,
    pub member_count: u32,
    pub online_count: u32,
    pub featured_emoji: Vec<FeaturedEmoji>,
    pub featured: bool,
    pub joined: bool,
}
/// `GET /api/v1/directory?q=&category=`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DirectoryPage {
    pub enabled: bool,
    pub featured: Vec<DirectoryEntry>,
    pub active: Vec<DirectoryEntry>,
    /// Search or category results (empty without `q` or `category`).
    pub results: Vec<DirectoryEntry>,
    pub categories: Vec<(DirectoryCategory, u32)>,
}
/// `GET /api/v1/directory/{g}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryPreview {
    pub entry: DirectoryEntry,
    pub gate: JoinGate,
    pub welcome: Option<WelcomeScreen>,
    /// Names of the channels highlighted on the welcome screen.
    pub channel_names: Vec<(ChannelId, String)>,
}

/// A role in a template. `id` is a local reference; 0 is @everyone.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TemplateRole {
    pub id: u32,
    pub name: String,
    pub color: u32,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub mentionable: bool,
    pub permissions: Permissions,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateOverwrite {
    /// A `TemplateRole::id`.
    pub role: u32,
    pub allow: Permissions,
    pub deny: Permissions,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateChannel {
    /// A local reference, unique in the template, never 0.
    pub id: u32,
    pub parent: Option<u32>,
    pub kind: ChannelKind,
    pub name: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub slowmode: u32,
    #[serde(default)]
    pub age_restricted: bool,
    #[serde(default)]
    pub overwrites: Vec<TemplateOverwrite>,
}
/// A server's structure. Role and channel IDs inside `onboarding` and
/// `automod` are local references (`Id(local)`), mapped on creation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TemplateSnapshot {
    /// Lowest first; @everyone (id 0) first.
    pub roles: Vec<TemplateRole>,
    /// Sidebar order, parents before children.
    pub channels: Vec<TemplateChannel>,
    #[serde(default)]
    pub verification_level: VerificationLevel,
    #[serde(default)]
    pub admission: Option<Admission>,
    #[serde(default)]
    pub onboarding: Option<Onboarding>,
    #[serde(default)]
    pub automod: Vec<AutoModRule>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildTemplate {
    /// The share code (`?template=<code>`), or `builtin:<name>`.
    pub code: String,
    pub name: String,
    pub description: String,
    pub source_guild_id: Option<GuildId>,
    pub creator_id: Option<UserId>,
    pub created_at: UnixMillis,
    pub updated_at: UnixMillis,
    pub uses: u32,
    /// The source server changed since the template was last synced
    /// (only computed for the source server's managers).
    #[serde(default)]
    pub out_of_sync: bool,
    pub snapshot: TemplateSnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum TemplateRef {
    Code(String),
    Builtin(String),
}

/// One day of a server's insights. `day` counts days since 1970-01-01 (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InsightDay {
    pub day: u32,
    pub joins: u32,
    pub leaves: u32,
    pub messages: u32,
    pub voice_minutes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightSource {
    pub source: JoinSource,
    /// The invite code, for invites.
    pub code: String,
    pub joins: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightChannel {
    pub channel_id: ChannelId,
    pub name: String,
    pub messages: u32,
    pub voice_minutes: u32,
}
/// New members who joined on `day` and, of those old enough, how many were
/// still members and had talked 1 and 7 days later.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RetentionCohort {
    pub day: u32,
    pub joined: u32,
    pub retained_1d: Option<u32>,
    pub retained_7d: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunnelStep {
    pub label: String,
    pub reached: u32,
}
/// `GET /api/v1/guilds/{g}/insights?days=` (owner, View Audit Log or Manage
/// Server). Only aggregate counts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Insights {
    pub guild_id: GuildId,
    pub member_count: u32,
    pub today: u32,
    pub days: Vec<InsightDay>,
    pub sources: Vec<InsightSource>,
    pub channels: Vec<InsightChannel>,
    pub retention: Vec<RetentionCohort>,
    pub funnel: Vec<FunnelStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommunityCommand {
    /// Opt this connection in to `Event::Community`.
    Subscribe,
    /// Manage Server.
    SetAdmission {
        guild_id: GuildId,
        screening: bool,
        description: String,
        rules: Vec<String>,
        /// Ask every member to accept the changed rules again.
        reaccept: bool,
        applications: bool,
        questions: Vec<ApplicationQuestion>,
    },
    AcceptRules {
        guild_id: GuildId,
        rules_version: u32,
    },
    Apply {
        via: JoinVia,
        answers: Vec<String>,
        accept_rules: bool,
    },
    Withdraw {
        guild_id: GuildId,
    },
    /// Kick Members.
    Review {
        guild_id: GuildId,
        user_id: UserId,
        approve: bool,
        reason: Option<String>,
    },
    /// Kick Members: a private group conversation with the applicant (created
    /// once; later reviewers are added to it).
    Interview {
        guild_id: GuildId,
        user_id: UserId,
    },
    /// Manage Server and Manage Roles.
    SetOnboarding(Onboarding),
    /// The member's own answers; changing them adds and removes mapped roles.
    Respond {
        guild_id: GuildId,
        responses: Vec<OnboardingResponse>,
        /// Also mark the onboarding flow finished.
        finish: bool,
    },
    /// Browse Channels: the opt-in channels to show, and Show All Channels.
    SetChannels {
        guild_id: GuildId,
        opted_in: Vec<ChannelId>,
        show_all: bool,
    },
    /// A Read task of the server guide.
    CompleteTask {
        guild_id: GuildId,
        task_id: u32,
    },
    /// Join a listed server, following its admission settings.
    JoinDiscovered {
        guild_id: GuildId,
    },
    /// Owner or Manage Server. `None` removes the listing.
    SetListing {
        guild_id: GuildId,
        listing: Option<ListingDraft>,
    },
    /// Manage Server.
    CreateTemplate {
        guild_id: GuildId,
        name: String,
        description: String,
    },
    SyncTemplate {
        code: String,
    },
    DeleteTemplate {
        code: String,
    },
    CreateFromTemplate {
        template: TemplateRef,
        name: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommunityResult {
    Ack,
    Admission(Box<Admission>),
    Onboarding(Box<Onboarding>),
    Me(Box<MemberCommunity>),
    Application(Box<MyApplication>),
    Listing(Option<Box<Listing>>),
    Template(Box<GuildTemplate>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommunityEvent {
    /// The server's admission or onboarding settings changed: fetch them again.
    Updated { guild_id: GuildId },
    /// Your own state in a server (your devices).
    Me(Box<MemberCommunity>),
    /// Your application changed (approved, rejected, interview opened).
    Application(Box<MyApplication>),
    /// Reviewers: the queue changed.
    Queue { guild_id: GuildId, pending: u32 },
}

/// The rights onboarding may give: what an ordinary member has. Everything
/// else is forbidden, in the role and in any channel overwrite for it,
/// including a bit added later until it is classified here (security review
/// G4; test `every_bit_is_classified_for_onboarding`).
pub const ONBOARDING_ALLOWED: Permissions = Permissions::from_bits_retain(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::READ_HISTORY.bits()
        | Permissions::ATTACH_FILES.bits()
        | Permissions::ADD_REACTIONS.bits()
        | Permissions::CREATE_INVITES.bits()
        | Permissions::CHANGE_NICKNAME.bits()
        | Permissions::EMBED_LINKS.bits()
        | Permissions::CONNECT.bits()
        | Permissions::SPEAK.bits()
        | Permissions::STREAM.bits()
        | Permissions::VIDEO.bits()
        | Permissions::USE_VOICE_ACTIVITY.bits()
        | Permissions::USE_SOUNDBOARD.bits()
        | Permissions::CREATE_PUBLIC_THREADS.bits()
        | Permissions::CREATE_PRIVATE_THREADS.bits()
        | Permissions::SEND_MESSAGES_IN_THREADS.bits()
        | Permissions::USE_EXTERNAL_EMOJI.bits()
        | Permissions::USE_EXTERNAL_STICKERS.bits()
        | Permissions::REQUEST_TO_SPEAK.bits()
        | Permissions::SEND_TTS_MESSAGES.bits(),
);
/// The rights onboarding may never grant: every other one.
pub const ONBOARDING_FORBIDDEN: Permissions = Permissions::all().difference(ONBOARDING_ALLOWED);
/// Whether a role with these rights (its own and its channel overwrites'
/// allows) may be given by onboarding. Unknown bits are refused too.
pub fn onboarding_grantable(rights: Permissions) -> bool {
    rights.difference(ONBOARDING_ALLOWED).is_empty()
}

/// UTC day number for a Unix millisecond time.
pub fn day_of(ms: UnixMillis) -> u32 {
    (ms / 86_400_000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_round_trip_through_messagepack() {
        let cmd = Command::Community(CommunityCommand::Apply {
            via: JoinVia::Invite {
                code: "x".repeat(22),
            },
            answers: vec!["hi".into()],
            accept_rules: true,
        });
        let bytes = rmp_serde::to_vec_named(&cmd).unwrap();
        assert_eq!(rmp_serde::from_slice::<Command>(&bytes).unwrap(), cmd);
        let old: Onboarding = serde_json::from_str(r#"{"guild_id":1,"enabled":true,"questions":[{"id":1,"title":"t","answers":[{"id":1,"label":"a"}]}],"opt_in_channel_ids":[],"welcome":{"description":"","highlights":[]},"tasks":[],"resources":[]}"#).unwrap();
        assert!(!old.questions[0].multi && old.questions[0].answers[0].role_ids.is_empty());
    }
    #[test]
    fn categories_parse_by_name() {
        for c in DirectoryCategory::ALL {
            assert_eq!(DirectoryCategory::parse(c.name()), Some(*c));
        }
        assert!(ONBOARDING_FORBIDDEN.contains(Permissions::ADMINISTRATOR));
        assert!(!ONBOARDING_FORBIDDEN.contains(Permissions::SEND_MESSAGES));
        assert_eq!(day_of(86_400_000 * 3 + 5), 3);
    }
    /// A new permission bit fails this until it is put on one side: an
    /// ordinary member right onboarding may give, or one it may not.
    #[test]
    fn every_bit_is_classified_for_onboarding() {
        let not_for_onboarding = Permissions::MANAGE_MESSAGES
            | Permissions::MANAGE_CHANNELS
            | Permissions::MANAGE_GUILD
            | Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MANAGE_MEMBERS
            | Permissions::ADMINISTRATOR
            | Permissions::OWNER
            | Permissions::MANAGE_ROLES
            | Permissions::MANAGE_NICKNAMES
            | Permissions::MENTION_EVERYONE
            | Permissions::MUTE_MEMBERS
            | Permissions::DEAFEN_MEMBERS
            | Permissions::MOVE_MEMBERS
            | Permissions::PRIORITY_SPEAKER
            | Permissions::MANAGE_EMOJI
            | Permissions::VIEW_AUDIT_LOG
            | Permissions::MODERATE_MEMBERS
            | Permissions::MANAGE_SOUNDBOARD
            | Permissions::MANAGE_THREADS
            | Permissions::PUBLISH_MESSAGES
            | Permissions::MANAGE_EVENTS
            | Permissions::CREATE_EVENTS;
        assert!((not_for_onboarding & ONBOARDING_ALLOWED).is_empty());
        let unclassified = Permissions::all() - not_for_onboarding - ONBOARDING_ALLOWED;
        assert!(
            unclassified.is_empty(),
            "classify for onboarding: {unclassified:?}"
        );
        assert_eq!(ONBOARDING_FORBIDDEN, not_for_onboarding);
        assert!(!onboarding_grantable(Permissions::from_bits_retain(
            1 << 63
        )));
        assert!(onboarding_grantable(
            Permissions::SEND_MESSAGES | Permissions::CONNECT
        ));
    }
}
