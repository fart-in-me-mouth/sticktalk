# StickTalk web — mobile QA

Date: 2026-11-13. Branch: `t3/web-mobile-qa-polish`.

## Release assessment

**Not launch-ready: intermittent H.264 reception remains a release blocker.**
Substantially improved, but not physical-device launch-certified. This is real-server
Chromium QA, not fixture-only screenshots. Remaining backend/product gaps and device
certification are listed below; disabled controls do not pretend to save settings.

## Scope and evidence

- Production Leptos CSR build; real Rust server, disposable SQLite databases and SFU.
- Headless Chromium, touch 390×844 / 412×915, tablet 820×1180, desktop 1440×900;
  the existing live suite also captures 1280×800 and 1920×1080.
- All listeners are loopback-only. Accounts, messages, media and OAuth identities
  are synthetic. No visible windows, OS notifications, hardware capture, external
  Google login or real push delivery were used.
- [Phone contact sheet](../crates/web/screenshots/qa/contact-phone.png),
  [all contact sheets/index](../crates/web/screenshots/qa/contact-index.json).
  Individual PNGs are unmodified real-browser captures. Camera layout PNGs in
  `prior-camera-success/` belong to the last green full run (`9afc59fa41f8d85ff553`),
  not the final failing camera attempt; current phone screen-share/settings captures
  and all JSON summaries identify their build.

## Feature → status

| Feature | Status and evidence |
|---|---|
| Password accounts | **PASS.** Real register/login/logout, current-password change, confirmed account erasure (DB tombstone and zero remaining message content asserted); session-specific/all-other-device revocation and sign-in recovery. |
| Google accounts | **PASS · mock provider.** Mock authorization server, PKCE verification, RSA JWKS/token, username collision, existing-account login and operation-bound deletion reauthentication. Loopback HTTPS; no external requests. |
| Navigation | **PASS · Chromium.** Actual touch swipe/drawer dismissal, browser Back, settings list/subpages, keyboard-sized viewport and injected 47/34px safe areas. Physical Safari back/keyboard still needs certification. |
| Messages | **PASS.** Real cross-browser send/edit/delete-own/reply/reaction/pin, markdown, explicit mentions, typing, unread channel badge, autocomplete and exact reply-target jump. Phone long-press sheet replaces hover-only actions. |
| Long history | **PASS · emulated.** 10,000 real SQLite rows served through actual history APIs; adjacent keyset windows cover gaps between sparse extent anchors, distant seeks and at most 50 mounted rows. See performance below. |
| Uploads | **PARTIAL · OS picker/device certification pending.** Real photo input/upload, image original-on-demand and genuine two-touch pinch; existing live suite covers 17 MiB upload, pause at 16 MiB, reselect and server-offset resume. No native OS picker/camera or HEIC certification. |
| Inline video | **PASS · WebM / browser codec dependent.** Bounded, playsinline video with download fallback. Legacy octet-stream uploads use a narrow video-extension decoder hint, not trusted MIME. Synthetic WebM decoded in Chromium; hardware/codec matrix remains. |
| Emoji / GIF | **PARTIAL.** Phone-sized emoji search/insertion and GIF dialog. KLIPY adapter validation covered without external requests; live provider delivery, favorites and rich categories remain. |
| Voice / screen viewing | **PARTIAL · camera receive blocker remains.** Two Chromium processes exchange real SFU audio and decode H.264 video; phone receives a synthetic screen publication (at least 10 decoded frames required). Mute/deafen/leave cleanup and an injected browser-denial edge tested; positive grant/rejoin uses real SFU RTP. Real microphone quality, OS screen capture and impaired-network/large-call tests remain. |
| Social | **PASS.** Friend request/accept, DMs/group DMs, invite URL preserved across recovery, prefilled explicit join confirmation, real join/revoke/ban/unban. |
| User settings | **PARTIAL · backend gaps.** Account/Profile/About, Appearance (device-only bounded preferences), scoped Notifications, Voice & Video, Sessions, deletion, blocked-user list. Unsupported privacy switches and global notification defaults explicitly disabled/explained. |
| Server settings | **PARTIAL · backend gaps.** Overview/name/icon/description/banner, roles/colors/permissions/hoist/mentionability/order/member assignment, channel create/rename/topic/delete, paged invites/bans and confirmations. Permission downgrade purges privileged cached invite/ban data; forged mutation denied. |
| Administration gaps | **OPEN.** Custom emoji management and full fine-grained channel permission-overwrite authoring are not implemented. Custom emoji is also absent in the native/backend path inspected. No fake local-only mutations. |
| Install / offline / updates | **PASS · automated; physical install pending.** Manifest PNG dimensions, maskable and Apple metadata, actual worker-controlled offline reload, private API/history exclusion, online recovery, waiting update prompt, draft-in-another-channel reload guard, explicit activation and obsolete cache cleanup. Runtime JS and split WASM are release-fingerprinted; the first lazy import remains on the old release when stable aliases change. Physical iOS/Android installation remains. |
| Web Push | **PARTIAL.** Existing self-hosted VAPID/subscription path exercised with mocked browser permission/PushManager and real REST subscription. Generic lock-screen worker text/deep links unit-tested. Provider delivery/OS presentation not certified; no paid service required by this implementation. |
| Visual polish | **PASS · headless review.** Dark tokens, overflow/ellipsis/wrapping, safe-area/viewport handling, native-style settings and message sheets; 44px primary controls, reaction/reply/avatar targets and momentum/overscroll behavior. Inline text links retain text geometry. |

## Defects fixed during QA

- Cross-channel draft ownership leak; update guard losing another channel’s draft.
- Mobile hover action toolbar accidentally replying when touching message text.
  Long-press sheets now survive the opening finger’s release; Chromium adjusted
  author hits are checked against actual geometry.
- Sparse history anchors skipping intermediate messages, continuous-scroll debounce,
  distant-seek placement and previously unwired reply-target navigation.
- Typing’s first notification suppressed; deleted reply target left stale composer state.
- Async form preservation preventing untouched settings/member roles from hydrating.
- Chromium TCP ICE candidates rejected by UDP-only SFU; offers now retain only UDP
  candidates, with Opus/H.264 preference and no SFU/native-client changes.
- Revoked session stuck indefinitely on stale private chat; 4001 now purges account,
  drafts/media/sheets and permits sign-in with the surviving CSRF cookie.
- Late session-revocation acknowledgment closing newly opened settings.
- Video uploads stored by the server as octet-stream never reaching inline playback.
- Unversioned lazy runtime JS could mix a new host adapter with an old waiting-update
  WASM tab. All eight runtime modules and their imports now use release fingerprints.
- Added bounded, nonce-correlated subscription acknowledgments and receiver ordering;
  repeated camera activation now has dedicated coverage, but the original full-suite
  camera failure remains open as described below.
- Auth keyboard clipping, drawer history races, selected-server indicator leaking into
  icon preview, white modal focus outline and update toast covering the composer.

## Open quality / deployment gates

- **OPEN / release blocker:** the original full two-browser suite still intermittently
  stalls one camera receiver. Final attempt: one peer decoded 768 frames, the other
  received one video packet and decoded zero; both audio streams remained live.
  The focused three-cycle camera test passes. The web adapter now awaits nonce-matched
  Subscribe acknowledgments before enabling receivers, but this did **not** fully
  resolve the original scenario. Root cause remains unisolated; no SFU/media/native
  client code was changed. [Compact history](../crates/web/screenshots/qa/known-camera-flake.json),
  [final raw diagnostic](../crates/web/screenshots/qa/live/camera-final-failure.json).
- Successful synthetic camera cycles decoded at 320px width initially, then 1280px
  on reactivation. This is functional decoding evidence, not competitive call-quality
  certification.
- One large-account erasure showed `Unavailable` under shared-host load; root cause
  is unconfirmed. The final clean run needed zero retries and checked the actual DB.
  No server timeout or auth limiter was weakened. [Observation](../crates/web/screenshots/qa/known-erasure-transient.json).
- Privacy policy controls, global notification defaults, custom emoji administration,
  rich GIF features and full channel overwrite authoring still need backend/product work.
- Physical Safari/iOS and Android installation, safe-area/keyboard interactions,
  OS media pickers, HEIC/HEVC compatibility, real push delivery and call quality
  on impaired networks remain unverified.
- Retain previous `pkg/<digest>` **and** `public/runtime/<digest>` assets while old
  tabs exist. When adopting this first fingerprinted runtime, reload pre-fingerprinting
  tabs once. No external deployment was performed.

## QA integrity notes

The installed Playwright 1.65 returned immediately for an async false
`waitForFunction` predicate; async API/RTP predicates now use an explicitly awaited,
bounded Node poll. The fake-media UI flag bypasses Chromium's denied permission;
only that negative getUserMedia edge is injected, while positive media uses real
fake-device tracks and measured SFU RTP/decoded frames. Viewports use isolated
servers and bounded Retry-After waits, never disabled production auth rate limits.

## Measurements

Build pair: `15275f8e65328dd1cb91` → `a520c4ee8c84107af622`. Exact bytes, not rounded marketing KB.

| Bundle | Before (B) | After (B) |
|---|---:|---:|
| Core WASM, Brotli | 440,238 | 438,398 |
| All packaged assets, Brotli estimate | 776,634 | 834,596 |
| Installable shell cache (not cold transfer) | 598,745 | 603,809 |

Core change: -1,840 B (-0.42%). There are 28 actual lazy WASM chunks. The task’s earlier “421 KB” figure is historical; the reproduced checkout baseline here was 440,238 B. Added settings/admin functionality increased the complete lazy-asset inventory.

| Lighthouse TTI | Before | After |
|---|---:|---:|
| Cold progressive login | 1.439 s | 1.436 s |
| Cold authenticated chat | 4.125 s | 4.264 s |
| Warm authenticated chat | 1.667 s | 0.701 s |
| Desktop progressive login | 0.255 s | 0.256 s |

Warm custom `sticktalk-chat-ready`: 1760.7 → 875.4 ms. Lighthouse TTI can precede complete chat hydration. **Cold chat still misses the 2 s target; warm ready still misses 0.8 s**, despite the lower warm TTI. Login paint is not chat readiness.

Cold mobile uses Lighthouse 13.6.0 simulated throttling; warm uses actual DevTools 4× CPU / 150 ms RTT / 1,638.4 Kbps with HTTP/SW caches retained. Single paired runs on a shared Windows host are not causal performance proof. Accessibility and best-practices scores were 100 in the final audited views.

[Before/after chart](../crates/web/screenshots/qa/performance.png); raw [Lighthouse summary](../crates/web/screenshots/qa/lighthouse-summary.json), [bundle inventory](../crates/web/screenshots/qa/bundle-sizes.json).

- 10,000-row history scroll: **60.00 fps**, p95 frame interval **16.7 ms**, 0 intervals >25 ms, maximum **50 mounted message rows** over 3.02 s. This is a requestAnimationFrame/DOM-scroll sample in headless Chromium, not physical 60 Hz certification.
- Idle, voice/video inactive, 60.0 s: **0.066% of one logical core**. Chromium process-tree working set **253.9 MiB**, private bytes **111.1 MiB**; blank-browser deltas **76.6 / 36.6 MiB**. Working-set sums double-count shared pages. This is not native-client/physical-phone memory acceptance; whole-browser RAM exceeds 100 MiB.


## Validation and reproduction

Prerequisites: Node/Playwright Chromium, Rust wasm target, Git for Windows OpenSSL,
and Pillow for contact sheets. Use one target directory outside OneDrive, `CARGO_INCREMENTAL=0`, and the workspace
profiles. Install Node dependencies with `npm ci`; use per-user Playwright Chromium.
From `crates/web`:

```powershell
$env:CARGO_TARGET_DIR='C:/Users/dwhee/.cargo-target/sticktalk-qa-web'
$env:CARGO_INCREMENTAL='0'
# Build the real server from the workspace, then the packaged web build:
cargo build -p sticktalk-server
./scripts/build.ps1
$env:STICKTALK_QA_DIR='screenshots/qa/live'
npm test
$env:STICKTALK_QA_DIR='screenshots/qa'
npm run test:qa
npm run audit
npm run test:idle
python scripts/qa-gallery.py
cargo test -p sticktalk-web
cargo clippy -p sticktalk-web --all-targets -- -D warnings
cargo clippy -p sticktalk-web --target wasm32-unknown-unknown --profile web-release --all-targets -- -D warnings
cargo fmt --all -- --check
```

Do not rebuild the Windows server executable while a test/preview is using it.
TLS OAuth tests use an ephemeral self-signed certificate file, never an installed
certificate. The private test key/databases live only in the disposable task target.

### Final validation

- **PASS:** web Rust tests (12), native and WASM clippy with warnings denied,
  workspace fmt, and untouched server tests (127 passed, 1 pre-existing ignored)
  run serially. No production limiter/timeout was weakened.
- **PASS:** `npm run test:qa`: 43 grouped real-server checks plus three two-process
  camera cycles, platform/loading regressions, zero JavaScript errors/CSP violations.
  Screen-share assertions require at least 10 decoded frames. Audit and idle runs pass.
- **FAIL / retained blocker:** `npm test` stops at intermittent camera reception after
  23 successful checks. No stale success summary or weakened assertion masks it.
- Reviewed gallery: 178 browser screenshots in 18 contact sheets, including explicitly
  labeled prior-success camera captures. Final artifact: `a520c4ee8c84107af622`.
- Implementation commits: `5910edb`, `31a6d1b`, `8d6fece`, `43d4bcd`, `3c19bfc`.
  Final harness/evidence commits are discoverable in this branch history.
- Target peak observed: 5.912 GiB, below the 10 GiB limit. **Cleanup blocked by the
  execution tool policy:** guarded native PowerShell deletion of the exact verified
  `C:/Users/dwhee/.cargo-target/sticktalk-qa-web` directory was rejected. It remains
  for owner/lead cleanup; no alternate deletion bypass was attempted. All task-owned
  browser/server processes and preview tabs are closed.

Machine-readable validation: [validation.json](../crates/web/screenshots/qa/validation.json);
[retained command logs](../crates/web/screenshots/qa/validation-logs/).
