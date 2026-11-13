use crate::*;
use serde::{Deserialize, Serialize};

pub type RoleId = Id;
pub const MAX_ROLES_PER_GUILD: usize = 64;
pub const MAX_CHANNEL_OVERWRITES: usize = 128;
pub const MAX_PINS_PER_CHANNEL: usize = 50;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: RoleId,
    pub guild_id: GuildId,
    pub name: String,
    /// RGB 0xRRGGBB; zero means no color.
    pub color: u32,
    /// Greater positions are higher; @everyone has ID=guild_id and position zero.
    pub position: u16,
    pub hoist: bool,
    pub mentionable: bool,
    pub permissions: Permissions,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwriteKind {
    Role,
    Member,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub kind: OverwriteKind,
    pub target_id: Id,
    pub allow: Permissions,
    pub deny: Permissions,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildProfile {
    pub guild_id: GuildId,
    pub description: String,
    /// One of the ten launch banner presets (0..9).
    pub banner_color: u8,
    pub created_at: UnixMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildPreview {
    pub guild: Guild,
    pub profile: GuildProfile,
    pub online_count: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user: UserSummary,
    pub banner_color: u32,
    pub pronouns: String,
    pub about_me: String,
    pub created_at: UnixMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutualServer {
    pub guild: Guild,
    pub member_since: UnixMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDetail {
    pub profile: UserProfile,
    pub mutual_servers: Vec<MutualServer>,
    pub mutual_friends: Vec<UserSummary>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    All,
    Mentions,
    Nothing,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub level: NotificationLevel,
    pub muted: bool,
    pub mute_until: Option<UnixMillis>,
    pub suppress_everyone: bool,
    pub hide_muted_channels: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSession {
    /// Opaque management ID, never a bearer token or token hash.
    pub id: String,
    pub device_name: String,
    pub platform: String,
    pub last_active: UnixMillis,
    pub place: String,
    pub current: bool,
}
/// Additive launch commands. Existing v1 command names and numeric bits are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum LaunchCommand {
    SetSlowmode {
        channel_id: ChannelId,
        seconds: u32,
    },
    RenameEmoji {
        guild_id: GuildId,
        emoji_id: Id,
        name: String,
    },
    DeleteEmoji {
        guild_id: GuildId,
        emoji_id: Id,
    },
    CreateRole {
        guild_id: GuildId,
        name: String,
    },
    UpdateRole {
        role: Role,
    },
    DeleteRole {
        guild_id: GuildId,
        role_id: RoleId,
    },
    /// Complete list ordered lowest to highest, including @everyone first.
    ReorderRoles {
        guild_id: GuildId,
        role_ids: Vec<RoleId>,
    },
    SetMemberRoles {
        guild_id: GuildId,
        user_id: UserId,
        role_ids: Vec<RoleId>,
    },
    /// Complete replacement; empty removes all overwrites.
    SetOverwrites {
        channel_id: ChannelId,
        overwrites: Vec<PermissionOverwrite>,
    },
    UpdateGuildProfile {
        guild_id: GuildId,
        description: String,
        banner_color: u8,
    },
    UpdateUserProfile {
        banner_color: u32,
        pronouns: String,
        about_me: String,
    },
    SetNickname {
        guild_id: GuildId,
        user_id: UserId,
        nickname: Option<String>,
    },
    PinMessage {
        channel_id: ChannelId,
        message_id: MessageId,
        pinned: bool,
    },
    SetNotifications(NotificationSettings),
    UpdateDevice {
        device_name: String,
        platform: String,
    },
    LogoutSession {
        session_id: String,
    },
    LogoutAllOthers,
}
