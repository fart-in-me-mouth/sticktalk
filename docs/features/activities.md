# Shared activities (task `activities`)

Activities people in a call run together: Watch Together, a Whiteboard, and
three games (Four in a Row, Checkers, Sketch & Guess), plus activities the
instance administrator registers by URL. Everything is free. For activity
developers (SDK, protocol, state model, limits, the security boundary) see
`docs/ACTIVITIES.md`.

Code:
- `crates/proto/src/activities.rs`: wire types and limits.
- `crates/server/src/activities.rs` (sessions, relay, hooks), `activities/catalog.rs`
  (built-in and registered activities, server toggles, REST, admin CLI),
  `activities/frames.rs` (frame serving and policies), `activities/games.rs`
  (the games' rules), `activities/cards.rs` (invite cards, results, audit),
  `activities/tests.rs`. Migration `202611231801_activities.sql`.
- Web: `crates/web/src/live/activities.rs`, `src/live/ui/activities.rs`,
  `public/activities.js` (loaded on first use), `public/activities/` (the SDK,
  the shell, the built-in activities, the example), `styles/activities.css`.
- Browser test: `crates/web/tests/activities.mjs`.

## Permission

| Bit | Name | Default | Checked for |
|---|---|---|---|
| 48 | `USE_ACTIVITIES` | on for `@everyone` (migration for existing servers, a trigger for new ones) | launching and joining an activity in a guild voice channel (role and channel overwrites apply) |

Direct and group calls allow activities for everyone in the call. A timeout
removes the bit (a timed-out member keeps only View Channel and Read History)
and also disconnects them from voice, which takes them out of the activity.
Every rights change runs the voice reconcile, which now also removes
participants who lost Use Activities. Listed in the role editor (Voice &
Video) and the channel permission editor (Voice group, voice channels).

**Stage channels have no activities** (decided here): a Stage is one-to-many
audio for up to 1000 listeners, while a session holds 16 devices and every
participant writes; the audience is receive-only by design. The button is not
shown on a Stage and `Launch` there is `invalid_request` (`forbidden` for a
listener).

## Sessions

- One session per call (`conflict` otherwise). It belongs to the call's
  channel and lives only in server memory: participants, a document of at
  most 256 KB / 2048 keys, at most 32 shared files, at most 4 invite cards.
  With at most 64 voice sessions server-wide, the worst case is about 16 MB.
- Launch: the caller's call session must be connected (`joined`) in that
  channel and not a listener; Use Activities; the activity must be on in that
  server (or offered in direct calls). The launcher joins at once and is the host.
- Join: in the call, Use Activities, at most 16 participants (devices). A
  late joiner gets `Session` (with its own `you`) and the whole document.
- Leave, or its call session ending (`voice::remove` hook), leaves the
  session; the host passes to the earliest-joined participant. The last
  participant leaving, or the last call session leaving, ends it.
- End: the host, or a moderator of the channel (View Channel + Move Members,
  also from outside the call), or the server turning the activity off.
- Ending tells the call (`Ended` with the summary), rewrites the invite cards,
  keeps the result (`activity_results`, newest 50 per conversation), and
  writes the audit log in guild channels.

Nothing runs for a call without an activity: no task, no timer, one `HashMap`
lookup on voice removal. The server has no timer for activities at all.

## Wire (all additive, in the voice envelope)

`VoiceSignal::Activities(ActivitySignal)` (host-handled; the SFU rejects it)
and `VoiceEvent::Activities(ActivityEvent)`. Answers are `Ack` or `Error`
with the signal's nonce.

| Signal | Who | Notes / rate per user or device |
|---|---|---|
| `Launch { channel_id, activity_id }` | in the call | 6/min, burst 2 |
| `Join { session_id }` / `Leave` | in the call | 30/min |
| `End { session_id }` | host, or Move Members on the channel | 12/min |
| `SetHost { session_id, pid }`, `SetShared { session_id, shared }` | host | |
| `Write { session_id, ops: [{ key, value?, expect? }] }` | participant; key modes in ACTIVITIES.md | 600/min per device, 1800/min per session |
| `Send { session_id, to, data }` | participant | 1200/min per device, 3600/min per session |
| `Action { session_id, data }` | participant of a built-in game | 240/min |
| `ShareMedia { session_id, attachment_id }` | participant who can read the file | 30/min; audio/video only |
| `SaveResult { session_id, text }` | host (not for games: they save their own) | 10/min; ≤160 chars |
| `Invite { session_id, channel_id }` | participant with Send Messages there | 6/min; ≤4 per session |

| Event | To |
|---|---|
| `Session(ActivitySession)` | everyone connected to the call (on change, and when a call session connects) |
| `Ended { session_id, channel_id, summary }` | the call and the participants |
| `State { session_id, reset, done, version, server_time, entries }` | participants: the document in parts of 48 KB after joining, then each change |
| `Message { session_id, from, data }` | the named participants, or all but the sender; `from: 0` is the server |

Events go through `voice::send_payload` with the normal (non-critical)
budget: a slow connection misses ephemeral messages instead of queueing them,
and voice signaling keeps its reserve.

`ReportTarget::Activity { channel_id, session_id, user_id }` (appended): a
report about someone's part in a running session, by someone in that call.
Evidence: the activity, the channel and server, and the last five pieces of
text that person showed in it. Guild channels go to that server's report
queue, direct calls to the instance administrator. Filed from the tile's
options ("Report …"), through the app's report dialog.

`AuditAction::ActivityStart`, `ActivityEnd` (appended; `activity_start`,
`activity_end`), target the channel, with the activity's name; an end by
everyone leaving is noted with the starter as actor and the reason "Everyone
left".

`MessageExtra.activity: Option<Box<ActivityCard>>` (appended): an invite card.
Older apps show the message text ("X invited you to Y in a call…").

## REST

| Route | Who | Answer |
|---|---|---|
| `GET /api/v1/activities?channel_id=` | can see the call channel | `ActivityCatalog { activities, can_launch, results }` (a Stage or text channel: none) |
| `GET /api/v1/activities/media?channel_id=` | Read History there | newest 30 audio/video attachments; without `channel_id`: your own uploads you can still read |
| `GET /api/v1/guilds/{id}/activities` | Manage Server | every activity with `enabled` |
| `PUT /api/v1/guilds/{id}/activities/{activity_id}` `{ enabled }` | Manage Server | 204; turning one off ends its sessions there |
| `GET /activities/<path>` | anyone (static) | built-in activity files with the frame policy |
| `GET /activities/frame/custom-<n>` | anyone (static) | the shell of a registered activity |

`GET /api/v1/files/{id}` also serves a file shared into a session to that
session's participants while it runs (an in-memory check, before the usual
ones). IDs in these JSON bodies are decimal strings.

Admin CLI: `sticktalk-server admin activities list|add|remove` (see
ACTIVITIES.md §7). The app's own policy gains `frame-src 'self'`; frames under
`/activities/` are served without `X-Frame-Options: DENY` and with
`frame-ancestors` the app's origin.

## Built-in activities

- **Watch Together**: a shared queue (`q/<file>`, anyone adds), the timeline
  in `h/player` (the host, or everyone once shared). The app plays the file
  in the tile through the call's output device, at the activity volume,
  silent while deafened, and keeps viewers within the shared timeline by rate
  nudging (seek only beyond 1.5 s). Only files on this server; "Upload to
  This Call's Chat" sends the file to the call's chat first, so everyone who
  can read the chat can read it there too.
- **Whiteboard**: pen, highlighter, eraser, rectangle, ellipse, line, arrow,
  text, sticky notes, 8 colors, per-person undo/redo (Ctrl+Z / Ctrl+Y), pan
  (tool, middle button, two fingers), zoom (wheel, pinch, buttons, fit), live
  cursors with names, export to PNG. Each finished item is one key
  (`i/<pid>-<n>`; strokes simplified, delta-encoded); strokes in progress and
  cursors are ephemeral messages at most 15 a second per person. Drawing only
  redraws in an animation frame when something changed. The host can clear
  the board. The board shows how full the 256 KB document is.
- **Four in a Row** and **Checkers** (English draughts: forced captures,
  multi-jumps, crowning ends the turn, 80 quiet moves draw): two seats,
  spectators, resign, leave seat (forfeit during a game), rematch with the
  other player first. The server runs the rules.
- **Sketch & Guess** (original): the server picks a word from a built-in list
  and tells only the drawer; only the drawer writes strokes (`d/`) during the
  round; guesses are actions, screened by AutoMod, scored by order (10, 8, 6…,
  the drawer 2 per correct guess), "So close!" privately for one-letter misses;
  every participant draws once; the winner is kept as the result.

## Client

- An **Activities** button in the call controls (voice channels, DM and
  group calls; not Stages) with a dot while one runs. The launcher (a dialog
  from `activities.js`) lists name, icon, description and player range,
  Start, Join for a running one, and recent results.
- "X started Y" with Join and Dismiss above the people in the call view.
- The tile: the app keeps a box for it in the call view (`#live-activity-slot`,
  split with the people or focused), and `activities.js` draws the activity in
  one layer outside the app's re-rendered tree that follows that box; when the
  call view is not shown, or minimized, it floats as a small window (desktop
  bottom right, phones above the composer). The frame never reloads while
  the layout changes. Its bar: people (speaking ring), Invite, Focus / Show
  Everyone, Minimize, Options (activity volume, host: hand over, share
  control, end for everyone; Report …), Leave.
- Invite: a conversation picker (the call's channel first, the server's
  text and voice channels, your DMs); the card shows LIVE, who is in it and
  Join, updates as people join and leave, and the summary at the end. Join
  joins the call (answering a DM call) and then the activity, in one step.
- Server Settings › Activities (Manage Server): a switch per activity.
- Loading: nothing at sign-in. `activities.js` loads only when an activity
  is seen in the call or the launcher opens; each activity loads only when
  launched or joined. Neither is in the service worker's precache.

## Decisions to revisit

- Stage channels have no activities.
- Activity sound from the shared player goes through the call's output;
  sound an activity's frame makes itself reaches the default output (a
  sandboxed frame cannot pick a device). The built-in activities make none.
- A registered activity runs in an opaque origin (no storage of its own),
  the price of the strict sandbox.
- A file shared into Watch Together can be streamed by everyone in the
  session, even someone who cannot read the conversation it came from (like
  showing it on a screen share); the sharer must be able to read it.
- Host-authoritative (`h/`) keys trust the host's client; the built-in games
  run on the server instead.
- Cards are limited to four per session and only in the same server's
  channels or your DMs; a DM call's card only in that DM.

## End-to-end encrypted media (later task)

Activities carry no call media: the shared document and messages go over
the WebSocket through the server (which must read them to enforce key modes,
AutoMod and the games' rules), and Watch Together streams a stored file over
HTTPS, not through the SFU. E2EE call media does not change any of this. If
activity data should later be end-to-end encrypted too, AutoMod screening and
server-run games would have to move to clients (or be given up for encrypted
sessions).

## Measurements

From `crates/web/tests/activities.mjs` (see the results file the test writes).
