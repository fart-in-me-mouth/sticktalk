# StickTalk web shell

Leptos 0.8 CSR / wasm32, built with Trunk, in the shared root workspace and
lockfile. Its dedicated `web-release` size profile lives in the root manifest without
changing native profiles. It remains a fixture shell and does
not yet depend on `client` / `client-core`; live adapter wiring is follow-up work.

## Build and test (PowerShell, per-user tools only)

```powershell
# Reuse the ONE target directory already chosen for the whole task.
if (!$env:CARGO_TARGET_DIR) { throw 'Set the task-wide CARGO_TARGET_DIR first' }
$env:TRUNK_BUILD_DIST = Join-Path $env:CARGO_TARGET_DIR 'web-dist'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_INCREMENTAL = '0'
cd crates/web
rustup target add wasm32-unknown-unknown
trunk build --release --cargo-profile web-release
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --all -- --check
npm ci
npx playwright install chromium
npm test
npm run audit
```

Do not use `trunk serve` with a wildcard address. The included test server binds
**127.0.0.1 only**. Both proof scripts launch headless Chromium, mute audio,
disable notifications, close their browser and stop their own server. Lighthouse
uses a loopback-only debugging address. Nothing requests elevation, media access,
Google OAuth, push permission, or an OS window.

`dist/` is deployable at the server's HTTPS origin root. Serve `.wasm` as
`application/wasm`; negotiate adjacent `.br` files with `Content-Encoding: br`
and `Vary: Accept-Encoding`. Do not cache `index.html` / `sw.js` indefinitely.
The post-build hook creates a SHA-256-revisioned app-shell service worker and a
size inventory. Its allowlist never intercepts `/api/`, non-GETs, or other origins.
Old StickTalk shell caches are deleted on activation. Offline launch shows
“Reconnecting…”. No account or message payloads are stored by the worker.

## Adapter boundary

- `src/model.rs`: local `AppVm`, `PersonVm`, `MessageVm`, `RoleVm`, fixture data,
  local action reducer, markdown sanitization, and bounded history-window math.
- `src/components.rs`: stateless DOM primitives.
- `src/views/`: pure view functions of local models; only the message viewport
  has independent scroll-window state. At most the visible history plus ten rows
  on either side are mounted, along with the eight rich fixture tail messages.
- `public/platform.js`: DOM-event adapter (actions, touch gestures, long press,
  visualViewport, focus, keyboard shortcuts, file-drop preview, PWA registration).
  Only one-shot long-press/toast deadlines and idle service-worker startup; no animation frames,
  polling, audio, notifications, browser launching, or remote requests.
- `build.rs`: derives CSS properties directly from `docs/UI_SPEC.md`.
- `scripts/assets.py`: rebuilds WOFF2 Latin/punctuation subsets from the native
  assets, Lucide inline symbols from native/vendored SVGs, and the simple PWA
  mark. Requires Python fontTools, Brotli and Pillow; no network access.

Replace fixture construction with client-core conversion, then translate local
actions into core commands. No view should acquire a socket or deserialize wire
messages. The markdown renderer rejects raw HTML, unsafe URLs and remote images;
mentions are inserted only into text nodes, never link attributes or code.

## Proof and scope

`npm test` writes each screen at 1440×900, 390×844, 412×915 and 820×1180 to
`screenshots/`. It also covers both phone panels, CDP long-press and swipe,
keyboard-resize handling, scrolling bounds, sending a local message, Quick
Switcher, role editing, confirmation guards, auth errors, and offline reload.
The keyboard PNGs use a **labeled emulated keyboard and visualViewport resize**:
headless Chromium cannot show or certify a real iOS/Android OS keyboard.

`npm run audit` records current Lighthouse mobile/desktop performance and
accessibility, plus Lighthouse **11.7.1**'s PWA category separately (removed from
Lighthouse 12). Scores are lab measurements on this PC, not physical-phone or
4G/60-fps certification. See `screenshots/REPORT.md` for final numbers and gaps.

This is a **fixture UI shell**, not a connected client. Authentication, real
message persistence, uploads/downloads, GIF search, sessions, media/WebRTC,
notifications/Web Push and server permissions are not connected. Account and
server mutations are local previews. No account credentials leave the page.

Visual limitations dictated by the authoritative spec: the muted token,
white-on-danger small badges, and 30%-opacity offline rows fail WCAG text
contrast; they are retained rather than silently replacing owner-approved tokens.
Settings and long content scroll natively. All screen motion uses CSS transforms
and one-shot transitions; reduced-motion disables them.

Fonts are Inter and JetBrains Mono (OFL). Icons are Lucide (ISC/MIT); reaction
images are Twemoji (CC-BY 4.0). License copies ship with the static assets.
