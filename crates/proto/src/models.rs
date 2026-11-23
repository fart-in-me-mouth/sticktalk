use crate::{AttachmentId, ChannelId, GuildId, MessageId, Permissions, UnixMillis, UserId};
use serde::{Deserialize, Serialize};

/// Explicit patch semantics in JSON and MessagePack; `clear` is only valid for nullable fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum Patch<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Idle,
    Dnd,
    Invisible,
    #[default]
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct UserSummary {
    pub id: UserId,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_id: Option<AttachmentId>,
    pub status_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SelfUser {
    pub user: UserSummary,
    pub desired_presence: PresenceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProfilePatch {
    #[serde(default)]
    pub username: Patch<String>,
    #[serde(default)]
    pub display_name: Patch<String>,
    #[serde(default)]
    pub avatar_id: Patch<AttachmentId>,
    #[serde(default)]
    pub status_text: Patch<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Presence {
    pub user_id: UserId,
    pub status: PresenceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Friend,
    IncomingRequest,
    OutgoingRequest,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Relationship {
    pub user: UserSummary,
    pub kind: RelationshipKind,
    pub since: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Guild {
    pub id: GuildId,
    pub owner_id: UserId,
    pub name: String,
    pub icon_id: Option<AttachmentId>,
    pub member_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Owner,
    Admin,
    #[default]
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct Member {
    pub guild_id: GuildId,
    pub user: UserSummary,
    pub role: MemberRole,
    pub permissions: Permissions,
    pub joined_at: UnixMillis,
    #[serde(default)]
    pub role_ids: Vec<crate::Id>,
    #[serde(default)]
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuildSummary {
    pub guild: Guild,
    pub my_role: MemberRole,
    pub my_permissions: Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuildPatch {
    #[serde(default)]
    pub name: Patch<String>,
    #[serde(default)]
    pub icon_id: Patch<AttachmentId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Category,
    Text,
    Direct,
    GroupDm,
    /// Phase 2 guild voice channel; older servers may reject creation.
    Voice,
}

/// Topic is deliberately excluded from the initial sidebar snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChannelSummary {
    pub id: ChannelId,
    pub kind: ChannelKind,
    pub guild_id: Option<GuildId>,
    pub parent_id: Option<ChannelId>,
    pub name: Option<String>,
    pub position: u16,
    pub recipient_ids: Vec<UserId>,
    pub owner_id: Option<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Channel {
    pub summary: ChannelSummary,
    pub topic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChannelPatch {
    #[serde(default)]
    pub name: Patch<String>,
    #[serde(default)]
    pub topic: Patch<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChannelPlacement {
    pub channel_id: ChannelId,
    pub parent_id: Option<ChannelId>,
    pub position: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Invite {
    pub id: crate::Id,
    pub code: String,
    pub guild_id: GuildId,
    pub creator_id: UserId,
    pub expires_at: UnixMillis,
    pub max_uses: u32,
    pub uses: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Ban {
    pub guild_id: GuildId,
    pub user: UserSummary,
    pub actor_id: UserId,
    pub reason: Option<String>,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Thumbnail {
    pub width: u16,
    pub height: u16,
    pub byte_size: u32,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Attachment {
    pub id: AttachmentId,
    pub filename: String,
    pub content_type: String,
    pub byte_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub thumbnail: Option<Thumbnail>,
    /// Description, spoiler flag and voice-message details (`content.rs`).
    #[serde(default, skip_serializing_if = "crate::AttachmentMeta::is_empty")]
    pub meta: crate::AttachmentMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: u32,
    pub me: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: UserId,
    pub content: String,
    pub created_at: UnixMillis,
    pub edited_at: Option<UnixMillis>,
    pub reply_to: Option<MessageId>,
    pub attachments: Vec<Attachment>,
    pub mention_ids: Vec<UserId>,
    pub reactions: Vec<ReactionSummary>,
    /// Present on the system message a finished direct or group call leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<crate::CallSummary>,
    /// Poll, forwarded snapshot and link previews (`content.rs`).
    #[serde(default, skip_serializing_if = "crate::MessageExtra::is_empty")]
    pub extra: crate::MessageExtra,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageDraft {
    pub channel_id: ChannelId,
    pub content: String,
    pub reply_to: Option<MessageId>,
    pub attachment_ids: Vec<AttachmentId>,
    pub mention_ids: Vec<UserId>,
}

/// Content and mentions are replaced together; attachments/reply target are immutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageEdit {
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub content: String,
    pub mention_ids: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReadState {
    pub channel_id: ChannelId,
    pub last_read_id: Option<MessageId>,
    pub latest_message_id: Option<MessageId>,
    pub unread_count: u32,
    pub mention_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    Left,
    Kicked,
    Banned,
    Deleted,
}
