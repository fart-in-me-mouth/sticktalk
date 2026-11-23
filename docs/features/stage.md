# Stage channels and scheduled events (task `stage`)

Everything here is free for every server. StickTalk has no boosts, so a
Stage's audience (with or without video) is limited only by server capacity.
Wire additions are additive: new enum variants appended at the end, new
optional fields with serde defaults, new routes.

Code: `crates/proto/src/stage.rs`, `crates/proto/src/schedule.rs`;
`crates/server/src/stage.rs` (+ `stage/tests.rs`), `crates/server/src/schedule.rs`
(+ `schedule/recur.rs`, `schedule/routes.rs`, `schedule/tests.rs`), hooks in
`voice.rs`; `crates/sfu/src/engine.rs` (listeners) and `crates/sfu/tests/stage.rs`;
`crates/web/src/live/stage.rs`, `crates/web/src/live/ui/stage.rs`,
`crates/web/public/stage.js` (loaded with `media.js`), `crates/web/styles/stage.css`.
Browser test: `crates/web/tests/stage.mjs`. Migration `202611231301_stage.sql`.

## Permissions

| Bit | Name | Default | Checked for |
|---|---|---|---|
| 52 | `MANAGE_EVENTS` | off | create, edit, start, end, cancel and delete any event; upload its cover |
| 53 | `CREATE_EVENTS` | off | create events, and manage the ones you created |
| 54 | `REQUEST_TO_SPEAK` | on for `@everyone` (migration for existing servers, a trigger for new ones) | raising your hand on a Stage (role and channel overwrites apply) |

A **Stage moderator** is anyone with Manage Channels, Mute Members and Move
Members on the Stage channel, as Discord defines it (owners and Administrators
have all three). No extra bit: the existing overwrites already express it.
Server Settings › Roles lists the new bits ("Request to Speak" under Voice &
Video, an "Events" group).

## 1. Stage channels

A new channel kind, `stage` (`ChannelKind::Stage`, appended). The migration
rebuilds `channels` exactly as the threads migration did (same columns, same
order, its index and the `thread_parent_deleted` trigger). A Stage behaves
like a voice channel everywhere else: its own text chat (read states, unread
counts, search, the age gate), its voice bitrate setting, Connect/View rules.

### Roles

- **Speakers** publish like anyone in a voice channel: microphone, camera,
  screen and stream sound, push to talk, per-person volume, devices, the
  soundboard. At most `MAX_STAGE_SPEAKERS` (16) people, and the SFU's 16
  publishing sessions per room.
- **The audience** listens only. Its lease has no Speak, Stream, Video, Voice
  Activity, Priority Speaker or Use Soundboard (`stage::scope`), and the SFU
  treats its session as a **listener** (`Authorizer::listener`): it refuses any
  offer that publishes a track (`forbidden`), drops anything it sends, and never
  lists it in `Joined` or `Participant`. Audience sessions are not in
  `GuildState`/`GuildSnapshot`/`Ready.voice_states`: the channel list counts
  them instead, so a thousand listeners cost the rest of the server nothing.
- **Moderators** start, edit and end the Stage, accept and dismiss raised
  hands, invite people to speak, move speakers back to the audience, and may
  move themselves onto the Stage. Server mute, deafen and disconnect work on a
  Stage as in voice channels; **moves into or out of a Stage are refused**
  (`invalid_request`): the Stage has its own way on and off.

### Lifecycle

- **Not live**: only moderators can join (as audience, "backstage"). Anyone
  else gets `forbidden` "This Stage hasn't started yet."
- **Start** (`StageSignal::Start { channel_id, topic, privacy_note, event_id }`,
  moderators): topic 1–120 characters, an optional note for listeners (≤200,
  "This Stage is recorded"). The starter is a speaker whether they joined
  before or after starting. A scheduled Stage event goes live with it (the one
  named, or the one due at that channel within 30 minutes of its start).
- **Audience joins muted with no microphone prompt**: the web app opens no
  capture, runs no audio processing and sends nothing upstream (verified in the
  browser test: zero `getUserMedia` calls, zero outbound RTP).
- **Request to Speak** raises a hand (`RaiseHand`, needs Request to Speak);
  moderators see the queue in order and **Accept** (the person becomes a speaker
  at once) or **Dismiss**. **Invite** asks someone who has not raised a hand;
  they must **accept** (`AnswerInvite`) before anything changes, and their
  microphone is opened only then. Invited and accepted speakers start muted.
- **To the audience**: a moderator moves a speaker, or a speaker moves
  themselves (`ToAudience`). Their captures stop and they are a listener again.
- **End** (moderators), or the last person leaving: everyone is disconnected,
  the Stage's chat gets a system message with `extra.stage.record =
  StageSummary { topic, started_at, ended_at, peak_listeners, speakers }`
  (and readable text for older clients), its event ends, and
  `StageEvent::Ended` tells the channel list.
- A speaker whose last device drops speaks again on rejoining within two
  minutes (a reconnect keeps the role).
- Start, topic changes and End are in the audit log (`stage_start`,
  `stage_update`, `stage_end`).

### Wire

`VoiceSignal::Stage(StageSignal)` (host-handled; the SFU rejects it) and
`VoiceEvent::Stage(StageEvent)`, in the voice envelope like calls.

| Event | To | Meaning |
|---|---|---|
| `Live(StageInstance)` | everyone who can see the channel | started or changed; counts are combined, at most once a second |
| `Ended { guild_id, channel_id }` | same | over |
| `Snapshot { stages }` | the connection, after Ready, Resumed and joining a server | every live Stage you can see |
| `Roster(StageRoster)` | a connection that joins the Stage (and again if it missed a change) | everyone on it, speakers first |
| `Members { channel_id, upserts, removed }` | every connection on the Stage | changes, combined over 400 ms |

```text
StageInstance { channel_id, guild_id, topic, privacy_note?, started_by, started_at, speakers, listeners, event_id? }
StageMember   { user_id, name, avatar_id?, role: speaker|audience, moderator, hand_raised_at?, invited }
```

`hand_raised_at` and `invited` are shown only to moderators and to the person
themselves: each change is encoded twice (moderators' view, everyone else's),
not once per listener. Signals: `Start`, `Update`, `End`, `RaiseHand`,
`Accept`, `Dismiss`, `Invite`, `AnswerInvite`, `ToAudience`, `BecomeSpeaker`;
rate 60/min, burst 10 per person. Raised hands: at most 250 per Stage.

Timing: one task (`stage::spawn`) sleeps until the next flush and is woken
only when something changed; with nothing pending it waits with no deadline.

### Capacity (SFU)

The audience is `MAX_GUILD_MEMBERS` (1000) listener sessions per Stage and in
all Stages together (`Config::max_listeners`, `max_room_listeners`), apart from
the 64/16 publishing caps. To keep each listener's cost bounded and nothing
quadratic:

- Each datagram is matched to its peer through a source-address index (at most
  4 addresses per peer) instead of asking every peer (`accepts`) in turn.
- `synchronize()` (looking for host-side room moves) runs only after a policy
  change, not on every packet.
- `allocate()` groups routes by viewer and room in one pass; a bitrate estimate
  for a peer that receives no video does not run it.
- Timers: only peers with something due are touched.
- Listeners' offers have their own room budget (`max_room_listeners` + 120 a
  minute), so an audience arriving at an event's start does not lock speakers
  out of renegotiating; each peer keeps its own 30-a-minute limit.
- The web audience negotiates four receive slots up front, so a new speaker is
  a `Subscribe`, never a renegotiation of the whole audience.
- **Video to an audience** is bounded by the server's video capacity: a
  listener may hold a video subscription only while every listener video route
  can get `LISTENER_VIDEO_BPS` (500 kbps) of `--voice-egress-bps` (64 seats at
  the default 32 Mbps); beyond that the subscription is refused (`limit_exceeded`)
  and audio is unaffected. Raise `--voice-egress-bps` for larger video audiences.
- When a listener leaves, its routes, address entries, budgets and caches are
  gone; the measurement checks that the SFU then holds only the speaker.

**Measured** (`crates/sfu/tests/stage.rs`, `stage_audience_capacity`, headless,
no sockets: 1000 real str0m receivers with full ICE/DTLS/SRTP, each like the web
audience — one recvonly audio m-line, no data channel; one speaker sending
Opus at 50 packets a second for 3 s of simulated time; SFU time is wall time
inside SFU calls on one thread, `--profile server`, this PC with other builds
running):

| Listeners | Heard | SFU time per second of audio | Per listener | SFU heap per listener |
|---|---|---|---|---|
| 300 | 300 | 41 ms (4.1% of one core) | 136 µs/s | ~104 KB |
| 1000 | 1000 | 192 ms (19% of one core) | 192 µs/s | ~105 KB |

Split at 1000: fan-out 116 ms, listener feedback (≈17,000 RTCP packets a
second) 46 ms, timers 30 ms. Before the changes above the same run took 811 ms
(81% of a core), mostly the per-packet scans. After every listener left: one
session, zero routes, 0.4 ms of SFU time per second. Each additional speaker
adds roughly the fan-out share again. Audio egress is the network's limit: 1000
listeners × 64 kbps ≈ 70 Mbps per speaker on the wire.

Reproduce: `$env:STICKTALK_STAGE_LISTENERS='1000'; cargo test --profile server
-p sticktalk-sfu --test stage stage_audience_capacity -- --nocapture` (the debug
default is 40 listeners, as a correctness test).

### Call extras on a Stage

- **Soundboard**: speakers only (listeners get `forbidden`).
- **Entrance sounds**: none on a Stage; hundreds of people arrive at once.
- **Clips**: speakers' controls include Save Clip as in voice; the audience
  controls do not offer clips (no capture runs for them).
- **Push to talk, per-person volume, devices, reconnect**: speakers use the
  voice channel's code unchanged (the microphone path is the same function).
  Per-person volume applies to every speaker tile for listeners too.

### Web app

- Channel list: a Stage icon; when live, a line under the row with LIVE, the
  topic and speaker/listener counts; a chat button. Clicking a live Stage joins
  as audience; a moderator clicking one that is not live gets **Start Stage**.
- The Stage view: topic, note and counts; Edit Topic and End Stage for
  moderators; an invitation card (Accept / Not Now); speakers' tiles (the call's
  tiles, with speaking rings and volume); the request queue for moderators; the
  audience as a **virtual list** (`stage.js`: only visible rows exist; a scroll
  redraws in an animation frame, nothing runs otherwise); controls reduced to
  the role (audience: Request to Speak, Deafen, devices, chat, Leave; speakers:
  the voice controls plus Move to Audience).
- The Stage's chat beside the Stage or on its own, with a banner to join.

## 2. Scheduled events

### Model and recurrence

One `scheduled_events` row per series. The rule is the start's **civil date
and time in the creator's IANA time zone** (`start_local` + `time_zone`) and a
recurrence: none, daily, weekly, every two weeks, monthly, yearly, weekdays
(Monday–Friday). Occurrence *n* is computed from the first one (never from the
previous), converted to UTC through the bundled IANA database (`jiff` 1.0,
`tzdb-bundle-always`), so "Mondays 19:00 in Berlin" stays 19:00 across DST and
monthly events on the 31st do not drift. Each occurrence is identified by its
rule time (`occurrence`), which edits never change. Times are stored in UTC and
shown in each viewer's time zone with the zone named (`Intl`).

- **Edit "this event"**: an exception row for that occurrence (name,
  description, start, end, location), at most 100 per series.
- **Edit "this and following"**: the series ends there (`until_at`) and a new
  series starts with the changes; interested members and the cover carry over.
  Editing from the first occurrence (or a one-off event) changes the series.
- **Cancel** this, this and following, or a one-off event.
- A live event keeps its time and rule until it ends.

### Status and the timer

`status`: scheduled, live, ended, cancelled. Each series keeps `next_at`, the
next time anything is due for it: its start announcement, an external event's
end, an unstarted occurrence lapsing (`EVENT_LAPSE_MS` after its end, or its
start), or its deletion. **One task** sleeps until the earliest `next_at` (an
indexed `MIN`), handles the due rows (100 at a time) and re-arms; commands that
set an earlier deadline wake it. With no events it waits with no deadline.

- At an occurrence's start, interested members get `ScheduleUpdate::Starting`
  in the app (no toast in Do Not Disturb) and a Web Push ("An event is
  starting", generic lock-screen text) on devices they are not using, unless in
  Do Not Disturb or the server is muted / set to Nothing for them.
- **External** events go live at their start and end at their end (an end
  time is required for them).
- **Stage and voice** events go live when the channel's session starts (a Stage
  starting, or the first person joining the voice channel, within 30 minutes
  before the start) or when a manager presses Start. They end when the channel
  empties after the end time (or at once if empty), when the Stage ends, by
  hand, or at most a week after going live. A recurring event then moves to
  its next occurrence.
- Ended and cancelled events are deleted `EVENT_RETENTION_MS` (7 days) after
  they end. A server holds at most 100 scheduled or live events (and 200 rows
  in all; the oldest ended one makes room).

### Wire

`Command::Schedule(ScheduleCommand)`, `CommandResult::Schedule(ScheduleResult)`,
`Event::Schedule(ScheduleUpdate)` (only to connections that sent a
`ScheduleCommand`; the web app sends `Subscribe` after every Ready).

| Command | Who | Notes |
|---|---|---|
| `Subscribe` | anyone | answers `Summary { guilds: [GuildEvents { guild_id, count, live }] }` |
| `Create(NewEvent)` | Create Events or Manage Events | name 1–100, description ≤1000, place ≤100; start ≥ now − 1 min and ≤ 5 years ahead; end after start, ≤ 7 days; a Stage/voice channel of this server you can see and connect to |
| `Update { event_id, occurrence, scope, patch }` | Manage Events, or the creator with Create Events | `scope`: `this` or `following` |
| `Cancel { event_id, occurrence, scope }` | same | |
| `Delete { event_id }` | same | |
| `SetInterested { event_id, interested }` | members who can see it | 60/min |
| `Start`, `End { event_id }` | same as Update | |
| `Share { event_id, channel_id }` | anyone who can see it and send messages in a channel of the same server | posts "Event: name" with `extra.stage.event = EventCard` |
| `CreateInvite { event_id, ttl_seconds, max_uses }` | Create Invite | an ordinary invite, also linked to the event |

Events: `Upsert(ScheduledEvent)` and per-user `Summary(GuildEvents)` to the
members who can see it (on create, edit, start, end, cancel, cover);
`Removed { guild_id, event_id }`; `Interest { event_id, interested }` to your own
devices; `Starting(ScheduledEvent)`. Marking Interested sends nothing to anyone
else (the count is read with the detail). Summaries are computed with one
batched audience per event, not one permission check per member and event.

An event at a channel is visible only to members who can see that channel.

### REST (cookie or bearer)

| Route | Answer |
|---|---|
| `GET /api/v1/guilds/{id}/events` | `EventPage { events, interested, can_create, can_manage }`: the current occurrence of each event, live first |
| `GET /api/v1/events/{id}?occurrence=` | `EventDetail { event, me_interested, can_manage, location_name?, upcoming }` (next five occurrences) |
| `GET /api/v1/events/{id}/interested?after=` | `InterestedPage { users, next }`, 50 a page |
| `GET /api/v1/events/{id}/calendar.ics` | the series as iCalendar: `DTSTART;TZID=<zone>`, `RRULE` (with `UNTIL`), `EXDATE` for cancelled occurrences, the event's link; `attachment`, `no-store` |
| `GET`/`PUT`/`DELETE /api/v1/events/{id}/cover` | the cover image (PNG, JPEG, WebP or GIF, ≤ 4 MB, 16–8192 px, sniffed from the bytes); `PUT`/`DELETE` need the event's manage right |
| `GET /api/v1/invites/{code}/event` | `InviteEvent { event: EventCard, status }` for a valid event invite |

Joining a server through an event invite marks the new member Interested.
Create, edit, cancel and delete are in the audit log (`event_create`,
`event_update`, `event_cancel`, `event_delete`; target kind `event`); an event
invite is logged as `invite_create` with the event named.

### Web app

- An **Events** entry at the top of the server's channel list, with the count
  and LIVE when one is live (shown to members who can create events even with
  none).
- The events list (Interested toggles), the event detail (cover, time range in
  your zone with its name, place, recurrence and its zone, description, Join
  when live, Interested, **Add to Calendar** (.ics download), **Copy Link**,
  **Invite** (an invite link that leads to the event), share to a channel,
  the next occurrences, interested members, and for managers Edit, Start
  Stage/Event, End, Cancel this / this and following, Delete, cover upload).
- The create/edit form: name, location (Stage, voice or somewhere else), start
  and end in your time zone, repeats, "this event / this and following", description.
- Event cards in chat (LIVE and Join when live), the Stage record card, and the
  event in the Join dialog's invite preview.
- An event's link, `/?event=<id>` (Copy Link, the start notification, the
  calendar entry), opens that event after sign-in.

## Decisions to revisit

- Stage moderator follows Discord (three existing bits) instead of a new bit.
- Moves into and out of Stages are refused rather than turning the moved
  person into audience.
- Invited and accepted speakers start muted; a moderator who starts the Stage
  or becomes a speaker by themselves starts unmuted.
- Ending a Stage disconnects everyone (Discord's behavior) rather than
  returning speakers to a still-open room.
- Interested is per series, not per occurrence.
- The audience cannot save clips (it runs no capture); speakers can.
- Create Events is off for `@everyone` by default.

## End-to-end encrypted media (later task)

Nothing here prevents it: the SFU still only forwards opaque RTP payloads for
speakers (no mixing, no transcoding), listeners only receive, and roles live in
the host's lease, so a later frame-level encryption (SFrame-style insertable
streams) can wrap speakers' payloads unchanged. Points to plan for: a Stage's
key must reach up to 1000 listeners and be re-keyed when speakers or the
audience change (the roster events give the membership); listeners never send
media, so they need only receive keys; the audio policer and speaking detection
read the RTP header extension (audio level), which stays in the clear.
