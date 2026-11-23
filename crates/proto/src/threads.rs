//! Threads, forum channels and announcement channels (with following).
//!
//! Model: a thread is a channel of kind `Thread` whose parent is the text,
//! announcement or forum channel it belongs to. A forum post is a thread in a
//! forum channel. Threads are never listed in `Ready.channels`,
//! `GuildDetail.channels` or `ChannelUpsert`; they reach clients only through
//! `Event::Threads`, which the server sends only to connections that sent
//! `ThreadCommand::Subscribe`, and through the REST reads below. Messages in a
//! thread are ordinary messages whose `channel_id` is the thread.
//! See `docs/features/threads.md`.
use crate::*;
use serde::{Deserialize, Serialize};

/// Auto-archive choices, in minutes: 1 hour, 24 hours, 3 days, 1 week.
pub const THREAD_ARCHIVE_MINUTES: &[u32] = &[60, 1440, 4320, 10080];
pub const MAX_THREAD_NAME_CHARS: usize = 100;
/// Active (not archived) threads and forum posts in one server.
pub const MAX_ACTIVE_THREADS_PER_GUILD: usize = 1000;
/// Thread members returned in one page.
pub const MAX_THREAD_MEMBERS_PAGE: usize = 100;
/// Threads returned in one page of the Threads browser or a forum.
pub const MAX_THREADS_PAGE: u16 = 50;
/// Joined active threads sent in the subscription snapshot.
pub const MAX_JOINED_THREADS_SNAPSHOT: usize = 500;
pub const MAX_FORUM_TAGS: usize = 20;
pub const MAX_TAGS_PER_POST: usize = 5;
pub const MAX_FORUM_TAG_CHARS: usize = 20;
pub const MAX_FORUM_GUIDELINES_CHARS: usize = 4096;
/// Slowmode for forum posts and for replies in them, at most 6 hours.
pub const MAX_FORUM_SLOWMODE_SECONDS: u32 = 21_600;
/// Servers that may follow one announcement channel.
pub const MAX_FOLLOWERS_PER_CHANNEL: usize = 100;
/// Announcement channels one text channel may follow.
pub const MAX_FOLLOWS_PER_TARGET: usize = 10;
/// Messages one announcement channel may publish per hour.
pub const MAX_PUBLISHES_PER_HOUR: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadNotify {
    /// Follow the parent channel's notification setting.
    #[default]
    Default,
    All,
    Mentions,
    Nothing,
}

/// One thread or forum post, as anyone who can see it sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ChannelId,
    pub guild_id: GuildId,
    pub parent_id: ChannelId,
    /// The thread's name; a forum post's title.
    pub name: String,
    pub owner_id: UserId,
    /// The message it was started from (in the parent). A forum post's first
    /// message is in the post itself and is `first_message_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_message_id: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_message_id: Option<MessageId>,
    /// A post in a forum channel.
    #[serde(default)]
    pub forum_post: bool,
    #[serde(default)]
    pub private: bool,
    /// Private threads: members other than moderators may add people.
    #[serde(default)]
    pub invitable: bool,
    #[serde(default)]
    pub archived: bool,
    /// Only people with Manage Threads may post, unarchive or change it.
    #[serde(default)]
    pub locked: bool,
    /// Pinned to the top of its forum.
    #[serde(default)]
    pub pinned: bool,
    pub auto_archive_minutes: u32,
    pub created_at: UnixMillis,
    pub last_activity_at: UnixMillis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<UnixMillis>,
    /// Messages in the thread (a forum post's first message included).
    pub message_count: u32,
    pub member_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_id: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_ids: Vec<u32>,
}

impl ThreadInfo {
    /// The channel row clients keep so ordinary message code works in it.
    pub fn summary(&self) -> ChannelSummary {
        ChannelSummary {
            id: self.id,
            kind: ChannelKind::Thread,
            guild_id: Some(self.guild_id),
            parent_id: Some(self.parent_id),
            name: Some(self.name.clone()),
            position: 0,
            recipient_ids: vec![],
            owner_id: None,
        }
    }
}

/// Your membership in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMembership {
    pub thread_id: ChannelId,
    pub joined_at: UnixMillis,
    pub notify: ThreadNotify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMember {
    pub user: UserSummary,
    pub joined_at: UnixMillis,
}

/// `MessageExtra::threads`: present only when one of these applies.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageThreads {
    /// A thread was started from this message; shown only to people who can
    /// see the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadPreview>,
    /// Delivered from an announcement channel this channel follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosspost: Option<Crosspost>,
    /// Published from this announcement channel to its followers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub published: bool,
}

impl MessageExtra {
    pub fn thread(&self) -> Option<&ThreadPreview> {
        self.threads.as_ref().and_then(|t| t.thread.as_ref())
    }
    pub fn crosspost(&self) -> Option<&Crosspost> {
        self.threads.as_ref().and_then(|t| t.crosspost.as_ref())
    }
    pub fn published(&self) -> bool {
        self.threads.as_ref().is_some_and(|t| t.published)
    }
    /// Changes the thread details, dropping them when nothing is left.
    pub fn set_threads(&mut self, f: impl FnOnce(&mut MessageThreads)) {
        let mut t = self.threads.take().map(|b| *b).unwrap_or_default();
        f(&mut t);
        self.threads = (t != MessageThreads::default()).then(|| Box::new(t));
    }
}

/// Shown under a message a thread was started from (`MessageThreads::thread`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadPreview {
    pub thread_id: ChannelId,
    pub name: String,
    pub message_count: u32,
    pub last_activity_at: UnixMillis,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub private: bool,
}

/// A copy delivered into a following channel (`MessageExtra::crosspost`). The
/// text and files are the `forwarded` snapshot; the source link there is only
/// present for people who can read the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crosspost {
    pub source_guild_name: String,
    pub source_channel_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForumSort {
    #[default]
    Activity,
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForumLayout {
    #[default]
    List,
    Gallery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumTag {
    /// Unique within the forum; kept when other tags change.
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Only people with Manage Threads may put it on a post.
    #[serde(default)]
    pub moderated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForumSettings {
    pub channel_id: ChannelId,
    #[serde(default)]
    pub guidelines: String,
    #[serde(default)]
    pub tags: Vec<ForumTag>,
    #[serde(default)]
    pub require_tags: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reaction: Option<String>,
    #[serde(default)]
    pub sort: ForumSort,
    #[serde(default)]
    pub layout: ForumLayout,
    /// Seconds between posts by one person (moderators are exempt).
    #[serde(default)]
    pub post_slowmode: u32,
    /// Seconds between messages by one person inside a post.
    #[serde(default)]
    pub reply_slowmode: u32,
    pub default_auto_archive_minutes: u32,
}

/// One row of the forum view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumPost {
    pub thread: ThreadInfo,
    /// The first message, or None when it was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<Message>,
    /// Reactions on the first message, summed.
    pub reaction_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForumPage {
    pub posts: Vec<ForumPost>,
    pub users: Vec<UserSummary>,
    pub has_more: bool,
    /// Pass as `before` for the next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThreadPage {
    pub threads: Vec<ThreadInfo>,
    /// Which of these you have joined.
    #[serde(default)]
    pub joined: Vec<ChannelId>,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThreadMembersPage {
    pub members: Vec<ThreadMember>,
    pub has_more: bool,
}

/// `GET /api/v1/threads/{thread}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDetail {
    pub thread: ThreadInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<ThreadMembership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_state: Option<ReadState>,
}

/// Answer to `ThreadCommand::Subscribe`: your joined active threads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub threads: Vec<ThreadInfo>,
    pub memberships: Vec<ThreadMembership>,
    pub read_states: Vec<ReadState>,
}

/// A text channel following an announcement channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelFollow {
    pub id: Id,
    pub source_channel_id: ChannelId,
    pub source_guild_id: GuildId,
    pub source_guild_name: String,
    pub source_channel_name: String,
    pub target_channel_id: ChannelId,
    pub target_guild_id: GuildId,
    pub target_guild_name: String,
    pub target_channel_name: String,
    pub created_by: UserId,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelFollows {
    pub follows: Vec<ChannelFollow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThreadPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_archive_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invitable: Option<bool>,
    /// Forum posts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    /// Forum posts only: the complete tag list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThread {
    pub parent_id: ChannelId,
    pub name: String,
    /// Start it from this message in the parent (public threads only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    #[serde(default)]
    pub private: bool,
    #[serde(default = "yes")]
    pub invitable: bool,
    pub auto_archive_minutes: u32,
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPost {
    pub forum_id: ChannelId,
    pub title: String,
    /// The first message. Attachments are uploaded to the forum channel and
    /// move into the post when it is created.
    pub content: String,
    #[serde(default)]
    pub attachment_ids: Vec<AttachmentId>,
    #[serde(default)]
    pub attachments: Vec<AttachmentMetaPatch>,
    #[serde(default)]
    pub mention_ids: Vec<UserId>,
    #[serde(default)]
    pub tag_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_archive_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForumSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidelines: Option<String>,
    /// The complete list. A tag without an ID (0) is new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<ForumTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_tags: Option<bool>,
    /// Some("") clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reaction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<ForumSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<ForumLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_slowmode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_slowmode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_auto_archive_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ThreadCommand {
    /// Opt this connection in to `Event::Threads`; answers `Snapshot`.
    Subscribe,
    Create(NewThread),
    CreatePost(NewPost),
    Update {
        thread_id: ChannelId,
        patch: ThreadPatch,
    },
    Delete {
        thread_id: ChannelId,
    },
    Join {
        thread_id: ChannelId,
    },
    Leave {
        thread_id: ChannelId,
    },
    AddMember {
        thread_id: ChannelId,
        user_id: UserId,
    },
    RemoveMember {
        thread_id: ChannelId,
        user_id: UserId,
    },
    SetNotify {
        thread_id: ChannelId,
        notify: ThreadNotify,
    },
    UpdateForum {
        forum_id: ChannelId,
        patch: ForumSettingsPatch,
    },
    /// Deliver an announcement to every following channel.
    Publish {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    Follow {
        source_channel_id: ChannelId,
        target_channel_id: ChannelId,
    },
    /// Remove a following, from either side.
    Unfollow {
        follow_id: Id,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ThreadResult {
    Ack,
    Snapshot(ThreadSnapshot),
    Thread(Box<ThreadInfo>),
    /// A new forum post and its first message.
    Post {
        thread: Box<ThreadInfo>,
        message: Box<Message>,
    },
    Forum(ForumSettings),
    Follow(ChannelFollow),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ThreadEvent {
    /// Created or changed; sent to everyone who can see it.
    ThreadUpsert(ThreadInfo),
    /// Deleted, or no longer visible to you.
    ThreadRemoved {
        thread_id: ChannelId,
        parent_id: ChannelId,
    },
    /// You joined (Some) or left (None), on any device.
    Membership {
        thread_id: ChannelId,
        membership: Option<ThreadMembership>,
    },
    ForumUpdated(ForumSettings),
    /// A message in an announcement channel was published.
    Published {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    /// Followings of this channel (either side) changed; refetch them.
    FollowsChanged {
        channel_id: ChannelId,
    },
}
