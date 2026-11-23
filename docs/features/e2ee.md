# End-to-end encrypted calls (`e2ee`)

Voice channels, direct calls and group calls are end-to-end encrypted: audio,
camera, screen share and stream sound. Stage channels are not. The design,
threat model and limits are in [`docs/E2EE.md`](../E2EE.md); this file is the
wire, route and code reference. Everything is free for every user.

## Code

| Where | What |
|---|---|
| `crates/proto/src/e2ee.rs` | wire types (`E2eeSignal`, `E2eeEvent`, `E2eeAnnounce`, `E2eeMember`, `E2eeSettings`), limits |
| `crates/server/src/e2ee.rs`, `e2ee/tests.rs` | policy, announcement checks, the device key bound to the session, roster, relay, settings routes |
| `crates/server/migrations/202611232001_e2ee.sql` | `e2ee_device_keys`, `e2ee_guild_settings`, `e2ee_dm_settings` |
| `crates/sfu/src/tap.rs` | test-only (`media-tap` feature): capture forwarded payloads, inject loss |
| `crates/web/src/live/e2ee.rs` | can this browser encrypt; `Hello` before `Join` |
| `crates/web/public/e2ee-core.js` | the protocol: SFrame (RFC 9605) frames, H.264 layout, key schedule, ratchet and rekey, sender signatures, codes |
| `crates/web/public/e2ee-worker.js` | the call's worker: keys and every frame transform |
| `crates/web/public/e2ee.js` | identity key (IndexedDB), worker wiring, the panel, verification, warnings |
| `crates/web/public/e2ee-qr.js` | QR codes drawn on the device (byte mode, level M, versions 1–10) |
| `crates/web/styles/e2ee.css` | its styles (theme tokens only; the QR code is fixed dark-on-light artwork) |
| `crates/web/tests/e2ee-protocol.mjs` | protocol tests in Node: vectors, 16/32 members, adversarial relay |
| `crates/web/tests/e2ee.mjs` | browser suite with real WebRTC; needs a server built with `--features media-tap` |
| `crates/web/tests/e2ee-firefox.mjs` | a Chromium and a Firefox participant in one encrypted call (audio); needs `STICKTALK_FIREFOX_EXE` (a per-task Playwright Firefox) |
| `crates/web/tests/fixtures/e2ee-vectors.json` | frame vectors, also decrypted by an independent Rust implementation in the server tests |

Hooks in shared files: `VoiceSignal::E2ee` / `VoiceEvent::E2ee` (appended
variants), `voice.rs` (admission, `joined`, `left`, `moved`, the lease's
encryption flag), `state.rs` (`Core.e2ee`), `http.rs` (routes), the SFU's
rejection list, `media.js` (load on Welcome, transforms on each sender and
receiver, `pcConfig`, stop), `operations.rs` (`Hello`), `ui/stage.rs` (the
"not encrypted" note), `styles/app.css`, `scripts/version-runtime.mjs` and
`scripts/finalize.mjs` (the four modules are versioned with `media.js` and
kept out of the app-shell cache), `authsec/export.rs` (the device keys are in
"Request my data").

## Wire (all additive)

`VoiceSignal::E2ee(E2eeSignal)`, host-handled (the SFU refuses it):

| Signal | Answer | Notes |
|---|---|---|
| `Hello { version: 1 }` | none (errors only) | sent by the web app right before `Join` when it can encrypt; remembered per connection |
| `Announce(E2eeAnnounce { identity, dh, sig, signature })` | `Ack` | once per voice session (`conflict` after); 65-byte P-256 points and a 64-byte signature as base64url; the server checks the signature over the bytes in `docs/E2EE.md` 5.1 and stores `identity` for the session |
| `Relay { to: VoiceSession, data }` | none (errors only) | `data` ≤ 342 base64url characters (256 bytes); only between announced sessions of the same call |

`VoiceEvent::E2ee(E2eeEvent)`, only to sessions in that call:

| Event | To | Notes |
|---|---|---|
| `Welcome { channel_id, guild_id, session, device_id, allow_unencrypted }` | the joining connection (and a moved one, with the new channel) | `guild_id` 0 for DMs |
| `Roster { channel_id, allow_unencrypted, members: [E2eeMember] }` | every session in the call, on every join, announcement, leave, move or setting change | full replacement; `E2eeMember { session, device_id, device_name, supported, announce }` |
| `Relay { from, data }` | the named session | `from` is set by the server |

Old clients: they never send `Hello`, so they are refused from calls that
require encryption (`unsupported_feature`, with a message saying why) and, in
calls that allow it, join without encryption and everyone is told. Unknown
`VoiceEvent` variants fail to decode in old clients and are dropped.

## Routes

| Route | Who | Answer |
|---|---|---|
| `GET /api/v1/guilds/{id}/e2ee` | members (others `not_found`) | `E2eeSettings { allow_unencrypted, can_manage }` |
| `PUT /api/v1/guilds/{id}/e2ee` `{allow_unencrypted}` | Manage Server (`forbidden` otherwise) | audit log: `guild_update`, change `allow_unencrypted_calls` |
| `GET /api/v1/channels/{id}/e2ee` | members of that DM / group | as above |
| `PUT /api/v1/channels/{id}/e2ee` `{allow_unencrypted}` | 1:1: either person; group: its owner | `invalid_request` for a server channel |

A change reaches live calls at once (new roster); turning it off removes
clients that cannot encrypt from those calls, with the refusal message.

## Limits and rates

`Hello` 60/min (burst 10) per user; at most 1,024 remembered connections
(entries without a live connection are pruned first). `Announce` 30/min
(burst 10) per user. `Relay` 1,200/min (burst 256) per voice session.
The SFU's microphone policer allows the call's bitrate plus 24 kbps
(`E2EE_AUDIO_OVERHEAD_BPS`) for encrypted sessions. Room size is the SFU's
(16, configurable to 32).

## Permissions

| Action | Check |
|---|---|
| join an encrypted call without support | refused unless the server's or conversation's setting allows it |
| announce, relay | the connection's own voice session; relay target in the same call and announced |
| read the setting | server member / conversation member |
| change the setting | Manage Server / 1:1 member / group owner |

## Behavior summary

- The Windows app (WebView2), Chrome and Edge: `RTCRtpScriptTransform` in a
  worker. Older Chromium: encoded streams moved to the same worker.
- The lock in the call opens the panel: state, call verification code, every
  device holding keys (with verified, new, changed, can't-encrypt and invalid
  marks), "Only join end-to-end encrypted calls (this device)", and for
  managers "Allow unencrypted calls".
- Verify: a 30-digit safety number and a QR code; Scan Their Code uses the
  camera (BarcodeDetector or the vendored jsQR). Stored on this device only.
- Warnings in the call: a verified person's key changed, an invalid device, a
  forged or unverifiable stream, the server altering your own announcement, and
  people who cannot hear an encrypted call because their app can't encrypt.
- Stage view: "Stage channels aren't end-to-end encrypted."

## Running the suites

```powershell
$env:CARGO_PROFILE_RELEASE_PANIC = 'unwind'
cargo build --profile server -p sticktalk-server --features media-tap   # test-only tap
$env:STICKTALK_SERVER_EXE = "$env:CARGO_TARGET_DIR\server\sticktalk-server.exe"
node crates/web/tests/e2ee-protocol.mjs
node crates/web/tests/e2ee.mjs        # STICKTALK_E2EE_SKIP_MEASURE=1 skips the four measurement calls
node crates/web/tests/e2ee-firefox.mjs
```

Screenshots go to `STICKTALK_E2EE_SHOTS` (call with the lock, the panel, the
manager's panel, verification and scanning, key change, refusal, unencrypted
fallback on both sides), at 1440×900 and 390×844.

## Measurements

See the end of `docs/E2EE.md`.
