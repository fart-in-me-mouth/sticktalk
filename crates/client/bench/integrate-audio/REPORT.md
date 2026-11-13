# Audio round 2 integration — 2026-11-13

Merged `t3/voice-settings-silence-latency` into the integrated desktop UI.

- The sole Voice & Video view is `ui/settings/voice.rs`, using the current nav, content shell, dropdown styles and shared 40×24 toggles. Removed the older page. Voice preferences apply immediately and persist with a debounce, like the other app-preference pages; profile/server unsaved-change behavior is untouched.
- Retained all round-2 audio/device controls, mic test, recorder, per-user settings, fallback notices, silence gating, adaptive framing and latency harness. Connected local volume/mute to the current main-window user context menu.
- Voice settings subscriptions use the named page, not the former index (now Privacy). Leaving via settings navigation stops the meter/recorder but preserves the call. Escape cancels recording first. Added regression tests for these lifetimes, persistence and fallback notices.

## Validation

`cargo test -p sticktalk -p sticktalk-client-core -p sticktalk-media`: **180 passed, 3 ignored, 0 failed**. The existing hardware/file-video test remains opt-in.

`cargo clippy -p sticktalk -p sticktalk-client-core -p sticktalk-media --all-targets -- -D warnings` and `cargo fmt --all -- --check`: passed. Logs are alongside this report.

Headless tiny-skia only, no OS windows: **204 PNGs**, 1280×800 and 1920×1080, under `screenshots/audio2` (7 views, including the six requested states plus processing), `screenshots/settings` (52), and `screenshots/main-parity` (43). Reviewed every image in contact sheets, all audio images at full size, and representative main images at full size against UI_SPEC §1–§7. No new intentional visual deviations. Current mock timestamps refresh older snapshots.

Target peak/final: **8.14 GiB** with `CARGO_INCREMENTAL=0`, no release build; task target removed after validation.

## Not requalified

This integration does not claim physical Bluetooth/hot-plug, acoustic latency/quality or desktop idle-RAM acceptance. Round-2 benchmark limitations, including High-mode talking CPU and physical mouth-to-ear latency, remain documented in `../audio2/REPORT.md`; no new performance measurement is claimed.
