# Integration rules across the Discord-parity features (task `qa`)

The seven feature branches (calls, nav, content, moderation, callextras,
authsec, threads) were built in parallel. This file records the rules that
make them work together. Each rule has a server test (named below) or a
browser check (`crates/web/tests/integration.mjs`).

## Permissions

- **Channel permission editor** (`live/moderation.rs` `CHANNEL_PERMISSIONS`,
  `applies`): offers every channel-scoped bit, grouped General, Text, Threads,
  Voice. A bit is shown only where it means something, for the channel itself
  or anything nested inside it (overwrites flow down the tree):
  - Threads group (Create Public/Private Threads, Send Messages in Threads,
    Manage Threads): text, announcement and forum channels.
  - Publish Messages: announcement channels.
  - Voice group, including Use Soundboard: voice channels.
  - Categories show everything.
  - Server-wide rights (Manage Soundboard, View Audit Log, Time Out Members,
    Manage Server, Kick, Ban) are only in the role editor, as in Discord.
- **Role editor** lists every bit (General, Text incl. Publish Messages,
  Threads, Voice & Video incl. Use/Manage Soundboard, Moderation incl. Time Out
  Members and View Audit Log). Audit entries name the new bits too.
- **Batched evaluation applies the thread rules.** `capacity::members_with`
  (every channel audience and event fan-out) now runs the same
  `threads::ThreadScope::apply` as `permissions::evaluate`: private thread
  membership, locks, Send Messages in Threads. `threads::private_audience` is
  gone. Test: `capacity::tests::batched_visibility_matches_evaluate` (private
  and locked threads added).

## Moderation on every way text is created

`moderation::precheck` runs before every command; `moderation/coverage.rs`
adds the paths other features created.

| Path | Timeout | Verification level | AutoMod |
| --- | --- | --- | --- |
| Messages in threads, forum post replies | yes (send path) | yes | yes |
| Thread name (create, rename) | yes | create | yes |
| Forum post title + first message | yes | yes | yes (the forum code sends the first message itself, so it is screened here) |
| Forum guidelines and tag names | yes | no | yes (exempt: Manage Server) |
| Poll question and answers | yes | yes | yes |
| Poll vote | **yes** (voting only needs Read History, which a timeout keeps) | no | no |
| Voice message, GIF, forward, scheduled message | yes | yes | yes |
| Soundboard sound name (upload, rename) | yes | no | keyword rules that block (exempt: Manage Server); logged as an AutoMod block |
| Announcement crosspost into another server | n/a | n/a | the **receiving** server's keyword rules that block; its channel exemptions apply, no member is exempt. A blocked copy is not delivered; an edit that becomes blocked removes the copy. Logged in the receiving server's audit log (AutoMod, target channel, the followed channel). |

Tests: `threads::tests::integration::thread_names_forum_posts_and_votes_are_moderated_like_messages`,
`threads::tests::integration::crossposts_are_screened_by_the_receiving_servers_automod`,
`callextras::tests::integration::sound_changes_and_bitrate_are_audited_and_names_are_screened`.

## Timeouts everywhere

A timed-out member keeps View Channel and Read History only
(`permissions::evaluate`), which already covers sending, reacting, creating
threads and posts, publishing, joining voice and the soundboard (Use
Soundboard is removed with everything else). Added on top:

- Owner-only thread actions (rename, archive, tags, invite, remove from a
  private thread, delete) are refused while timed out.
- Poll votes are refused.
- **1:1 calls with a non-friend** who shares a server (`calls::admit` ->
  `coverage::call_check`, on every join): that server's DM pause and
  verification level apply, as for a DM, and a timeout there blocks starting
  or answering the call. Friends are never affected; group calls are not
  gated. DM *messages* keep Discord's behavior (a timeout does not block them).
  Test: `moderation::tests::integration::direct_calls_follow_the_shared_servers_dm_rules_and_timeouts`.

## Audit log coverage

New `AuditAction` variants (appended; old names unchanged): `thread_update`,
`thread_delete`, `thread_member_remove`, `forum_update`, `message_publish`,
`channel_follow`, `channel_unfollow`, `sound_create`, `sound_update`,
`sound_delete`, `poll_end`, `embeds_remove`. New `AuditTarget::Sound`.

- Threads and posts: changes by someone other than the thread's owner, and
  every lock/unlock and post pin/unpin (Manage Threads only), with before/after
  for name, archived, locked, pinned, invitable, auto-archive and tags.
  Deleting someone else's thread, and removing someone from a thread, are
  logged. A member managing their own thread is not a moderation action and is
  not logged. Creating a thread is ordinary activity and is not logged.
- Forum settings and tags (`forum_update`), with before/after per field.
- Publishing an announcement (`message_publish`, with the author and the
  number of followers). Following (`channel_follow`) and unfollowing
  (`channel_unfollow`) are logged in the following server; an unfollow from
  the announcement side is also logged in the announcing server.
- Soundboard sound add, change and delete; a voice channel's bitrate change
  (`channel_update`, field `bitrate`). Written in the change's own transaction.
- Ending someone else's poll (`poll_end`) and removing someone else's link
  previews (`embeds_remove`).
- Scheduled messages can only be changed by their author, so there is no
  moderator action on them to log.

Implementation: `moderation/feature_audit.rs`, hooked into `audit::prepare` /
`audit::record` like the existing commands. Test:
`threads::tests::integration::moderator_actions_on_threads_forums_announcements_and_polls_are_audited`.

## Who can see what (private threads)

Every per-user check (push, mentions and the inbox, search, link preview
images, pins, saved messages and reminders, forwarding's destinations and
source links, typing, reactions, read states) goes through
`permissions::evaluate`, which applies `threads::scope`. The many-recipient
paths go through `db::audience` -> `capacity::members_with`, which now applies
the same rules. Read states in threads go only to members
(`threads::readers`). The data export contains only the account's own content.
The audit log viewer shows thread names to people with View Audit Log, as
Discord does.

## Scheduled messages with files

`ScheduleMessage` takes `attachment_ids` (the composer's uploads to that
channel, at most 10; older clients send none). `UpdateScheduled` takes
`attachment_ids: Option<Vec<_>>` (`None` keeps them). `ScheduledMessage`
carries `attachments`. Migration `202611231101_scheduled_attachments.sql`.

- Scheduling claims each upload like sending does (yours, for that channel,
  unexpired, not attached or held elsewhere) and holds it (no expiry,
  `scheduled_attachments`). Held uploads keep counting toward the owner's
  upload quota, and `db::gc_uploads` never collects them.
- Delivery (timer or Send Now) hands them to the ordinary send path inside the
  delivery's transaction; a refused delivery keeps them held for a reschedule.
- Cancelling (or editing them out) releases them to the ordinary collection of
  unattached uploads, which removes them at the end of that command.
- Alt text and spoiler marks are not kept for scheduled files.

Test: `content::tests::integration::scheduled_messages_hold_their_files_until_sent_or_cancelled`.

## Threads in voice-channel chat

Not offered (the simpler correct option; Discord does not either). The server
refuses a thread whose parent is a voice channel; the web app shows no
Threads, Create Thread or thread permissions in voice-channel chat or on voice
channels. Browser check in `integration.mjs`.

## Search, inbox, saved messages and the Quick Switcher

- Result cards and inbox rows name the place: `#general`, `#lounge` (voice
  chat), `#general › Launch party` (a thread or forum post), or the DM's people.
- A result in a thread the client has not loaded used to read "Unknown
  conversation". The web app now learns those threads when a page of results
  arrives (`threads::learn`: one `GET /api/v1/threads/{id}` per unknown ID, at
  most 25, only then), so they are labeled and open directly. Jumping already
  fetched the thread.
- Announcement, forum and thread rows use their own icons (megaphone, speech
  bubbles, thread) in Pinned, Favorites and the Quick Switcher; threads show
  "Server · #parent".

## Data export

Adds `threads.json` (threads and posts you started), `scheduled.json` (with
held files), `soundboard/` (your uploaded sounds with their audio) and
`camera-backgrounds/` (your custom backgrounds), and to `settings.json`:
`poll_votes`, `thread_members`, `soundboard_favorites`, `soundboard_entrance`,
`soundboard_entrance_enabled`. Saved messages, reminders, pins and favorites
were already there; thread and forum messages are in `messages/` like any
message. Test: `authsec::tests::integration::data_export_includes_threads_votes_scheduled_sounds_and_backgrounds`.

## Header, composer and message actions

UI_PARITY 1.3's header order holds with every feature on: (DM call buttons,
thread or channel tools) Threads, Notification Settings, Pinned Messages,
Member List, Search, Inbox.

- Desktop: Create Thread is no longer a header button (it is in the Threads
  browser and on each message), so a text channel's header is the spec's.
- Phones: Notification Settings, Pinned Messages and Member List join the one
  "Channel Options" menu (with the thread or channel items), so the header is
  options, Search and Inbox (plus call buttons in DMs). Before, the title of a
  text channel was squeezed to 22 px and a DM's header overflowed and clipped
  Inbox at 390 px.
- `integration.mjs` checks every kind of conversation at 1440x900 and 390x844
  for page and header overflow, clipped buttons, duplicates, a squeezed title
  and the header order.

## Sign-in rate limits

QR sign-in: the waiting page redeems its ticket against the per-address
sign-in bucket, which every login, registration and re-authentication from
that address shares. A busy bucket made an approved sign-in fail (the page
fell back to the login form). The page now waits `retry_after_ms` and retries
while the 60-second ticket is valid (`public/authsec-login.js`). The ticket is
checked after the rate limit, so it is not spent by a refused attempt.

## Channel switching (REST requests per switch)

With every feature merged, opening a channel made four REST reads (the
channel for its topic, slowmode status, the newest page, the history extent),
every time, on one per-user budget of 120 a minute with a burst of 30 shared
by all REST reads. Browsing eight channels a few times exhausted it; history
then failed to load ("You're doing that too quickly") and `live.mjs` failed.

| Requests per switch (`switching.mjs`, 8 channels, 3 rounds) | Before | After |
| --- | --- | --- |
| First visit | 4 | 3 (topic once, newest page, extent) |
| Back to one of the 4 most recent channels | 6–11 (retries after 429) | 0 |
| Back to an older one (no longer cached) | 8–10 | 2 (newest page, extent) |
| Refused (429) over 20 switches | 78 | 0 |

- **Recent conversations reopen from the cache** (`live/history_cache.rs`).
  The model keeps the four most recently visited conversations, and message
  events reach every conversation you can see, open or not, so once a
  conversation's newest page was loaded in this stream its cached tail is
  complete. Switching back at the present shows it with no request. A
  disconnect, a resync, an older or anchored page (which may trim the newest
  messages) or eviction forgets it; the extent of a conversation not open is
  kept counted by message events.
- **Slowmode is asked only where it is on.** `ModerationState` (fetched once
  per server) lists the slowmode of the channels the viewer can see
  (`slowmode: [{channel_id, seconds}]`, appended, omitted when empty) and
  `SlowmodeUpdated` keeps it current. The client asks `/slowmode` (for the
  personal cooldown) only for a listed channel.
- **History reads have their own budget**: `GET channels/{id}/messages` and
  `history-extent` use `history:{user}`, 240 a minute with a burst of 60
  (`history::rate`), instead of the shared `rest:{user}` (unchanged: 120 a
  minute, burst 30). About 30 new conversations can be opened in a row, then
  one every half second. Abuse stays bounded: each read is index-only and at
  most 100 messages or 67 seeks, the budget is per account, and other reads,
  sockets, commands and uploads keep their own limits.
- The topic is still fetched once per channel per session (the channel list
  carries no topics). Server members and `moderation/me` are fetched once per
  server open.
- Also at sign-in: the empty `CallSnapshot` sent after every Ready no longer
  loads the 416 KB voice module on an idle screen (`wire::EMPTY_CALL_SNAPSHOT`).

Tests: `crates/web/tests/switching.mjs` (no 429, at most 3 requests for a first
visit, none for a recent one, no slowmode request in channels without it),
`moderation::tests::integration::moderation_state_lists_slowmode_of_visible_channels_only`,
`moderation::tests::integration::history_reads_have_their_own_budget`.

## Open: first-load size and idle memory

`loading.mjs` still fails its 490 KB budget: the core WASM is 816 KB Brotli
(recorded before the parity work: 421 KB; first load 1.11 MB vs 580 KB). The
idle tab's private memory follows it (the Chromium tree's delta over a blank
browser is 70 MiB, recorded 36 MiB; JS heap 4 MiB and WASM memory 1.6 MiB are
small, the rest is compiled code). Idle CPU is unchanged (0.09% of one core).

Measured with `twiggy` on the split core: most of the growth is MessagePack
decoding generated for the new protocol types (Deserialize for `Event`,
`CommandResult` and their thread, content, moderation and nav payloads:
about 1.9 MB of `rmp_serde` decode code before `wasm-opt`), which is in the core
because the socket decodes every event there. The feature UIs add about
250 KB. Moving that decoding out of the core needs a decision: decode the
envelope in the core and hand feature payloads (as bytes) to lazily loaded
modules, which changes how the web client consumes `Event`. The three feature
branches still in progress add more events, so this is worth settling first.
