# StickTalk media: Windows benchmarks

**Audio round 2 (2026-11-12):** current device-free silence CPU, per-chirp
full-pipeline timing, DFN3-LL delay and objective NS scores are in
[`../client/bench/audio2/REPORT.md`](../client/bench/audio2/REPORT.md).
That report supersedes the historical voice figures/limitations below, not the
screen/video measurements. No physical/audio-device acceptance is claimed.

The original screen proof below is historical (2026-11-11). The call-media
implementation changed twice on 2026-11-12. **The final HQ qualification section
at the end supersedes the earlier voice results, jitter/device limitations and
validation counts.** Historical measurements remain explicitly labeled.

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

1. No networking, RTP packetization, REMB/TWCC, SFU, UI/renderer integration,
   or persistence of quality. Those are intentionally outside this crate proof.
   **Ten-minute game/video to two viewers and no-growing-network-latency** remains
   required for the integrated Phase 2 product.
2. **Resolved in the update:** Source now crops the final odd row/column.
   A generated odd-size GPU encode/decode/readback test verifies this.
3. **Reworked in the update:** MFT preparation/retirement run off the paced output
   path. The old stream continues until the replacement emits IDR/SPS/PPS.
   See new transition measurements below; not every driver is certified.
4. Hardware encoder events use bounded 1 ms waits rather than an IMFAsyncCallback.
   Event deadline is two seconds, but a wedged native driver call is not forcibly
   cancellable. Screen-capture device removal/driver failure surfaces as an error; automatic
   screen-session recovery is not implemented (voice recovery is separate).
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

## Original screen-proof validation commands

All commands used `CARGO_TARGET_DIR=C:/Users/dwhee/.cargo-target/sticktalk-media`:

- `cargo build -p sticktalk-media --release`
- `cargo test -p sticktalk-media` — **5 passed**, no ignored tests
- `cargo clippy -p sticktalk-media --all-targets -- -D warnings`
- `cargo fmt --all -- --check` — entire workspace formatting check, no edits to
  other crates
- `cargo build -p sticktalk-media --release --examples`

These are package-scoped build/test/clippy gates, **not** claims about the other
agents' client/server compilation. The update has separate gates below.

## API references used

- [WGC free-threaded capture](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded)
- [D3D11 VideoProcessorBlt](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorblt)
- [Asynchronous MFT event contract](https://learn.microsoft.com/en-us/windows/win32/medfound/asynchronous-mfts)
- [Low-latency codec mode](https://learn.microsoft.com/en-us/windows/win32/medfound/codecapi-avlowlatencymode)
- [D3D11 hardware decode integration](https://learn.microsoft.com/en-us/windows/win32/medfound/supporting-direct3d-11-video-decoding-in-media-foundation)

## Initial call-media update (2026-11-12, BEFORE native APM/DFN3 upgrade)

Same Ryzen 7 5700X / 16 logical CPUs / RTX 3060 Ti / Windows 11 build 26200.
Release/MSVC, `CARGO_TARGET_DIR=C:/Users/dwhee/.cargo-target/sticktalk-calls`,
`CMAKE_GENERATOR=Visual Studio 17 2022`, `CARGO_INCREMENTAL=0`.
CMake installed here does not recognize the newer VS 2026 generator; choosing
installed VS 2022 builds static libopus. No system-wide installations/admin actions needed.
The generated workspace Cargo.lock is an explicitly user-approved scope exception.

### Voice: generated-input CPU and digital loopback

[Raw INITIAL run](bench-results/voice-dsp.txt), `callbench --seconds 10`.
48 kHz, 100 paced 10 ms ticks/second, mono 20 ms Opus at 32 kbit/s target,
FEC/DTX, AEC3 + AGC2 + nnnoiseless, one remote decoder/jitter/mixer.
The talking stimulus is generated voiced harmonics/envelope with PTT held open,
**not recorded human speech**. Silent workload feeds zeros with the gate closed.
No WASAPI devices in these rows; no sound, windows, sockets or consent requests.

| Workload | Seconds | CPU machine | CPU one core | RSS | Opus packets | Actual Opus bitrate |
|---|---:|---:|---:|---:|---:|---:|
| Synthetic talking + 1 remote | 10.000 | **0.2344%** | 3.7499% | 12.47 MiB | 500 / 50.00 per sec | 29.030 kbit/s |
| Silent in-call DSP + 1 remote | 10.000 | **0.1855%** | 2.9686% | 12.60 MiB | 500 / 50.00 per sec | 0.922 kbit/s |
| Outside call, engines dropped | 10.000 | **0.0000%** | 0.0000% | 12.00 MiB | 0 | 0 |

CPU uses GetProcessTimes deltas; machine percentage divides by 16. Single
10-second intervals, no confidence bounds; quantization and the owner's other
workloads limit precision. In-call silence is not zero DSP. Outside-call zero
means zero measured process-time delta; native session ownership separately
joins workers/drops audio clients on stop. No devices/threads start implicitly.

**Digital end-to-end loopback:** generated PCM -> packetization/Opus -> fixed
two-packet jitter -> decode/mixed PCM. Positive cross-correlation of a tagged
250 ms signal after silence, search 0..200 ms:

- RNNoise off: **51.188 ms / 2457 samples**.
- RNNoise on: **61.042 ms / 2930 samples**.

This excludes WASAPI/hardware buffers, acoustics, networking/SFU and wall-time
scheduling: **physical mouth-to-ear remains NOT MEASURED**. No loudspeaker
stimulus was played. Do not relabel this as physical latency acceptance. The
synthetic delayed-echo unit test independently verifies AEC3 attenuation, not
room-acoustic quality or an ERLE certification.

Before the later no-prompts instruction, an earlier native muted-call smoke
opened already-accessible defaults and stopped successfully: 500 packets in
10.001 s, **0.1855% machine / 2.9683% one-core CPU**, 15.61 MiB RSS
([raw](bench-results/voice-wasapi-idle.txt)). This predates the final stereo
playback extension; it is **not the final native pipeline benchmark claim**.
No microphone/camera streams were opened after that instruction. Notification
registration/unregistration succeeded; no physical unplug/default change was
induced on the owner's PC. No captured audio payloads were saved.

### Camera: synthetic 720p30 codec proof, no real camera CPU number

MF enumeration found **zero cameras** ([availability](bench-results/camera-availability.txt)).
**Real camera 720p30 CPU: NOT MEASURED.** No camera was activated.

The safe `callbench --video-fixture --seconds 10` instead uses generated NV12
through `CameraFrameEncoder`, the same H.264 MFT and strict GPU decoder:
**300 frames / 10.000 s = 30.00 fps**, **0.2344% machine / 3.7500% one-core CPU**,
**80.40 MiB RSS** ([raw](bench-results/video-fixture.txt)). Includes CPU test
pattern generation, one NV12 upload, hardware encode and hardware decode. Omits
MF device capture, USB driver conversion, real callback cadence and exposure.
This proves the camera-ingress/codec seam, **not real camera capture CPU**.
Async source-reader callback/error handling also has a fake callback unit test.

### Odd dimensions, handover and hardware selection

- Generated **801x601 BGRA -> 800x600 NV12 -> NVIDIA H.264 -> GPU decode -> test
  readback** passes. Green sentinel pixels in the removed last row/column are
  absent at all output corners. This is a headless GPU test, no OS window/camera;
  actual WGC odd-window capture is not separately claimed.
- A second MTA worker prepares the next encoder while the old stream continues.
  Commit waits for **IDR + SPS/PPS**. Old MFT shutdown runs off the paced path;
  another preparation waits for retirement, bounding pools to two. Failed
  preparation keeps the old stream; WGC/device never restart.
- Active-share pacing uses an unnamed **high-resolution waitable timer**, not
  global timeBeginPeriod or a busy loop. It exists only during an active share.
- AMF/QSV/NVENC use the same MFT/DXGI path. Adapter-vendor-first ordering then
  tries other/unknown names; hardware activation still proves support. Fake
  enumerations test AMD/Intel/NVIDIA, missing/unknown candidates, failures and
  explicit software fallback policy. **Only NVIDIA hardware physically tested.**
- Windows process-loopback activation/stop worked while self emitted no audio:
  **zero packets**, not a content-capture test ([raw](bench-results/process-loopback.txt)).
  Stereo codec/mixer preservation has generated-PCM tests. Real system/app
  content capture, playback fidelity, acoustics and physical hot-plug remain open.

Final transition log: [seamless.txt](bench-results/seamless.txt). The owner's
mostly static desktop remained untouched; no visible moving fixture was opened.
Repeats are counted, so this is not a new moving-game/endurance acceptance test.
Final numeric transition/fallback results follow below.

### Initial remaining gaps (superseded by final HQ section)

1. Physical mouth-to-ear and final native voice talking/idle benchmarks require
   permitted isolated devices without sound/consent prompts; synthetic DSP
   numbers above deliberately do not substitute for them.
2. No camera: native capture, permission denial, real-driver disconnect/stop and
   actual 720p30 camera CPU are unverified. Only file/generated ingress tested.
3. Real AMF/QSV/hybrid systems, sustained 128-stream load, long jitter/clock drift,
   thermal/resource soak, room acoustics and perceptual moving-source transitions
   need other hardware/test conditions. Jitter is fixed/bounded, not adaptive
   NetEq or a clock-drift correcting playout system.
4. Native driver calls/process-loopback activation are not forcibly cancellable.
   Device invalidation/default changes are reported; the host handles reopening
   and previously-granted permissions. Library code never requests consent or
   opens privacy settings. Real-device bench modes require an explicit
   --devices-already-permitted assertion; do not run them during agent work.
5. Client/server/SFU integration, RTP, permissions UX, congestion control, renderer
   integration and the ten-minute two-viewer acceptance remain outside this scope.

### Initial update gates (superseded by final HQ section)

- `cargo test -p sticktalk-media`: **23 passed**, 1 intentionally ignored
  hardware-only diagnostic in the ordinary fake/no-device suite.
- `cargo test -p sticktalk-media odd_nv12_source -- --ignored --nocapture`:
  **that one headless GPU diagnostic passed** separately.
- `cargo clippy -p sticktalk-media --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed; no formatting edits to other crates.
- `cargo build -p sticktalk-media --release --examples`: passed.
- `callbench --seconds 10`, `callbench --video-fixture --seconds 10`: passed.
- Final live NVIDIA reconfiguration/decoder and explicit software fallback:
  see the raw logs and final results below.

### Final screen-transition numbers

`sharebench --seconds 9 --reconfigure --vbr`, final high-resolution pacer:
**9.016 s**, **49.91 encode/decode fps** across 1080p60 -> 720p30 -> 1080p60
(expected mixed-rate mean approximately 50 fps), **zero output drops**, two
confirmed IDR generations. Generation-boundary PTS gaps: **13.438 ms** and
**33.958 ms**; maximum PTS gap anywhere **33.958 ms**, maximum receive gap
**39.057 ms**. The second switch follows a 30 fps cadence (33.333 ms expected),
not a requirement to maintain 60 fps during the intentionally selected 30 fps
segment. Mean/p99 encode **3.436/4.994 ms**, CPU **0.303% machine / 4.853% one-core**,
RSS/sample peak **110.70 MiB**. Capture only 26.07 unique fps, 228 repeated outputs:
mostly-static desktop, no fabricated moving-source guarantee. Every emitted
new generation began with IDR; decoder followed both, requested keyframe confirmed.

The first scalar OpenH264 rerun was functionally decodable but **not realtime**:
**4.61 fps**, 55 dropped slots, **6.077% machine / 97.232% one-core CPU** over
2.169 s ([raw](bench-results/software-calls-update.txt)). Build output identified
missing NASM assembly. An official NASM 2.16.03 ZIP was then extracted only to
`C:/Users/dwhee/.cargo-tools/sticktalk-call-hq (moved out of the deleted Rust target)`, without installer, elevation,
registry or system-PATH changes (see README for URL/hash). `NASM` is process-local,
and only the task target's OpenH264 package was cleaned/rebuilt. The accelerated
rerun below is kept separate; no bad result is silently relabeled as a pass.


## Final HQ qualification (2026-11-12, native WebRTC + DFN3-LL)

**Implementation delivered; latency/physical-device acceptance is NOT complete.**
This section supersedes the initial voice implementation/results above. Same
5700X / 16 logical CPUs / RTX 3060 Ti / Windows build 26200. Release opt-level 3,
fat LTO, Opus complexity10, **static native WebRTC and static C/C++ CRT** via
`tools/build-env.ps1`. The only final task target is the short `C:/mhq`, outside
OneDrive (MSVC native build paths exceeded limits with the original long path).
Build tools are per-user under `.cargo-tools`, never installed system-wide.

### What actually runs

- `webrtc-audio-processing` 2.1.0 wrapper, Windows-portable pinned sys 2.1.1:
  native **AEC3 + high-pass -> external NS -> native adaptive-digital AGC2**.
  Separate native APM stages ensure AGC follows suppression. No hardware mic
  volume control. Vendored wrapper provenance/modifications/licenses in README.
- NS **Off / Standard nnnoiseless / HighQuality upstream DFN3 low-latency model**.
  The full/non-LL model failed Tract 0.21.4 codegen with
  `duplicate name /convt3/Conv.bias` ([diagnostic](bench-results/df-full-diagnostic.txt)).
  It is NOT silently represented as a successful full-model test. libDF and
  model weights are pinned at Git `7b14b895fe7fc42cab72b4a4c1e49c982bd6404b`.
  x86_64 inference scopes FTZ/DAZ to avoid inaudible subnormal-state costs during
  silence, restoring the caller thread's FP environment; dedicated test covers
  restoration. Other architectures need their own performance qualification.
- Default mono 64k, feedback-driven32..128k, FEC/DTX. Studio stereo 192k default,
  128..256k, no microphone processing/VAD/DTX. Transport-neutral bounded APIs.
- Adaptive NetEQ-inspired quantile jitter + time stretching from `neteq` 0.9.1,
  with registered libopus decoder and codec-gap FEC/PLC repair. **Not Chromium
  NetEq.** Default target floor 20 ms, cap 80 ms; a larger explicit floor can raise
  that cap. Repair attempts and already-concealed-time drops are measured.
  Upstream's incorrectly indexed operation counters are deliberately not exposed
  by the clean `PlayoutStatistics` API; delay/queue/packet-rate remain available.
- IMM notification-driven default/pinned device recovery, friendly switch events,
  console/media output to avoid requesting Bluetooth communications/HFP, auto/
  manual sensitivity, PTT release delay, explicit mic-test meter/self-monitoring.
  Tests use fake enumerations/generated PCM, not physical unplug/profile changes.
- Nothing starts at library import. Explicit sessions own devices/worker waits;
  no call means no devices, polling, timers or workers. Model probe runs only on
  explicit invocation/first Auto call and caches per process; host can persist it.

### Final generated voice CPU and digital latency

`voice_quality_bench --seconds 10 --clean-wav <fixture>`; all input is generated
or file data. **No audio device, sound, preview, socket or permission request.**
Each CPU row is a real-time 10 s interval, 100 x 10 ms capture/render ticks/s,
20 ms mono Opus 64k (Studio 192k), AEC3/HPF/AGC2 except Studio, selected NS, one
adaptive decoder/mixer. Independent pre-encoded far-end playback supplies the
actual AEC reference; capture adds a 50 ms delayed 0.2-gain echo. Far-end encode CPU
is excluded (it belongs to the other participant). Talking stimulus is modulated
harmonics, not a representative speech/noise corpus. Idle means **silent but
unmuted, PTT held open**, including all DSP/Opus DTX, not a cheap muted gate.
Target bitrate is not conflated with actual variable/DTX payload rate.

[Final raw voice log](bench-results/voice-hq.txt).

| Mode | Workload | CPU machine % | One-core % | RSS MiB | Actual payload kbit/s |
|---|---|---:|---:|---:|---:|
| Off | Talking | 0.2539 | 4.0625 | 49.25 | 59.284 |
| Off | Silent/unmuted | 0.2148 | 3.4374 | 58.54 | 20.065 |
| Standard | Talking | 0.2832 | 4.5311 | 58.45 | 54.333 |
| Standard | Silent/unmuted | 0.2930 | 4.6872 | 58.61 | 20.101 |
| HighQuality | Talking | 0.8886 | 14.2183 | 98.63 | 8.408 |
| HighQuality | Silent/unmuted | 1.4355 | 22.9678 | 98.49 | 20.047 |
| Studio stereo (DSP off) | Talking | 0.1855 | 2.9687 | 58.04 | 136.823 |

After all processors/mixers drop: **0.0000% machine / 0.0000% one-core**
over a separate 5 s interval; retained process RSS **56.70 MiB**. No devices
were opened, so this verifies the device-free idle lifetime, not a native driver
resource/handle soak.

Auto probe chose **HighQuality**: mean **0.265 ms**, p95 **0.386 ms** per
10 ms frame, below its 6 ms threshold (24 measured frames after 4 warmups).

| Digital PCM-loopback mode | Correlation delay ms |
|---|---:|
| Off | 99.542 |
| Standard | 109.229 |
| HighQuality | 115.667 |

CPU is `(process kernel+user seconds / wall seconds)*100 / logical processors`.
One-core percentages are also given; short intervals have Windows accounting
quantization and competing owner workloads/frequency effects. RSS rows are one
sequential process and include allocator/library residency, not isolated model
working sets or an incremental client RAM claim. The release benchmark executable
is about 49 MiB (bundled native code/model weights), not the final desktop binary.

The digital loopback correlation measurement is paced PCM capture -> suppression/
Opus -> immediate in-memory packet handoff -> adaptive playout -> PCM. It excludes
WASAPI buffers, acoustics, devices, network/SFU and physical playback. **Even this
digital measurement misses the <=80ms target.** Time stretching can affect waveform
alignment, so correlation delay is a diagnostic, not physical mouth-to-ear proof.
No latency budget acceptance or Discord superiority claim is made.

### Objective enhancement score, not a market/perceptual claim

The fixture is upstream `assets/clean_freesound_33711.wav`, mono 48k PCM16,
10.595 s, from the pinned DeepFilterNet checkout. Upstream identifies this as
Freesound 33711, CC BY-NC 3.0. **No audio fixture/output is redistributed in this
commit**; do not package that non-commercial test asset with the client.
A deterministic PRNG (seed 42) adds approximately 0 dB uniform white noise; peak
normalization prevents clipping. AEC and AGC are disabled to isolate suppression;
HPF remains enabled. Clean/output are aligned over 0..200 ms, SI-SDR computed over
the interior excluding 0.5 s at each end, on processed PCM before the codec.

| NS mode | Aligned SI-SDR dB | Enhancement delay ms |
|---|---:|---:|
| Off | -0.775 | 0.958 |
| Standard | 4.427 | 10.958 |
| HighQuality | 7.240 | 10.958 |

This is one file / one synthetic noise condition: **not DNSMOS, PESQ, a MOS study,
codec/network end-to-end perceptual quality or a Discord comparison**. DNSMOS was
not available in the test environment. Representative multi-speaker/noise/music,
room echo/double-talk and blind listening qualification remain necessary. Binary/
RAM spending alone cannot establish "10x Discord".

### Final safe camera seam / native linkage

[Final synthetic video log](bench-results/video-fixture-hq.txt): generated
NV12 720p30 -> same hardware H.264 -> GPU decoder, **300 / 10.000 s = 30.00 fps**,
**0.2051% machine / 3.2811% one-core CPU**, **78.40 MiB RSS**.

This includes pattern generation, pitched CPU NV12 upload, actual hardware H.264
and strict GPU decode. It omits MF camera capture/USB drivers/exposure/cadence.
**Physical camera 720p30 CPU: NOT MEASURED (zero cameras enumerated).** No camera
was opened. It is not legitimate to relabel the generated ingress number as real
camera CPU. Only NVIDIA was physically exercised; AMF/QSV selection/fallback is
covered by fake enumerations, not certification on those GPUs.

[DLL imports](bench-results/hq-dll-imports.txt) of the static-CRT voice executable
contain Windows OS DLLs only: no external APM/Opus/model or VC-runtime DLL install.
No elevation/UAC, firewall rule, visible window, notification or network listener
was requested. Real microphone/camera diagnostic modes were NOT run after the
lead's no-prompts instruction; no further real-device tests are authorized here.

### Current gaps / handoff requirements

1. **Latency target missed** by adaptive digital playout; physical mouth-to-ear
   cannot be claimed. Native Chromium NetEq parity is NOT established. Upstream
   noise-based underrun expansion, codec repair after gaps, drift/loss/reorder and
   time-stretch quality (including upstream interleaved-stereo decision units)
   need deeper qualification or replacement. `FixedVoiceMixer`
   is available as the deterministic libopus FEC/PLC alternative, not represented
   as adaptive. The native VoiceSession currently uses the adaptive path.
2. `push_at` retains actual arrival time for packet aging, but upstream's delay
   manager samples processing time. Audio-worker queue scheduling distortion in
   jitter estimation is still open, despite capturing network arrival metadata.
3. Full DFN3 model/other Tract-runtime compatibility remains open; the tested
   working high-quality model is the official low-latency DFN3 variant. CPU mode
   choice is a quick local inference probe, not a sustained thermal guarantee.
4. No physical camera, unplug/default-change/Bluetooth HFP recovery, permission-
   denied driver behavior or system/app loopback content-fidelity certification.
   These cannot be safely induced on the owner's active PC. Silent process
   loopback activation/stop and early already-accessible muted WASAPI smoke are
   historical only, not final native voice quality/CPU acceptance.
5. No full 128-stream/resource soak, real AMD/Intel/hybrid tests, native driver
   cancellation guarantee or new moving-game 10 min/two-viewer acceptance. Existing
   headless odd-size and screen-transition evidence is retained above, including
   bad scalar software numbers. Hardware camera/screen failure recovery is not
   conflated with voice endpoint recovery.
6. Client/server/SFU integration, RTP/congestion transport, permission UI/settings
   persistence and renderer integration intentionally remain outside this scope.

### Final gates and cleanup

- `cargo test -p sticktalk-media`: **30 passed, 2 deliberately ignored**.
- `cargo test -p sticktalk-media -- --ignored`: **2 passed** separately: generated
  odd-size GPU encode/decode/readback and DFN3-LL inference. No mic/camera opened.
- `cargo clippy -p sticktalk-media --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed; other crates were not edited.
- `cargo build -p sticktalk-media --release --examples`: passed, static CRT.
- Final file/generated voice + camera-ingress benchmarks: raw logs linked above.
- Generated root Cargo.lock included by explicit scope exception. No other crate
  changes. Task Rust target deleted **after final validation**, per lead request;
  per-user build tools are separate from the deleted target.
