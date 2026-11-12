use serde::{Deserialize, Serialize};

/// Positive Snowflake integer; aliases share a representation, not a namespace.
/// Valid generated IDs fit SQLite's signed INTEGER (top bit is always zero).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Id(pub u64);

pub type UserId = Id;
pub type GuildId = Id;
pub type ChannelId = Id;
pub type MessageId = Id;
pub type AttachmentId = Id;
pub type UnixMillis = u64;
pub type EventSeq = u64;
pub type Nonce = u64;

/// Resume cursors are scoped to an account and the server's current stream epoch.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResumeCursor {
    pub stream_id: String,
    pub seq: EventSeq,
}
