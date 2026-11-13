# Live web QA — mobile parity and split WASM

Production revision `4ad95a5c0079c5550716`. **Partial B8, not launch-complete.**
Real Rust server, binary WS/SFU, loopback-only, headless Chromium; fake AV devices,
muted OS audio and mocked push permission/provider. No visible windows.

## Changes / evidence

- Seven-minute author grouping, compact continuation rows and hover gutter time;
  measured virtual-scroll anchors and stale-navigation guards.
- Camera tiles cover 16:9; shares contain on black; bottom-left names, 2 px mint
  speaking outline, responsive two-tile layout and channel/chat-icon header.
- Four-trigger keyboard autocomplete; desktop settings / phone list-to-subpage
  navigation, profile/avatar editing, hoisted-role members with loaded-page counts,
  invite create/join/copy and group-DM create/manage.
- Gesture-lazy viewer: zoom, pan, two-pointer pinch, save and open original.
- Enabled direct KLIPY mode opens only exact API/media CSP hosts; proxy/disabled
  modes remain closed. Server mode/CSP test and bounded provider-adapter unit test.
- Leptos `#[lazy]` screen/codecs/data splitting; lazy viewer/GIF/media JS; removed
  production pulldown-cmark dependency. Twiggy audit: [top retainers](twiggy-top.txt).
- z / fat LTO / abort, `wasm-opt -Oz --strip-debug`, static boot skeleton, parallel
  session/compile, versioned split packages. Worker precaches shell/core; HTML SWR,
  versioned assets cache-first, secondary modules on demand, reload-gated updates.

**28 real-server checks**, **0 application errors**, **0 CSP violations**.
Includes 55-row history/seek, bounded 50 mounted messages, 17 MiB resumable upload,
two-process real SFU audio/video and a third isolated account for group DMs.
[Results](results.json) · [RTP](voice-stats.json).

**54 PNGs**: login, chat, grouped-chat, voice, settings, profile-settings, members,
autocomplete, image-viewer at **390×844, 412×915, 820×1180, 1440×900, 1280×800,
1920×1080**. Opened/reviewed every screen/size in contact sheets plus representative
full images against UI_SPEC §8; fixed avatar alignment, call error toast, scroll
anchoring, Logout color and phone composer sizing before retaking.
[390 grouped chat](grouped-chat-390x844.png) · [390 call](voice-390x844.png) ·
[390 settings](settings-390x844.png) · [1440 profile](profile-settings-1440x900.png).
This is the nine-screen web matrix, not all §6 launch screens. Existing muted-text
AA override (#A0A0AA) remains. Member counts cover loaded pages, with Load More.

## Performance / sizes

Lighthouse **13.6.0**, real authenticated URL. Cold uses mobile simulated throttling;
warm uses DevTools 4× CPU / 150 ms / 1.6 Mbps with HTTP/SW caches retained.
Only one authenticated tab is present per run (one WS per cookie session).
Earlier warm samples with competing same-session tabs are superseded.

| Page | Perf / accessibility / best practices | TTI |
| --- | --- | --- |
| Cold mobile login | 100 / 100 / 100 | 1.441 s |
| Cold authenticated mobile chat | 87 / 100 / 100 | **4.018 s** |
| Warm authenticated mobile chat | 95 / 100 / 100 | **0.949 s** |
| Desktop login | 100 / 100 / 100 | 0.252 s |

**Both chat targets missed** (≤2.0 s cold / ≤0.8 s warm). Skeleton paint is not
chat interactivity. Cold chat LCP 4.011 s, TBT 39 ms, CLS 0; warm LCP 0.970 s,
TBT 249 ms, CLS 0. Further core/boot reduction is required.
[Summary](lighthouse-summary.json) · [warm audit](lighthouse-chat-warm-mobile.json).

- Core WASM: **1,797,367 raw / 421,493 Brotli bytes** (previous core 526,282 Brotli).
- **18 lazy WASM chunks: 174,945 Brotli bytes**: shared 43,453; autocomplete 1,293;
  dialog REST 8,067; shortcode 1,011; voice events 43,503; GIF 5,444; call UI 1,762;
  dialogs/settings/roles/admin 36,135; profile 1,051; upload 9,646; voice actions 23,580.
- All-assets Brotli **756,005 bytes**; installable shell inventory **580,049 bytes**,
  not actual cold transfer. [Every chunk and asset](bundle-sizes.json).
- 60.02 s inactive Friends: **0.086%** of one logical CPU. Whole Chromium tree
  working set **252.2 MiB**, private **110.6 MiB**, private delta **36.0 MiB**.
  Shared pages double-count in working sets; not a native/physical-phone RAM pass.
  [Idle sample](idle.json).

## Validation passed

- `cargo test -p sticktalk-web -p sticktalk-server` — **110 tests**.
- `cargo clippy -p sticktalk-web -p sticktalk-server --all-targets -- -D warnings`.
- `cargo clippy -p sticktalk-web --target wasm32-unknown-unknown --all-targets -- -D warnings`.
- `cargo fmt --all -- --check`; `git diff --check`.
- Split production WASM build; `npm test`; `npm run test:server`; `npm run audit`;
  `npm run test:idle`; `npm audit --audit-level=high` (**0 vulnerabilities**).

## Remaining / cleanup

- Full native settings/admin/profile-popout parity, role reorder and rich GIF
  embeds/favorites/categories remain. Viewer forwarding/Copy Image not exposed.
- No real KLIPY key/provider delivery, OAuth rerun, physical phone/Safari,
  display-capture/device-switching, large-original decode-memory or impaired/large
  call certification. Full offline/multi-tab version-upgrade QA remains.
- Incremental disabled; no Rust rebuilds after the urgent disk request. Final target
  **6,037,403,576 bytes** (<10 GB), node_modules **189,239,828 bytes**. Verified exact
  paths, but native PowerShell `Remove-Item -LiteralPath ... -Recurse -Force` was
  **blocked by tool policy** for both directories, including target-local browser
  caches. Earlier debug/release cleanup was also blocked; artifacts remain and need
  owner/lead cleanup. No alternate deletion mechanism used to bypass that block.
