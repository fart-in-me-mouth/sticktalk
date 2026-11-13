# Native call integration evidence — 2026-11-12

**Partial delivery, not Phase 2 acceptance.** See [integration boundaries and
remaining work](../../VOICE_INTEGRATION.md). Measurements were collected on the
owner's Windows PC without opening a visible window, real microphone/camera,
playing sounds, or injecting input. All network tests bind only loopback.

## Build and implementation

- Normal PowerShell `cargo check -p sticktalk-media` passes without sourcing
  `build-env.ps1` or custom native-tool environment variables. Cargo's task
  target override is the only required environment setting for build hygiene.
  CMake defaults to installed VS 2022; vendored APM discovers MSVC and existing
  per-user native prerequisites. See `media/vendor/BUILD_CHANGES.md`.
- Core call reducer, reconnect/rejoin, server gates, persisted local volumes,
  settings and bounded consent model; native SRTP/SCTP transport; active-call
  VoiceSession/mixing; capture publication and hardware video decode; passive
  global hooks; one-shot picker thumbnails; additive call/settings UI hooks.
- No renderer bridge: decoded textures are released, not displayed. No
  production SendInput: incoming native control is revoked, consent only offers
  Deny. Missing Stop banner/surface-bound injector must ship together. Device
  dropdowns, mic-test UX, monitor process chooser and camera chooser are gaps.

## Latency: useful improvement, requirement still unqualified

Same synthetic PCM → Opus → adaptive playout → PCM tone diagnostic, release
build, no devices/network/acoustics:

| Mode | Before, 20 ms packets | After, 10 ms packets |
|---|---:|---:|
| Off | 99.542 ms | 23.583 ms |
| Standard | 103.229 ms | 33.438 ms |

NetEq starts at its configured 20 ms floor, not an empty-histogram-derived
80 ms. Real ingress timestamps feed the estimator. WASAPI tries IAudioClient3
minimum shared period; fallback requests 10 ms. AEC receives negotiated device
buffer delay. The WASAPI/AEC changes have **not** been exercised on real devices.

Full DFN3 initializes in 1.141 s; 100 ten-ms inference hops total 109.092 ms.
High/Auto now use the full pinned model, with the existing CPU fallback. Its
synthetic tone correlation result is 154.667 ms and the tone is heavily
suppressed. Do not interpret this as a qualified speech or acoustic latency.

The new real-speech short-window diagnostic **fails**: only 52/109 voiced
windows correlate above 0.5 (required ≥70%). Among those selected windows,
p50=23.667 ms, p95=78.083 ms, max=194.250 ms. These selected-window statistics
are **not accepted latency results**. Global fixed-lag correlation was also
unstable with adaptive time stretching. No <=80 ms mouth-to-ear claim is made.
Next work is a robust speech-timing harness plus controlled acoustic loopback.

## CPU / memory (DSP loopback, not physical calls)

Three-second generated-signal release runs, process CPU as a percent of **one
logical core** (16 logical CPUs on this PC). Short runs are noisy, not p95 call
budgets; silence still has an active processor/transport loop.

| Mode | Talking | Silence | Approx. RSS during run |
|---|---:|---:|---:|
| Off | 3.6452% | 2.0832% | 21.92–22.10 MiB |
| Standard | 4.6868% | 2.6038% | 22.02–22.12 MiB |
| Full High | 5.7282% | 9.8952% | 41.63–42.32 MiB |
| Studio | 3.1249% | not sampled | 30.51 MiB |

After all processors dropped: measured CPU 0.0000% over 5 seconds; RSS 29.57 MiB.
Actual desktop idle working set, private bytes, physical call CPU and hot-plug
are not measured. These numbers must not be presented as complete client RAM.

File enhancement check with deterministic approximately 0 dB additive noise:
SI-SDR Off −0.775 dB, Standard 4.427 dB, full High 7.147 dB. This is neither
DNSMOS/PESQ nor a competitor comparison. Input was upstream's
`clean_freesound_33711.wav`; it is not redistributed here (see original media
BENCH attribution/licensing). No speech/waveform fixtures are committed.

## Real-server qualification

- Two headless native endpoint **objects**, not two GUI processes, plus real
  HTTP/WS/SFU server and real UDP ICE/DTLS/SRTP/SCTP. Looping file audio: 75/75
  Opus frames received, correlation 1.0000; speaking and guild state verified.
- Same test: request → explicit grant → SCTP key event → revoke, dry-run
  injector only; held keys released. Timeout/rate/bounds/reconnect unit tests
  supplement this. No real input or OS remote-control prompt was exercised.
- Explicit 60-second synthetic 1080p60 test: hardware-encoded NV12 fixture FILE,
  actual SFU transport and hardware decoder: **3597/3597 decoded, 59.950 fps,
  zero receiver queue drops**. The decoder is prewarmed before measurement.
  Aggregate harness CPU delta 32.9844 s over 60 s (~55.0% of one core), final RSS
  108.25 MiB, peak 129.93 MiB. This includes both peers/server/decoder; it does
  not measure live desktop capture, live encoding, GPU display or two viewers.
- All 108 standard mock screenshots rendered headlessly via tiny-skia. Eight
  final call/picker/consent/settings images at 1280×800 and 1920×1080 are in
  `../../screenshots/voice-integration/`. No OS window was created.

## Final checks

Final crate suite: **87 passed, 0 failed, 3 ignored** (including the explicitly
run 60-second video test, which passed separately). Clippy with warnings denied,
workspace format check and the normal desktop build pass.

```powershell
cargo check -p sticktalk-media  # plain PowerShell, no native build-env script
cargo test -p sticktalk-client-core -p sticktalk-media -p sticktalk -- --test-threads=1
cargo clippy -p sticktalk-client-core -p sticktalk-media -p sticktalk --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test -p sticktalk --test voice -- --ignored --nocapture
```

Final gate logs, dedicated SFU test logs, successful build log, DSP benchmark
logs and the **failing speech qualification diagnostic** are kept alongside this
report. The one task target directory is deleted after validation.

## Important handoff

The SFU's str0m answerer can panic with a sparse H.264 High-only renegotiation;
this client offers the full compatible ladder as a workaround. Server-side
malicious-SDP hardening remains outside these crates and is a security followup.
Do not equate successful normal-client tests with that boundary being hardened.

Other unqualified work: rendered video/camera/fullscreen, real desktop control
safety unit, two-process file-device harness, 10-minute live share/two-viewer
acceptance, game-focus hotkeys, physical audio/Bluetooth, physical <=80 ms
mouth-to-ear, acoustic quality and complete Voice & Video settings UX.
