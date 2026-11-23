# End-to-end encrypted calls

Status: implemented for voice channels, direct calls and group calls (audio,
camera, screen share and stream sound). Stage channels are excluded. Wire and
route reference: `docs/features/e2ee.md`. Measurements: the end of this file.

This document is the design contract. Anything the code does that this file
does not describe is a bug in one of the two.

## 1. What changes

Before: every call was encrypted hop by hop (DTLS-SRTP between each browser and
the server's forwarding unit, the SFU). The SFU decrypts SRTP to route packets,
so the server could read and record every call.

After: each sender encrypts every audio and video **frame** in the browser,
before packetization, with a key the server never sees. The SFU forwards
ciphertext. SRTP still protects each hop, unchanged.

## 2. Threat model

### Who we defend against

The **server operator, or anyone who controls the server or the SFU** (a
compromised host, a malicious admin, someone on the box): they can read,
modify, drop, delay, reorder, replay and inject any packet and any signaling
message, invent sessions and devices, and lie about anything the server
reports (membership, policy, device keys, names).

Also: a **network attacker** between any two hops (already stopped by SRTP), and
a **former call member** who keeps their keys after leaving.

### What such a server cannot do (guarantees)

1. **Read media** of an end-to-end encrypted call: audio, camera, screen and
   stream sound frames are AES-GCM ciphertext under keys that only the devices
   in the call hold.
2. **Inject or alter media undetected.** Every frame is authenticated (AES-GCM,
   128-bit tag, with the codec headers that stay readable included as
   associated data). A frame that fails is dropped, never played.
3. **Replay** frames: a per-key counter window rejects repeats.
4. **Silently make a call unencrypted.** Clients decide the call's mode from
   the signed key announcements of its members. A call is shown as "not
   end-to-end encrypted" only when a visible member's app genuinely cannot
   encrypt *and* the server's policy allows it, and every member then sees a
   persistent banner saying so and naming who. In an encrypted call, plaintext
   frames are dropped. A per-device setting, "Only join end-to-end encrypted
   calls" (in the encryption panel), refuses that mode outright.
5. **Add an invisible listener.** A device can only receive keys if it is in
   the member list that every client shows. Each client lists every device
   holding keys, marks new and unverified devices, and shows a **call
   verification code** derived from the complete member list. If the server
   shows different members to different people, their codes differ.
6. **Impersonate a verified person** without a visible warning. A device's
   identity key is created on the device and its private half never leaves
   it. Pairwise verification pins that key; a different key for a verified
   person shows a warning and removes the verified mark until re-verified.
7. **Let someone who left keep listening.** When a member leaves or is
   removed, every remaining sender switches to a fresh random key that the
   departed member never receives. **Bound:** a sender keeps its previous key
   for at most 1 second after the departure while the others confirm receipt
   of the new one (otherwise everyone would hear a gap). During that second the
   departed member could decrypt only if the server, which has already stopped
   forwarding to them, keeps relaying media to them.
8. **Let a newcomer hear the past.** On a join every sender ratchets its key
   one-way (HKDF) and gives the newcomer only the new step. Earlier media stays
   unreadable to the newcomer even if the server recorded it.
9. **Decrypt recorded calls later** by compromising a device's long-term key.
   Group keys travel inside pairwise channels built from **per-call ephemeral**
   ECDH keys that exist only in memory and are discarded at the end of the call
   (forward secrecy). The long-term identity key only signs.

### What a malicious server can still do (honest limits)

- **It serves the web app's code.** StickTalk's web app and the Windows app's
  window are loaded from the server. A malicious server can ship JavaScript
  that leaks keys or plaintext, and no protocol running inside that code can
  prevent it. End-to-end encryption therefore protects against: an operator who
  records or inspects traffic and storage, a compromised SFU or database, a
  later compromise of the server, a subpoena of stored data, and network
  attackers. It does **not** protect against a server that is malicious at the
  moment it serves you the app. This is the same trust decision the owner made
  and documented for remote control (2026-11-21): the Windows app trusts the
  server's page. What would remove it: a signed, versioned web bundle verified
  by the Windows app before loading (the app already verifies ed25519-signed
  updates, see `docs/PROTOCOL.md` "Signed Windows update contract"), or a
  native client that does not load code from the server. Neither is built here.
- **Metadata stays visible** (section 7): who is in which call, when, from which
  device, for how long, packet sizes and timing, who is speaking and how loudly
  (the RTP audio-level extension), video resolution, frame types and rate.
- **Denial of service**: it can drop key messages or media, refuse joins or end
  calls. Clients show the resulting state; they never fall back to plaintext.
- **Unverified first contact**: if you never verify someone, a server that
  substitutes their key on first contact is not detected (trust on first use).
  It is detected when you compare the call code or verify the person.
- **A call member who colludes with the server** can, as with every deployed
  group-call design (Signal, WhatsApp, Discord's DAVE, Webex/MLS-SFrame), make
  frames that decrypt under another member's sender key, because group media
  keys are symmetric. StickTalk adds **sender signatures** (section 5.6) so this
  is *detected* within about one second, the stream is muted and everyone is
  warned. Preventing even the first forged frame from playing would need a
  public-key signature checked before every frame is played; at 50 audio
  frames per second per speaker that costs tens of kilobits per second and a
  verification per frame per listener, so it is not done.
- **Browser and OS compromise**: malware on a participant's device sees what
  that participant sees.

## 3. Building blocks (all WebCrypto, no new primitives)

| Purpose | Construction | Why |
|---|---|---|
| Device identity | ECDSA P-256, SHA-256; private key `extractable: false`, stored as a `CryptoKey` in IndexedDB | The one signature algorithm every target engine (Chromium/WebView2, Firefox, Safari) supports with non-extractable keys. Ed25519 is newer and not universal. |
| Per-call key agreement | ECDH P-256, one ephemeral non-extractable key pair per device per call, in worker memory only | Forward secrecy for distributed keys; standard curve, constant-time in browsers. |
| Per-call sender signatures | ECDSA P-256 ephemeral key, certified by the identity key in the announcement | Detects member-forged media (section 5.6) without using the long-term key on media. |
| Key derivation | HKDF-SHA-256 (RFC 5869) | Standard extract-and-expand. |
| Pairwise messages | AES-256-GCM, random 96-bit nonce, associated data names sender, recipient and call | Few messages per key, so random nonces are safe. |
| Frame encryption | **SFrame, RFC 9605**, cipher suite `AES_128_GCM_SHA256_128` (0x0004): its key schedule, header encoding, nonce construction and AAD rule, checked against the RFC's test vectors | An IETF standard designed for exactly this: per-frame AEAD with a key ID and counter. Full 128-bit tag: no truncated-tag analysis needed. |
| Verification codes | PBKDF2-HMAC-SHA-256, 10,000 iterations over a SHA-256 transcript, rendered as 30 decimal digits (6 groups of 5) | Iteration makes grinding a matching pair of fake keys (a birthday search on the code) cost far more than a call lasts. |

## 4. Keys and identities

- **Device**: one StickTalk session (one sign-in in one browser profile or one
  Windows app install). The server names it by `session_devices.id`.
- **Identity key**: created by the device the first time it joins an encrypted
  call, stored in IndexedDB (`sticktalk-e2ee`, store `identity`, keyed by the
  signed-in user ID, so the account switcher keeps one key per account). It is
  bound to the server session: the device sends the public key in its first
  announcement and the server stores it in `e2ee_device_keys`, whose row
  belongs to the session row (`ON DELETE CASCADE`). The server stores and
  serves **only public keys**.
- **Sign-out or session revocation** deletes the session, which deletes its key
  row and ends its calls. Others never receive that key again from an honest
  server. A device that signs in again is a **new device** with a new key: the
  stored key records the device ID it was created for, and a different device
  ID makes the browser create a new key (the old one belonged to a revoked
  session).
- **Persistence**: keys survive restarts (IndexedDB). Where IndexedDB is
  unavailable (some private windows) the key lives in memory only; the device
  then appears as a new device on every load and the encryption panel says so.
- **Fingerprint**: `SHA-256("StickTalk-E2EE-v1 fingerprint" || u64(user) ||
  identity public key)`, shown as 8 groups of 4 hex characters.

## 5. The call protocol

Notation: `||` concatenation; `u64(x)` 8 bytes big-endian; a session is
`(user_id, connection_id)`, which the server assigns freshly for every join.
`LABEL` = `"StickTalk-E2EE-v1 "`.

### 5.1 Join and announcement

1. The client's `Join` carries `capabilities.e2ee = 1` when the browser has
   WebRTC encoded transforms and WebCrypto. The server applies the call's
   policy (section 6): in a call that requires encryption, a client without it
   is refused with a clear message and never gets media.
2. The server answers the joining connection with `Welcome { channel_id,
   session, device_id, allow_unencrypted }`.
3. The client creates the call's ephemeral ECDH key `dh` and signing key `sig`
   in its worker and sends `Announce { identity, dh, sig, signature }` where
   `signature = ECDSA_identity(LABEL || "announce" || u64(channel) ||
   u64(user) || u64(connection) || device_id || identity || dh || sig)`.
   Public keys are 65-byte uncompressed points; the signature is 64 bytes.
4. The server checks the signature (ring), stores `identity` for this session
   if absent or replaces it (a replaced key is a key change everyone sees), and
   sends every member the full **roster**: each session with device ID, server
   reported device name, support flag and announcement.
5. Every client verifies every announcement itself. An invalid one is excluded
   (that device gets no keys) and reported in the panel.

Because the connection ID is fresh per join and is signed, an announcement
cannot be replayed into another call or another join.

### 5.2 Pairwise channels

For members `A < B` (ordered by session):

```
Z   = ECDH(dh_A, dh_B)                                   (32 bytes)
salt= SHA-256(announce_bytes_A || announce_bytes_B)
K_AB= HKDF(Z, salt, LABEL || "pairwise")                  (AES-256-GCM key)
msg = nonce(12) || AES-GCM(K_AB, nonce, AAD = LABEL || "msg" || from || to || u64(channel), body)
```

Bodies: `SenderKey { epoch: u32, step: u8, secret: 32 bytes }` and
`Ack { epoch, step }`. The server relays them opaquely to the named session
only, if both are in the same call (size ≤ 256 bytes, rate limited).

### 5.3 Sender keys, ratchet and rekey

Each device has its own **sender secret** `S` (32 random bytes) identified by
`(epoch, step)`; no global epoch is coordinated between devices.

- **Ratchet** (someone joins): `S' = HKDF(S, "", LABEL || "ratchet")`, step+1.
  The sender switches at once. Existing members compute `S'` themselves when a
  frame names the next step (at most 8 steps ahead), so a join costs them no
  message and no gap. Only the newcomer is sent `S'` in a `SenderKey`
  message. The newcomer cannot invert HKDF to reach `S`.
- **Rekey** (someone leaves, is removed, or an announcement becomes invalid,
  or the ratchet passes 63 steps): fresh random `S`, epoch+1, step 0, sent to
  every remaining member. The sender keeps encrypting with the previous key
  until every remaining member has acknowledged the new one, or for at most 1
  second, then switches. Receivers keep a superseded key for 2 seconds to
  decrypt frames still in flight, then erase it.
- **Join and leave in one roster change**: one rekey covers both.
- The newcomer creates its own fresh `S` and sends it to everyone.

**Cost.** A join costs `2(n-1)` pairwise messages, a leave `(n-1)(n-2)` plus as
many acknowledgments, each under 100 bytes. Pairwise keys are derived once per
pair per call. The SFU room cap is 16 sessions (configurable to 32), so the
worst leave is 210 messages, about 30 KB through the server. A tree-based group
protocol (MLS, RFC 9420) would make rekeying logarithmic, which matters at
hundreds of members, not at 16 to 32; it would also be a large new dependency.
Tested sizes: the protocol simulator runs 16 members with churn and 32 members
with a leave (`crates/web/tests/e2ee-protocol.mjs`); real browsers run 3
(`crates/web/tests/e2ee.mjs`).

### 5.4 Frame keys and format

```
base_key(kind) = HKDF(S, "", LABEL || "track " || kind)[0..16]    kind = audio|camera|screen|system_audio
KID            = epoch * 64 + step
sframe_key, sframe_salt = RFC 9605 derive_key_salt(KID, base_key)   (suite 0x0004)
nonce          = sframe_salt XOR CTR      (CTR counts frames per track and KID, from 0)
```

The SFrame header (config byte, KID, CTR; RFC 9605 section 4.3) is carried at
the **end** of the frame so the codec bytes that must stay readable can stay at
the front:

```
frame = body_with_ciphertext || tag(16) || header || ext || u16(len(header)+len(ext)) || 0xE2
AAD   = header || metadata
```

- **Audio (Opus, microphone and stream sound)**: the whole Opus packet is
  encrypted; metadata is empty.
- **Video (H.264)**: the frame is a list of NAL units with start codes. These
  stay readable and are authenticated through `metadata = for each NAL unit:
  u32(length) || readable bytes`:
  - start codes and every NAL unit header byte;
  - SPS (7) and PPS (8) parameter sets, AUD (9) and end-of-sequence/stream
    units, entirely;
  - for slices (1–5): the first three slice-header fields `first_mb_in_slice`,
    `slice_type`, `pic_parameter_set_id` (up to the byte holding the last bit).
  Everything else (all picture data, SEI payloads) is encrypted. The tag and
  trailer are appended to the last NAL unit. The sender rejects any result
  containing `00 00 01` or `00 00 00` in or after encrypted bytes (a start
  code would make the packetizer split there) and re-encrypts with the next
  counter; with random ciphertext this happens for about 1 in 170 frames of
  100 KB.
- **ext** carries at most one chunk of a sender signature batch (5.6).
- `0xE2` marks an encrypted frame; it only speeds up the plaintext check in
  unencrypted-fallback calls.

### 5.5 Receiving

The receiver knows which member and track kind the server *claims* a stream
belongs to, and decrypts with **that member's** key for that kind only. A
server that relabels Alice's stream as Bob's makes it fail to decrypt. Per
member and kind it keeps the current and at most one superseded key, and per
KID a window of the last 256 counters: older or repeated counters are dropped
(replay). Reordering inside the window is accepted. Each frame stands alone (no
chaining), so one lost packet costs at most the frame it belonged to, exactly
as without encryption. A video stream whose key arrived after its frames asks
the sender for a key frame once.

### 5.6 Sender signatures

Every 400 ms (or 32 frames) each sending track signs, with its call signing key
`sig`, the list of the frames it sent in that interval:

```
signed = LABEL || "batch" || u64(channel) || u64(user) || u64(connection) || u8(kind)
         || u32(KID) || u64(first CTR) || u8(n) || first 8 tag bytes of each frame (zeros for a skipped counter)
```

The signed batch (`u32 KID || u64 CTR || u8 n || prefixes || 64-byte
signature`) rides in `ext` of following frames, split into chunks of at most
96 bytes for audio. A receiver checks the signature against the sender's
announced `sig` key and the prefixes against the frames it received. A received
frame whose counter a valid batch covers with a different prefix, or not at all,
is **forged**: the stream is muted and everyone in the call is warned. Frames
that a later batch should have covered but none did count as unverified; a
sustained unverified share shows a softer warning (loss of a few batch chunks
is tolerated). Forging a matching prefix needs about 2^64 attempts per frame.

## 6. Policy, downgrade and mode

- Calls are end-to-end encrypted by default whenever every member supports it.
- Per server and per DM conversation, **Allow unencrypted calls** (off by
  default). Server: Manage Server. DM: either member; group DM: its owner.
- At join the server refuses a client without support unless that setting is
  on (message: the call is end-to-end encrypted and this app cannot join it).
- **Each client computes the mode** from the roster: *encrypted* if every member
  announced validly; *unencrypted* only if some member has no support **and**
  the roster says the setting is on. In that mode every member sees a banner
  "Not end-to-end encrypted: <name>'s app can't encrypt calls" and the
  indicator turns into an open lock. A roster that claims "unencrypted" while
  every member supports encryption is ignored (the call stays encrypted).
- **"Only join end-to-end encrypted calls"** (the encryption panel, per
  device): leave instead of entering the unencrypted mode.
- **Stage channels** are never end-to-end encrypted (one-to-many audiences of up
  to 1,000; Discord makes the same choice). The Stage view says so.

## 7. What the SFU still reads, and why

| Readable | Used by the SFU for | Note |
|---|---|---|
| RTP header (SSRC, sequence, timestamp, marker, payload type) | routing, rewriting, RTX/NACK, pacing, the frame-rate statistic | outside the frame |
| RTP header extensions: MID, RID, transport-wide sequence, abs-send-time | bundling, bandwidth estimation, fair sharing | outside the frame |
| RTP audio-level extension | speaking indicators, Stage | reveals who speaks and how loudly |
| H.264 NAL headers, SPS/PPS, slice type | key-frame detection and gating on new subscriptions and layer switches | the receiver's depacketizer also needs them; they describe resolution and frame type, not picture content |
| Packet and frame sizes, timing | pacing, bandwidth sharing, the audio policer | inherent to any SFU |

The SFU code is unchanged except: the host's lease adds the encryption
overhead (`E2EE_AUDIO_OVERHEAD_BPS`, 24 kbps) to the bitrate the SFU's
microphone policer allows for encrypted sessions, and a test-only build feature
(`media-tap`, never in a normal build) lets the browser suite capture forwarded
payloads and inject packet loss.

## 8. Features that touch media

| Feature | With encryption |
|---|---|
| Push to talk | unchanged: no track, no frames, no work |
| Camera backgrounds | applied before encoding, so before encryption |
| Quality controls, encoder caps, need-based bandwidth sharing | unchanged; bitrates now include the frame overhead (section 10) |
| Viewer-side statistics | unchanged (receiver `getStats`) |
| Clips | record the decrypted local tracks on the clipping device, as before; every member is still told when someone has clips on |
| Soundboard and entrance sounds | not call media: the server stores the sounds and relays who played which; each listener fetches and plays the file. Not end-to-end encrypted, and the panel says so |
| Remote control | the control data channel stays hop-by-hop (the SFU must check grants, sequence numbers, rates and bounds to fail closed); the screen it controls is encrypted |
| Activities / Watch Together | not call media; not covered |
| Text chat in voice channels and DMs | not covered (messages are not end-to-end encrypted) |

## 9. Verification

- **Call code**: `PBKDF2(SHA-256(LABEL || "call" || u64(channel) || every
  member's session, identity, dh, sig, sorted)), 10,000)` → 30 digits. Everyone
  in the call sees the same code if and only if they see the same devices and
  keys. Compare it aloud.
- **Pairwise**: for two devices, `PBKDF2` over both users' IDs and
  fingerprints, sorted → 30 digits, and a QR code containing
  `sticktalk-verify:1:<user>:<fingerprint>:<peer>:<peer fingerprint>` that the
  in-app scanner (BarcodeDetector or the vendored jsQR, as in QR sign-in)
  checks against the keys in the call. A match, or the user confirming the
  digits match, marks that device key verified.
- **Storage**: verified keys and seen keys are stored **per device** in
  IndexedDB and are not synced between a user's devices. A synced list would
  need the user's own devices to verify each other first and sign the list
  (the server could otherwise forge it); that is not built.
- **Key change**: if a person you verified appears with a key you did not
  verify (a new device or a substituted key), the verified mark is replaced by
  a warning in the tile, the device list and a banner, until you verify again.
  It is never accepted silently. A device never seen before for someone you
  have not verified shows "new device".

## 10. Engines

| Engine | Result |
|---|---|
| Windows app (WebView2), Chrome, Edge | supported; tested headless in Chromium (`RTCRtpScriptTransform`) with audio, camera and 1080p60 screen share |
| Firefox | Firefox 159 (Playwright, headless) has `RTCRtpScriptTransform` and no `createEncodedStreams`. **Tested**: a Chromium and a Firefox participant in one encrypted call (`tests/e2ee-firefox.mjs`) hear each other's tones through decryption, see the same call code, 0 frames failed authentication. Video was not run in Firefox: it needs the per-task OpenH264 plugin download described in `docs/QA_WEB_ENGINES.md`; the H.264 layout follows the libwebrtc packetizer Firefox also uses |
| Safari | `RTCRtpScriptTransform` exists in Safari 15.4+, but Windows WebKit has no WebRTC at all (`docs/QA_WEB_ENGINES.md`), so nothing could be tested; Safari clients that lack the API are refused like any unsupported client |

Detection: `RTCRtpScriptTransform` (preferred, runs in our worker) or
`RTCRtpSender.prototype.createEncodedStreams` (older Chromium; the streams are
transferred to the same worker), plus `crypto.subtle`. Without either the
client sends `e2ee` absent and is refused from encrypted calls.

## 11. Efficiency

Nothing runs outside a call: the module, its worker and its keys are loaded
when a call starts; the identity key is created once per device. No timers run
in idle calls beyond one 1-second deadline while a rekey waits for
acknowledgments. The worker is terminated when the call ends and every
ephemeral key and sender secret is dropped with it.

## Measurements

`crates/web/tests/e2ee.mjs`, 2026-11-23, this PC (other builds running at
times), two headless Chromium participants in one browser, real server and
SFU on loopback, a 1 kHz / 1.5 kHz tone as microphones, a moving 1920×1080
canvas as the screen. "Unencrypted" is the same call with both clients lacking
encoded transforms in a server that allows that: no transform runs at all, the
pre-change code path. One run each; CPU is renderer + GPU + utility process CPU
divided by two participants (% of one core).

| | Unencrypted | Encrypted |
|---|---:|---:|
| Time from Join to hearing the other person | 593 ms | 624 ms |
| Added one-way latency (time in the sender's + receiver's transform, p50 / p95), audio | 0 | 0.0–0.1 / 0.1 + 0.1 ms |
| Same, 1080p60 screen frames | 0 | 0.1 / 0.2 + 0.1 / 0.2 ms |
| CPU per participant, voice only | 3.87% | 3.58% |
| CPU per participant, 1080p60 screen share running | 48.8% | 50.0% |
| Microphone send bitrate (64 kbps channel) | 64.3 kbps | 78.2 kbps (+13.9: trailer 21–24 B/packet, signatures) |
| Screen share send bitrate (encoder-capped) | 6.53 Mbps | 6.27 Mbps (overhead below 0.5%) |
| Viewer frame rate, 1080p60 | 59.7 fps | 56.5 fps |
| Audio concealed during a third person's join and leave (bob's stream at alice) | — | 0 samples, 0 frames lost |
| Audio concealed with 5% injected loss | 4.66% | 4.76% |
| H.264 re-encryptions for start-code emulation | — | 1 in about 570 frames |

Notes, stated plainly:

- The frame-rate difference (59.7 vs 56.5) is within this headless, software-
  encoded run's variation; it was not repeated enough to separate the two.
- Under 5% loss both screen shares collapsed to under 1 fps: the test tap drops
  packets *before* the SFU's retransmission cache, so NACK cannot repair them.
  That is a property of the test, not of encryption (the unencrypted run
  collapsed the same way); audio, which has no retransmission, shows the
  comparison that matters. One encrypted frame under loss failed
  authentication and was dropped.
- What the SFU sees: 100% of forwarded audio payloads are encrypted frames;
  decoding the SFU's copy as Opus yields 0.03% of the speaker's tone (control,
  unencrypted call: 99.4%); decoding its copy of the screen share yields 0
  frames and decoder errors (control: 152 of 152 frames).
- Rekeying: the newcomer decrypted both others 1.27 s after clicking Join
  (mostly ICE/DTLS setup); existing members lost no frame and concealed no
  audio across the join and the leave.
- A user not in a call: no module, worker, key or timer (checked: no `e2ee`
  resource is loaded before the first call; the worker is terminated on
  leave). The web app's first-load bytes are unchanged (1,505,477 after,
  1,505,764 before; the modules are lazy and outside the app-shell cache).
