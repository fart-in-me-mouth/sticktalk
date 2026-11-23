//! Apps, bot users, application commands, interactions, message components and
//! incoming webhooks. See `docs/features/apps.md` and `docs/BOTS.md`.
//!
//! Wire rules (all additive):
//! - `Command::Apps(AppCommand)`, `CommandResult::Apps(AppResult)` and
//!   `Event::Apps(AppEvent)`. People's connections receive `Event::Apps` only
//!   after sending `AppCommand::Subscribe`; a bot's connection always does.
//! - `MessageExtra::app` carries rich embeds, components, the webhook's name
//!   override and the command a message answers.
//! - `UserStyle::bot` marks bot and webhook accounts ("APP" tag).
use crate::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bot tokens start with this; the rest is 43 URL-safe base64 characters.
pub const BOT_TOKEN_PREFIX: &str = "stkb_";
pub const MAX_APPS_PER_USER: usize = 25;
pub const MAX_APP_NAME_CHARS: usize = 32;
pub const MAX_APP_DESCRIPTION_CHARS: usize = 400;
/// Commands one app may register globally, and again per server.
pub const MAX_COMMANDS_PER_SCOPE: usize = 100;
pub const MAX_COMMAND_NAME_CHARS: usize = 32;
pub const MAX_COMMAND_DESCRIPTION_CHARS: usize = 100;
pub const MAX_COMMAND_OPTIONS: usize = 25;
pub const MAX_OPTION_CHOICES: usize = 25;
pub const MAX_COMMAND_CHOICE_CHARS: usize = 100;
/// Encoded size of all of one scope's command definitions.
pub const MAX_COMMANDS_BYTES: usize = 64 * 1024;
pub const MAX_WEBHOOKS_PER_CHANNEL: usize = 15;
pub const MAX_WEBHOOKS_PER_GUILD: usize = 100;
pub const MAX_WEBHOOK_NAME_CHARS: usize = 80;
pub const MAX_EMBEDS: usize = 10;
/// Characters in all of one message's embeds together.
pub const MAX_EMBED_TOTAL_CHARS: usize = 6000;
pub const MAX_EMBED_TITLE_CHARS: usize = 256;
pub const MAX_EMBED_DESCRIPTION_CHARS: usize = 4096;
pub const MAX_EMBED_FIELDS: usize = 25;
pub const MAX_EMBED_FIELD_NAME_CHARS: usize = 256;
pub const MAX_EMBED_FIELD_VALUE_CHARS: usize = 1024;
pub const MAX_EMBED_FOOTER_CHARS: usize = 2048;
pub const MAX_EMBED_AUTHOR_CHARS: usize = 256;
/// Images one message may ask the server to fetch (avatar, images, thumbnails).
pub const MAX_FETCHED_IMAGES: usize = 6;
pub const MAX_ACTION_ROWS: usize = 5;
pub const MAX_ROW_BUTTONS: usize = 5;
pub const MAX_SELECT_OPTIONS: usize = 25;
pub const MAX_MODAL_INPUTS: usize = 5;
pub const MAX_CUSTOM_ID_CHARS: usize = 100;
pub const MAX_BUTTON_LABEL_CHARS: usize = 80;
pub const MAX_TEXT_INPUT_CHARS: u16 = 4000;
pub const MAX_AUTOCOMPLETE_CHOICES: usize = 25;
/// A bot answers an interaction within this time, or it fails.
pub const INTERACTION_RESPONSE_MS: u64 = 3_000;
/// Edits and follow-ups are accepted this long after the interaction.
pub const INTERACTION_TOKEN_MS: u64 = 15 * 60_000;
/// Autocomplete answers later than this are dropped.
pub const AUTOCOMPLETE_TIMEOUT_MS: u64 = 2_000;

bitflags::bitflags! {
    /// Event classes a bot receives. Guild, channel and role structure, its
    /// own member record and interactions are always delivered.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Intents: u32 {
        /// Member joins, leaves and changes in every server it is in.
        const GUILD_MEMBERS = 1 << 1;
        /// Bans and unbans.
        const GUILD_MODERATION = 1 << 2;
        /// Message created, edited and deleted events.
        const GUILD_MESSAGES = 1 << 3;
        const GUILD_MESSAGE_REACTIONS = 1 << 4;
        const GUILD_MESSAGE_TYPING = 1 << 5;
        /// The text, files and embeds of messages that neither mention the bot
        /// nor were sent by it. Each server grants it on the consent screen.
        const MESSAGE_CONTENT = 1 << 7;
    }
}
impl Serialize for Intents {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.bits())
    }
}
impl<'de> Deserialize<'de> for Intents {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        u32::deserialize(d).map(Self::from_bits_truncate)
    }
}

/// A finite `f64` that compares by bits, so protocol types stay `Eq`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Number(pub f64);
impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for Number {}
impl Serialize for Number {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}
impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        f64::deserialize(d).map(Number)
    }
}

/// An application (app) and its bot, as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<AttachmentId>,
    pub owner_id: UserId,
    pub bot: UserSummary,
    /// Anyone with Manage Server may install it; otherwise only its owner.
    pub public: bool,
    /// Event classes the bot asks for.
    pub intents: Intents,
    /// Permissions the install link asks for.
    pub permissions: Permissions,
    /// Turned off everywhere by the instance administrator.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(default)]
    pub install_count: u32,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// An uploaded avatar (`UploadPurpose::Avatar`) for the app and its bot.
    #[serde(default)]
    pub icon: Patch<AttachmentId>,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub intents: Intents,
    #[serde(default)]
    pub permissions: Permissions,
}

/// Who may create applications on this instance (set by its administrator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCreation {
    #[default]
    Everyone,
    /// Only accounts the administrator allowed.
    Allowlist,
    /// The feature is off: no new apps, and no bot may connect.
    Disabled,
}

/// `GET /api/v1/applications/@me`: the Developer section.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeveloperState {
    pub apps: Vec<AppInfo>,
    pub creation: AppCreation,
    /// This account may create apps.
    pub can_create: bool,
}

/// One server the person could add an app to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallGuild {
    pub id: GuildId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<AttachmentId>,
    /// What the person may grant: their own permissions there.
    pub my_permissions: Permissions,
    pub owner: bool,
    pub installed: bool,
}

/// `GET /api/v1/applications/{id}/install`: the consent screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPreview {
    pub app: AppInfo,
    pub owner: UserSummary,
    /// Servers where the person has Manage Server.
    pub guilds: Vec<InstallGuild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRequest {
    pub app_id: Id,
    pub guild_id: GuildId,
    /// Exactly what the managed role gets; never more than the installer has.
    pub permissions: Permissions,
    /// Grant the Message Content intent in this server.
    #[serde(default)]
    pub message_content: bool,
    /// Required (and only allowed for the owner) when `permissions` includes
    /// Administrator.
    #[serde(default)]
    pub confirm_administrator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Role,
    Channel,
}

/// One allow or deny in a command's (or a whole app's) permission overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandTarget {
    pub kind: TargetKind,
    pub id: Id,
    pub allow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOverrides {
    /// A command's ID, or the app's ID for rules on all its commands.
    pub id: Id,
    pub targets: Vec<CommandTarget>,
}

/// An app added to a server, for Server Settings › Integrations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    pub app_id: Id,
    pub guild_id: GuildId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<AttachmentId>,
    pub bot: UserSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<RoleId>,
    pub permissions: Permissions,
    pub message_content: bool,
    pub installed_by: UserId,
    pub installed_at: UnixMillis,
    pub commands: Vec<ApplicationCommand>,
    pub overrides: Vec<CommandOverrides>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

/// `GET /api/v1/guilds/{id}/integrations` (Manage Server or Manage Webhooks).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuildIntegrations {
    pub apps: Vec<Installation>,
    pub webhooks: Vec<Webhook>,
}

// ---------------------------------------------------------------- commands

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// A slash command typed in the composer.
    #[default]
    ChatInput,
    /// Apps › on a member's menu.
    User,
    /// Apps › on a message's menu.
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    SubCommand,
    SubCommandGroup,
    String,
    Integer,
    Boolean,
    User,
    Channel,
    Role,
    Mentionable,
    Number,
    Attachment,
}

/// A typed option value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OptionValue {
    String(String),
    Integer(i64),
    Number(Number),
    Boolean(bool),
    User(UserId),
    Channel(ChannelId),
    Role(RoleId),
    /// A user or a role.
    Mentionable(Id),
    /// An upload (`UploadPurpose::Message`) to the channel.
    Attachment(AttachmentId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub name: String,
    /// A string, integer or number, matching the option's kind.
    pub value: OptionValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOption {
    pub kind: OptionKind,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// Sub-options of a subcommand or group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CommandOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_value: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_value: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    /// Suggestions come from the bot as the person types.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autocomplete: bool,
}

/// A command as the app registers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandDraft {
    #[serde(default)]
    pub kind: CommandKind,
    pub name: String,
    /// Required for slash commands; empty for context-menu commands.
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CommandOption>,
    /// Members need these permissions unless the server's overrides say otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_member_permissions: Option<Permissions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationCommand {
    pub id: Id,
    pub app_id: Id,
    /// None for a global command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
    pub kind: CommandKind,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CommandOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_member_permissions: Option<Permissions>,
}

/// One app's commands the viewer may use in a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogApp {
    pub app_id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<AttachmentId>,
    pub bot_id: UserId,
    pub commands: Vec<ApplicationCommand>,
}

/// `GET /api/v1/channels/{id}/commands`: what the `/` picker offers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandCatalog {
    pub channel_id: ChannelId,
    pub apps: Vec<CatalogApp>,
}

/// An option as the person filled it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokedOption {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<OptionValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<InvokedOption>,
    /// The option being typed (autocomplete only); its value may be partial.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    pub channel_id: ChannelId,
    pub command_id: Id,
    #[serde(default)]
    pub options: Vec<InvokedOption>,
    /// The member (user commands) or message (message commands) it was used on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Id>,
}

// -------------------------------------------------------------- components

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Success,
    Danger,
    /// Opens `url`; sends no interaction.
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    pub style: ButtonStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Required, except for link buttons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
    /// Link buttons only: an `https://` or `http://` URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectKind {
    String,
    User,
    Role,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

fn one() -> u8 {
    1
}
fn is_one(v: &u8) -> bool {
    *v == 1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectMenu {
    pub kind: SelectKind,
    pub custom_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub min_values: u8,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max_values: u8,
    /// String selects only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Component {
    Button(Button),
    Select(SelectMenu),
}

/// One row: up to five buttons, or one select menu.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionRow {
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextInputStyle {
    #[default]
    Short,
    Paragraph,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextInput {
    pub custom_id: String,
    pub label: String,
    #[serde(default)]
    pub style: TextInputStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Pre-filled text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default)]
    pub min_length: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
}

/// A form a bot opens in answer to an interaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modal {
    pub custom_id: String,
    pub title: String,
    pub inputs: Vec<TextInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalValue {
    pub custom_id: String,
    pub value: String,
}

// ------------------------------------------------------------------ embeds

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedFooter {
    pub text: String,
}

/// An image asked for by URL. The server fetches it once through the
/// hardened preview fetcher; viewers load the stored copy, never the URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedUrl {
    pub url: String,
}

/// An embed as a bot or webhook sends it (Discord's shape).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EmbedDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 0xRRGGBB accent bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<EmbedField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<EmbedUrl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<EmbedUrl>,
    /// RFC 3339, e.g. `2026-11-23T17:00:00Z`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// An embed as viewers receive it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RichEmbed {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<EmbedField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    /// The stored copy (same origin); absent until fetched, or if it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<EmbedImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<EmbedImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<UnixMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MentionParse {
    Users,
    Roles,
    Everyone,
}

/// Which mentions in the text notify anyone. Webhooks default to users only.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AllowedMentions {
    #[serde(default)]
    pub parse: Vec<MentionParse>,
    #[serde(default)]
    pub users: Vec<UserId>,
    #[serde(default)]
    pub roles: Vec<RoleId>,
}

/// A message as a bot sends it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppMessageDraft {
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<EmbedDraft>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ActionRow>,
    /// Interaction answers only: shown to the person who used it, not stored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_mentions: Option<AllowedMentions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionMeta {
    /// The command as used, e.g. `weather forecast`.
    pub name: String,
    pub user_id: UserId,
    pub kind: CommandKind,
}

/// `MessageExtra::app`: what an app or webhook added to a message.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_id: Option<Id>,
    /// A webhook's per-message name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// A webhook's per-message avatar (stored copy, same origin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<RichEmbed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ActionRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<InteractionMeta>,
}

/// An interaction answer only the person who used the command sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralMessage {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub interaction_id: Id,
    pub author: UserSummary,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub app: AppMessage,
    /// "<app> is thinking…": a deferred answer still to come.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thinking: bool,
    pub created_at: UnixMillis,
}

// ------------------------------------------------------------ interactions

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Resolved {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandData {
    pub command_id: Id,
    pub name: String,
    pub kind: CommandKind,
    #[serde(default)]
    pub options: Vec<InvokedOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Id>,
    #[serde(default)]
    pub resolved: Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Button,
    StringSelect,
    UserSelect,
    RoleSelect,
    ChannelSelect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentData {
    pub custom_id: String,
    pub kind: ComponentKind,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalData {
    pub custom_id: String,
    pub values: Vec<ModalValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum InteractionData {
    Command(CommandData),
    Autocomplete(CommandData),
    Component(ComponentData),
    ModalSubmit(ModalData),
}

/// Delivered to the app's bot over its WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    pub id: Id,
    /// Proves the answer comes from the bot that received this; valid for
    /// `INTERACTION_TOKEN_MS`.
    pub token: String,
    pub app_id: Id,
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub user: UserSummary,
    /// The person's permissions in the channel.
    pub member_permissions: Permissions,
    /// The bot's permissions in the channel.
    pub app_permissions: Permissions,
    pub data: InteractionData,
    /// Components: the message the component is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Box<Message>>,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum InteractionResponse {
    /// Post a message (or, with `ephemeral`, show it only to the person).
    Message(AppMessageDraft),
    /// "<app> is thinking…"; finish with `EditOriginal` within 15 minutes.
    Defer {
        #[serde(default)]
        ephemeral: bool,
    },
    /// Components: acknowledge now, edit the message later.
    DeferUpdate,
    /// Components: replace the message the component is on.
    UpdateMessage(AppMessageDraft),
    /// Open a form; its submission arrives as a `ModalSubmit` interaction.
    Modal(Modal),
    /// Autocomplete suggestions.
    Autocomplete { choices: Vec<Choice> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionAnswer {
    pub interaction_id: Id,
    pub token: String,
}

// ---------------------------------------------------------------- webhooks

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Webhook {
    pub id: Id,
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_id: Option<AttachmentId>,
    /// The account its messages are posted as.
    pub user_id: UserId,
    pub creator_id: UserId,
    pub created_at: UnixMillis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<UnixMillis>,
}

/// A webhook with its secret URL: returned only when created or rotated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookSecret {
    pub webhook: Webhook,
    pub url: String,
}

/// Internal: a webhook post, already authenticated by its secret URL.
/// Refused from any credential other than that webhook's own.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WebhookPost {
    /// The webhook's channel, or a thread in it.
    pub channel_id: ChannelId,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<EmbedDraft>,
    #[serde(default)]
    pub allowed_mentions: AllowedMentions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_ids: Vec<AttachmentId>,
    /// Forum channels: start a new post with this title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_name: Option<String>,
}

// ---------------------------------------------------------------- the wire

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppCommand {
    /// Opt this connection in to `Event::Apps`.
    Subscribe,
    // Developer section (an app's owner).
    CreateApp(AppDraft),
    UpdateApp {
        app_id: Id,
        draft: AppDraft,
    },
    DeleteApp {
        app_id: Id,
    },
    /// Replaces the bot token; the old one stops working at once.
    ResetToken {
        app_id: Id,
    },
    // Servers (Manage Server).
    Install(InstallRequest),
    Uninstall {
        guild_id: GuildId,
        app_id: Id,
    },
    /// Replace the overrides of one command, or (with the app's ID) of all of an app's.
    SetCommandPermissions {
        guild_id: GuildId,
        id: Id,
        targets: Vec<CommandTarget>,
    },
    // Webhooks (Manage Webhooks).
    CreateWebhook {
        channel_id: ChannelId,
        name: String,
        #[serde(default)]
        avatar_id: Option<AttachmentId>,
    },
    UpdateWebhook {
        webhook_id: Id,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        avatar: Patch<AttachmentId>,
        #[serde(default)]
        channel_id: Option<ChannelId>,
    },
    /// New secret URL; the old one stops working at once.
    RotateWebhook {
        webhook_id: Id,
    },
    DeleteWebhook {
        webhook_id: Id,
    },
    // People using apps (Use AppInfo Commands).
    Invoke(Invocation),
    /// Ask the bot for suggestions; they arrive as `AppEvent::Autocomplete`.
    Autocomplete {
        invocation: Invocation,
        request: u32,
    },
    /// A button press or select on a message (or an ephemeral message).
    Component {
        channel_id: ChannelId,
        message_id: MessageId,
        custom_id: String,
        #[serde(default)]
        values: Vec<String>,
    },
    SubmitModal {
        interaction_id: Id,
        values: Vec<ModalValue>,
    },
    // Bots.
    /// Replace the bot's global commands (`guild_id: None`) or one server's.
    SetCommands {
        #[serde(default)]
        guild_id: Option<GuildId>,
        commands: Vec<CommandDraft>,
    },
    Respond {
        answer: InteractionAnswer,
        response: InteractionResponse,
    },
    /// Replace the first answer (after `Defer`, or any message answer).
    EditOriginal {
        answer: InteractionAnswer,
        draft: AppMessageDraft,
    },
    DeleteOriginal {
        answer: InteractionAnswer,
    },
    FollowUp {
        answer: InteractionAnswer,
        draft: AppMessageDraft,
    },
    /// Post with embeds and components.
    Send {
        channel_id: ChannelId,
        draft: AppMessageDraft,
    },
    /// Edit one of the bot's own messages.
    Edit {
        channel_id: ChannelId,
        message_id: MessageId,
        draft: AppMessageDraft,
    },
    // Internal (a webhook's secret URL).
    ExecuteWebhook(Box<WebhookPost>),
    EditWebhookMessage {
        channel_id: ChannelId,
        message_id: MessageId,
        post: Box<WebhookPost>,
    },
    DeleteWebhookMessage {
        channel_id: ChannelId,
        message_id: MessageId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppResult {
    Ack,
    App(Box<AppInfo>),
    /// The new bot token, shown once.
    Token {
        app: Box<AppInfo>,
        token: String,
    },
    Installation(Box<Installation>),
    Webhook(Box<Webhook>),
    WebhookSecret(Box<WebhookSecret>),
    Commands(Vec<ApplicationCommand>),
    /// Waiting for the bot's answer.
    Interaction {
        interaction_id: Id,
    },
    Message(Box<Message>),
    Ephemeral(Box<EphemeralMessage>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppEvent {
    /// Bots: someone used a command, component or form.
    Interaction(Box<Interaction>),
    /// The person who used a command: an answer only they see (replaces one
    /// with the same ID).
    Ephemeral(Box<EphemeralMessage>),
    EphemeralRemoved {
        channel_id: ChannelId,
        id: MessageId,
    },
    Modal {
        interaction_id: Id,
        app_id: Id,
        channel_id: ChannelId,
        modal: Modal,
    },
    Autocomplete {
        request: u32,
        choices: Vec<Choice>,
    },
    /// The app did not answer in time, or could not be reached.
    Failed {
        interaction_id: Id,
        channel_id: ChannelId,
        message: String,
    },
    /// Commands in this server changed: refetch the picker's catalog.
    CommandsUpdated {
        guild_id: GuildId,
    },
    /// Apps or webhooks in this server changed (its managers).
    IntegrationsUpdated {
        guild_id: GuildId,
    },
    /// The owner's apps changed (the Developer section).
    AppsUpdated,
}

impl AppCommand {
    /// The bot-only commands.
    pub fn bot_only(&self) -> bool {
        matches!(
            self,
            Self::SetCommands { .. }
                | Self::Respond { .. }
                | Self::EditOriginal { .. }
                | Self::DeleteOriginal { .. }
                | Self::FollowUp { .. }
                | Self::Send { .. }
                | Self::Edit { .. }
        )
    }
    /// The commands only a webhook's own credential may send.
    pub fn webhook_only(&self) -> bool {
        matches!(
            self,
            Self::ExecuteWebhook(_)
                | Self::EditWebhookMessage { .. }
                | Self::DeleteWebhookMessage { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_and_responses_round_trip_as_named_maps() {
        let r = InteractionResponse::Message(AppMessageDraft {
            content: "hi".into(),
            components: vec![ActionRow {
                components: vec![Component::Button(Button {
                    style: ButtonStyle::Primary,
                    label: Some("Go".into()),
                    emoji: None,
                    custom_id: Some("go".into()),
                    url: None,
                    disabled: false,
                })],
            }],
            ..Default::default()
        });
        let b = rmp_serde::to_vec_named(&r).unwrap();
        assert_eq!(rmp_serde::from_slice::<InteractionResponse>(&b).unwrap(), r);
        for v in [
            OptionValue::Integer(-3),
            OptionValue::Number(Number(1.5)),
            OptionValue::Boolean(true),
            OptionValue::User(Id(9)),
            OptionValue::String("x".into()),
        ] {
            let b = rmp_serde::to_vec_named(&v).unwrap();
            assert_eq!(rmp_serde::from_slice::<OptionValue>(&b).unwrap(), v);
            let j = serde_json::to_string(&v).unwrap();
            assert_eq!(serde_json::from_str::<OptionValue>(&j).unwrap(), v);
        }
        let s: SelectMenu =
            serde_json::from_str(r#"{"kind":"string","custom_id":"a","options":[]}"#).unwrap();
        assert_eq!((s.min_values, s.max_values), (1, 1));
        let t: TextInput = serde_json::from_str(r#"{"custom_id":"a","label":"L"}"#).unwrap();
        assert!(t.required);
    }
    #[test]
    fn unknown_intent_bits_are_dropped() {
        let i: Intents = serde_json::from_str("4294967295").unwrap();
        assert_eq!(i, Intents::all());
    }
}
