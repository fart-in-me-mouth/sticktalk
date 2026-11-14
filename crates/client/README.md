# StickTalk desktop

Native Rust / iced 0.14 client; wgpu by default with automatic tiny-skia fallback. No webview.

## Build

```powershell
$env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
$env:CARGO_TARGET_DIR = "$env:USERPROFILE/.cargo-target/sticktalk-client"
cargo build -p sticktalk --release
```

Ordinary users run `sticktalk.exe`. The remembered server wins over build-time `STICKTALK_DEFAULT_SERVER`; its development fallback is `http://127.0.0.1:8080`. “Change server” reveals the address field. HTTPS is required outside loopback. Password auth uses `/api/v1`; Google/meta use the documented `/api` aliases. Session tokens use Windows Credential Manager; passwords and Google tokens are not persisted.

## Agent-safe screenshots

```powershell
& crates/client/tools/capture.ps1
# Equivalent native command (no OS window is constructed):
sticktalk --mock --screenshots crates/client/screenshots --sizes 1280x800,1920x1080
```

48 states × two sizes = 96 PNGs, plus `screenshots/manifest.json` with dimensions and SHA-256 hashes. This renders the actual iced view tree with the native tiny-skia offscreen compositor, including measured virtual rows, overlays, fonts, thumbnails and editors. No browser, notification, focus change, network subscription, or OS window. Legacy `--mock --screen <state> --size 1280x800 --capture <path.png>` also uses this offscreen path.

Never launch ordinary windows for agent screenshot work. When a real window is unavoidable for native integration/performance, set `STICKTALK_AGENT_WINDOW=secondary`: it is created hidden, placed on a non-primary monitor with `WS_EX_NOACTIVATE` and `SWP_NOACTIVATE`, and then shown. No secondary monitor means a hidden exit, not a primary-monitor fallback. Close it after testing. `--agent-window-report <path.json>` records placement, client size and foreground checks. Mock/agent mode suppresses browser and focus actions.

## Scrolling / profiling

Default Windows GPU backend is D3D12 (respects `WGPU_BACKEND` overrides); iced automatically falls back to tiny-skia when GPU initialization fails. `ICED_BACKEND=tiny-skia` explicitly selects software; `cargo build -p sticktalk --release --no-default-features` builds software-only. Headless screenshots always use tiny-skia, regardless of the default.

```powershell
sticktalk --mock --bench-scroll crates/client/scroll-after-gpu.json --gpu
sticktalk --mock --bench-scroll crates/client/scroll-after-tiny.json
# Real native fixture, only in guarded agent mode during testing:
$env:STICKTALK_AGENT_WINDOW = 'secondary'
sticktalk --mock --screen scroll-5000 --size 2560x1440
```

The benchmark profiles the real view/layout/draw path at 2560×1440 with 5,000 fixture messages, 60 warmups and 600 recorded frames per trace. Traces cover fractional wheel-sized offsets, worst-case random cold seeks, and fast-drag static skeletons. GPU timing waits for submission completion, not just CPU submission; no OS window or pixel readback. This does not measure physical display cadence. A separate headless test exercises actual iced fractional wheel and scrollbar press/drag events.

Message bodies retain iced's shaped paragraph Tree and layout in a bounded 256-body LRU across visible-window eviction. Rows are keyed by message ID; an indexed prefix-height table locates visible rows with four-row overscan. Compact metadata for up to 20,000 visited messages survives the 1,000-body projection cap. Fast seeks are detected by viewport jump or event-rate-independent velocity, and use static skeletons and one input-driven 60 ms settle timer; visited-but-evicted regions fetch directly by their known keyset cursor, with stale-response suppression and anchoring. Read receipts wait until non-skeleton content settles. The current server protocol has no total count, index/offset or arbitrary jump endpoint: **unvisited history cannot truthfully appear as a full-channel random-access scrollbar** without server/proto changes. Unknown older history retains ordinary pagination.

## Google configuration

At build time set `STICKTALK_GOOGLE_CLIENT_ID` and `STICKTALK_GOOGLE_CLIENT_SECRET` to the Desktop OAuth application's values. Do not commit credentials. The button appears only if `/api/meta` advertises the same client ID and both build values are nonempty. A desktop client secret is distributed configuration, not a confidential server secret. Mock screenshots deliberately simulate enabled capabilities without requiring credentials.

Implemented: PKCE S256/state, random-port IPv4 loopback, system-browser authorization, bounded callback parsing, two-minute timeout/cancel, token exchange, existing-account session, signup ticket/username completion, Google link/unlink, password set/change, and `LastLoginMethod` guidance. Production browser handoff remains intentional for real users; automated tests inject mock authorization and token endpoints and never open a browser.

## Navigation and messages

Home friends tabs and DMs; server rail/channel categories; full-screen server/user settings; searchable members with moderation menus; upload/download/replies/reactions/edit/delete; contextual keyboard navigation. Markdown supports bold, both italic markers, underline, strike, inline/fenced code, quotes, spoilers, three heading levels, lists, URLs, masked links with URL confirmation and backslash escapes. Parsed trees are cached per retained message and invalidated on edits. Code uses bundled JetBrains Mono. Mark Unread is local to the current client session: protocol v1 only supports forward read receipts.

Enter sends/saves; Shift+Enter inserts a newline; Up edits the latest own message when the composer is empty. Esc dismisses menus/modals/settings. Tab/Shift+Tab navigate focus; menus also support Up/Down and Enter/Space.

## Code map / proof

`app.rs`: controller; `model.rs`: bounded reducers; `backend.rs`: real/mock transport; `mock.rs`: fixtures; `oauth.rs`: installed-app flow; `markdown.rs`: cached parser; `headless.rs`: offscreen captures; `message_layout.rs`: persistent shaped text/layout; `scroll_bench.rs`: offscreen frame profiling; `agent_window.rs`: guarded native placement; `ui.rs`: views; `theme.rs`: tokens; `media.rs`: bounded image caches.

One Tokio runtime / two workers / one WebSocket; no render tick, animations or REST polling. See [PERF.md](PERF.md), [REPORT.md](REPORT.md), [UI_REVIEW.md](UI_REVIEW.md), [VALIDATION.txt](VALIDATION.txt), and [ASSETS.md](ASSETS.md).

```powershell
cargo test -p sticktalk
cargo clippy -p sticktalk --all-targets -- -D warnings
cargo fmt --all -- --check
```
