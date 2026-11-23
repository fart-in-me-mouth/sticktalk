# Rich message content (task `content`)

Polls, forwarding, voice messages, attachment descriptions and spoilers, link
previews, GIF favorites and categories, scheduled messages, and ten attachments
per message. Everything is free for every user.

Code: `crates/proto/src/content.rs` (wire), `crates/server/src/content/`
(server), `crates/web/src/live/content.rs` and `live/ui/content.rs` (web),
`crates/web/public/content.js` (recording and playback, loaded on first use),
`crates/web/styles/content.css`. Migration `202611230401_content.sql`.

## Wire protocol (additive)

- `Message.extra` (`MessageExtra`, omitted when empty): `poll`, `forwarded`
  (snapshot), `embeds` (link preview cards), `gif` (provider ID of a GIF sent
  from the picker).
- `Attachment.meta` (`AttachmentMeta`, omitted when empty): `description` (alt
  text, at most 1024 characters), `spoiler`, `duration_ms` and `waveform`
  (voice messages: up to 256 amplitude bytes computed once while recording).
- `Command::Content(ContentCommand)`, `CommandResult::Content(ContentResult)`.
- `Event::Content(ContentEvent)`: sent **only** to connections that sent
  `ContentCommand::Subscribe` (the web app sends it after every Ready). Older
  clients never receive a variant they cannot decode.

| Command | Who | Notes |
|---|---|---|
| `Subscribe` | anyone | opts this connection in to `Event::Content` |
| `SendMessage { draft, attachments }` | sender | the normal send path, plus details for the sender's own unsent uploads in the draft. A voice message is one Ogg/WebM/MP4 audio upload with no text. |
| `CreatePoll(PollDraft)` | Send Messages | question ≤ 300 chars, 2–10 answers ≤ 55 chars each with optional emoji, single or multiple choice, duration 1/4/8/24/72/168/336 hours. The question is the message text (search, notifications, replies and older apps show it). |
| `VotePoll { answers }` | Read History | replaces your vote; empty removes it; open poll, valid indexes, one answer unless multiple choice |
| `EndPoll` | author, or Manage Messages | |
| `SendGif { gif_id, url }` | Send Messages | `klipy:` ID and a `https://static{,1,2}.klipy.com/` URL |
| `Forward { channel_id, message_id, destinations, comment }` | Read History on the source; Send Messages (and Attach Files for files) in each of 1–5 destinations; DM block and slowmode apply | snapshot of the text (mentions written out as `@name`) and attachments; files are the same stored uploads, referenced again |
| `RemoveEmbeds` | author, or Manage Messages | removes the cards and stops them coming back |
| `ScheduleMessage { send_at }`, `UpdateScheduled`, `CancelScheduled`, `SendScheduledNow` | author only | text (with mentions), up to 30 days ahead, at most 50 per person |

Events: `PollUpdated { poll, voter }` (tallies; `voter` names whose vote
changed, so each client keeps its own `my_votes`), `EmbedsUpdated`,
`ScheduledUpserted` and `ScheduledRemoved { sent }` (author's sessions only).

REST (cookie or bearer auth):
- `GET /api/v1/channels/{c}/messages/{m}/poll/answers/{i}/voters?after=` →
  `PollVotersPage` (50 a page; readers of the channel only).
- `GET /api/v1/channels/{c}/messages/{m}/embeds/{i}/image` → the stored preview
  image (readers only; `private, max-age=86400`, `nosniff`, sandbox CSP).
- `GET /api/v1/users/@me/scheduled-messages[?channel_id=]` → your list.
- GIF favorites and categories use the existing `/api/v1/gifs/*` routes.

`MAX_MESSAGE_ATTACHMENTS` is now 10.

## Behavior

- **Timers.** One task sleeps until the earliest open poll deadline or
  scheduled message and is woken when either changes; it does no work while
  there is nothing pending. Both survive restarts (they are in the database).
- **Polls** close at their deadline (`closed_at = expires_at`) and everyone who
  can see the channel gets the final tallies; the winner is the answer (or
  answers) with the most votes.
- **Scheduled delivery** goes through the ordinary `SendMessage` path as the
  author, so permissions, slowmode and moderation checks apply at that moment.
  If it is refused, the message stays in the list with the reason (`failure`),
  the author is told, and it is not retried until rescheduled.
- **Forwarded source link**: present only in copies delivered to (and loaded
  by) people who can read the source and while the source exists. Downloads of
  a forwarded file are allowed to readers of any conversation holding a copy;
  file garbage collection keeps a file while any copy references it.
- **Link previews**: started after a message is sent or edited, for up to 5
  `http(s)` links. `<https://...>`, links in code and in spoilers are skipped.
  Fetching: http/https only, no credentials, at most 4 redirects; every hop's
  host is resolved by the server's own resolver and refused unless every
  address is public (loopback, private, link-local, CGNAT, multicast,
  documentation and other reserved IPv4/IPv6 ranges, NAT64/6to4-embedded
  private IPv4, reserved names such as `.local`, `.internal`, `.test`); the
  client connects only to the checked addresses (no DNS rebinding); IP-literal
  hosts are checked directly; 3 s connect and 8 s total timeouts; 512 KiB page
  cap (head only), 5 MiB image cap, content-type checks; 4 fetches at once and
  64 waiting; a 256-entry/16 MiB cache for 30 minutes; 10 previews a minute per
  author. The preview image is downloaded once, re-encoded to at most 480 px
  (≤ 256 KiB) and served by this server. `--link-previews false`
  (`STICKTALK_LINK_PREVIEWS=false`) turns all outbound fetches off.
  `--mock-link-preview-host` is a hidden, loopback-only test switch.
- **Voice messages** are recorded with `MediaRecorder` (Opus in Ogg, or WebM
  where Ogg is not offered), up to 20 minutes. Desktop: click the microphone to
  start, then Send or Cancel. Touch: hold to record, release to send, slide
  left to cancel, slide up to lock. The microphone, AudioContext and sampling
  timer exist only while recording; playback uses one `<audio>` element and its
  events (no timers or animation loops).
- **Web settings**: Appearance › "Show website previews for links" (stored on
  the device). Spoilers stay covered until clicked; alt text is the image's
  `alt` and shows under it with the ALT button.

## Changes to existing behavior

- Upload burst: `upload:` rate limit is still 20 a minute, now bursting to 20
  (two sets of ten attachments) instead of 4.
- Thumbnail downloads (`/files/{id}/thumbnail`, at most 64 KiB) no longer
  count toward the three-concurrent-transfers cap, so a grid of ten loads.
- Uploads that are Ogg Opus are stored as `audio/ogg` (were
  `application/octet-stream`).
- Message deletion also removes the message's poll, forwarded snapshot
  references and preview cards.
- Markdown renders `<https://...>` as a plain link without the brackets.
- `tests/ui-kit.mjs` selects the open menu (`ui-menu-content[data-open]`): the
  composer now has a second menu.

## Not included

- Scheduled messages carry text and mentions, not attachments.
- The moderation task's AutoMod/timeout checks run for anything sent through
  `SendMessage` (normal sends, polls, voice messages, GIFs, scheduled
  deliveries). Forwarding writes its own message row and does not call them;
  the lead should add the moderation call to `content::forward::destination`.
