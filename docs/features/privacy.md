# Privacy, status and notifications (task `privacy`)

Presence selection with durations and an automatic idle, custom status with
emoji and expiry, friend nicknames and private notes, Ignore, who can DM you
(message requests and a spam list), the full notification control set,
in-app reports, and the server side of age-restricted channels. Everything is
free for every user. There is no second sign-in step anywhere.

Code: `crates/proto/src/privacy.rs` (wire), `crates/server/src/privacy.rs` and
`privacy/{status,people,requests,notify,reports,gate,tests}.rs` (server),
`crates/web/src/live/privacy.rs` and `live/ui/privacy.rs` (web),
`crates/web/public/alerts.js` (sounds, desktop notifications, badge),
`crates/web/tests/privacy.mjs` (browser suite). Migration
`202611230901_privacy.sql`.

## Wire protocol (all additive)

- `Command::Privacy(PrivacyCommand)`, `CommandResult::Privacy(PrivacyResult)`.
- `Event::Privacy(PrivacyEvent)`: sent **only** to streams that sent
  `PrivacyCommand::Subscribe` (the web app does after every Ready).
- `UserSummary.status_emoji` (optional, omitted when empty): the custom
  status emoji beside `status_text`.
- `AuditAction::ReportResolve` (`report_resolve`), appended.

| Command | Notes |
|---|---|
| `Subscribe` | opts this stream in to `Event::Privacy` |
| `SetStatus { status, until }` | Online, Idle, Dnd, Invisible; `until` (not for Online) at most 24 h ahead; then back to Online |
| `SetCustomStatus { text, emoji, clear_at }` | text ≤ 128 chars; emoji is one Unicode emoji grapheme or `<:name:id>`/`<a:name:id>`; `clear_at` ≤ 366 days ahead; all empty clears |
| `SetFriendNickname { user_id, nickname }` | friends only, ≤ 32 chars, empty removes; shown only to you |
| `SetNote { user_id, note }` | anyone whose profile you can see, ≤ 256 chars, 500 notes; empty removes |
| `Ignore`/`Unignore { user_id }` | anyone whose profile you can see, 100 at most; they are never told |
| `SetDmPrivacy(DmPrivacy)` | `allow: friends | server_members` plus per-server overrides (servers you are in) |
| `AcceptRequest`/`DeclineRequest { channel_id }` | recipient only; the sender is never told |
| `SetNotifications(NotificationOptions)` | the existing `NotificationSettings` plus `suppress_roles` and `mobile_push` (server scope); keeps `mute_until` and `hide_muted_channels` |
| `SetGlobalNotifications(GlobalNotifications)` | desktop, sounds (master + message/mention/direct/reaction), badge mode, reaction alerts |
| `Report(ReportDraft)` | message, user or server; category and optional note ≤ 1,000 chars |

Results: `Settings(PrivacySettings)` (after every setting change),
`Note(UserNote)`, `Filed(MyReport)`.

Events: `Settings` (other devices, timers), `Alert(Alert)` (see below),
`Requests(RequestCounts)` (quiet badge), `GuildReports { guild_id, open }`
(online moderators), `ReportUpdated(MyReport)` (the reporter), `Note`.

REST (session cookie or bearer):

| Route | Who | Answer |
|---|---|---|
| `GET /api/v1/users/@me/privacy` | you | `Snapshot<PrivacySettings>` |
| `GET /api/v1/users/@me/message-requests` | you | `Snapshot<MessageRequests>` (pending and spam, 100 each, newest 3 messages) |
| `GET /api/v1/users/@me/reports` | you | `Snapshot<Vec<MyReport>>` (newest 50) |
| `GET /api/v1/users/{id}/note` | you | `Snapshot<UserNote>` (empty when none) |
| `GET /api/v1/guilds/{id}/reports?status=open\|closed&before=` | Manage Messages, Kick, Ban or Time Out Members (or owner/admin) | `Snapshot<ReportPage>` (50 a page, never the reporter) |
| `POST /api/v1/guilds/{g}/reports/{id}/resolve` | same, plus the action's own permission | body `ReportResolution { action, reason }`; answers the `Report` |

Admin CLI: `sticktalk-server admin reports list [--all] | show <id> | dismiss <id> | disable-user <id>`.

## Behavior

**Status.** The chosen status (with its end) is stored on the account. One
task sleeps until the nearest deadline (timed status, custom status clear,
notification mute, automatic idle) and is woken when a nearer one is set. The
automatic idle needs no client timer: every connection already reports window
focus and visibility (`ClientMsg::Focus`); when none of a person's windows has
been focused for 10 minutes, Online shows as Idle until one is focused. It
never changes a manual choice. Invisible shows as offline to everyone else
(including @here) but notifies normally.

**Notifications — one decision.** `privacy::notify::decide` is used by Web
Push, the in-process notification hook and the app's alerts. In order: the
author is blocked or ignored → nothing; a hidden message request or an
unconfirmed age-restricted channel → nothing; Do Not Disturb → nothing; then
the effective setting. Precedence: the server's setting, then each channel the
conversation is nested in from the outside in, then the channel itself; the
nearest setting gives the level and the suppression switches; a mute anywhere
on that chain mutes it. DMs and group DMs have their own setting (default all
messages; server default only @mentions). Mobile push is the server's switch.

| Setting | Unread dot | Mention badge | Inbox Mentions | Inbox Unreads | Alert / push |
|---|---|---|---|---|---|
| All messages | yes | yes | yes | yes | every message |
| Only @mentions | yes | yes | yes | yes | mentions and DMs |
| Nothing | yes | yes | yes | yes | none |
| Muted (any level) | hidden (app) | yes | yes | hidden | none |
| Suppress @everyone/@here | — | not counted | excluded | — | not a mention |
| Suppress role mentions | — | not counted | excluded | — | not a mention |
| Ignored author | not counted | not counted | excluded | not counted | none |
| Do Not Disturb | — | — | — | — | none (incl. ringing) |

Suppression is decided when a mention is stored (`suppressed_mentions`, also on
edits) and again at delivery time from the stored mention kinds, so a later
change of settings applies to push and alerts too. Read-state counts exclude
suppressed mentions and ignored authors (`db::recount`, recounted for the
person on Ignore/Unignore).

Mutes with a duration (15 min … 24 h, or until turned off) end through the
deadline task (`NotificationSettingsUpdated` with `muted: false`) and are
treated as ended as soon as their time passes even before it runs.

**Alerts.** After a message or reaction commits, online subscribed recipients
the decision allows get `PrivacyEvent::Alert { kind, channel_id, message_id,
user_id, emoji, at }` (`message`, `mention`, `direct`, `reaction`). The app
plays the sound for that kind if enabled and shows a desktop notification
(title: who and where; body: the message text) only when the app is not
focused and permission was already granted; never in Do Not Disturb, never
for the conversation being looked at, and not for alerts older than 10 s
(replayed streams). Reaction alerts follow "Notify me about reactions"
(DMs only by default). `public/alerts.js` creates nothing until an alert plays.
The unread badge (window title and `navigator.setAppBadge`) counts mentions
and DMs, every unread conversation, or nothing.

**Ignore.** Separate from block. Ignored people's messages are collapsed
behind "Ignored message" (click to show); their messages and mentions do not
badge, alert, push or reach the Inbox; their typing is not shown to you and
their custom status is not shown on your friend list. They can still see you
and are never told.

**Who can DM you; message requests.** Friends only by default (as before).
With "Friends and people in my servers" (overridable per server), a
first-contact DM from someone sharing an allowed server is created as a
request: the recipient's side stays hidden (`dm_members.visible=0`), the
final delivery gate (`privacy::gate::filter`) keeps every event of that
conversation from them, nothing alerts or pushes, and they get only a quiet
count. Accept makes it a normal DM (replying or opening it from your side also
accepts); Ignore keeps it hidden; "Block and Report" ignores the request,
blocks and files a report. The sender is not told which. Spam list: a burst of
first contacts (5 within 10 minutes), a new account (under 3 days) starting 2
within 10 minutes, or a links-only message while pending. First contacts are
limited to 10 an hour and 30 a day per sender. A server's paused DMs and
verification level apply when the request is started and on every message.

**Reports.** The report keeps the reported content as it was (author, text up
to 4,000 chars, attachment names, times, channel and server names, profile
text), so later edits or deletion do not remove it. Message reports in a
server go to that server's Reports queue; reports about DMs, people and
servers go to the instance administrator (CLI above; reports whose server was
deleted go there too). One open report per person and target; 10 an hour
(burst 3); 50 open at once. The reporter is never shown to moderators, the
administrator queue listing, or the reported person. Moderator actions
(dismiss, delete message, time out, kick, ban) run the normal command as the
moderator (its own permission check and audit entry), then close the report
and write a `report_resolve` audit entry. The reporter sees the status.

**Age-restricted channels, on the server.** For someone who has not confirmed
their age there: no new messages, edits, reactions, typing, poll tallies or
link previews over the WebSocket (deletions, read states and counts still
come); no history, pins or saved messages (`db::text` with Read History asks
the moderation gate); no push or alerts; no Inbox mentions or unread entries;
no search results.

**Friend nicknames and notes.** Stored per account on the server, private.
The web app shows a friend's nickname in place of their name everywhere
(messages, member list, DMs, friends, profile heading), with their real name
on the profile. A former friend's nickname is not shown.

## Changes to existing behavior

- Saving notification preferences no longer clears the mute deadline or turns
  off "hide muted channels" (old form fixed too).
- Invisible users are now notified and pushed like anyone else; only Do Not
  Disturb holds notifications back (`push/tests.rs` updated).
- A suppressed @everyone/@here (or role) mention no longer counts in mention
  badges (`mentions.rs` test updated: 4, was 5).
- `SetPresence` clears a timed status's end.
- `Model::name` (client-core) prefers a friend nickname; `UserUpdated` now also
  refreshes the friends list's copy of that user (status and name were stale).

## Not included

- Desktop notifications show the message text (Discord does too); push keeps
  its generic lock-screen text.
- Custom server emoji in the status are accepted as `<:name:id>` but the picker
  offers Unicode emoji only until the custom emoji work is merged.
