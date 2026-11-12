use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HealthResponse {
    pub version: String,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    InvalidCredentials,
    Unauthenticated,
    SessionExpired,
    Forbidden,
    NotFound,
    Conflict,
    UsernameTaken,
    AlreadyExists,
    Blocked,
    InvalidInvite,
    InviteExpired,
    InviteExhausted,
    Banned,
    OwnerRequired,
    HierarchyViolation,
    RateLimited,
    LimitExceeded,
    PayloadTooLarge,
    UnsupportedMediaType,
    InvalidAttachment,
    VersionMismatch,
    InvalidSequence,
    ResyncRequired,
    UnsupportedFeature,
    Internal,
    Unavailable,
    FeatureDisabled,
    InvalidSignupTicket,
    IdentityAlreadyLinked,
    LastLoginMethod,
}

/// Public server capabilities; never contains an OAuth client secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaResponse {
    pub version: String,
    pub protocol_version: u16,
    pub google_client_id: Option<String>,
    #[serde(default)]
    pub gifs: GifCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleAuthRequest {
    pub id_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum GoogleAuthResponse {
    Session(AuthResponse),
    SignupRequired(GoogleSignupRequired),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleSignupRequired {
    pub ticket: String,
    pub expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleCompleteRequest {
    pub ticket: String,
    pub username: String,
}

/// Linking proves control of Google as well as the bearer-authenticated account.
pub type GoogleLinkRequest = GoogleAuthRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleUnlinkRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodsResponse {
    pub has_password: bool,
    pub google_linked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPasswordRequest {
    /// Required when a password already exists; null for Google-only accounts.
    pub current_password: Option<String>,
    pub new_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub field: Option<String>,
    pub retry_after_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthResponse {
    pub token: String,
    pub expires_at: UnixMillis,
    pub user: SelfUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PageQuery {
    /// Message history only: an inclusive ID pivot, mutually exclusive with other cursors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around: Option<Id>,
    /// Message history only: Unix milliseconds, resolved through the Snowflake index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around_timestamp: Option<UnixMillis>,
    pub before: Option<Id>,
    pub after: Option<Id>,
    pub limit: Option<u16>,
}

/// All collection endpoints use keyset pagination, never OFFSET.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub users: Vec<UserSummary>,
    pub next_before: Option<Id>,
    pub next_after: Option<Id>,
    pub has_more: bool,
    pub cursor: ResumeCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Snapshot<T> {
    pub data: T,
    pub cursor: ResumeCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuildDetail {
    pub summary: GuildSummary,
    pub channels: Vec<ChannelSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPurpose {
    Message,
    Avatar,
    GuildIcon,
}

/// `purpose` and optional `channel_id` are multipart text fields; `file` is binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadMetadata {
    pub purpose: UploadPurpose,
    pub channel_id: Option<ChannelId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadResponse {
    pub attachment: Attachment,
    pub expires_at: UnixMillis,
}

/// SHA-256 is lowercase hexadecimal of the entire file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUploadRequest {
    pub purpose: UploadPurpose,
    pub channel_id: Option<ChannelId>,
    pub filename: String,
    pub content_type: String,
    pub byte_size: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumableUpload {
    pub upload_id: String,
    pub offset: u64,
    pub byte_size: u64,
    pub chunk_size: u32,
    pub expires_at: UnixMillis,
    pub completed: Option<UploadResponse>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub committed_bytes: u64,
    pub partial_bytes: u64,
    pub reserved_bytes: u64,
    pub min_free_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetPasswordRequest {
    pub username: String,
    pub code: String,
    pub new_password: String,
}
/// Signature covers the UTF-8 manifest string exactly, not a reserialized object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpdateManifest {
    pub manifest: String,
    /// Standard base64 Ed25519 signature.
    pub signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub target: String,
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
}
