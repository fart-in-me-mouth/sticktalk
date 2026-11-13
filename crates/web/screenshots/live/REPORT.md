# Live web client QA — partial launch implementation

Build revision `5470c80f88a389066a73`. **Not completion of B8 or every Phase 1/launch flow.**
All proof uses the default production build, a real Rust server and two independent
headless Chromium processes. HTTP, binary WebSocket and SFU traffic are real and
loopback-only. No fixture API responses. Fake camera/microphone devices; OS audio
and notifications disabled. Push permission/PushManager are mocked, not provider delivery.

## Implemented and exercised

- Cookie login/register/recovery, correct CSRF header, client-core web adapters,
  bounded model, heartbeat/reconnect/resume wiring and exact 64-bit signaling IDs.
- Friends, DMs, server create/invite/join, text/voice channels; role color,
  permission/hoist editing, role assignment and private-channel grant/revoke.
- Markdown, replies, reactions, edits, pins, mentions, notifications and device list.
  Real history extent/anchor seeks with 55 seeded history messages and <=50 mounted.
- **17 MiB** Blob.slice upload, paused after **16 MiB**, same-file reselect and real
  server-offset resume. Attachment delivery, thumbnail-only initial load and original
  image fetch on viewer gesture. Emoji crate search/composer insertion.
- Real SFU audio/camera exchange, mute/deafen/leave and complete publication,
  subscription and media-element release. SDP dynamic-SSRC/MSID normalization fixes
  Chromium's decoded-but-black remote video without altering DTLS fingerprints.
- Push explainer gesture -> one permission request -> real subscription POST 201.
  Worker contract tests cover generic content, exact decimal deep links and same-origin
  click reuse/focus. Focus hint follows visibility/focus and selected conversation.
- Private history/pin caches and drafts are purged on permission loss/account switch;
  logout then password login as another account is covered.

22 live checks passed; application errors **0**, CSP violations **0**.
[Machine-readable results](results.json), [worker/media tests](../../tests/platform.mjs).

## Call evidence

- Browser 1: 61,440 decoded audio samples, 17 decoded remote H.264 frames; remote video 320 px wide. RTP and DOM playback evidence: [voice-stats.json](voice-stats.json).
- Browser 2: 61,920 decoded audio samples, 15 decoded remote H.264 frames; remote video 320 px wide. RTP and DOM playback evidence: [voice-stats.json](voice-stats.json).

These are short synthetic loopback tests, **not** voice quality, HD bitrate,
network-loss or large-call certification. The fake remote video rendered at 320x180.

## Screenshots

Real-server login, conversation and call PNGs at **1440x900**, **390x844**,
**412x915** and **820x1180**: `login-*`, `chat-*`, `voice-*` in this directory.
[Desktop chat](chat-1440x900.png) · [390 phone chat](chat-390x844.png) ·
[412 phone call](voice-412x915.png) · [820 tablet call](voice-820x1180.png).
[Image viewer](image-viewer-1440x900.png).

Phone conversations hide bottom tabs and show # before text-channel names. Call tiles
use accent speaking rings, no presence dots, full bottom controls and sidebar
camera/share/disconnect. `text.muted` **#74747C -> #A0A0AA** (5.21:1 on selected,
5.81:1 on elevated); dark role text is lightened without changing saved swatches.

## Performance and size

Lighthouse **13.6.0**, real-server cold runs. Scores are
performance / accessibility / best practices.

| Page | Scores | TTI |
| --- | --- | --- |
| Cold mobile login | 100 / 100 / 100 | 1.437 s |
| Cold authenticated mobile chat | 85 / 100 / 100 | 4.436 s |
| Desktop login | 100 / 100 / 100 | 0.250 s |

**Cold authenticated chat misses <2 s.** Progressive login defers WASM; it does not
solve chat startup. CSS is smaller, fonts preload/swap, media/push are lazy JS modules;
heavy Rust settings/role/picker code still shares the main WASM.

- WASM: **2,354,735 raw / 526,282 Brotli bytes**.
- Distribution: **2,609,329 raw / 676,346 all-assets Brotli bytes**.
- Installable shell inventory: **669,234 bytes**, not cold-login
  network transfer. [Asset inventory](bundle-sizes.json), [Lighthouse summary](lighthouse-summary.json).
- 60 s idle Friends screen, voice inactive: **0.072%** of one
  logical CPU. Whole Chromium process tree working set **474.6 MiB**,
  private **116.2 MiB** (private delta vs blank browser
  **39.1 MiB**). Working-set totals double-count shared pages;
  this is not a native-client/physical-phone <100 MB pass. [Idle sample](idle.json).
- QA dependency audit: **0 total vulnerabilities, 0 high**. Removed obsolete PWA
  Lighthouse tooling; current Playwright/Lighthouse lockfile. [Audit](../dependency-audit.json).

## Validation passed

- `cargo test -p sticktalk-web` — **7 tests**.
- `cargo clippy -p sticktalk-web --all-targets -- -D warnings`.
- Same clippy command with `--target wasm32-unknown-unknown`.
- `cargo fmt --all -- --check` and `git diff --check`.
- Trunk release `web-release` production build; `npm test`; `npm run test:server`
  (strict CSP and blocked inline probes); `npm run audit`; `npm run test:idle`;
  `npm audit` (0 vulnerabilities).

## Remaining / not certified

- Full Phase 1 administration, moderation, group DMs and invite management are not
  all exposed. Settings/profile/avatar layouts, role reorder/member grouping,
  message grouping/mention autocomplete, picker polish and complete viewer actions
  remain. Existing functional dialogs are not full UI_PARITY implementations.
- Google redirect/completion is wired but mock-OAuth browser E2E was not rerun.
  Proxy GIF search is wired but not provider-tested; direct KLIPY mode needs a
  coordinated server CSP change (outside this task's allowed crate).
- Display-capture FPS/resolution controls and device/output pickers are implemented
  but actual screen capture/device switching are not headless-certified. Real
  phones/Safari, OS/provider push delivery, offline/version-upgrade behavior,
  impaired networks and large-call quality remain unverified.
- True WASM screen splitting and cold-chat <2 s are unfinished. Scope is a tested
  working foundation replacing fixtures, **not a launch-complete app**.
