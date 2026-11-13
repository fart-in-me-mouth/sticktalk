# Calls2 qualification — 2026-11-12/13

**Acceptance: FAIL. Not launch-ready. Viewer throughput and latency-drift
requirements remain unmet; CPU and RAM requirements pass.**

## Final 600-second 1080p60 result

| Endpoint | FPS avg / min 1 s | Drift ms | Machine CPU | Working set MiB (delta) | Private MiB (delta) | GPU engine mean |
|---|---:|---:|---:|---:|---:|---:|
| Sharer | 59.999 / 59 | n/a | 0.633% | 120.10 (+101.85) | 130.17 (+127.55) | encode 10.830%, 3D 0.960% |
| Viewer B | 12.837 / 0 | +166.447 | 0.126% | 84.53 (+66.22) | 87.43 (+84.76) | decode 1.620% |
| Viewer C | 12.950 / 0 | +169.988 | 0.115% | 84.45 (+66.18) | 87.71 (+85.07) | decode 1.615% |

Sharer peak WS: **128.02 MiB**. Payload bitrate: sharer **0.993 Mbps**, viewers
**0.201 / 0.203 Mbps**. Capture encoded 36,000 measured frames, but reported
**1,150 encoder reconfigurations** across the run. Delivery/pacing/rate-feedback
needs further instrumentation; these numbers do not identify a proven single
remaining cause. Enlarging endpoint UDP buffers from the measured **65,536**
to **262,144 bytes** did not resolve it. No acceptance threshold was weakened.
Historical failed attempts are retained as `release-attempt1-*` and
`release-attempt2-*`; the unbounded sender backlog was fixed, not the FPS failure.

Method: stock optimized release, 8-second warmup, real invisible primary-monitor
WGC → NVIDIA H.264 Encoder MFT / RTX 3060 Ti → real SFU on 127.0.0.1 → two
hardware-decoder processes. CPU is whole-machine normalized. Memory deltas use
pre-media startup baselines. External coordinator PDH counters sum per-process
engines, not whole-adapter utilization. Drift is last-minute minus first-minute
mean wall time relative to RTP time, **not** glass-to-glass latency. The long test
is decode-only, without rendering; desktop content was uncontrolled, not a
reproducible moving-game workload.

## Implemented / evidence

- GPU NV12 plane shader → NT shared BGRA texture/fence → iced DX12 primitive;
  bounded leases, screen/camera tiles, focused strip, fullscreen/pop-out, stats,
  logical window/channel visibility unsubscription.
- Refreshed GPU picker snapshots, measured tiny CPU fallback, both source tabs,
  application-process audio, camera chooser, persisted default 1080p60 and atomic
  start/mid-stream quality (not reset by later bitrate hints).
- Consent/timeout, bundled native Stop banner/injector/hotkeys, physical takeback,
  independent emergency release, held-input cleanup, stale-grant rejection,
  focused viewer capture/Esc hold. Bounds use visible DWM frames rather than
  invisible resize borders ([Microsoft reference](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getwindowrect)).
  Construction/recording-mock tests cover bounds, DPI, negative origins, revoke,
  queue saturation and transport/control-channel failure.
- Offscreen hardware pixel test passed: `[130,130,130,255]`; **0 production
  streaming CPU pixel copies**. Picker GPU snapshot mean **36.87 ms**; CPU
  fallback **58.82 ms**, 57,600 bytes per 160×90 preview.

## Validation / limits

**87 normal tests passed**, 6 ignored by the ordinary suite; **2 explicit GPU
tests passed**. The explicit 10-minute acceptance test **failed**. Clippy
all-targets with warnings denied and fmt passed. Commands (one release tree):

```
cargo test --release -p sticktalk-media -p sticktalk -j2 -- --test-threads=1
cargo clippy --release -p sticktalk-media -p sticktalk --all-targets -j2 -- -D warnings
cargo fmt --all -- --check
cargo test --release -p sticktalk --test video -- --ignored --nocapture --test-threads=1
cargo test --release -p sticktalk --test voice live_calls2_two_viewers_ten_minutes -- --ignored --exact --nocapture
```

Reviewed all **22** tiny-skia PNGs at 1280×800 and 1920×1080 against UI_SPEC §8;
re-shot label/strip clipping and reviewed the final changed camera/banner frames.
Camera avatars/taskboard are privacy-safe fixtures; banner PNG is iced, not the
real OS bar. **No real SendInput, visible OS window, mic/camera, sound or
notification was used. Captured desktop pixels were not saved.**

Remaining: failing end-to-end performance; DX12/same-adapter display only;
no 10-minute rendered-UI test or live native-control/window/game qualification.
Decode ms is CPU MFT-call wall time; dropped counts cover local display delivery,
not missing RTP access units. Visibility is logical view/window, not per-pixel
occlusion of scrolled tiles. These are not complete production acceptance claims.

Release target peak **8,354,432,691 bytes**, below the hard 10 GB limit;
`CARGO_INCREMENTAL=0`. `cargo clean --release` ran immediately after each
measurement. Final whole-target deletion recorded in `clean-target.log`.
