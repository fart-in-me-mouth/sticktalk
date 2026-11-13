# Audio round 2 — 2026-11-12/13

Device-free qualification only. No microphone/camera stream, loudspeaker sound,
visible window, browser, notification, LAN listener or input injection was used.
Physical mouth-to-ear, physical hot-plug/Bluetooth and fullscreen-game hotkeys
still need controlled manual acceptance. Camera rendering belongs to the parallel
video integration; this change supplies the persisted chooser hook only.

## Implementation

- Persisted Default/friendly input/output choices, IMM notification snapshots,
  input/output gain, explicit mic monitoring, one active test-only meter,
  automatic/manual sensitivity, PTT recording/clear/release delay, Off/Standard/
  High/Auto suppression, AEC/AGC, Studio/Music, attenuation (default off), camera
  choice and existing 0–200% per-user mix/mute persistence.
- Settings edits use a bounded latest-value control channel: dragging sensitivity
  or faders does not recreate the microphone test. Closing settings stops it.
  Fake endpoints go through the same enumeration validation and selection policy;
  tests include conflicting Console/A2DP vs Communications/HFP defaults.
- A cheap energy/noise-floor gate precedes APM/neural work, drains six tail hops,
  fades transitions (5 ms attack / 20 ms release), and primes Opus DTX before
  reducing silence encoding to 400 ms comfort updates. Automatic Voice Activity
  also gates DFN on neural activity; PTT/manual thresholds avoid redundant neural
  VAD when it is not needed for suppression.
- Dormant receive streams stop NetEQ expansion/CNG work. DTX resumes flush stale
  queued silence; long outages and RTP timestamp wrap recover boundedly. Audio-only
  transport blocks until UDP/RTC deadlines or explicit command/audio/hotkey wakes,
  rather than waking every 5 ms. Video's separate pacing policy is unchanged.
- High uses the pinned official **DFN3-LL** weights: 48 kHz, FFT 960, hop 480,
  conv/DF lookahead zero: **10 ms algorithmic overlap delay**. Runtime validates
  that configuration. Auto persists a first-run noisy-signal probe, reserving
  roughly 2% of one core for APM/Opus outside its 0.4 ms inference p95 budget.
- Adaptive packet policy starts at 10 ms, degrades to 20 ms at >=3% loss or >=15 ms
  measured jitter, and requires five clean feedback intervals (<=1%, <=5 ms) to
  recover. The NetEQ floor follows validated 10/20 ms packetization. About 80 bytes
  of transport headers x 50 extra packets/s x 8 = **32 kbit/s** extra at 10 ms;
  this is a header-budget estimate, not a measured network bitrate.

## Measurements

Ryzen 7 5700X, 16 logical CPUs; release opt-level 3/fat LTO/codegen-units 1.
**Partial acceptance:** clean Standard digital latency and silence budgets pass
this device-free run; **High talking CPU misses the <=6% target**. Physical
mouth-to-ear <=80 ms and "better than Discord" remain unqualified.

### CPU and memory

Ten seconds per state, synthetic near/far-end signals, actual APM/NS/Opus/stereo
NetEQ, deadline sleeps; percentages are **one logical core**, not machine total.
No native device buffers or real network. This is a bench process, not client
idle-memory acceptance. Windows process-time granularity is visible in these
short samples; Off/Standard silence have little margin below 0.5%.

| Mode | Talking CPU | Silence CPU | Silence target | Talking RSS / private |
|---|---:|---:|---:|---:|
| Off | 2.656% | **0.469%** | <=0.5% | 49.75 / 6.33 MiB |
| Standard | 2.656% | **0.469%** | <=0.5% | 49.83 / 6.23 MiB |
| High (DFN3-LL) | **11.718% — FAIL** | **0.156%** | <=1.5% | 90.23 / 46.29 MiB |
| Studio/Music | 1.094% | not measured | — | 49.21 / 5.27 MiB |

After all processors dropped: **0.000%** over 5 s, RSS 47.54 MiB/private 3.84 MiB.
Silent encoded payload: 0.473 kbps (excludes transport headers). Auto chose
**Standard**: noisy-frame probe mean 261.9 us/p95 401.7 us. Auto is a short local
heuristic, not a sustained/thermal CPU guarantee. High needs further inference
runtime optimization; it is not silently replaced with a cheaper model.

### Per-marker digital latency (ms)

Each clean row matched **16/16** markers; no PLC/FEC or discarded late samples.
P50 uses the upper sample median; p95 uses nearest rank (with 16 observations,
p95 equals max). Results include real stereo native-decoder sample accounting.

| Mode | Opus frame | Clean p50 / p95 / max | Impaired p50 / p95 / max | Impaired coverage |
|---|---:|---:|---:|---:|
| Off | 10 ms | 25.417 / 25.438 / 25.438 | 98.896 / 110.896 / 110.896 | 16/16 |
| Off | 20 ms | 35.417 / 35.438 / 35.438 | 106.958 / 123.979 / 123.979 | 16/16 |
| Standard | **10 ms** | **35.417 / 35.417 / 35.417** | 97.646 / 113.646 / 113.646 | 16/16 |
| Standard | 20 ms | 45.417 / 45.417 / 45.417 | 92.229 / 125.417 / 125.417 | 16/16 |
| High | 10 ms | 35.417 / 35.417 / 35.417 | 55.417 / 55.479 / 55.479 | 16/16 |
| High | 20 ms | 45.417 / 45.438 / 45.438 | **unqualified** | 15/16 |

Choose **10 ms on clean links**: Standard gains 10 ms and passes <=40 ms;
20 ms misses it. Adaptive policy changes to 20 ms at >=3% loss or >=15 ms jitter,
then requires five clean feedback intervals (<=1% loss and <=5 ms jitter) to
recover. Twenty milliseconds halves packet rate/header cost under impairment;
the impaired fixture is not evidence of a universal latency/quality win. NS/DTX
changes queue history, so High's impaired numbers are not an apples-to-apples
jitter-algorithm comparison. Impaired High/20 returns exit **1** for failed
coverage; its selected-subset distribution is diagnostic only.

Qualification uncovered/fixed a hidden 20 ms histogram floor, expansion of a
complete 10 ms leftover, and stereo/per-channel allocation/filter/clock units.
The native-stereo clean 10/20 ms continuity test now asserts zero late discard.
Unprefixed raw logs are the **initial failed pass** (Standard 42.417 ms,
silence 1.250%); `final-*` logs are authoritative. No failed evidence was erased.

### Objective suppression metric (offline bench only)

Aligned **SI-SDR**, deterministic approximately 0 dB white-noise speech mixture:
**Off -0.775 dB; Standard 4.427 dB; High 7.240 dB**. The upstream local
`assets/clean_freesound_33711.wav` fixture is not redistributed. AEC/AGC are off
for this isolation test, before codec/time stretching. Measured enhancement
alignment: Off 0.958 ms, Standard/High 10.958 ms, consistent with the 10 ms model
overlap delay. This is not DNSMOS/PESQ, a perceptual score, a diverse noise corpus
or a competitor comparison. No ONNX scorer/dependency is shipped in the client.

## Timing method

`voice_timing` sends sixteen independently correlated 10 ms chirps, separated by
350 ms, over a voiced harmonic bed. It uses the same f32-LE stereo format helpers,
stereo-to-mono capture, native AEC3/HPF, selected NS, AGC2, Opus, adaptive NetEQ/
decode and playout format seam. Render runs before capture, as in WASAPI: a newly
captured block cannot be inserted into a buffer already handed to playback.
The sender's independent far-end reference is silent, not its own delayed signal.
Every marker's lag/confidence is logged; coverage requires all sixteen peaks to
correlate >=0.45. Rejected markers are not silently discarded from an accepted
latency claim. Search resolution is one 48 kHz sample, range 0–150 ms. The impaired
fixture adds deterministic 5% packet loss and 0/20/30 ms jitter/reordering.
This excludes actual WASAPI/hardware buffering, network/SFU and acoustics.

## UI review

Fourteen real iced/tiny-skia PNGs, **no OS window**, in
`../../screenshots/audio2/`: the six requested states plus a scrolled processing/
camera state, at both 1280x800 and 1920x1080. All were visually inspected against
UI_SPEC sections 1–8. The review fixed actual dropdown-open capture timing,
selected radio dots and 40x24 switches. Shared settings navigation is inherited,
not replaced by this audio task. Manifest includes dimensions and SHA-256 values.

## Reproduction and build hygiene

One target only, `C:/Users/dwhee/.cargo-target/sticktalk-audio2`. Following the
lead's emergency 10 GB limit, incremental compilation is disabled, debug/test
symbols are disabled, and debug/release trees are never retained together.
Debug gates and screenshots precede cleaning debug; release measurements precede
`cargo clean --release` and final target cleanup.

Final gates: **109 passed, 3 intentionally ignored** across client, client-core
and media; all-target Clippy with warnings denied and workspace formatting pass.
Raw validation logs are alongside this report. The impaired timing exit 1 is an
honest coverage failure, not a passing quality gate. Debug peaked at **7.37 GB**,
release at **2.34 GB**, never together, below the 10 GB hard target limit.

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-audio2'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p sticktalk-media -p sticktalk-client-core -p sticktalk
cargo clippy -p sticktalk-media -p sticktalk-client-core -p sticktalk --all-targets -- -D warnings
cargo fmt --all -- --check
sticktalk --mock --screenshots crates/client/screenshots/audio2 --audio-only
cargo clean --profile dev
$env:CARGO_BUILD_JOBS = '1'
cargo build -p sticktalk-media --release --example voice_quality_bench --example voice_timing
voice_quality_bench --seconds 10 --ten-ms --cpu-only --clean-wav <local-upstream-fixture>
voice_timing
voice_timing --impaired
cargo clean --release
cargo clean
```
