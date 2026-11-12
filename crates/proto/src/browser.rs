use crate::*;
use serde::{Deserialize, Serialize};

/// Browser auth never exposes the bearer credential to script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSession {
    pub user: SelfUser,
    pub expires_at: UnixMillis,
    pub csrf_token: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushKey {
    pub public_key: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscribeRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscription {
    pub id: String,
    pub endpoint: String,
    pub created_at: UnixMillis,
}
/// Generic notification: no message body or author profile leaks to a lock screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPayload {
    /// Same-origin navigation with decimal IDs preserved for JavaScript service workers.
    pub url: String,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub count: u32,
    pub title: String,
    pub body: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryAnchor {
    /// Approximate zero-based rank, oldest first; gaps from deletion are interpolated.
    pub rank: u64,
    pub message_id: MessageId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryExtent {
    pub channel_id: ChannelId,
    pub message_count: u64,
    pub anchors: Vec<HistoryAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCapabilities {
    pub cookie_sessions: bool,
    pub google_redirect: bool,
    pub web_push: bool,
}
