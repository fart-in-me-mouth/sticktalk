# Login/channel polish and native RAM audit

Branch: `t3/login-channel-ui-polish`. Master merged at `8c3fbfd` (includes `f91944d`); both server-search/emoji/slowmode/folders and this branch's UI/probes retained. Final head is in the handoff.

## Shipped UI

- First launch: removed the stray Done label, shared full-width Google G secondary button and OR divider, capability-aware enablement, inline Connected/check and error states. Enter connects; stale metadata cannot mark an edited address connected.
- Channels settings: added navigation entry, uppercase muted 12px category headers/collapse chevrons, indented normal-weight hash/speaker/lock channel rows, hover highlight and gear/delete/up/down sibling actions. Per-category Create Channel and Create Category controls; permission/boundary checks; editor returns to Channels.
- Sweep fixes: visible connecting skeleton contrast, specific destructive confirmation titles/action labels, useful filtered-members empty state with Clear filters, no empty filtered navigation headings, correct member-menu capture anchor. Collapse state prunes deleted/unknown categories.
- Post-merge capture matrix: **458 PNGs / 229 scenes**, each at 1280×800 and 1920×1080, all headless. See `../../screenshots/manifest.json` and `polish2-capture.json`; reviewed every contact-sheet tile and primary changed screens at full resolution. No edits to the other agents' feature implementations.

## RAM result

**The <100 MiB idle working-set target is not achieved.** No rendering-quality/backend reduction, working-set eviction, or cache-flushing trick was shipped. The LFH reclamation experiment did not materially improve memory and remains feature-gated, off by default. Default idle has no new collection timer.

Measurements: optimized release, system allocator, real iced/wgpu DX12 swapchain at 1280×800, secondary-monitor/no-activate window. Same mock history fixture and isolated LOCALAPPDATA. 15s warmup; 60s per sample; 2,000 messages traversed through history transport and scroll operations (1,000 retained). Emoji picker and fixture GIF picker measured separately. WS/private are endpoint counters, not averages; CPU is process CPU over each interval, normalized to one core. Mock backend's 5,000 synthetic messages are included, not subtracted. This is not a connected production-server measurement.

Final post-merge numbers are in `after-native.json`; baseline in `before-native.json`, exact executable SHA256 and placement/focus confirmation in companion files.

Memory columns are MiB. Single runs, not a statistically isolated performance comparison (final includes master features).

| Scenario | Before WS | After WS | Before private | After private | Before CPU | After CPU |
|---|---:|---:|---:|---:|---:|---:|
| Idle main chat | 139.80 | 139.88 | 153.47 | 153.69 | 0.078% | 0.000% |
| After 2,000 messages | 142.22 | 142.27 | 156.05 | 156.14 | 0.156% | 0.156% |
| Emoji picker open | 145.93 | 147.79 | 169.83 | 169.98 | 0.130% | 0.260% |
| GIF fixture picker open | 148.09 | 149.89 | 179.96 | 180.02 | 0.104% | 0.130% |
| Pickers closed | 148.42 | 150.30 | 180.02 | 180.11 | 0.052% | 0.078% |

Rejected LFH experiment: idle **139.66 / 153.77 MiB** WS/private versus baseline **139.80 / 153.47 MiB**; six successful collections, no meaningful gain. `lfh-candidate-native.json` is not the final/default build.

## What the profile found

- Added opt-in `heap-profile` DHAT allocation stacks, stripped-release MSVC map symbolization, and native GetProcessMemoryInfo + VirtualQuery/QueryWorkingSetEx region reports. Diagnostic RAM/CPU must not be compared to shipping RAM/CPU.
- Offscreen retained Rust heap: **3,326,594 bytes**, peak **3,822,709**, 6,515 allocation sites. Native full-stack diagnostic peak: **9,700,036 bytes**, 12,409 sites; shutdown live heap **2,211,467 bytes** (not an idle checkpoint). Native driver/C heaps and mapped images are outside DHAT. Optimized/inlined frames are not always recoverable; raw and symbolized stacks are preserved, not inferred site names.
- Baseline native idle: private resident about **68.2 MiB**, executable image about **20.0 MiB**, NVIDIA compiler about **14.8 MiB**, NVIDIA driver about **11.1 MiB** resident. Region data is in the JSON, not a claim that all private committed bytes are live Rust allocations.
- 298 font database faces; seven bundled fonts total **848,684 bytes**, registered once. Decoded image cache is **0 bytes at idle**, already bounded to 16 MiB/64 entries with LRU. Audio inactive throughout; media DLL resident pages total under 1 MiB. Tokio is configured for two workers/four maximum blocking threads; native process observed at 14 total threads. No client SQLite cache (SQLite belongs to the server).
- Renderer source inspection: eager image atlas starts at 2048×2048 RGBA (~16 MiB GPU storage); configured staging chunk sizes are 100 KiB (renderer), 2 MiB (raster), 4 MiB (image worker), allocated on use. Glyph atlas initial size is 256×256. These are upstream/private renderer knobs; no unapproved workspace renderer fork or lower-quality change was made in this timebox. Actual GPU heap/atlas occupancy was not instrumented and GPU storage is not equated with process private bytes.

## Validation and cleanup

Passed after the merge (logs alongside this report):

```text
cargo build --release -p sticktalk
cargo test -p sticktalk -j 4 --offline -- --test-threads=2
cargo clippy -p sticktalk --all-targets -j 4 --offline -- -D warnings
cargo clippy -p sticktalk --all-targets --all-features -j 4 --offline -- -D warnings
cargo test -p sticktalk --lib --features windows-heap-collection -j 4 --offline resources::tests -- --test-threads=1
cargo fmt --all -- --check
git diff --check
```

Default suite: **166 passed, 10 opt-in hardware/window/long-running tests ignored**. Heap-collection live-data test also passed with the actual Windows experiment enabled. An initial boundary test incorrectly assumed the third text channel was the last desktop sibling; fixed it to derive the last sibling including voice/private rows, then reran the full suite. No test was weakened to allow an invalid reorder.

Reproduction tools: `tools/measure-polish2.ps1`, `tools/capture-polish2.ps1`, `tools/summarize-heap.py`. All builds used the single `C:/Users/dwhee/.cargo-target/sticktalk-polish2` directory with `CARGO_INCREMENTAL=0`; peak release tree approximately 4.21 GiB, dev/validation tree below 8 GiB. Release tree removed before dev validation; entire task target removed after validation. Native probe windows closed automatically and foreground was unchanged.

Remaining: target-level RAM reduction needs a renderer/native-allocation investigation, not further shaving the small retained Rust heap. Access/audit-log settings remain existing feature placeholders; no server work added here. Old `main-local-search`/`audit-search-empty` fixtures no longer represent results after master's server-search migration; `server-search` and `server-search-empty` capture the new panel.
