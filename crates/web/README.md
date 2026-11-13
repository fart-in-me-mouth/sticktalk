# StickTalk web / phone client

Leptos CSR with `sticktalk-client-core`'s **web** adapters and bounded shared
model. The default build connects to the real same-origin StickTalk server.
**This is a working live-client foundation, not completion of every launch flow.**
See [live QA and remaining work](screenshots/live/REPORT.md).

## Runtime boundary

- `src/live/`: cookie session recovery/login/register/Google completion; one binary
  MessagePack WebSocket; typed protocol commands; heartbeat deadlines, jittered
  reconnect and bounded nonce correlation. Identify has an empty bearer token and
  opts into launch events. Focus follows document visibility/focus.
- `CookieFetch` adds **X-StickTalk-CSRF** to unsafe requests. It wraps core `Fetch`
  without its obsolete `X-CSRF-Token` option. Same-origin cookies, response limits,
  timeouts and fetch cancellation remain core-owned. No bearer credential is
  exposed to script, URLs or localStorage.
- Core `Model` owns event ordering, snapshot reconciliation, message/cache bounds,
  unread state and names. Core markdown renders escaped DOM text, not untrusted
  HTML. Browser launch projections add roles, devices and notification settings.
- History uses the real count/anchors endpoint and around-message seeks, estimated
  extent spacers and at most 50 rendered messages. A newer navigation aborts the
  previous fetch, including its browser AbortController.
- Uploads hash 64 KiB `Blob.slice` reads and send one bounded protocol chunk at a
  time. SessionStorage stores only nonsecret upload ID/hash metadata. Selecting
  the same file validates its digest and resumes the server offset. Cancel pauses
  between chunks; completed uploads are retained for their original channel.
- `public/live.js`: DOM/focus/draft/visualViewport/gesture adapter.
- `public/media.js`: dynamically imported only for calls/device settings. One SFU
  RTCPeerConnection; Opus plus H.264; complete offers, recvonly subscriptions,
  camera/display capture, mute/deafen, RTP speaking state and active-call-only
  local level measurements. Answer normalization supports the SFU's dynamic forwarding
  SSRCs while retaining fingerprints/MIDs. Tracks, timers and peer state stop on leave/WS loss.
  Microphone constraints enable echo cancellation, noise suppression and AGC.
  Device pickers use enumerateDevices/devicechange; output uses setSinkId when
  supported. Desktop display capture offers 15/30/60 fps and 720/1080/1440p ideals.
- `public/push.js` is also lazy. Permission is requested only from the explainer's
  button handler. The generated service worker caches only an app-shell allowlist,
  never API/account content; its push handler uses generic lock-screen text and
  validates same-origin deep links without rounding Snowflake IDs.
- `public/boot.js` provides a real progressive login form. The packager injects
  dynamic WASM startup only after successful login, session recovery or Google
  completion. This improves **login** TTI; it does not prove cold chat TTI.

## Design changes requested by the lead

`text.muted`: **#74747C -> #A0A0AA** in the generated web token sheet (the only
explicit UI_SPEC token override). Contrast is 5.21:1 on bg.selected and 5.81:1
on bg.elevated. Very dark custom role-name colors are lightened for AA legibility;
the saved role swatch stays exact. Call tiles have no presence dots, speaking uses
an accent ring, controls sit at the bottom, and the sidebar has camera/share/leave.
Phone conversations have no bottom tabs and retain the header's # icon.

Fonts preload and use font-display: swap. Production critical CSS is `live.css`;
the much larger fixture stylesheet and media/push modules are not on the login
critical path. Heavy Rust settings/picker code is **not yet split out of WASM**.

## Build and validate (PowerShell)

Use exactly one task target directory outside OneDrive, and delete it after QA.
Trunk, wasm-bindgen matching Cargo.lock, wasm-opt, Rust's wasm32 target, Node and
headless Playwright Chromium must be available per-user; no admin install needed.

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-web-live'
$env:TRUNK_BUILD_DIST = Join-Path $env:CARGO_TARGET_DIR 'web-dist'
$env:PLAYWRIGHT_BROWSERS_PATH = Join-Path $env:CARGO_TARGET_DIR 'browsers'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
cd crates/web
npm ci
trunk --skip-version-check build --release --cargo-profile web-release
cargo build -p sticktalk-server
$env:STICKTALK_SERVER_EXE = Join-Path $env:CARGO_TARGET_DIR 'debug/sticktalk-server.exe'
npx playwright install chromium
cargo test -p sticktalk-web
cargo clippy -p sticktalk-web --all-targets -- -D warnings
cargo clippy -p sticktalk-web --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --all -- --check
npm test
npm run test:server
npm run audit
npm run test:idle
npm audit --audit-level=high
```

All QA servers and remote debugging bind to 127.0.0.1. Tests launch **headless**
browsers, mute OS audio, disable real notifications, use fake media devices/UI,
and close processes/temporary profiles. The extra Chromium loopback-ICE flag is
necessary for a loopback-only SFU; no external STUN/TURN is used. Push permission
and PushManager are mocked in browser QA; the subscription REST endpoint is real.
Provider delivery, actual OS notifications and real Google are never launched.

`npm test` runs the pure service-worker contract and real-server two-browser flow.
It writes login/chat/call screenshots at 1440x900, 390x844, 412x915 and 820x1180,
plus RTP evidence. `npm run audit` separately measures progressive login and cold
authenticated chat with current Lighthouse. `sizes.json` is a full bundle/cache
inventory; its firstLoadBytes field is **not** a cold-login transfer measurement.

## Opt-in screenshot fixtures

Production WASM excludes fixture modules. The packager also removes fixture
images, emoji PNGs, fixture JS and its stylesheet. To run historical visual tests:

```powershell
$env:STICKTALK_WEB_MOCK = '1'
trunk --skip-version-check build --release --cargo-profile web-release --features mock
npm run test:mock
```

The fixture build still requires `?mock`; all other URLs use the live app. Clear
STICKTALK_WEB_MOCK and rebuild without --features mock before deployment. Old
root screenshots/reports are historical fixture baselines, not live-flow proof.

## Known remaining launch work

- Full Phase 1 administration/group-DM/invite-management flows and all native
  settings/profile/avatar interactions are not yet exposed in the live UI.
- Settings are compact functional dialogs, not the complete two-pane parity
  layouts. Role reorder, complete member/message grouping, mention autocomplete,
  complete picker UX, rich GIF
  embeds/favorites/categories and the full image-viewer toolbar remain.
- Proxy GIF search is wired; direct KLIPY metadata requests are intentionally not
  attempted because the server's browser CSP only permits same-origin connect-src.
  Direct mode needs a coordinated server CSP/provider-policy change.
- Google redirect/completion is wired and capability-gated, but this work has not
  rerun mock OAuth end-to-end. Screen capture options are implemented but not
  certified by a headless display-capture test. Real phone/Safari behavior, external
  push delivery, offline/version-upgrade behavior, network impairment and large-call
  quality need separate validation.
- Real WASM screen splitting and the <2 s **cold authenticated chat** target remain;
  do not confuse the progressive login result with completion of that requirement.
