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
- Alt text and spoiler marks are kept (task `qa2`, below).

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

# QA pass 2 (task `qa2`)

Branches merged after the first pass broke several browser suites and left
known defects. These are the rules this pass established; each has a server
test or a browser check named with it.

## Custom elements kept across renders (Leptos/tachys patch)

`tachys` 0.2.19 (the newest 0.2) compared a custom element's tag as written
(`ui-dialog`) with the DOM's upper-case `tagName` (`UI-DIALOG`), so **every
render rebuilt every `<ui-*>` element from scratch**: dialogs, menus,
tooltips, avatars. Typed text, focus and open menus were lost whenever
anything re-rendered (the profile editor lost an unsaved name when an avatar
upload finished; `live.js` papered over it for most forms by snapshotting
fields), and every avatar was redrawn. The workspace patches tachys
(`crates/web/vendor/tachys`, `[patch.crates-io]` in `Cargo.toml`, like
`sqlx-sqlite`):

- `html/element/mod.rs`: the tag comparison is case-insensitive.
- `renderer/dom.rs`: `checked` and `selected` written as attributes also set
  the live property. Tachys writes an attribute only when the view's value
  changed, so a control the person touched follows the view again (a saved
  setting, a refused change, another device), like `prop:checked`. Before,
  re-creating the element did that by accident.

Remove the patch once upstream compares case-insensitively. Checks:
`call-and-composer.mjs` (the unsaved name survives an avatar upload),
`privacy.mjs` (a radio group follows the saved value), `community.mjs` (the
rules checkbox).

## Rate-limit budgets

The first pass gave history reads their own budget. A real person hit the
remaining one: the per-account command budget (120 a minute, burst 20) was
also spent by the commands the app sends by itself. Every Ready (sign-in,
reload, reconnect) sends eight feature `Subscribe`s and the device name (9 of
the 20), and every message landing in the open conversation sent a
`MarkRead`. A reload followed by a few actions, or a busy channel, left
nothing for what the person did ("You're doing that too quickly"; `live.mjs`
failed at the role and channel steps). Now (`server/src/budgets.rs`):

- session setup (`*::Subscribe`, `UpdateDevice`) and read markers have
  budgets of their own;
- the web app coalesces read markers: the first at once, the rest as one
  after a one-second gap, never a second one while the first is unanswered
  (`operations::mark_read`).

Test: `budgets::tests::setup_and_read_markers_do_not_spend_the_actions_budget`
(three reloads and 40 read markers leave the whole action burst),
`budgets::tests::every_budget_still_refuses_a_flood`; `live.mjs` passes.

Every budget (per account unless said; "a minute / burst"):

| Budget | Limit | Drawn on by | Normal heavy use |
| --- | --- | --- | --- |
| `cmd` | 120 / 20 | every durable WebSocket command a person sends | bursts of ~10 actions (reactions, a role save, channel edits) |
| `session` | 120 / 60 | `Subscribe` of content, threads, expression, privacy, community, schedule, appearance, moderation; `UpdateDevice` | 9 per Ready: six reloads back to back, then one every 5 s |
| `read` | 240 / 60 | `MarkRead` | at most one a second per tab after coalescing |
| `send` | 60 / 10 | messages, polls, GIFs, forwards, voice messages, sticker sends (on top of `cmd`) | a fast typist sends < 1/s |
| `typing:{user}:{channel}` | 15 / 1 | `Typing` (not durable) | one per 4 s while typing |
| `control:{connection}` | 600 / 20 | WebSocket control frames (focus, heartbeat) | a few a minute |
| `rest` | 120 / 30 | every REST read without its own budget (members, profiles, settings, `moderation/me`, community, events, threads, reports, polls' voters, badges, covers, backgrounds, push, slowmode, folders, nav, privacy) | a server open is 3 to 6 reads |
| `history` | 240 / 60 | `GET channels/{id}/messages`, `history-extent` | ~30 new conversations in a row |
| `file` | 600 / 120 | attachments and thumbnails, link preview images, stickers, **custom emoji**, **soundboard audio** (these two had no limit) | a screen of images is < 100; cached images ask once per page |
| `upload` | 20 / 20 | file uploads (plain and resumable) | 10 files per message |
| `inbox` | 60 / 10 | Inbox mentions and unreads | opening the Inbox and its tabs |
| `search` | 30 / 5 | message search (and the MCP search tool) | typing a query and refining it |
| `nav-write` | 60 / 20 | saved messages, reminders, favorites, folders, pins of the rail | |
| `profiles` | 30 / 10 | profile card fetches | |
| `community` | 120 / 30 | community REST reads | |
| `onboarding` / `apply` / `template-build` | 60 / 20; 2 / 5; 6 / 3 | onboarding steps; applications to join; building a server from a template | |
| `events` / `event-interest` / `event-cover` | 30 / 10; 60 / 20; 10 / 3 | event create and edit; interested; cover uploads | |
| `stage` | 60 / 10 | Stage signals | |
| `themes` / `tts` | 30 / 10; 10 / 3 | theme library and settings; `/tts` messages | |
| `thread` / `follow` | 10 / 5 | creating threads and posts; following announcement channels | |
| `voice-join` / `voice-moderate` / `call-ring` / `call-decline` | 30 / 5; 120 / 20; 12 / 4; 60 / 10 | voice joins; server mute, move; ringing again; declining | |
| soundboard | list 60 / 10, upload 20 / 5, edit 60 / 10, favorite 120 / 20, entrance 30 / 10, play 20 / 4 per person and 60 / 8 per room | | |
| clips, call quality, camera backgrounds | 30 / 6 and 12 / 3; 30 / 10; 10 / 3 uploads | | |
| `emoji-upload` / `sticker-upload` | 10 / 2 | | |
| `gifs` | 30 / 10 | GIF search | |
| `push-sub` | 30 / 10 | push subscriptions | |
| reports | 10 an hour, burst 3 (filing); 60 / 10 (resolving) | | |
| `automod-alert:{user}:{rule}` | 6 / 1 | AutoMod alert messages | |
| sign-in (per address) | 10 / 5 | login, registration, re-authentication, QR redemption, Google, email reset; plus per-name 5 / 3, per-account 10 / 5, Google subject 10 / 5, password-reset mail 3 an hour, passkeys 10 / 5 per account and 20 / 10 per address, QR 10 / 5 | |
| `export-user` / `mcp` / admin | 5 / 3; 120 / 30; 10 / 5 | data exports; MCP tools; admin endpoints | |

Every route that serves data has a budget; the two that had none (custom
emoji images, soundboard audio) now draw on `file`. No bucket is so large it
stops a flood: the largest bursts (`file` 120, `session` and `read` 60) are
all per account, cheap reads, and refused past their limit.

## Images under `/api` are cached (and still permission-checked)

Every `/api` answer used to be `no-store`, emoji and sticker images included,
so they were fetched again on every page. Now a handler may choose its own
policy (`browser::boundary` keeps it; everything else stays `no-store`), and
images choose one of two (`server/src/caching.rs`):

- **Checked** (custom emoji, stickers, attachments and thumbnails, link preview
  images, event covers): `private, no-cache` with a strong ETag naming the
  asset and its version. The browser keeps the bytes and asks before use
  (once per page; Blink reuses an image validated in the same document); the
  server runs the permission check, then answers `304` without the body.
  Someone who may no longer see the image gets 404, and so does another
  account signed in to the same browser (they share the HTTP cache). This is
  why these are not `max-age`: a fresh cached copy would be shown to anyone
  using that browser without asking.
- **Immutable** (badge icons: any signed-in account may see them and an icon
  never changes): `private, max-age=31536000, immutable`.

The read-only image routes no longer hold the global command lock across
their permission checks (emoji, attachments, badge icons, soundboard audio):
the lock is taken for the rate budget only.

Test: `expression::tests::emoji_images_are_cached_and_revalidated_against_permissions`
(304 for the viewer, 404 for another account and after leaving the server,
JSON still `no-store`, the `file` budget refuses a loop).

## Age-restricted channels cover everything nested inside them

Every age check looked at the conversation's own row, so a thread or forum
post in an age-restricted channel was readable, searchable, pushed and in the
Inbox without confirming (security review F3). Now a conversation is
age-restricted when it **or any channel it is nested in** is
(`moderation/age.rs`): threads, forum posts and, by the lead's decision,
channels nested inside a restricted one. `moderation::age_gate` (history,
pins, saved messages and reminders, link preview images, forward sources),
`privacy::gate` (live events, push, alerts, the Inbox, mentions), search and
the mentions feed all use it. Confirming from inside a thread confirms each
restricted channel above it. The web app gates a thread the same way
(`moderation::gated` walks the parent chain).

Test: `privacy::tests::threads_inside_an_age_restricted_channel_are_restricted_with_it`
(live message, push and alert decision, history, pins, mentions, search,
saved messages, forwarding, confirming from the thread).

## Forwarding and privacy

A forward's snapshot is shown to everyone who can read the destination, so a
message from an age-restricted conversation (or a thread inside one) can only
be forwarded into another age-restricted channel. The source link is shown
only to people who can read the source now (`text()`: private threads, age
restriction, permissions); a channel hidden by Browse Channels opt-in is still
readable by permission, so its link is shown, as in Discord. Test: in the
age-restriction test above.

## One notification decision

`privacy::notify::decide` decides for messages; `privacy::notify::notice`
(new) for everything else, in the same order: who it is from (blocked or
ignored), a hidden message request or an unconfirmed age-restricted
conversation, Do Not Disturb, the mute and level of the conversation or
server, and the server's mobile push switch for pushes.

| Notification | Path | Decision |
| --- | --- | --- |
| Messages, mentions, DMs, reactions (alerts and push) | `decide` | as before |
| AutoMod alerts | a message in the alert channel | `decide` (the alert channel's settings) |
| An event starting (in-app notice and push) | `notice(EventStarting)` | server mute or Nothing, the event channel's settings, DND, mobile push (was: server row only, no mobile push switch) |
| Reminders (push) | `notice(Reminder)` | DND, age restriction, mobile push; not mutes (you asked for it) (was: DND only) |
| A call ringing (push) | `notice(Ring)` | blocked or ignored caller, an unaccepted message request, a muted conversation, DND (was: DND and block) |
| Stage invitations to speak, application decisions, report status | state shown where you look | never an alert or a push |

Test: `privacy::tests::events_reminders_and_rings_go_through_the_one_notification_decision`.

## Message requests, ignored people and calls

A first-contact DM that is still a request (pending, spam or ignored) could
be called: the call rang the recipient, pushed "Incoming call" and un-hid the
conversation (security review F2). Now the requester cannot start or join a
call there ("You can call them once they accept your message request"); the
recipient may call back. Nobody who ignores the caller, or holds the
conversation as a request, is rung, in DMs and group DMs, and a call never
un-hides a request. Test:
`privacy::tests::requests_and_ignored_callers_never_ring_or_unhide_the_conversation`.

## Ignore

Someone you ignore shows no presence (no dot, listed as offline in the member
list, friends and DMs) and no custom status or activity, on the friend list
and on their profile card; they do not ring you. Their typing, messages,
mentions, reactions alerts and thread and forum activity already followed the
rule (the privacy gate and `decide`). The ignored person is not told, except
that a call to you no longer shows you as ringing.

## Reports: evidence only for those who can read it

The server's Reports queue showed a reported message's captured text to every
moderator, including those who cannot read its channel (security review F4).
A moderator who cannot read the channel now sees the report, its category,
target and the reported person, with `evidence_hidden: true` and no text,
attachments or channel name (queue and resolve). Test:
`privacy::tests::reported_text_is_shown_only_to_moderators_who_can_read_the_channel`.

## AutoMod on text that is not a message

Added to the coverage table above (blocking keyword rules, Manage Server
exempt, each refusal in the audit log; security review F6):

| Path | Timeout | AutoMod |
| --- | --- | --- |
| Your own nickname (`SetNickname`) | yes | yes |
| Event name and description (create, edit) | yes | yes |
| Stage topic (start, change) | yes | yes |
| Custom emoji name (upload, rename), sticker name and description (upload, edit) | yes | yes |
| Application answers (joining a server with questions) | n/a | yes (the applicant is not a member yet: no exemption) |

Test: `threads::tests::integration::nicknames_events_and_expression_names_are_screened_by_automod`.

## Data export and audit log

The export adds `reports.json` (reports you filed and their status, without
the reported content, which is someone else's), and to `account.json` the
saved custom status (emoji, clear time) and name style; `settings.json` adds
per-server profiles, cosmetics and the badges shown, badges earned, custom
themes and appearance settings, notification extras and global notification
settings, DM privacy, friend nicknames, notes, ignored people, expression
settings, age confirmations and events you are interested in. Server theme
changes are in the audit log (`guild_update`, field `theme`, before and
after). Tests:
`authsec::tests::integration::data_export_includes_threads_votes_scheduled_sounds_and_backgrounds`,
`themes::tests::server_themes_need_manage_server_and_reach_members`.

## Scheduled messages keep alt text and spoiler marks

`ScheduleMessage` takes `attachments: Vec<AttachmentMetaPatch>` (appended,
omitted when empty; older clients send none), stored on the uploads before
they are held, exactly as a send stores them; delivery hands the same uploads
to the send path, which reads them back. Test:
`content::tests::integration::scheduled_files_keep_their_alt_text_and_spoiler_marks`.

## Permissions of the later features

| Bit | Role editor | Channel editor (where) | @everyone default | Timed out / screening pending | Owner, Administrator |
| --- | --- | --- | --- | --- | --- |
| View Audit Log, Time Out Members | Moderation | server-wide, not offered | no | lost | yes |
| Create Public/Private Threads, Send Messages in Threads | Threads | text, announcement, forum | yes (migration and trigger) | lost | yes |
| Manage Threads | Threads | text, announcement, forum | no | lost | yes |
| Publish Messages | Text | announcement | no | lost | yes |
| Use Soundboard | Voice & Video | voice, Stage | yes | lost | yes |
| Manage Soundboard | Voice & Video | server-wide | no | lost | yes |
| Use External Emoji, Use External Stickers | Text | every text kind | yes | lost | yes |
| Create Events, Manage Events | Events | server-wide (checked without a channel) | no | lost | yes |
| Request to Speak | Voice & Video | **Stage** (was missing) | yes | lost | yes |
| Send Text-to-Speech Messages | Text | every text kind | yes | lost | yes |

Also fixed: the role editor did not offer bits the server enforces (Change
Nickname, Manage Nicknames, Embed Links, Mention @everyone, Priority Speaker,
Mute, Deafen and Move Members), so nobody below Administrator could be given
voice moderation; the channel editor did not offer voice rights on Stage
channels. The lists are in `web/src/permission_lists.rs`, tested against the
protocol (`role_editor_lists_every_bit`,
`channel_editor_lists_every_channel_right_where_it_applies`); the server rules
in `permission_tests::late_bits_follow_owner_admin_timeout_and_screening_rules`.

## Smaller fixes

- **The composer keeps room to type** beside a call (380 px panel) and on
  phones: below 32rem of composer width the GIF, Stickers and Kaomoji buttons
  move into More Options (a container query, `@container/composer`), below
  24rem the gaps tighten. The message box beside a call was 0 px wide
  (`calls.mjs`); on phones it was under 80 px (`phone.mjs`).
- **An invite says whether it has an event**: `InvitePreview.event` (omitted
  when false). The Join dialog asks for the event card only then; before, every
  invite made a request that failed with 404 (`content.mjs`, `threads.mjs`).
- **The profile editor follows `ProfileUpdated`**: About Me saved, then
  Profiles opened at once, showed the old bio (`polish.mjs`).
- **Styled names on light profile cards**: a style is checked against the dark
  chat when saved; a card tinted with the person's colors can be light. The
  card now checks the name against both ends of its own gradient
  (`name_reads_on_card`) and, if it does not reach 4.5:1, draws it in the
  card's text color (font kept); the light-theme lightness clamp no longer
  re-darkens names on tinted cards. Badges sit on their dark chip on every
  card. Test: `profiles::tests::names_are_checked_against_the_card_they_are_on`.
- `preferences.js` asks `matchMedia` on first use, not at import (Node tests
  import it).

## Announcement follows end with their creator's access

A follow kept copying published messages (and their files) into the
follower's server after the person who made it was banned or lost access to
the announcement channel (security review G1). A follow is now kept only while
its creator can read the announcement channel and manage the channel it posts
in: every delivery checks (`threads/announce.rs`, `still_allowed`); a ban,
kick or leave in either server, or a role or overwrite change there, re-checks
that server's follows at once (`revalidate_follows`, after the command). A
follow that fails is removed and logged in both servers' audit logs
(`channel_unfollow`, with the reason). Removals of copies always propagate.
Test: `threads::tests::integration::follows_stop_when_their_creator_loses_access`.

## Links say where they go

A link could show one site and open another (security review G2). The shared
renderer (`client-core/src/markdown.rs`, used by messages, bios, statuses and
everything else drawn as Markdown) now:

- does not make a masked link (`[text](url)`) whose text names another site (a
  URL or a domain-shaped word whose host differs, `www.` aside): the Markdown
  is shown as typed, and any URL in it links to exactly where it reads;
- refuses URLs with a login part (`https://paypal.com@evil.example`) and URLs
  containing bidirectional controls (U+202A to U+202E, U+2066 to U+2069, the
  marks); a control ends an autolink and is removed from link text;
- shows an autolink whose host is in another script in the ASCII (punycode)
  form the browser will use.

In the web app every web link carries its real host (tooltip and
`data-link-host`). Opening a masked link, or a link to a host this browser has
not been told to trust, asks first in the app's own dialog, naming the host
and the full address, with Cancel, "Always open <host>" (remembered per
browser, at most 200) and Open Link. Link preview cards show the real host,
with the page's own site name only beside it. Test:
`markdown::tests::links_show_where_they_go`.

## Onboarding gives ordinary member rights only

`ONBOARDING_FORBIDDEN` listed eight rights; Mention Everyone, voice
moderation, Manage Nicknames, Manage Threads, View Audit Log, Manage
Expressions, Events and Soundboard and anything added later passed, and a
channel overwrite was not looked at (security review G4). Now onboarding may
give a role only if its rights, including every channel overwrite's allows for
it, are within `ONBOARDING_ALLOWED` (view, send, read history, attach,
react, invite, change nickname, embed links, connect, speak, video, stream,
voice activity, soundboard, create and send in threads, external emoji and
stickers, request to speak, TTS), at save and again at grant. A new bit fails
`every_bit_is_classified_for_onboarding` until it is classified. Test:
`community::tests::onboarding_refuses_broadcast_and_moderation_rights_and_overwrites`.

## Two more fixes found by the suites

- **SFU keyframes**: a viewer waits for a keyframe before getting a video
  route's packets; the one request sent when the route was made could be lost
  while the publisher's encoder started, and that viewer then never got
  video (`live.mjs`, about one run in three). While a route waits, each packet
  from the source asks again (at most once a second per source).
- **Checkboxes with an action** (`data-live` on a native checkbox or radio):
  `live.js` no longer cancels the click, which made the browser flip the box
  back after the app had drawn the new state (`community.mjs`, the rules box).
- **Typed text and focus are restored only into replaced elements**: `nav.js`
  and `live.js` snapshot fields before a render and put them back after the
  next DOM change. With elements now kept, a render that changed nothing left
  a stale snapshot that a later change restored over newer typing (the search
  ran with the previous query, `nav.mjs`), and could move focus back. They
  restore only into an element the render replaced, and focus only when it
  was lost.
- **The chat beside a call stays in its pane**: the call view shrinks and its
  controls wrap; before, at 1440 px with the member list open, the chat ran
  under the member list (`integration.mjs` checks it).
- **A created role's answer no longer closes its editor**: only a saved
  role's editor returns to the roles list (`moderation.mjs`, one run in four).

## Tests updated because a control moved on purpose

- `expression.mjs`: the Super Reactions animation switch is in Accessibility
  (themes), reached from Expression's link.
- `privacy.mjs`: the Do Not Disturb dot is the `bg-dnd` token class.
- `platform.mjs`: preferences gained the themes fields; the test checks the
  basics and that unknown keys and values are dropped.
- `polish.mjs`: six permission groups and 42 rights in the role editor; the
  roles dialog's color swatch is looked up inside the dialog (member rows
  carry `role-color` too).
