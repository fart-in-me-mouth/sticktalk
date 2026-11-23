# Call extras: soundboard, entrance sounds, camera backgrounds, quality, clips

Everything here is free for every user and server. Wire additions are additive:
old clients ignore the new `VoiceEvent` variants (they fail to decode and are
dropped), and the new REST routes are new paths. Nothing here does any work for
someone who is not using it: no timer, no audio graph, no model, no worker.

Code: `crates/proto/src/callextras.rs`, `crates/server/src/callextras.rs` (+
`callextras/audio.rs`, `callextras/backgrounds.rs`, tests in
`callextras/tests.rs`), `crates/sfu/src/quality.rs`, `crates/web/public/`
`callextras.js` (loaded with `media.js`), `soundboard.js`, `backgrounds.js`,
`backgrounds-worker.js`, `clips.js` (loaded on first use),
`crates/web/src/live/ui/call_extras.rs`, `crates/web/styles/callextras.css`.
Browser test: `crates/web/tests/callextras.mjs`.

## Permissions

| Bit | Name | Default | Checked for |
|---|---|---|---|
| 40 | `USE_SOUNDBOARD` | on for `@everyone` (migration sets it on existing servers; a trigger on new ones) | `PlaySound` and entrance sounds in a guild voice channel (role and channel overwrites apply) |
| 41 | `MANAGE_SOUNDBOARD` | off | upload, rename, change, delete a server's sounds (Manage Server also allows it) |

High bits keep clear of other engineers' additions (moderation uses 29 and 30).
Direct and group calls allow the soundboard for every participant. A
moderator's server mute or deafen also blocks the soundboard (`forbidden`), so a
mute cannot be bypassed by playing sounds. Playing a sound needs membership of
the sound's server; otherwise it does not exist for you (`not_found`).

## 1. Soundboard

### Delivery: a signalled event, played locally by every listener

`VoiceSignal::PlaySound { sound_id }` goes to the host (never the SFU, which
rejects it). After its checks, the host sends one
`VoiceEvent::SoundPlayed { channel_id, sound, by, entrance }` to every connection
in that call (the player included). Each listener fetches the clip once (then
from a bounded in-memory cache: 24 clips / 48 MB of decoded samples per call),
decodes it with `decodeAudioData`, and plays it through an `AudioContext` routed
to the call's output device (`setSinkId`), at the sound's volume times their own
soundboard volume.

Why not mix sounds into the player's microphone:

- **Sound quality.** A clip mixed into a mic is re-encoded at voice bitrate,
  through echo cancellation, noise suppression and automatic gain control, which
  treat music and effects as noise. Played locally it is the original file,
  full band, in stereo, once.
- **Listener control.** Each person sets their own soundboard volume, can mute
  the soundboard or entrance sounds, and hears nothing while deafened. A mixed
  sound could not be separated from the player's voice.
- **No waste.** No extra media stream, no SFU work, no bandwidth beyond fetching
  a ≤512 KB clip once; the player's microphone path is not touched at all (no
  regression risk for voice).
- The audio context exists only after a sound plays in the call, is suspended
  as soon as nothing is playing (no audio thread work), and is closed when the
  call ends. At most four sounds play at once per listener.

Trade-off: listeners start a sound when the event reaches them, so starts can
differ by a network round trip (tens of milliseconds); the first play of a clip
also waits for its fetch.

### Limits and validation (server)

- 48 sounds per server (`MAX_GUILD_SOUNDS`); 512 KB per file; at most 5.2 s.
- MP3 (MPEG-1/2/2.5 layer III), Ogg (Opus or Vorbis, one stream, every page
  CRC checked) or WAV (PCM or float). The **whole file** is checked and the
  duration is measured from the stream itself (frames, granule positions, data
  size over byte rate; LAME delay/padding and Opus pre-skip excluded), never a
  header claim. Trailing bytes, lying sizes and other formats are refused.
  `callextras/audio.rs`, including a no-panic sweep over arbitrary bytes.
- Names 1–32 characters, no control characters; emoji ≤16 characters, no ASCII
  letters, digits or markup; default volume 0–100.
- Rate limits: plays 20/min burst 4 per user and 60/min burst 8 per call;
  uploads 20/min; edits 60/min; favorites 120/min (≤100 favorites).

### REST (IDs are decimal strings in these JSON bodies)

| Route | Who | Does |
|---|---|---|
| `GET /api/v1/soundboard?channel_id=` | any user | `SoundboardLibrary`: the call's server first, then your other servers; `can_play` (Use Soundboard in that channel; always true in a DM call); `can_manage` per server; your favorites |
| `POST /api/v1/guilds/{id}/soundboard?name=&emoji=&volume=` (body: the file) | Manage Soundboard / Manage Server | adds a sound |
| `PATCH /api/v1/soundboard/sounds/{id}` `SoundPatch` | same | rename, emoji (`""` removes), volume |
| `DELETE /api/v1/soundboard/sounds/{id}` | same | deletes it (favorites and entrance choices with it) |
| `GET /api/v1/soundboard/sounds/{id}/audio` | members of its server, and anyone in a call with a member (who may have played it there) | the clip, `no-store`, `nosniff`, `attachment` |
| `PUT`/`DELETE /api/v1/soundboard/favorites/{id}` | members of its server | favorite |

### Client

A **Soundboard** button in the call controls (in **More** on phones) opens the
panel: search, favorites, the call's server, then the other servers; managers
get Add Sound and Edit. The listener's soundboard volume, Mute Soundboard and
Hear Entrance Sounds are per device (`localStorage` `sticktalk-call-extras`) and
also in Voice & Video settings. A toast says who played what; their tile shows
the sound's emoji while it plays. Tests replace the speaker with
`globalThis.__sticktalkSoundboard = (sound) => …`.

## 2. Entrance sounds

`GET`/`PUT /api/v1/soundboard/entrance` `EntranceSettings { enabled,
default_sound_id, overrides: [{ guild_id, sound_id | null }] }` (null means
"none in this server"). Only sounds of your own servers and servers you are in.
When a session's media connects (the SFU's `Joined`), the host picks the
server's override or the default, rechecks everything `PlaySound` checks, and
sends `SoundPlayed { entrance: true }` to the call. One entrance sound per user a
minute (`ENTRANCE_SOUND_COOLDOWN_MS`), however often they rejoin, plus the
call's sound rate. Listeners can turn off hearing entrance sounds.

## 3. Camera backgrounds

None, light or strong blur, seven built-in scenes (Dusk, Ocean, Aurora, Studio,
Forest, Night Grid, Paper: drawn by `backgrounds.js` at runtime, original, no
image files), and up to ten of your own: PNG, JPEG, WebP or GIF (a GIF shows its
first frame), or an MP4/WebM loop, each ≤10 MB.

- **Segmentation runs on the device**, in a module worker
  (`backgrounds-worker.js`) with MediaPipe Tasks Vision `ImageSegmenter` and the
  selfie segmenter (landscape, 144×256) model, GPU delegate with CPU fallback.
  Both Apache-2.0, vendored: the model is in `crates/web/assets/models/`
  (with `NOTICE.md`), the runtime is npm `@mediapipe/tasks-vision@1.2.3`, pinned
  exactly and copied by `scripts/vendor.mjs` to `/public/vendor/mediapipe/` at
  build time. No CDN or third-party request at runtime; CSP unchanged for it
  (`script-src 'self' 'wasm-unsafe-eval'`, `worker-src 'self'`). Not in the
  service worker's app-shell cache.
- **Lazy, and released**: the worker, model and WebAssembly load only when an
  effect is turned on, and the worker is closed (model released) when it is
  turned off, the camera stops or the call ends.
- **Pipeline**: in Chromium (and the Windows app),
  `MediaStreamTrackProcessor` → worker → `MediaStreamTrackGenerator`, so no
  frame touches the page's main thread. Elsewhere, frames go to the worker as
  `ImageBitmap`s and come back to a canvas `captureStream`. The mask is
  temporally smoothed and feathered; blur pads the frame so edges do not darken.
- **Holding the frame rate**: the worker measures its per-frame cost against the
  camera's frame interval. Over budget, it segments every second, then third
  frame (reusing the mask), then composes at 75% / 50% resolution; it steps back
  up when there is room. The user is told ("Your device is busy, so…").
- Changing the effect during a call replaces the published track
  (`replaceTrack`), without renegotiation.
- Custom backgrounds: `GET`/`POST /api/v1/users/@me/camera-backgrounds`,
  `GET`/`DELETE …/{id}`. Owner only (others get `not_found`); type from the
  bytes (magic numbers; still images also header-parsed, 16–8192 px); 10 per
  account (`limit_exceeded`); served `no-store`, `nosniff`, sandbox CSP.

## 4. Stream and call quality

### Video

- Screen share: **nothing to choose** (owner decision 2026-11-22, see
  `docs/PHASE2_REQUIREMENTS.md`). Always 30 fps at the source's own size (up to
  3840×2160), `contentHint: motion`, `degradationPreference:
  maintain-framerate`, capped only by the SFU's allowance (`media.js`:
  `displayVideo`, `streamEncoding`). The resolution, frame rate and
  "Optimize For" pickers this task first shipped were removed when the branches
  were merged; a shared screen sends no `TrackQuality`. The 1080p60 rows in the
  measurements below are from before that.
- Camera: 480p–1440p, 15/30/60 fps, default 720p30. Remembered per device; in
  Voice & Video and More → Camera Quality, applied live.
- Camera encoder: `maxBitrate` from the choice (H.264 at 30 fps: 1.2, 2.5, 4.5, 8,
  14 Mbps for 480p…4K; ×1.6 at 60 fps, ×0.6 at 15 fps), `maxFramerate`, and the
  degradation preference. Chromium's own default caps video near 2.5 Mbps; this
  is what lets 1080p60 and 4K60 actually be sent. The SFU's per-viewer allowance
  (`VoiceEvent::Bitrate`) still lowers it when the network needs that.
- **SFU allocation** (`crates/sfu/src/quality.rs`): a publisher declares what a
  track needs with `VoiceSignal::TrackQuality { mid, max_bps }` (100 kbps to
  64 Mbps, only for its own published video MIDs; cleared when the track is
  withdrawn). The global, room and per-viewer budgets are now split max-min
  fairly (water-filling): a 2.5 Mbps camera no longer holds back half a
  viewer's capacity from a 4K60 share. With no declared needs the split is
  exactly the old equal split (the existing SFU suites pass unchanged).
  Deployment note: the default global video egress is still 32 Mbps
  (`--voice-egress-bps`); 4K60 to several viewers at once needs more.
- **Viewers see what is delivered**: each remote stream/camera tile shows the
  received resolution and frame rate (`inbound-rtp`), and your own shows what
  you send, the bitrate, and "limited by the connection / your device" when the
  encoder adapts (`qualityLimitationReason`). Read from the `getStats` the call
  already takes for speaking levels, once a second; no new timer.

### Voice

- A guild voice channel has a bitrate, 8–384 kbps (default **64 kbps**, twice
  Chromium's 32 kbps Opus default that calls used before). Direct and group calls
  always use **128 kbps**. Edit Channel → Voice Quality (Manage Channels) sets it:
  `GET`/`PUT /api/v1/channels/{id}/voice-quality` `{ bitrate }`.
- On join, and to everyone in the channel when it changes, the host sends
  `VoiceEvent::AudioQuality { channel_id, bitrate }`. The client sets the Opus
  sender's `maxBitrate` (priority high) and adds `maxaveragebitrate` to the
  answer's Opus `fmtp` (which sets the encoder's target), renegotiating once.
- **Enforced by the SFU**: the host's lease carries the bitrate
  (`Authorizer::audio_bitrate`); a token bucket per microphone allows 1.5× the
  bitrate with half a second of burst (variable-rate Opus peaks on transients)
  and drops what is above it. A compliant encoder is never touched.
- Opus codes a mono microphone at up to about 256 kbps; settings above that
  only help stereo sources.

### Measured (browser test, headless Chromium, fake devices, loopback)

See "Measurements" at the end (filled from `callextras-results.json`).

## 5. Clips

Off by default (Voice & Video → Clips, or More → Turn On Clips). Length: the
last 30, 60 or 120 seconds.

- **Bounded**: the buffer exists only while the setting is on, you are in a
  call and a stream is playing (yours, or the one you are watching; the
  focused one first). It holds encoded H.264 (WebCodecs `VideoEncoder`, hardware
  preferred, one key frame a second, ≤1080p) and Opus (the call's sound: other
  people, your microphone and the stream, mixed only while clipping), evicted
  by whole groups of pictures by time and by a 96 MB byte cap. Frames are
  dropped rather than queued when the encoder is behind. Needs WebCodecs and
  `MediaStreamTrackProcessor` (Chromium, Edge, the Windows app); elsewhere the
  setting explains that.
- **Everyone is told**: `VoiceSignal::SetClipping { enabled }` →
  `VoiceEvent::Clipping { session, enabled }` to the call (and to people who join
  later); a notice above the tiles names who has clips on and their tile is
  marked. `VoiceSignal::ClipTaken` (only while clipping) →
  `VoiceEvent::ClipTaken { session }` → "Bob saved a clip". Rates: 30/min and
  12/min.
- **Save Clip** (call controls while active, More, or **Alt+C**) freezes the
  buffer and opens the editor: start and end (the start snaps to the key frame
  before it, at most a second earlier), Include Call Audio, name. The MP4
  (H.264 + Opus) is written in the page by `clips.js` (no muxing library).
  Download, Save to Library, or Share to Chat (attached to the call's chat
  composer like a chosen file; 25 MB upload limit shown).
- **Library**: IndexedDB on this device, at most 50 clips or 1 GB, oldest
  removed first.
- CSP: `media-src` now also allows `blob:`, so locally made clips can be
  previewed and played. Media cannot run script.

## Wire summary

Signals (appended; host-handled unless noted): `PlaySound { sound_id }`,
`SetClipping { enabled }`, `ClipTaken`, `TrackQuality { mid, max_bps }`
(SFU-handled). Events (appended): `SoundPlayed { channel_id, sound: SoundPlay,
by, entrance }`, `Clipping { session, enabled }`, `ClipTaken { session }`,
`AudioQuality { channel_id, bitrate }`.

Storage (`202611230501_soundboard.sql`): `soundboard_sounds`,
`soundboard_favorites`, `soundboard_entrance`, `soundboard_entrance_enabled`,
`voice_channel_quality`, `camera_backgrounds`, and the `@everyone` default.

## Measurements

From `crates/web/tests/callextras.mjs` (2026-11-23): two headless Chromium pages
through the real server and SFU on loopback, fake camera, a mono noise-and-tones
microphone, a 1920×1080 moving canvas as the shared screen, software H.264.
"Before" is the same build with these extras switched off
(`globalThis.__sticktalkLegacyQuality`), which is the pre-change code path:
no Opus bitrate, no encoder caps, no frame-rate cap, the old 720p30 share request.

| What | Before | After |
|---|---|---|
| Microphone, guild voice channel (outbound RTP) | 29.5 kbps (Chromium default) | 64.2 kbps (channel default 64) |
| Microphone, channel set to 256 kbps | | 256.3 kbps |
| Screen share received (1080p60 chosen) | 1920×1080, 53 fps, 5.9 Mbps | 1920×1080, 60 fps, 6.4 Mbps |
| Camera with strong blur (software WebGL, headless) | | 1280×720 sent at 18 fps, segmentation every 2nd frame (adapted) |
| Clip buffer after ~9 s of a 1080p60 share | | 206 H.264 + 286 Opus chunks, 4.3 MB; a 5.7 s MP4 that plays |

The fake screen source ignores size constraints, so the old path also received
1080p here; the difference measured is frame rate and the encoder's cap. With
real hardware GPUs the background effect is expected to hold 30 fps; the
headless run shows the adaptation working on software rendering.
