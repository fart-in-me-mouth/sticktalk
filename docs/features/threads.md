# Threads, forum channels and announcement channels (task `threads`)

Public and private threads, forum channels, announcement channels with
publishing and cross-server following, and search over all of them. Everything
is free for every server.

Code: `crates/proto/src/threads.rs` (wire), `crates/server/src/threads.rs` and
`threads/{forum,announce,routes,tests}.rs` (server),
`crates/web/src/live/threads.rs` and `live/ui/threads.rs` (web),
`crates/web/tests/threads.mjs` (browser test). Migration
`202611230701_threads.sql`.

## The model: one channel tree

StickTalk already lets any channel nest inside any other. Threads fit into that
same tree instead of beside it:

- **A thread is a channel** of the new kind `thread`, nested in the channel it
  belongs to (`parent_id`). Its details (owner, starter message, private,
  archived, locked, auto-archive, counts) are in `threads`; its members in
  `thread_members`. Because it is a channel, every message feature works in it
  unchanged: history, replies, reactions, attachments, polls, voice messages,
  forwarding, pins, typing, read states, mentions, search, the inbox.
- **A forum post is a thread** whose parent is a forum channel (new kind
  `forum`). A forum has no messages of its own; its children are its posts.
- **An announcement channel** (new kind `announcement`) is a text channel that
  can publish to followers. It can have threads like a text channel.
- **Rules of the tree.** Threads go in text and announcement channels (and
  posts in forums). Nothing nests inside a thread or a forum
  (`CreateChannel`/`ReorderChannels` refuse them as parents). Threads have no
  position, are never moved, and are never listed as channels: `db::channels`
  (Ready, `GuildDetail`, `ChannelUpsert`, reorders) leaves them out, and the
  32-channel cap does not count them. Deleting a channel deletes its threads
  (nested channels still move up a level, as before).
- **Permissions come from the parent.** A thread has no overwrites of its own
  (`SetOverwrites` refuses one), so `permissions::evaluate` computes its rights
  from the channels above it, exactly like a nested channel. `threads::scope`
  then applies the thread rules at the end of `evaluate`, so every check in the
  server (access, audience, history, fan-out, push, mentions, search) follows
  them without further changes:
  - a private thread is visible only to its members and to people with Manage
    Threads on it (everyone else loses View Channel and every message right);
  - Send Messages in a thread means **Send Messages in Threads** (and a forum
    post's first message means Send Messages in the forum, "create posts");
  - a locked thread: only Manage Threads may post, react, reopen or change it;
  - no threads inside threads.

## Permissions (new bits)

| Bit | Name | Default for @everyone |
| --- | --- | --- |
| 1 << 32 | Create Public Threads | yes |
| 1 << 33 | Create Private Threads | yes |
| 1 << 34 | Send Messages in Threads | yes |
| 1 << 35 | Manage Threads | no |
| 1 << 36 | Publish Messages (your own; Manage Messages publishes anyone's) | no |

The migration adds the three defaults to every existing @everyone role, and a
trigger adds them to new servers' @everyone (the existing `launch_guild`
trigger is left alone). A channel overwrite that **denied Send Messages** also
gets the three thread bits denied, so read-only channels stay read-only.
Server Settings › Roles lists the new bits (a "Threads" group and Publish
Messages under Text).

## Wire protocol (additive)

- `ChannelKind`: `Announcement`, `Forum`, `Thread` appended. Threads never
  appear in channel lists; announcement and forum channels do.
- `MessageExtra.threads: Option<Box<MessageThreads>>` (omitted when empty):
  `thread` (preview of the thread started from this message, only for people
  who can see it), `crosspost` (source server and channel names of a delivered
  announcement) and `published`. Accessors: `extra.thread()`,
  `extra.crosspost()`, `extra.published()`.
- `Command::Threads(ThreadCommand)`, `CommandResult::Threads(ThreadResult)`,
  `Event::Threads(ThreadEvent)`. Events go **only** to connections that sent a
  `ThreadCommand` (the web app sends `Subscribe` after every Ready), so older
  clients never see a variant they cannot decode.

| Command | Who | Notes |
| --- | --- | --- |
| `Subscribe` | anyone | opts the connection in; answers `Snapshot { threads, memberships, read_states }`: your joined active threads (at most 500, newest activity first) |
| `Create(NewThread)` | Create Public (or Private) Threads + View + Read History on a text or announcement channel | name 1–100 chars; auto-archive 60/1440/4320/10080 minutes; from a message (public only, one thread per message, not a call record) or from the channel |
| `CreatePost(NewPost)` | Send Messages in the forum | title, first message (text and/or files), up to 5 tags; files are uploaded **to the forum channel** and move into the post; the forum's tag rules and post slowmode apply |
| `Update { thread_id, patch }` | name, auto-archive, archive/reopen, tags: owner or Manage Threads; lock and pin: Manage Threads; invitable: private threads, owner or Manage Threads | a locked thread is changed only with Manage Threads; an archived one must be reopened first (except by Manage Threads); one pinned post per forum |
| `Delete { thread_id }` | Manage Threads | deletes it and its messages |
| `Join`, `Leave` | anyone who can see it | leaving a private thread hides it again |
| `AddMember` | private: owner, Manage Threads, or a member when invitable; public: anyone who can post | the person must see the parent channel |
| `RemoveMember` | Manage Threads, or the owner of a private thread | |
| `SetNotify { notify }` | members | `Default` (the parent channel's setting), `All`, `Mentions`, `Nothing` |
| `UpdateForum { forum_id, patch }` | Manage Channels on the forum | guidelines ≤ 4096 chars, ≤ 20 tags (name ≤ 20 chars, unique, optional emoji, moderator-only flag), require tags, default reaction, default sort (activity/created) and layout (list/gallery), post and reply slowmode ≤ 6 h, default auto-archive. Removing a tag removes it from posts |
| `Publish { channel_id, message_id }` | announcement channel; your own message with Publish Messages, anyone's with Manage Messages | 10 per channel per hour; not polls, call records or copies; once per message |
| `Follow { source_channel_id, target_channel_id }` | Read History on the announcement channel; **Manage Server** in the target's server (StickTalk has no webhooks, so this is the closest right) | target must be a text channel; ≤ 100 followers per announcement channel, ≤ 10 follows per text channel; 10/min per person |
| `Unfollow { follow_id }` | either side: Manage Server in the target's server, or Manage Channels on the announcement channel | anyone else gets `not_found` |

Results: `Ack`, `Snapshot`, `Thread(Box<ThreadInfo>)`, `Post { thread, message }`,
`Forum(ForumSettings)`, `Follow(ChannelFollow)`.

Events (`ThreadEvent`): `ThreadUpsert(ThreadInfo)` to everyone who can see the
thread (created, changed, new message, archived), `ThreadRemoved { thread_id,
parent_id }` (deleted, or no longer visible to you), `Membership { thread_id,
membership }` (your devices), `ForumUpdated`, `Published`, `FollowsChanged`.
Messages, reactions, typing and read states in threads use the ordinary events.

### REST (cookie or bearer; reads only)

| Route | Answer |
| --- | --- |
| `GET /api/v1/threads/{thread}` | `ThreadDetail { thread, me, read_state }`; 404 when you cannot see it |
| `GET /api/v1/threads/{thread}/members?after=<user>` | `ThreadMembersPage`, 100 a page |
| `GET /api/v1/channels/{channel}/threads?archived=&before=&limit=` | `ThreadPage` (≤ 50): active by latest activity, or archived by archive time; private ones only for members and Manage Threads |
| `GET /api/v1/channels/{forum}/forum` | `ForumSettings` |
| `GET /api/v1/channels/{forum}/posts?sort=&tags=1,2&q=&before=&limit=` | `ForumPage` (≤ 50): pinned post first, then by the order; tags match any; `q` words each match the title or the first message |
| `GET /api/v1/channels/{channel}/follows` | `ChannelFollows`: an announcement channel's followers (for its managers) or what a text channel follows (for Manage Server there) |

## Behavior

- **Membership and unreads.** You join a thread by creating it, by posting in
  it, by being mentioned in it (explicit mentions; in a private thread only when
  the sender may add people), by being the author of its starter message, by
  Join, or by being added. Read states exist only for members (created when
  you join, deleted when you leave), so unread counts, the sidebar badge, the
  inbox and mentions cover joined threads only. `MarkRead` in a thread you have
  not joined answers a zero read state.
- **Notifications.** In a thread, members follow their thread setting
  (`Default` uses the parent channel's level and mute); everyone else is
  notified only of mentions.
- **Archiving.** Each active thread has `archive_at = last activity + its
  auto-archive time`. One task (`threads::spawn`) sleeps until the earliest
  `archive_at`, archives everything due in batches of 100, and re-arms. It is
  woken only when a thread's new deadline is earlier than the one it sleeps
  for, or when a crosspost is queued; with no active thread it waits with no
  deadline. Sending a message in an archived (not locked) thread reopens it.
- **Limits.** 1000 active threads per server (creating, reopening or posting
  into an archived thread at the limit is `limit_exceeded`); 10 thread
  creations a minute per person; member pages of 100; browser and forum pages
  of 50; the subscription snapshot holds 500 joined threads.
- **Starter preview.** The message a thread started from carries
  `extra.threads.thread` (name, message count, last activity, archived, locked,
  private) for people who can see the thread; clients update it from
  `ThreadUpsert`.
- **Crossposting.** Publishing queues one delivery per following
  (`crosspost_queue`, one row per message and following; later edits fold into
  a waiting row, and the queue holds at most 10 000). After the command
  commits, the worker writes each copy in its own short transaction, 16 at a
  time, releasing the command gate between batches. A copy is a message in the
  target channel whose text is the published text with mentions written out as
  plain `@name` (no one in the target server is notified as mentioned), plus a
  forwarded-style snapshot of its files (the same stored files, kept by the
  existing forwarded-file rules) and the source server and channel names. The
  link back to the original is present only for people who can read the
  source. Nothing else from the source server is exposed. Editing the original
  updates every copy; deleting it deletes them (through the ordinary delete
  path, acting as the target server's owner). A copy cannot be edited by hand.
  A delivery that fails is dropped, not retried forever.
- **Forum slowmode.** Post slowmode counts from your last post in the forum;
  reply slowmode from your last message in that post. People with Manage
  Threads, Manage Channels or Manage Messages are exempt from post slowmode;
  Manage Threads from reply slowmode.

## Search

Server search (`GET /guilds/{g}/messages/search`) now covers text channels,
**voice-channel chat**, **announcement channels**, and **threads and forum
posts** under any channel the searcher can read. Private threads are included
only for their members and for people with Manage Threads there. A forum post
also matches by its **title** (the match is its first message). `in:` accepts
a thread or post name, quoted when it has spaces (`in:"Launch party"`), as
well as channel names and IDs.

## Web app

- **Sidebar:** the threads you joined (and the one open) are listed under
  their channel, with unread and mention badges. Announcement channels have a
  megaphone, forums a speech-bubbles icon.
- **Header:** in a text or announcement channel, Threads (browser) and Create
  Thread; Follow Channel in an announcement channel; Followed Channels in a
  text channel for Manage Server. In a thread: #parent › name with private,
  archived and locked marks, Back, notification level, Members, and a menu for
  settings, archive/close, lock, pin (posts), leave and delete. Phones get one
  menu instead of the buttons.
- **Messages:** Create Thread and Publish in the message actions (and the
  touch menu); the thread preview card under a starter message; "Published"
  under published announcements; crossposts shown as from their source server
  with a SERVER badge.
- **Forum view:** search box, Latest Activity / Newest, list or gallery (first
  image as cover), tag filter chips, collapsible guidelines, New Post (title,
  message, tags, files), each post with tags, author, excerpt, reply and
  reaction counts, last activity, closed/locked marks and a default-reaction
  button. Forum Settings for channel managers.
- **Dialogs:** Create Thread (name, archive time, private, invitable), Thread
  Settings, delete confirmation, Threads browser (Active/Archived, paging),
  Members (add, remove), New Post, Forum Settings, Follow, Followers/Followed
  Channels. Create Channel offers Announcement and Forum.
- A search result or message link into a thread the client has not loaded
  fetches the thread first. Forwarding can now target threads and
  announcement channels.

## Paths that create messages outside `SendMessage`

Thread messages and forum posts' first messages go through the ordinary send
path (`commands::mutate(SendMessage)` / `content::send`), so the moderation
check on that path covers them. **Crosspost copies** (`threads/announce.rs`,
`deliver`) are written directly: they are server deliveries of a message that
already passed the source server's send path, and the target server's rules
for its own members (slowmode, timeouts) do not fit them. If the target
server's AutoMod should screen crossposts, call it from `announce::deliver`
before the copy is inserted.

## Decisions to revisit

- Threads in **voice-channel chat** are not offered (Discord does not either).
- Starting a thread **without a starter message** does not post a "started a
  thread" system message in the parent; the thread appears in the sidebar of
  its members, in the Threads browser and in search.
- A crosspost's author ID is the person who created the following (it is shown
  as the source server); the copy is not attributed to the original author, so
  no source-server user ID reaches the target server.
- The source side's managers can see which servers and channels follow their
  announcement channel (names only).
- Older clients that predate this change cannot decode a Ready that contains
  an announcement or forum channel (new `ChannelKind` variants). The web app is
  served by the same server, so only a tab left open across the upgrade is
  affected until it reloads.
