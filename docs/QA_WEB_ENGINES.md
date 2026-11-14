# StickTalk phone QA — browser engines

Date: 2026-11-14. Branch: `t3/webkit-firefox-phone-qa`.

## Release gate

**Not iPhone-certified.** The requested WebKit voice/H.264 stream acceptance gate
cannot be completed with this Windows WebKit build. It has no
`RTCPeerConnection`, `navigator.mediaDevices`, or `AudioContext`; MP4 playback
also rejects with `NotSupportedError`, despite `canPlayType()` saying `probably`.
The two WebKit real-media suites are explicitly skipped on Windows, not replaced
with mock RTC, fake frame counters, or Chromium screenshots.

The final automated run and screenshot review are recorded below. A green
non-media WebKit run is **not** proof that iPhone calls work.

## Engine × feature matrix

`PASS` means real packaged CSR + disposable real Rust server, with the noted
browser-edge injections. No REST/WS fixture substitutes. `BLOCKED` is not a pass.

| Feature | iPhone 15 / WebKit 27.2 (Windows) | Pixel 7 / Chromium 158 | Firefox 159 phone viewport |
|---|---|---|---|
| Password registration/login/logout; deletion → login | PASS | PASS | PASS |
| Mock Google signup/collision/login; deletion reauthentication | PASS | PASS | PASS |
| Drawers, browser Back, settings list/subpages | PASS¹ | PASS | PASS¹ |
| Typing, mention, reply/jump, reaction, edit/delete; friends/DMs/unread | PASS | PASS | PASS |
| Long-press action sheet, swipe dismissal, image pinch zoom | PASS¹ | PASS · native CDP touch | PASS¹ |
| Photo upload and original-on-demand viewer | PASS · synthetic file picker | PASS · synthetic file picker | PASS · synthetic file picker |
| 17 MiB upload, pause at 16 MiB, reselect/resume, byte-exact download | PASS | PASS | PASS |
| Inline H.264 MP4 | BLOCKED decode; real upload/download fallback verified | PASS | PASS |
| Emoji insertion / GIF dialog | PASS · provider disabled | PASS · provider disabled | PASS · provider disabled |
| Profile/preferences, sessions, server roles/channels/invites/bans/permissions | PASS | PASS | PASS |
| 10,000-row history, distant seeks, ≤50 mounted message rows | PASS | PASS | PASS |
| Offline banner/reconnect; offline reload; deliberate worker update/draft guard | PASS | PASS | PASS² |
| Session revocation while live/offline; private cache/draft purge | PASS | PASS | PASS |
| Blocked localStorage/sessionStorage/IndexedDB | PASS · injected denial | PASS · injected denial | PASS · injected denial |
| Keyboard viewport resize **and pan**, injected 47/34px safe areas | PASS · geometry only | PASS · geometry only | PASS · geometry only |
| Manifest/Apple icon/capable/status-bar metadata | PASS · automated | PASS · automated | PASS · automated |
| Voice join, real SFU audio, denied capture → rejoin, mute/deafen/leave | BLOCKED | PASS · fake devices | PASS · fake devices |
| H.264 screen receive, ≥10 decoded frames, playsinline/muted autoplay | BLOCKED | PASS · real SFU | PASS · real SFU |
| Focused in-app fullscreen and screen-video pinch zoom | BLOCKED media; handler UI is not device-certified | PASS¹ | PASS¹ |
| Three camera enable/disable cycles with decoded frames in both directions | BLOCKED | PASS · synthetic camera | PASS · synthetic camera + real Cisco GMP |

1. Playwright exposes native taps across these projects, but its public WebKit /
   Firefox API has no multi-touch/hold injection. Those engines use explicit DOM
   pointer/touch sequences for hold, swipe and pinch handlers. Only Chromium's
   multi-touch is native CDP injection. These tests do **not** certify iOS gesture
   recognition, text-selection callouts, pinch physics or an OS keyboard.
2. Firefox request interception prevents native service-worker offline navigation.
   The harness instead blocks external HTTP(S) with a loopback rejecting proxy.
   After an offline navigation, Playwright changes `navigator.onLine` back to true
   without delivering `online`; only that missing event is injected. Cached reload,
   requests, cookies, websocket recovery and history remain real. No private chat
   is stored in the shell cache; the offline document must be a **new** document.

## Fixes and harness corrections

- Advertise only component-1 UDP ICE candidates to the UDP/RTCP-mux SFU. Firefox
  also emits separate component-2 RTCP candidates; retaining those rejected voice
  negotiation. The server's ingress checks were not weakened.
- Serialize bitrate `getParameters`/`setParameters` with negotiation. Concurrent
  SDP changes invalidated Chromium sender parameter transactions during share.
- Ignore guild voice-state broadcasts on text-only peers **before** loading the
  lazy WASM media decoder. A cold offline WebKit peer previously panicked while
  trying to import it and could not recover after its account was revoked.
- Page teardown stops local media without initiating a lazy Leave-signaling fetch.
  The closing websocket releases the SFU session. This removes WebKit teardown
  access-control errors without suppressing page errors in tests.
- Explicit inline/muted video setup before attaching streams; focused screen tiles
  now support bounded 1–4× two-pointer zoom/pan, reset on defocus, without making a
  pinch minimize the call. No animation loop or idle polling was added.
- Coarse-pointer message text/images disable WebKit callouts and text selection;
  Copy Message remains available in the action sheet. Desktop selection is kept.
- Missing browser media APIs produce actionable settings/join errors and release
  the attempted call, rather than throwing an undefined-API error.
- Reconcile connectivity on foreground/pageshow as well as network events.
- Require an actually **installed waiting** worker before offering an update;
  it must also be distinct from an activated active/controller worker. WebKit's
  first activation could otherwise leave a spurious toast over the Google button.
- Keep WebKit fixtures on ephemeral loopback HTTPS: unlike Chromium/Firefox, its
  Secure browser-session cookies cannot be relied on over plain loopback HTTP.
  No certificate was installed in an OS trust store.
- Google start interception uses a local mock-provider redirect page because
  WebKit cannot fulfill an intercepted redirect status. PKCE, nonce/state, token,
  RSA JWKS, cookies and server callback validation are real.
- Firefox needs both fake capture and disposable-profile media grants to gather
  usable loopback ICE. Its H.264 encoder is the real Cisco OpenH264 2.6.0 GMP,
  downloaded per-task and SHA-512 verified against Mozilla's published manifest.
  It is not a fake codec and is not shipped inside the StickTalk application.
- Playwright WebKit screenshots inject an inline `body {}` animation-sync rule,
  which production CSP correctly blocks. The harness observes that **exact rule**
  during screenshots and records its exact matching violation separately in
  `screenshot-tooling-csp.json`. Other CSP violations remain failures; CSP is not
  bypassed and application headers are unchanged. See the
  [Playwright screenshot implementation](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/screenshotter.ts).

## Evidence and validation

Final results: **102 passed, 6 explicitly skipped/blocked, 0 failures, 0 flaky,
0 retries** in **31.8 minutes**, three consecutive repetitions of every suite.
Each repetition: WebKit 10 passed + 2 media suites blocked; Chromium 12 passed;
Firefox 12 passed. The WebKit MP4 fallback checks pass, but decoding stays blocked.
The all-engine **real-media** acceptance gate is therefore still **not met**.

Tested application revision: `dc638f7a1017f6121ef8`; test code head:
`0a437a2` (later changes contain report/evidence-retention updates only).

- 12 Rust unit tests passed; host and WASM Clippy passed with `-D warnings`;
  workspace formatting check passed; Node platform/loading checks passed.
- Core WASM: **449,760 B Brotli**, **31** lazy chunks; existing 450 KB limit kept.
- 17 MiB upload/resume byte checks and ≥10 decoded H.264 frame assertions remain
  enabled. Nine camera rounds per supported engine passed in both directions.
- One target directory measured **5.271 GiB** before cleanup (10 GB budget).
  **Cleanup was blocked by execution policy**, including the exact requested
  `Remove-Item -LiteralPath 'C:\Users\dwhee\.cargo-target\sticktalk-xengine' -Recurse -Force`.
  The target remains; no alternate deletion mechanism or elevation was attempted.
  Exploratory screenshot runs are ignored/uncommitted; the reviewed final snapshot
  is explicitly committed. No test server or task browser remains running.
- [Compact counts, frame evidence and cleanup status](../crates/web/screenshots/engines/final-run/summary.json)
- [Validation output](../crates/web/screenshots/engines/final-run/validation.log)

- [Machine-readable run](../crates/web/screenshots/engines/final-run/results.json)
- [Environment, exact device descriptors, code head and bundle inventory](../crates/web/screenshots/engines/final-run/environment.json)
- [Screenshot review index](../crates/web/screenshots/engines/final-run/contact-index.json)

Screenshot review: all **92 WebKit first-run screenshots** were inspected across
8 contact sheets, with Chromium/Firefox comparisons for auth, chat, drawers,
keyboard geometry, settings, uploads, action sheets and media. No additional
layout defect was found; native select/color/video controls differ by engine.
The unsupported WebKit video fallback is intentionally not a decode pass.

The iPhone project uses the unmodified Playwright `iPhone 15` descriptor:
393×659 viewport, 393×852 screen, DPR 3, `isMobile` and touch enabled in the
context options. This Windows WebKit still reports `navigator.maxTouchPoints=0`;
its emulation is not physical iPhone hardware. Pixel 7 uses
its actual descriptor (412×839 viewport, DPR 2.625). Firefox uses a 412×839 touch
viewport, **not** `isMobile` (unsupported by Playwright Firefox) and not a claim
about Firefox for Android. Browser engines run headlessly; no owner browser,
OS notification, sound, real capture device, Google account or visible window.
All fixture HTTP/TLS/proxy listeners and Rust SFU sockets bind to 127.0.0.1.

### Reproduce (PowerShell, repository root)

```powershell
$env:CARGO_TARGET_DIR='C:/Users/dwhee/.cargo-target/sticktalk-xengine'
$env:CARGO_INCREMENTAL='0'
$env:STICKTALK_QA_OPENSSL='C:/Program Files/Git/usr/bin/openssl.exe'
cargo build --locked -p sticktalk-server
cd crates/web
npm ci
npx playwright install chromium webkit firefox
. ./scripts/install-engine-tools.ps1
./scripts/build.ps1
$env:STICKTALK_ENGINE_EVIDENCE_ROOT='screenshots/engines/final-run'
npm run test:engines # repeatEach=3, workers=1, retries=0
python scripts/qa-engine-gallery.py screenshots/engines/final-run
node tests/platform.mjs
node tests/loading.mjs
cargo test --locked -p sticktalk-web
cargo clippy --locked -p sticktalk-web --all-targets -- -D warnings
cargo clippy --locked -p sticktalk-web --target wasm32-unknown-unknown --profile web-release --all-targets -- -D warnings
cargo fmt --all -- --check
```

Do not rebuild/replace the served distribution during a run. The TLS upload gate
pauses the **real proxy request**, not a fixture response, so service-worker-owned
uploads are tested too. Re-running removes old success/failure summary markers;
the gallery takes its screenshot list from current result JSON, not stale PNGs.

## Real-device / deployment acceptance still required

- Actual iPhone Safari **and installed Home Screen** app: voice permission grants,
  microphone routing/interruption/Bluetooth, H.264 decode/hardware behavior,
  inline/autoplay recovery, screen focus/zoom and sustained call quality.
- Actual iOS keyboard/address-bar/rotation/notch behavior, IME/composition,
  sticky overflow/momentum scrolling, native long-press callout suppression,
  pinch gestures, Back/edge swipes and background/foreground reconnects.
- iOS/Android OS photo/file/camera pickers, HEIC/HEVC, uploads under suspension,
  low-memory eviction, persistent storage/private-mode policies and reinstall.
- Physical Add to Home Screen installation, icon/status-bar appearance and worker
  updates across old releases. Desktop Playwright WebKit is not Mobile Safari.
- **iOS Web Push requires an installed Home Screen PWA** (iOS/iPadOS 16.4+), a user
  gesture and notification permission. A Safari tab alone is not acceptance.
  Real APNs/FCM/Firefox push delivery, OS presentation and deep links were not sent.
- Live GIF-provider delivery, acoustics, impaired networks, long-call/large-room
  load and screen-share quality targets remain outside this synthetic QA.

References: [Playwright browser/platform limitations](https://playwright.dev/docs/browsers),
[Mozilla's official OpenH264 manifest](https://github.com/mozilla-firefox/firefox/blob/main/toolkit/content/gmp-sources/openh264.json),
[WebKit Home Screen Web Push requirements](https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/).
