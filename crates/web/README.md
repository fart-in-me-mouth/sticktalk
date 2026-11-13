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
  completion. Session recovery and core streaming compilation run in parallel with the JS
  import. Static HTML supplies a non-animated, noninteractive chat skeleton while
  WASM boots. `sticktalk-chat-ready` is marked only after real history is rendered;
  screenshots of the skeleton are not used as proof of interactivity.

## Design changes requested by the lead

`text.muted`: **#74747C -> #A0A0AA** in the generated web token sheet (the only
explicit UI_SPEC token override). Contrast is 5.21:1 on bg.selected and 5.81:1
on bg.elevated. Very dark custom role-name colors are lightened for AA legibility;
the saved role swatch stays exact. Call tiles have no presence dots, speaking uses
an accent ring, controls sit at the bottom, and the sidebar has camera/share/leave.
Phone conversations have no bottom tabs and retain the header's # icon.

Fonts preload and use font-display: swap. Production critical CSS is `live.css`;
the much larger fixture stylesheet and media/push modules are not on the login
critical path. Settings/server-admin/roles/pickers, autocomplete and call/signaling code are now
Leptos 0.8 `#[lazy]` split modules. The image viewer, direct GIF provider, media
and push adapters are gesture-loaded JS. Split WASM and glue live together under
a content-versioned `pkg/<digest>/` path; do not flatten these directories.

## Build and validate (PowerShell)

Use one task target outside OneDrive, cap it at 10 GB, keep incremental off,
never retain debug and release trees together, and delete it after QA.
wasm-bindgen matching Cargo.lock, wasm-opt, Rust's wasm32 target, Node and
headless Playwright Chromium must be available per-user; no admin install needed.

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-web2'
$env:TRUNK_BUILD_DIST = Join-Path $env:CARGO_TARGET_DIR 'web-dist'
$env:PLAYWRIGHT_BROWSERS_PATH = Join-Path $env:CARGO_TARGET_DIR 'browsers'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
cd crates/web
npm ci
cargo install wasm_split_cli_support --version 0.2.5 --locked --features build-binary --root "$env:CARGO_TARGET_DIR/tools"
cargo install twiggy --version 0.8.0 --locked --root "$env:CARGO_TARGET_DIR/tools"
# Remove tool-install release artifacts before building native debug targets.
# Verify every deletion resolves inside this one task target directory.
Remove-Item -LiteralPath "$env:CARGO_TARGET_DIR/release" -Recurse -Force
cargo build -p sticktalk-server
cargo test -p sticktalk-web -p sticktalk-server
cargo clippy -p sticktalk-web -p sticktalk-server --all-targets -- -D warnings
$env:STICKTALK_SERVER_EXE = Join-Path $env:CARGO_TARGET_DIR 'tools/bin/sticktalk-server.exe'
Copy-Item "$env:CARGO_TARGET_DIR/debug/sticktalk-server.exe" $env:STICKTALK_SERVER_EXE
Remove-Item -LiteralPath "$env:CARGO_TARGET_DIR/debug" -Recurse -Force
# Only release trees from this point. Tools may also use WASM_BINDGEN / WASM_OPT.
./scripts/build.ps1
cargo clippy -p sticktalk-web --target wasm32-unknown-unknown --profile web-release --all-targets -- -D warnings
cargo fmt --all -- --check
npx playwright install chromium
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
It writes nine live screens at 390x844, 412x915, 820x1180, 1440x900, 1280x800 and
1920x1080, plus RTP evidence. A third cookie-isolated account exercises group DMs.
`npm run audit` separately measures progressive login and cold
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

- Full Phase 1 moderation/administration, invite management and every native
  settings interaction are not yet exposed in the live UI.
- User settings use desktop two-pane navigation and phone settings-list/subpages.
  Profile/avatar editing, hoisted-role member groups, seven-minute message grouping,
  four-trigger autocomplete and group-DM create/manage are exposed. Full role
  reorder, all settings pages, rich GIF embeds/favorites/categories and viewer
  forwarding/clipboard-image actions remain.
- Direct KLIPY uses only the public `/meta` key, credential-free bounded fetches
  and exact approved media hosts. The server opens CSP discovery/media sources
  only in enabled direct mode. Provider account approval/live key delivery was
  not exercised; the adapter is unit-tested without external calls.
- Google redirect/completion is wired and capability-gated, but this work has not
  rerun mock OAuth end-to-end. Screen capture options are implemented but not
  certified by a headless display-capture test. Real phone/Safari behavior, external
  push delivery, offline/version-upgrade behavior, network impairment and large-call
  quality need separate validation.
- See the live report for measured cold/warm TTI and any missed target. Do not
  confuse progressive login or a skeleton paint with authenticated chat TTI.

## Packaging and PWA updates

`scripts/build.ps1` builds with the workspace's `web-release` profile (z, fat LTO,
abort), preserves relocations for wasm-split, then runs wasm-bindgen and
`wasm-opt -Oz --strip-debug` on each output. A checked target-local distribution
is rebuilt without touching another task's artifacts. This also avoids Trunk's
global Windows tool-cache lock when other worktrees are building.

The worker precaches only the static shell/core, caches secondary assets on
demand, and never intercepts private API traffic. The HTML shell uses
stale-while-revalidate; content-versioned assets are cache-first to avoid warm
re-downloads and mixing incompatible versions. An update waits for Reload (and refuses
to discard an unsent draft). Deployments should retain the previous `pkg/<digest>`
directory until old tabs have reloaded, so an older open tab can still discover
a not-yet-opened secondary screen.

After final validation delete the one task target directory:
`Remove-Item -LiteralPath $env:CARGO_TARGET_DIR -Recurse -Force`.
