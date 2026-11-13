# Desktop media/platform implementation handoff

## Status (not a beta-complete feature bundle)

1. **Emoji: implemented.** Offline `sticktalk-emoji` search/category grid (visible rows plus overscan), decayed per-account recents, five skin tones, hover name/shortcode footer, composer/reaction picker, caret-based two-character autocomplete with keyboard completion, shortcode replacement before send. Bundled CC-BY Twemoji 17.0.3; newer Unicode 18 entries without artwork use readable names rather than tofu. The pack index and common toned/ZWJ sequences have regression tests.
2. **GIF: partial.** KLIPY direct public-key and authenticated proxy metadata paths; bounded search with 250 ms debounce/stale-result rejection, categories, server favorites, 24-result pages/96-item retained window, attribution and keyboard selection. Image GIF/WebP messages display inline; visible/focused/hover/never policy controls one-shot frame subscriptions, at most eight 4 MiB clips. **Not done:** MP4/WebM previews (the default provider renditions), hardware container playback, uploaded animated attachments, disk caching, masonry layout. Consequently production video-format tiles still show a static unavailable-preview placeholder. This must not be described as a finished GIF experience.
3. **Viewer: partial.** Full-window 85% backdrop, explicit original fetch, off-thread bounded decode, fit/10–800% cursor-relative zoom, drag, double-click/keyboard reset, attachment navigation, save, browser confirmation, clipboard image/link and channel forwarding. Close cancels the request and releases original handles; late results are ignored. Image-format KLIPY URLs open in the viewer. **Not done:** blurred loading thumbnail, embed images, unified attachment/GIF navigation, animated viewer playback. External video GIFs cannot decode as images.
4. **Uploads/media: upload portion implemented; video not done.** Two independent workers, 64 KiB source reads, up to 16 MiB acknowledged chunks, SHA-256 finish, per-server/account token-free checkpoint journals, server-authoritative offset after restart, source-change detection, progress/speed/ETA/cancel/retry, channel-switch survival, completed-attachment recovery and exact storage-full error. Forwarded attachment uploads send when complete. Existing real-server tests cover >25 MiB resume/range downloads; added tests cover stale offsets, bad acknowledgements, storage-full wording, channel switching and journal serialization. **Not done:** MP4/WebM inline hardware player, audio/seek/fullscreen controls, expired-session automatic re-creation. No CPU HD decoder was added.
5. **Platform: partial.** Shared pre-window/iced Tokio runtime, owner-scoped single-instance IPC, invite/message activation routing, server-synced notification policy, bounded toast delivery, unread taskbar overlay/mention flash, authenticated update watch/background staging and confirmed restart pill. Mock/headless/agent/dry-run paths do not invoke native effects; dialogs/clipboard/browser are mocked. Updates require the owner's embedded production public key. **Not done:** tray/WndProc/Explorer restart integration, close-to-tray/startup preferences, call mute/deafen actions, custom sound delivery (no reliable host DND confirmation), native effect acceptance tests. Existing tools/release already build/include both the new `sticktalk.exe` and maintenance helper; no installer registration or app launch is performed here.
6. **RAM: investigated; target and no-regression gate NOT achieved.** DX12/system allocator remain the shipping defaults. Vulkan/mimalloc reduced one headless sample to 101.73 MiB working set / 132.19 MiB private, but regressed scroll timing and were rejected as defaults. Optional event-driven allocator collection uses one idle timeout, never per frame. MSAA remains off; caches stay bounded; no software-renderer downgrade or forced working-set eviction. The idle resource harness retains the actual view/cache/GPU renderer but has **no OS surface/swapchain**, so its numbers must not be compared directly to the owner's native 141 MiB working-set / 160 MiB private baseline. Native/minimized and real monitor-cadence measurements remain outstanding. Stock iced/wgpu glyph-atlas and staging allocation knobs are not exposed without renderer changes outside this task's crates.

## Paired RAM / scroll evidence

Same-PC paired debug comparison, DX12/system allocator, implementation commit `43b7259` versus original main/app/ui/theme from `f29af5f`, with identical shared harness/core. All original source files were restored byte-for-byte after building the control. Host was not isolated from owner/other-agent activity; see `paired-method.json`.

| Measurement | Before | After |
| --- | ---: | ---: |
| Headless working set, MiB | 162.13 | 162.52 |
| Headless private bytes, MiB | 201.30 | 200.39 |
| Wheel p50 / p99, ms | 5.749 / 15.349 | 6.142 / 17.470 |
| Seek p50 / p99, ms | 60.558 / 97.005 | 62.161 / 259.219 |
| Skeleton p50 / p99, ms | 1.891 / 4.124 | 2.158 / 4.647 |

These results do **not** establish the required no-regression gate, particularly seek p99. Earlier candidate/backend experiments are retained separately, not substituted for this paired result. The hardening commit `38b7d34` changes upload/download/picker inactive paths and does not have a new paired benchmark. Production native memory/scroll acceptance is outstanding.

Final release-only headless sample (`38b7d34`, DX12/system): **119.87 MiB working set / 199.59 MiB private**. This is not an A/B comparison with the debug table or the native owner baseline.

## Validation

- `cargo test -p sticktalk -p sticktalk-client-core --offline`: **65 tests passed** (9 library, 25 desktop, 3 loopback live, 28 core). Includes actual native worker checkpoint completion/resume, atomic replacement downloads, shortcode replacement/caret, notification policy and worker identity regressions.
- `cargo clippy -p sticktalk -p sticktalk-client-core --all-targets --offline -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo build --release -p sticktalk -p sticktalk-desktop-platform --bins --offline`: passed. NSIS 3.13 packaged both binaries with the new emoji artwork attribution/full license: **9,780,276 bytes (9.33 MiB)**. Exact byte counts and hashes are in `installer.json`; installer was never run.
- Screenshots are **mock fixtures of real iced trees**, not proof of live provider/hardware/native OS integration. Fixtures include working previews to inspect intended layout despite unimplemented MP4/WebM rendering.

## Integration

- New modules: `features.rs`, `ui/{emoji_picker,gif_picker,viewer,uploads,media_settings}.rs`, `uploads.rs`, `animated.rs`, `gifs.rs`, `platform_glue.rs`, `resources.rs`; pure shared emoji/GIF/notification/upload policy in client-core.
- Shared-window/settings edits are additive routing/view calls. `ui::media_settings::view` can move into the settings agent's final Media page. `features::update_pill` belongs next to/above the final user panel.
- `--dry-run-platform` disables IPC and native notification/badge effects. Headless flags are processed before platform bootstrap. No visible windows, browser, toasts, tray icons, sounds, registry changes or installer execution were used for proof.
- `capture.ps1 -Binary <absolute sticktalk.exe>` generates 28 headless PNGs with a checksum manifest under `screenshots/media/`.
- `--mock --bench-resources <json>` measures idle offscreen RAM; set `STICKTALK_BENCH_BACKEND=dx12|vulkan`. `--mock --bench-scroll <json> --gpu` runs the existing 5,000-message scroll workload. These are not physical monitor cadence tests.
- Target directory for every Cargo operation and portable NSIS: `C:/Users/dwhee/.cargo-target/sticktalk-media-ui`; removed after final validation. No push and no history rewrite.
