# StickTalk media: real Windows benchmark

Measured **2026-11-11**, release/MSVC, on this PC. These are actual WGC capture →
D3D11 Video Processor → NVIDIA hardware H.264 → Media Foundation D3D11VA decode
runs, not estimates or generated input fed straight into the encoder.

**Standalone 1080p60 proof: PASS.** Final 30.005 s interval achieved **59.99 encode
and decode fps**, zero dropped output frames, 4.522 ms mean / 6.021 ms p99 encoding
latency. Sender-only budget: **0.254% total-machine CPU (4.061% of one core)** and
**64.01 MiB / 67.12 MB RSS**, below both the <5% CPU and <150 MB sender targets.
The later, stricter-decoder full round trip used 0.631% total CPU / 94.27 MiB RSS.
This does **not** establish the owner's later ten-minute game/video + two-viewer
network acceptance test, which is outside this standalone, no-network task.

## Machine / detected codecs

- Windows 11 Home, **10.0.26200**, x86_64-pc-windows-msvc.
- CPU: **AMD Ryzen 7 5700X**, 8 cores / **16 logical processors** (WMI).
- GPU: **NVIDIA GeForce RTX 3060 Ti**, driver **32.0.16.1088** (WMI), also identified
  by the actual D3D11 device/adapter used by the pipeline.
- Other WMI display adapter: Meta Virtual Monitor. No claim that every enumerated
  adapter provides a media encoder.
- Primary captured monitor: **2560x1440**, mode reported as **280 Hz** by
  EnumDisplaySettingsW. Secondary: 1920x1080 @ 75 Hz. Thus Source and 1440p have
  identical dimensions on this PC; **no 4K Source claim** is made.
- `MFTEnumEx(MFT_ENUM_FLAG_HARDWARE | SORTANDFILTER)` returned:
  - H.264: **NVIDIA H.264 Encoder MFT** — actually used in all hardware runs.
  - HEVC: **NVIDIA HEVC Encoder MFT** — availability only, not encoded here.
  - AV1: **none**. AMF/QSV: none registered on this machine.
- Decoder: Microsoft H.264 decoder MFT with IMFDXGIDeviceManager / D3D11VA. Final
  verification additionally requires an H.264/NV12 hardware decoder configuration
  and **D3D11_BIND_DECODER** output textures; CPU-memory/upload outputs are refused.

## Four-quality matrix: real 30-second intervals

All rows include capture, encode **and decode** in one process. Hardware encoder:
NVIDIA H.264 Encoder MFT, requested High profile, low-latency, configurable GOP,
CBR. Matrix rows ran sequentially; the renderer is not integrated. Raw output:
[bench-results/matrix.log](bench-results/matrix.log).

| Quality | Size | Seconds | Capture fps (unique WGC) | Encode fps | Decode fps | Mean/p99 encode ms | Drops | Repeats | Actual Mbps | CPU total % / one-core % | RSS / peak MiB | Encoder |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|---|---|---|
| P720@30 | 1280x720 | 30.007 | 59.99 | 29.99 | 29.99 | 3.452/7.868 | 0 | 0 | 1.793 | 0.179/2.864 | 86.11/86.11 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |
| P1080@60 | 1920x1080 | 30.000 | 112.50 | 60.00 | 60.00 | 4.582/6.025 | 0 | 3 | 8.173 | 0.413/6.614 | 109.52/109.52 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |
| P1440@60 | 2560x1440 | 30.001 | 118.73 | 60.00 | 60.00 | 7.324/7.881 | 0 | 0 | 7.292 | 0.397/6.354 | 120.84/123.39 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |
| Source@60 | 2560x1440 | 30.006 | 118.88 | 60.02 | 60.02 | 7.331/7.865 | 0 | 0 | 7.341 | 0.397/6.353 | 136.28/143.98 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |

The measured capture rate counts **WGC frames actually acquired** (not repeated
outputs). Several source updates can be coalesced into one paced output. At 60 fps
we acquired roughly 112–119 frames/s from this high-refresh monitor; those extra
source updates are intentionally coalesced, **not** encoder drops. The 1080p row
repeated only 3 frames across ~1800 outputs; unique-paced output was still >59 fps.

## Final 1080p60 verification with strict D3D11VA surface checks

Fresh process; added decoder hardware-configuration / decoder-bound-surface
checks enabled. Diagnostic RGBA readback ran **after** measurement and returned
8,294,400 bytes (checksum 1,068,159,264). It was never on the measured or production
path. Raw output: [bench-results/final-1080.log](bench-results/final-1080.log).

| Quality | Size | Seconds | Capture fps (unique WGC) | Encode fps | Decode fps | Mean/p99 encode ms | Drops | Repeats | Actual Mbps | CPU total % / one-core % | RSS / peak MiB | Encoder |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|---|---|---|
| P1080@60 | 1920x1080 | 30.005 | 113.28 | 59.99 | 59.99 | 4.522/6.021 | 0 | 4 | 8.123 | 0.631/10.103 | 94.27/94.27 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |

Four repeated outputs in ~1800 frames, zero scheduled/queue drops. Even subtracting
repeats, unique-paced output exceeds **59.8 fps**, above the ≥58 fps acceptance
threshold. Full-roundtrip RSS 94.27 MiB = 98.85 MB. CPU is reported both ways:
0.631% of the entire 16-thread machine versus 10.103% of one logical core. The
<5% **sharing-PC** budget is best judged by the separate sender-only run below,
which also passes a stricter single-core interpretation.

## Sender-only budget (capture + GPU conversion + hardware encode)

Fresh process, decode disabled, moving GPU fixture in a separate process. Actual
source still captured with WGC; encoded bytes drained without network or file
I/O. [Raw output](bench-results/sender.log).

| Quality | Size | Seconds | Capture fps (unique WGC) | Encode fps | Decode fps | Mean/p99 encode ms | Drops | Repeats | Actual Mbps | CPU total % / one-core % | RSS / peak MiB | Encoder |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|---|---|---|
| P1080@60 | 1920x1080 | 30.008 | 112.80 | 59.98 | 0.00 | 4.591/7.356 | 0 | 6 | 7.900 | 0.254/4.061 | 64.01/64.01 | Hardware { name: "NVIDIA H.264 Encoder MFT" } |

- **CPU:** 0.254% normalized over all 16 logical processors; **4.061% of one core**.
- **RSS / sampled peak / process lifetime peak:** 64.01 MiB (~67.12 decimal MB).
- **1080p60 target:** PASS (<5% CPU, <150 MB, ≥58 fps).
- Output: 59.98 fps, 7.900 Mbps; no drops, 6 repeats / ~1800 outputs.
- The full-roundtrip test is deliberately a heavier workload than the sharing
  client will run, since it decodes its own stream too.

## Additional exercised paths

- **All 15 quality combinations** were actually captured, hardware encoded and
  hardware decoded: every 15/30/60 × 480/720/1080/1440/Source, two measured seconds
  each plus two seconds warmup, in one process. All completed, zero output drops,
  all 60 fps modes ≥59.8 fps. [Full table](bench-results/all-qualities.md),
  [raw output](bench-results/all-qualities.log). These short runs prove mode
  negotiation/operation, not sustained guarantees at every combination.
- **Live resolution + FPS + bitrate changes**, VBR, cursor off/on and requested
  keyframe: six seconds, 1080p60 → 720p30 → 1080p60, two confirmed generations,
  timestamps strictly increasing, decoder followed both, final IDR confirmed.
  Four scheduled frames missed during encoder replacements (~67 ms at 60 fps);
  **capture was not restarted**. Mixed-mode average 49.15 fps is expected, not a
  failed 60 fps steady-state benchmark. [Raw output](bench-results/reconfigure.log).
- **Software path:** explicit SoftwareOnly requested at 1080p60; API reported
  `EncoderKind::Software`, a fallback notice, **1280x720 / 30 fps** effective
  settings. OpenH264 encoded and the hardware decoder decoded at 29.91 fps for
  3.009 s; CPU 2.889% machine / 46.222% one core; RSS 104.48 MiB; zero drops.
  Diagnostic RGBA readback also succeeded. This is an explicitly high-CPU fallback,
  not a 1080p60 solution. [Raw output](bench-results/software.log).
- **Window capture:** captured the GPU fixture window, **Source 2560x1440**,
  cursor disabled, 59.86 encode/decode fps over 3.007 s, zero drops/repeats,
  diagnostic readback of 14,745,600 RGBA bytes succeeded.
  [Raw output](bench-results/window.log).
- Sequential sessions initially exposed a native access violation in a cached
  WinRT activation-factory vtable after MTA apartment teardown. Fixed with one
  process-lifetime CoIncrementMTAUsage cookie; per-session COM/MF references still
  balance. Four-mode matrix and fifteen sequential sessions then completed.
- NVIDIA rejected the legacy AVEncCommonLowLatency and optional B-picture-count
  properties. Required **CODECAPI_AVLowLatencyMode** works; B-count is optional,
  while one-input/one-output and preserved output timestamps are enforced.
- Initial mostly-static desktop smoke runs repeated many frames. An initial GDI
  stimulus/default capture interval supplied only ~55 unique frames/s. The final
  separate GPU stimulus plus `MinUpdateInterval = 0` on this Windows build supplied
  >112 acquired WGC frames/s and proved real paced 60 fps. No static repeat result
  is substituted for the moving-source acceptance numbers above.

## Measurement definitions / reproducibility

Build the examples, then run `motion_fixture 180` in a **separate process** and
`sharebench --seconds 30 --matrix` in another. The fixture is a temporary native
D3D11 moving pattern on the primary monitor; it closes automatically. Stimulus
CPU/RSS is excluded from the measured sharing process. This is a GPU test pattern,
**not a claim of a real game/video endurance test**. Exact commands are in
[README.md](README.md). No captured screen video or screenshots are saved.

- Warmup: two seconds per mode, excluded from metrics; includes decoder pool init.
- CPU: delta process user+kernel time (`GetProcessTimes`) divided by measured
  wall time. One-core percentage is unnormalized; total-machine percentage divides
  by 16. This is **process CPU**, not total system load. Short runs are quantized.
- Memory: `GetProcessMemoryInfo` working set (RSS), sampled at least once/sec plus
  endpoints; raw log also prints lifetime peak working set. GPU VRAM and driver
  allocations outside process RSS are **not included**. Not a proof of total
  system incremental RAM/VRAM, or of production-client baseline deltas.
- Encoding latency: CPU wall time from submitting the video-processor conversion
  through MF compressed output availability, including event waits. No GPU pixel
  mapping in this interval. Not capture-to-photon/network end-to-end latency.
- p99: sorted sample at zero-based index ceil(0.99 * (N - 1)); mean is
  arithmetic mean. No percentile extrapolation from the mean.
- Bitrate: compressed output bytes × 8 / measured wall time. Configured targets:
  1.797 Mbps (720p30), 8.087 Mbps (1080p60), 14.377 Mbps (1440p/Source60). The NVIDIA
  CBR mode can undershoot on simple content; **strict filler/padding is not
  implemented**. Target configuration and actual bitrate are not conflated.
- Drops: missed scheduled encode slots + full output-queue drops. Repeats are
  counted separately; source oversampling/coalescing is not a dropped target slot.
- Output fps is measured from delivered access units; decode fps counts actual
  decoder textures. Source FPS is not fabricated from requested quality.

## Known gaps / what this does NOT certify

1. No networking, RTP packetization, REMB/TWCC, SFU, UI/renderer integration, audio,
   or persistence of quality. Those are intentionally outside this crate proof.
   **Ten-minute game/video to two viewers and no-growing-network-latency** remains
   required for the integrated Phase 2 product.
2. Source requires **even native dimensions** for NV12. Odd-sized Source windows
   return an explicit error; scaled modes work. Native padding/clean-aperture
   metadata, not silent scaling, is needed to close this limitation.
3. Resolution/bitrate/FPS changes replace the encoder MFT at an IDR boundary,
   without tearing down capture. Brief output interruption is measured above.
   Seamless in-place vendor reconfiguration is not implemented.
4. Hardware encoder events use bounded 1 ms waits rather than an IMFAsyncCallback.
   Event deadline is two seconds, but a wedged native driver call is not forcibly
   cancellable. Device removal/driver failure surfaces as an error; automatic
   device/session recovery is not implemented.
5. Only NVIDIA hardware on this PC was exercised. AMF, QSV, hybrid/cross-adapter
   systems, 4K Source, protected content, capture permission denial, display
   removal, minimized windows, HDR/tone mapping and every resize edge case remain
   unverified. Output is 8-bit SDR BT.709 NV12/H.264. HDR support is not claimed.
6. Renderer is a **texture handoff contract**, not a working iced playback view.
   RGBA readback/checksum verifies reachable decoded pixels, not perceptual image
   quality or a render synchronization/end-to-end visual audit.
7. The 15-mode churn process reached **156.72 MiB sampled peak** at Source60
   (~164.33 MB). The matrix Source row peaked at 143.98 MiB (~150.97 MB).
   **Do not claim <150 MB at every resolution or under arbitrary session churn**.
   The specified 1080p sender target is established by its fresh-process run;
   longer resource/churn soak tests and VRAM accounting remain necessary.
8. AV1/HEVC are detection-only. H.264 is the implemented codec. Software fallback
   is exercised explicitly; this PC has working hardware, so a physical no-hardware
   installation's automatic fallback trigger was not independently exercised.

## Validation commands passed

All commands used `CARGO_TARGET_DIR=C:/Users/dwhee/.cargo-target/sticktalk-media`:

- `cargo build -p sticktalk-media --release`
- `cargo test -p sticktalk-media` — **5 passed**, no ignored tests
- `cargo clippy -p sticktalk-media --all-targets -- -D warnings`
- `cargo fmt --all -- --check` — entire workspace formatting check, no edits to
  other crates
- `cargo build -p sticktalk-media --release --examples`

These are package-scoped build/test/clippy gates, **not** claims about the other
agents' client/server compilation. No git commit was made.

## API references used

- [WGC free-threaded capture](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded)
- [D3D11 VideoProcessorBlt](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorblt)
- [Asynchronous MFT event contract](https://learn.microsoft.com/en-us/windows/win32/medfound/asynchronous-mfts)
- [Low-latency codec mode](https://learn.microsoft.com/en-us/windows/win32/medfound/codecapi-avlowlatencymode)
- [D3D11 hardware decode integration](https://learn.microsoft.com/en-us/windows/win32/medfound/supporting-direct3d-11-video-decoding-in-media-foundation)
