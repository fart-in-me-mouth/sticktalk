# Share fix — handoff

Branch: `t3/fix-video-share-performance`.

**Not fully accepted on the final code. Stopped at the lead's wrap-up request.**
Both completed 600-second 1080p runs passed on an earlier candidate; they must
be repeated after the final CodecAPI change. Moving 1440p still freezes.

## Completed numbers

All runs use two hardware-decoder processes, NVIDIA RTX 3060 Ti, loopback SFU,
8-second warmup, no OS windows. FPS is actual decoded output. Drift is the
change in wall-time minus RTP-time offset, **not glass-to-glass latency**.

**Earlier candidate `68484bd` (security master already merged), 600 seconds:**

| Source / endpoint | FPS avg / min 1 s | Drift ms | Payload Mbps |
|---|---:|---:|---:|
| Moving synthetic / sender | 59.994 / 56 | — | 8.082 |
| Moving synthetic / B | 59.993 / 56 | −0.534 | 8.082 |
| Moving synthetic / C | 59.993 / 56 | −0.542 | 8.082 |
| Primary WGC / sender | 60.001 / 59 | — | 1.122 |
| Primary WGC / B | 60.002 / 59 | +0.166 | 1.122 |
| Primary WGC / C | 60.001 / 59 | +0.167 | 1.122 |

Both: **0 encoder reinitializations**. Sender CPU: **0.477% / 0.305%** of the
machine; working-set delta **62.91 / 69.76 MiB**, private delta **108.11 / 90.18
MiB**. WGC was mostly static (774 acquisitions, repeated GPU frames); the
synthetic source supplies continuous reproducible motion. These are decode-only
runs, not a ten-minute rendered-UI test.

**120-second checks:**

| Code / source / quality | B FPS avg/min; drift | C FPS avg/min; drift | Sender Mbps; reinitializations |
|---|---|---|---|
| Earlier candidate / WGC 1440p60 | 59.999/59; −0.613 ms | 59.999/59; −0.624 ms | 1.692; 0 |
| Earlier candidate / WGC Source 60 | 59.992/59; −0.277 ms | 59.999/59; −0.279 ms | 1.692; 0 |
| Final production code (`9e41815`) / moving 1440p60 | 60.000/59; −0.213 ms | **59.500/14; −10.459 ms** | 12.308; 0 |

Source here is **2560×1440**, not 4K. The last row **fails** the minimum-window
requirement. Its sender was 59.996/59 FPS, CPU 0.523%, WS delta 64.23 MiB,
private delta 117.91 MiB. Final Source and 1080p long runs were not started.

## Evidence and changes

- Merged `t3/calls-video-control`, then master `a9c0f13`, retaining modular UI,
  audio settings and session-only S5 overload handling. Fixed the E2E fixture
  to honor one-use Google nonces, reauthentication and rotated sessions.
- **Thrash removed:** historical 1,150 MFT rebuilds/600 s → 0. Bitrate changes
  use [ICodecAPI MeanBitRate](https://learn.microsoft.com/en-us/windows/win32/codecapi/avenccommonmeanbitrate-property),
  >15% hysteresis and ≥2 s spacing; format changes alone replace the transform.
  Applying controls after media types was necessary on this MFT. The hardware
  test reduced equal-length payload windows **2,229,403 → 660,465 bytes**,
  with 0 rebuilds, then verified one 720p30 format rebuild.
- **Not thrash alone:** controlled debug runs remained 7.165 → 7.163 FPS after
  removing rebuilds. Sender gates discarded ~2,000/2,278 frames; SFU forwarded
  ~98% of the bytes it received. Windows coarse deadlines delayed str0m's
  microsecond drain requests. Added one-shot high-resolution waits, including
  Winsock readiness in the native actor; no global timer period or idle polling.
- Added deterministic offscreen D3D11 acquisition through the same converter,
  encoder and transport, per-hop/BWE/NACK/PLI/FIR/keyframe counters, bounded
  100 ms H.264 HRD tuning, aggregated ≤1/s keyframe requests, and atomic wire
  budget admission with original-media headroom. Global/peer/room caps remain
  **32/32/64 Mbps**; ACL and overload cleanup are unchanged.

**Remaining:** moving 1440p showed original-media cap drops **103 / 163**, plus
4,310 queue drops on one viewer, despite zero MFT rebuilds. The pacer snapshot
also includes RTX/probes. An attempted separate-media queue / larger reserved
burst regressed to **55.406/6 and 56.189/11 FPS**, and was **reverted**. Its logs
are retained as `experimental-mq-1440-120`. Investigate actual pacer backlog,
probe accounting and cap/pacing interaction next; do not merge that experiment.

## Validation and artifacts

Final production sources are byte-identical to validated `9e41815` after the
experiment revert. **321 normal tests**, **1 live-bitrate hardware test**, and
**2 offscreen GPU tests** passed; clippy and fmt passed:

```powershell
$env:CARGO_TARGET_DIR='C:/Users/dwhee/.cargo-target/sticktalk-share-fix'
$env:CARGO_INCREMENTAL='0'
$p=@('-p','sticktalk','-p','sticktalk-media','-p','sticktalk-sfu','-p','sticktalk-server')
cargo test --profile server @p --lib --bins --tests -j2 -- --test-threads=1
cargo clippy --profile server @p --all-targets -j2 -- -D warnings
cargo fmt --all -- --check
```

Reviewed **22 headless PNGs** in `../../screenshots/share-fix`; final renders
match the individually reviewed hashes. GPU import returned `[130,130,130,255]`
with **0 production streaming CPU pixel copies**. No real browser, camera, mic,
sound, notifications or input injection; captured desktop pixels were not saved.

`summary.json`, `bwe-series.csv` and per-endpoint JSON retain every second; `trace-logs.zip`
contains raw diagnostics and validation logs. `summarize.py` reads either raw
logs or that archive. The dedicated target directory was deleted at handoff.
