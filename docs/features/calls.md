# Calls: direct and group calls, voice channel chat, push to talk

Everything here is free for every user. Wire additions are additive: old
clients ignore the new variants and the new optional field.

## 1. Direct and group calls

A call lives in a DM or group DM. Its media is the ordinary voice path: the
client sends `VoiceSignal::Join { channel_id: <the DM's channel ID> }` and the
SFU room is that channel. Camera, screen share, mute, deafen, per-person volume
and device selection are the guild voice room's, unchanged. There is no second
media stack.

### Admission (server, `crates/server/src/voice.rs` + `calls.rs`)

- Join is allowed only for members of the conversation (`db::access`, otherwise
  `not_found`). In a 1:1 DM, either side having blocked the other gives
  `blocked`. In a group DM a blocked member is simply not rung.
- A DM member's rights in a call are fixed: View, Connect, Speak, Video, Stream,
  Use Voice Activity. Nobody can moderate (mute, deafen, move, disconnect)
  anybody in a DM call: `VoiceSignal::Moderate` on a DM session is `forbidden`.
- The per-room cap is the SFU's (16), the same as a group DM's member cap.
- Leaving the group (`LeaveGroupDm`, `RemoveGroupRecipient`) or blocking someone
  (`BlockUser`) re-runs the voice ACL reconcile: the member leaves the call at
  once, and a blocked 1:1 call ends for both.

### Ringing

- The first Join in a conversation starts its call and rings every other member
  (except blocked pairs) for `CALL_RING_TIMEOUT_MS` (45 s). A hidden 1:1 DM is
  shown again to the person called, as a message would.
- A ring stops for a user when they join (answer) on any device, when they send
  `VoiceSignal::DeclineCall { channel_id }` from any device, when the call ends
  (the caller leaving alone cancels it), or when its time runs out.
- Time-outs use one server task armed for the earliest ring deadline (a
  `tokio::sync::Notify` re-arms it). With no ring it waits with no deadline.
- `VoiceSignal::Ring { channel_id, user_ids }` rings members again. The sender
  must be in the call (`forbidden` otherwise); the call must exist (`not_found`);
  targets must be other members, unique, 1 to 16 (`invalid_request`). Members
  already in the call or in a blocked pair are skipped. Rate: 12/min, burst 4.
  `DeclineCall`: 60/min, burst 10.
- Recipients with no live connection get a Web Push notification ("Incoming
  call", urgency high, TTL 45 s, topic per user+conversation) unless they are in
  Do Not Disturb, disabled, or blocked with the caller.

### Wire

New `VoiceSignal` variants (appended): `Ring`, `DeclineCall`. Both are handled
by the host and rejected by the SFU itself.

New `VoiceEvent` variants (appended):

| Event | To | Meaning |
|---|---|---|
| `CallState(DmCall)` | every member of the conversation | full replacement |
| `CallEnded { channel_id }` | every member; or only the leaving connection while the call goes on | the call is over (for you) |
| `CallSnapshot { calls }` | the connection, after Ready and after Resumed | calls in your conversations |

```text
DmCall { channel_id, started_by, started_at, participants: [VoiceParticipant], ringing: [UserId] }
```

`participants` are sessions whose media connected. DM sessions never appear in
`Ready.voice_states` or `GuildState/GuildSnapshot`.

### The call record

When the last session leaves, the call ends. If anyone's media connected, a
system message is created in the conversation, authored by the person who
started it, with `content` a readable fallback ("Missed call", "Started a call
that lasted 12 minutes.") and the new optional field:

```text
Message.call: Option<CallSummary { started_at, ended_at, missed, participant_count }>
```

`missed` means nobody but the starter connected. It is unread for the others
like any message, and a missed call is pushed like a DM. The record cannot be
edited (`EditMessage` → `forbidden`); it can be deleted by its author. Storage:
`call_records(message_id → messages ON DELETE CASCADE, …)`, migration
`202611230301_dm_calls.sql`.

### Client

- DM and group DM headers: Start Voice Call, Start Video Call, or Join Call
  while one is in progress. The conversation shows "Call in progress" with who is
  in it and Join / Join with Video.
- Incoming call card (`role="alertdialog"`): caller, Answer, Answer with Video,
  Decline. Top-centre on desktop, a bottom sheet on phones.
- In a call: members not in it, each with Ring (or "Ringing…").
- Ring sounds (`public/calls.js`): an `AudioContext` exists only while ringing,
  with the pattern scheduled on the audio clock (no JS timers), and is closed when
  the ring stops. Incoming rings never sound in Do Not Disturb. A user setting
  (Voice & Video → "Play a ringtone for incoming calls") turns both off.
  Tests replace the sound with `globalThis.__sticktalkRingtone = (kind) => …`.

## 2. Text chat in voice channels

- `db::text` (the message gate) now accepts voice channels. Every message path
  checks the same bits on that voice channel: View Channel (else `not_found`),
  Send Messages, Read Message History, Attach Files, Manage Messages.
- Voice channels have read states and unread counts like text channels:
  created for members when the channel or the membership is created or becomes
  visible, backfilled by the migration, and filled in on Ready if missing.
- Client: a chat button on each voice channel row opens its chat without
  joining; that chat shows who is connected and Join Voice. In a call, Show Chat
  opens the chat beside the call (desktop) or as a sheet (phone). In a DM call it
  is the conversation's own chat.

## 3. Push to talk

Voice & Video settings, stored per device (`localStorage` `sticktalk-voice-input`):

- Input Mode: Voice Activity (default) or Push to Talk.
- Push to Talk Shortcut: any key or the middle / back / forward mouse button,
  recorded by pressing it. Release delay 0–2000 ms (default 200).
- Push to Mute Shortcut (optional, in either mode).

While the key is not held the microphone's RTP sender has no track
(`RTCRtpSender.replaceTrack(null)`): no packets at all are sent, not silence.
The capture stays open, so pressing is instant. A visible pill shows "Push to
Talk · F" / "Transmitting" / "Muted"; on phones it is a full-width hold-to-talk
button. Keys that type text are ignored while typing in a text field.

**Windows app, global hotkey.** In a call with a binding, the page asks the
app (IPC `{"ptt":"start","talk":{"usage":<HID>}|{"button":1|3|4},"mute":…}` /
`{"ptt":"stop"}`) to watch input while another app has focus. `crates/desktop/src/ptt.rs`
installs passive `WH_KEYBOARD_LL` / `WH_MOUSE_LL` hooks on a thread of its own:
no administrator rights, no window, no focus change, input is never swallowed,
injected input is ignored, and the hooks are removed when the call ends or the
page changes. The app tells the page with a `sticktalk-host` event.
Limitation: Windows does not show a normal process the input of an app running
elevated (as administrator); a game started that way does not trigger push to
talk while it has focus. StickTalk is not elevated to work around it.
