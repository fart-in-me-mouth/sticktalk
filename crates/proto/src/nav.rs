//! Navigation DTOs: per-account pins, favorites, server rail order, saved
//! messages and reminders, the inbox, and direct-message search. See
//! `docs/features/nav.md`. Every list here is bounded by the constants below.
use crate::*;
use serde::{Deserialize, Serialize};

/// DMs, group DMs and channels one account can pin to the top of their list.
pub const MAX_PINNED_CONVERSATIONS: usize = 50;
/// Channels, DMs and group DMs in one account's Favorites.
pub const MAX_FAVORITES: usize = 200;
/// User-defined categories inside Favorites.
pub const MAX_FAVORITE_CATEGORIES: usize = 20;
/// Saved messages (bookmarks and reminders together) per account.
pub const MAX_SAVED_MESSAGES: usize = 200;
/// The furthest ahead a reminder may be set.
pub const MAX_REMINDER_AHEAD_MS: u64 = 366 * 86_400_000;
/// Channels listed by the Unreads inbox, and unread messages shown for each.
pub const MAX_INBOX_CHANNELS: usize = 50;
pub const MAX_INBOX_MESSAGES_PER_CHANNEL: usize = 3;
/// Channels one "mark as read" request may acknowledge.
pub const MAX_ACK_CHANNELS: usize = 100;

pub fn valid_category_name(name: &str) -> bool {
    valid_folder_name(name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteCategory {
    /// Account-local, positive key chosen by the client.
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteEntry {
    pub channel_id: ChannelId,
    /// None: listed above every category.
    #[serde(default)]
    pub category_id: Option<u32>,
}
/// One place on the server rail: a server on its own, or a folder of servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum RailItem {
    Guild(GuildId),
    Folder(u32),
}
/// Everything about how one account arranges its lists. Private to that account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavSettings {
    /// Pinned conversations, in order. Only ones the account can still see.
    #[serde(default)]
    pub pinned: Vec<ChannelId>,
    #[serde(default)]
    pub favorite_categories: Vec<FavoriteCategory>,
    /// In display order; entries of one category need not be adjacent.
    #[serde(default)]
    pub favorites: Vec<FavoriteEntry>,
    #[serde(default)]
    pub folders: Vec<ServerFolder>,
    /// Rail order. Servers not listed (and not in a folder) follow, oldest first.
    #[serde(default)]
    pub rail: Vec<RailItem>,
    #[serde(default)]
    pub saved: SavedSummary,
}
/// PUT /api/v1/users/@me/nav/pins
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPinned {
    pub channel_ids: Vec<ChannelId>,
}
/// PUT /api/v1/users/@me/nav/favorites (complete replacement)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFavorites {
    pub categories: Vec<FavoriteCategory>,
    pub favorites: Vec<FavoriteEntry>,
}
/// PUT /api/v1/users/@me/nav/rail (complete replacement of folders and order)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRail {
    pub folders: Vec<ServerFolder>,
    pub order: Vec<RailItem>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSummary {
    pub count: u32,
    /// Reminders still waiting for their time.
    pub pending: u32,
    /// Reminders whose time has come and that are still set.
    pub due: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMessage {
    /// Paging key: larger is saved more recently.
    pub id: u64,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub saved_at: UnixMillis,
    pub remind_at: Option<UnixMillis>,
    /// The reminder's time has come.
    pub due: bool,
    /// None when the message was deleted or its conversation can no longer be
    /// read: then nothing of it is sent, only that it is unavailable.
    pub message: Option<Message>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPage {
    pub items: Vec<SavedMessage>,
    pub users: Vec<UserSummary>,
    pub next_before: Option<u64>,
    pub has_more: bool,
    pub summary: SavedSummary,
}
/// PUT /api/v1/users/@me/saved/{message_id}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveMessage {
    pub channel_id: ChannelId,
    /// Some: remind then. None keeps (or saves without) a reminder, unless
    /// `clear_reminder` is set.
    #[serde(default)]
    pub remind_at: Option<UnixMillis>,
    #[serde(default)]
    pub clear_reminder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadChannel {
    pub read: ReadState,
    /// Newest first, at most MAX_INBOX_MESSAGES_PER_CHANNEL. Empty when the
    /// channel's history may not be read.
    pub messages: Vec<Message>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxUnreads {
    /// Most recently active first.
    pub channels: Vec<UnreadChannel>,
    pub users: Vec<UserSummary>,
    pub has_more: bool,
}
/// POST /api/v1/users/@me/unreads/ack
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckChannels {
    pub channel_ids: Vec<ChannelId>,
}
/// Response of the ack route: the read states that changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckResult {
    pub read_states: Vec<ReadState>,
}
