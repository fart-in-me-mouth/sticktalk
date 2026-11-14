# sticktalk-media

## Desktop integration update (2026-11-12)

The client integration is documented in `../client/VOICE_INTEGRATION.md`.
Normal PowerShell builds no longer need the historical `tools/build-env.ps1`:
use the portable root [build-shell helper](../../deploy/dev-env.ps1) and
[handoff guide](../../HANDOFF.md). They select VS 2022 Build Tools or any other
2022 edition with the C++ workload. The vendored native APM build discovers
MSVC and the existing portable per-user Meson/Ninja/libclang tools itself. See
`vendor/BUILD_CHANGES.md` for prerequisites and precise changes. No installer,
admin rights, global PATH edit, second target directory or prebuilt DSP DLL.

`VoiceOptions::packet_duration_ms` accepts 10 or 20 ms (legacy default 20; desktop
calls choose 10). The adaptive mixer accepts both durations and starts at its
explicit floor, not an empty-histogram-derived 80 ms. Real ingress timestamps
now feed the vendored delay estimator. Native WASAPI first tries IAudioClient3
minimum-period shared mode with a 10 ms auto-convert fallback, and seeds AEC3
from the negotiated endpoint buffers. Physical-device latency is not certified.

The **full** pinned DeepFilterNet3 model now initializes successfully: the
file-only retry took 1.141 s and 100 inference hops took 109.092 ms total on this
PC. High/Auto now use `default-model`, not the LL variant described in the
historical notes below. The CPU probe still selects Standard on an incapable
CPU. The updated call benchmark, rather than initialization alone, is the
CPU evidence, not a speech-latency or acoustic quality qualification. The new
speech-window latency diagnostic fails its coverage gate; <=80 ms mouth-to-ear
remains unqualified. See `../client/bench/voice-2026-11-12/REPORT.md`. No acoustic
superiority claim is made.

`VoiceSession::set_gains` applies validated 0–200% input/output gain without
changing hardware controls, and uses post-output-gain playback as AEC reference.
Picker-only `source_thumbnail` performs a bounded one-shot readback, never a
continuous preview or a readback in the share encode/decode path.

The remaining sections and older BENCH entries describe the pre-integration
baseline; their previous LL/full-model and latency gaps are historical.

Windows call-media APIs only: voice, optional system/process share audio, camera,
GPU screen capture and H.264. No client/server/SFU integration, networking, UI,
permission requests or implicit device activation. All sessions are explicit
start/stop/Drop lifetimes. Nothing polls or spins outside a session.

## Voice

```rust,no_run
use sticktalk_media::{VoiceDevices, VoiceSession, voice::{VoiceOptions, NoiseSuppressionMode}};
# fn example() -> sticktalk_media::Result<()> {
let mut call = VoiceSession::start(VoiceDevices::default(), VoiceOptions {
    noise_mode: NoiseSuppressionMode::Auto,
    ..Default::default()
})?; // only after join/enable, with already-permitted devices
call.add_remote(42)?;
call.set_volume(42, 150)?; // 0..200%
call.set_muted(true)?;
call.set_deafened(true)?; // gates mic too; retains independent own-mute
call.set_push_to_talk(false)?;
call.adapt_bitrate(2, 96_000)?; // loss %, available PAYLOAD bits/sec
// Network worker: push_remote(user, packet), recv_timeout(outgoing packet).
// Drain take_speaking_events(), take_device_events(), take_control_errors().
call.stop()?; // signals native wait, joins, closes clients; Drop does the same
# Ok(()) }
```

- `VoiceOptions`: mono 48 kHz, **64 kbit/s default**, adaptive **32..128 kbit/s**,
  Opus complexity 10, FEC/DTX enabled, 20 ms packets. `adapt_bitrate` applies bounded
  AIMD using caller-supplied congestion/loss feedback and updates Opus's loss hint.
  The library does not invent network estimates or integrate congestion transport.
- NS modes: **Off / Standard (`nnnoiseless`) / HighQuality (upstream DeepFilterNet3 low-latency variant)**.
  `Auto` runs the explicit device-free `benchmark_noise_quality()` on first use
  and caches its recommendation per process. HighQuality is chosen if DFN3 p95
  processing is <6 ms per 10 ms frame; otherwise Standard. Probe errors are exposed
  in `NoiseCpuProbe.failure`. The host can persist this result for first-run
  settings; no hidden file I/O or idle benchmark runs. Compatibility flag
  `noise_suppression=false` always bypasses suppression; NN VAD remains available.
- **Native WebRTC AudioProcessing**, not a custom echo/gain algorithm: AEC3+HPF
  stage, selected external suppressor, then a separate native APM AGC2 stage.
  This places suppression before automatic gain. AGC2 uses native adaptive-digital
  defaults, without changing the user's hardware microphone volume. Native delay
  estimation is enabled; `VoiceProcessor::set_stream_delay_ms` is an advanced
  calibrated override. The render reference is the actual post-volume/deafen mix.
- `VoiceOptions::studio()`: stereo **192 kbit/s default, 128..256 kbit/s range**,
  all microphone DSP/HPF/NS/AGC/AEC/VAD bypassed, Opus music signal mode, DTX off.
  PTT/mute remain deliberate user controls. Voice/profile/channel changes require
  a new negotiated session/track; `configure` rejects silent profile changes.
- Auto/manual input sensitivity: manual -96..0 dBFS RMS, or an NN-VAD-guided
  background-floor estimate plus 10 dB margin (clamped -65..-20 dBFS).
  Voice activity has a 200 ms gate hangover. PTT release delay is configurable
  **0..1000 ms**, default **80 ms**, quantized to capture ticks. Local speaking
  events are threshold crossings only, not a level stream.
- `voice::VoiceProcessor` is sans-I/O: 480-sample mono / 960-sample interleaved
  stereo capture ticks, `render`, `configure`, `capture_stereo`, `selected_noise_mode`,
  `last_processed`, `sensitivity_dbfs`, mute/PTT/adaptive bitrate. No devices,
  workers or timers. Inference and model state exist only while explicitly owned.
- `voice::VoiceMixer` defaults to bounded **adaptive NetEQ-inspired** playout:
  quantile delay estimator, acceleration/preemptive expansion, PLC and time
  stretching from `neteq` 0.9.1. Minimum target defaults to 20 ms, maximum 80 ms
  (a caller selecting a larger floor can intentionally raise it). Maximum 128
  registered streams, 12 packets each, payload <=1275 bytes, 20 ms Opus.
  Unknown/malformed packets cannot create streams or kill the call.
- The codec adapter preserves source gaps with libopus FEC/PLC and discards time
  already concealed by NetEQ. FEC attempts may yield codec PLC if redundancy is
  absent; they are not falsely reported as guaranteed recovery. `repair_statistics`
  exposes attempts/concealment. This is **not a bit-exact Chromium NetEq port**;
  representative loss/drift/music listening qualification remains required.
  `PlayoutStatistics` exposes delay/queue/rate only: upstream 0.9.1 misindexes
  operation-rate counters, so the wrapper deliberately does not expose them.
- `push_at` carries the actual network arrival instant for packet aging. The
  WASAPI session captures that instant in `push_remote`. Upstream's delay manager
  still samples processing time; audio-queue scheduling distortion in its jitter
  estimator remains open. Sequence numbers are extended u64; timestamps
  are 48 kHz sample positions. RTP mapping/unwrapping, negotiation, authorization
  and SFU transport are the host's responsibility.
- `add_stereo_remote` supports Studio/share-audio tracks. Use distinct caller keys
  for a participant's voice and share track. Mixer renders mono or stereo, applies
  0..200% gain, clamps output, and produces remote threshold-crossing events.
  `FixedVoiceMixer` remains an explicit deterministic fixed-delay/FEC alternative.
- Outgoing queues: eight packets; command queue: 128; coalesced speaking state;
  device/control events: 16. Drops/backpressure are bounded, not growing latency.
  Mute/deafen/PTT bypass packet congestion with atomics plus a native wake event.
  Device audio already submitted is not recalled.

### Devices and mic test

- `audio_devices()` returns endpoint IDs, friendly names, direction, console and
  communications defaults. Enumeration opens no streams. Call on an MTA worker.
- Sessions follow **console defaults unless pinned**. Native IMMNotificationClient
  notifications drive hot-plug/default/state changes, with no polling. Pinned
  endpoints remain pinned while available; unplug falls back with a
  `DeviceSwitched { device, fallback }` event for the host's toast. `set_devices`
  changes selection explicitly. Input/output-only operation remains usable when
  the other direction disappears. If neither opens, the worker waits for a native
  notification/explicit selection, not a retry timer. Stop still wakes instantly.
- Default render uses the console/media endpoint. Only the chosen mic is opened;
  no communications category or Bluetooth profile-switch API forces HFP when a
  different mic is selected. **Physical Bluetooth/unplug behavior is unverified**.
  Friendly names and recovery policy have fake enumeration tests.
- `AudioDeviceWatcher` is also available independently for an open settings picker:
  bounded notifications, `Rescan` on overflow, blocking stop channel, no devices.
- `MicTestSession::start` is an explicit test lifetime with `recv_level_timeout`:
  RMS/peak/clip/auto-threshold at <=33.3 Hz, only while open. Default monitor gain
  is **zero**; `set_loopback_gain(0..1)` enables processed stereo playback explicitly.
  AEC is disabled for self-monitoring so it does not cancel the near-end test.
  Configure effects/sensitivity, switch devices, and stop/Drop to close everything.
  The host should recommend headphones before enabling audible monitoring.
- Normal tests never create mic/camera streams. Real-device diagnostic modes need
  `--devices-already-permitted`; this is an assertion, **not** a consent request.
  The library never opens privacy settings, requests access, launches a browser,
  plays notification sounds or binds a socket. Driver calls are not forcibly
  cancellable; a wedged driver is not a certified recovery scenario.

## System/app share audio

`LoopbackSession::start(LoopbackSource::System { output_device }, bitrate)` captures
WASAPI render loopback (console default if None). `Process { pid }` uses
`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, includes that process tree, and
requires Windows build 20348+. Failure never silently captures the whole system.
48 kHz stereo / 20 ms Opus, FEC/DTX enabled, no mic DSP. Sample positions preserve
silence/discontinuity timestamp gaps; partial packets are discarded on a gap.
`recv_timeout`, stop/Drop; no output sound from capture. `OpusEncoder` and
`StereoDecoder` expose device-free codec APIs. Feed received audio to a stereo
mixer track. Silent process activation/stop was tested; content fidelity was not.

## Camera

`camera_devices()` enumerates MF IDs/names without activation. Explicit
`CameraSession::start(device, CameraOptions::default())` enables **1280x720 @30**,
2 Mbit/s hardware H.264. No camera starts just because voice/share exists.
Async MF Source Reader, same D3D11 manager/encoder, keyframe requests, bounded
three-frame output, flush/source shutdown/join on stop. Native timing chosen
before NV12 conversion; 29.97/59.94 accepted within 0.5%, unsupported modes error.
DXGI NV12 uses a GPU subresource copy; USB CPU NV12 uses a separate pitched upload
(no camera zero-copy claim). Queued sample leases drop before MF shutdown.

`CameraFrameEncoder` accepts generated/file NV12 with validated stride/length and
strictly increasing 100 ns timestamps, without a camera or worker. This is the
headless ingress QA seam. **No physical camera exists on this PC**, so native
capture and actual camera CPU remain unverified, not replaced by synthetic data.

## Screen share / decode

- `list_sources()`: monitors (primary flag), visible non-minimized windows, native
  size snapshot and OS handle. WGC's item size supersedes the snapshot.
- `ShareSession::start(source, Quality::default(), bitrate)`: hardware required.
  15/30/60 FPS independently combined with 480/720/1080/1440/Source; default1080p60.
  Scaled presets preserve aspect to even NV12 dimensions. **Odd Source crops only
  the final row/column**, never stretches the source down a pixel.
- `start_with_options`: cursor, CBR/peak VBR, GOP and explicit hardware/fallback/
  software selection. NVENC/AMF/QSV share vendor-neutral MFT enumeration and D3D11
  activation; adapter-vendor-first, then other/unknown names. Fake enumeration
  tests cover ordering/failures/fallback. Only NVIDIA physically tested here.
- OpenH264 fallback is opt-in, reports a notice and caps to a 1280x720 box /30 FPS.
  **NASM is needed for fast assembly**; scalar builds work but are very slow.
- Hardware: WGC BGRA textures -> GPU Video Processor/NV12 -> hardware MFT; no
  staging/readback/pixel copies to CPU. Compressed output only crosses to CPU.
- `set_quality` coalesces FPS/resolution/bitrate changes: old encoder continues
  while another MTA worker prepares a replacement. Commit only on IDR+SPS/PPS;
  old MFT retirement also runs off the paced path. Capture/device stay running,
  at most two encoder pools; failed preparation keeps the old stream.
- Active-share pacing uses a session-scoped high-resolution waitable timer (no
  global timer-resolution changes). No timer remains after stop. Hardware MFT
  events still use bounded 1 ms waits during encoding, not an idle ticker.
- Three-frame/8 MiB-per-frame bounded output; backpressure drops request an IDR,
  terminal errors retained separately. `request_keyframe`, cursor, stats, stop.
  Public size/effective-quality are startup snapshots; frames carry current size,
  generation, timestamp/duration (100 ns), IDR and measured encoding latency.
- `Decoder::new/decode` requires an MTA media worker and strict D3D11VA decoder
  surfaces. Keep the **entire DecodedFrame sample lease** until rendering finishes.
  The device/NV12 texture/subresource handoff is not renderer integration.
  Diagnostic readback is test-only. H.264 implemented; HEVC/AV1 detection-only.
- COM/MF per-session references balance. One process-lifetime MTA usage cookie
  protects windows-rs factory caches; it holds no capture/codec/GPU device.

## Building and safe qualification

MSVC/Windows SDK, Rust stable, Windows 10 2004+ (process loopback20348+).
Native APM is source-bundled/static; the Windows portability sys package is pinned
at 2.1.1. The high-level wrapper is vendored under `vendor` with its original
COPYING and a small documented compatibility change. DFN3-LL source/weights
are pinned to Git `7b14b895fe7fc42cab72b4a4c1e49c982bd6404b` (MIT/Apache).
Tract is pinned to0.21.4 because later caret-selected patches break libDF's API.
The full/non-LL DFN3 model failed Tract codegen with a duplicate Conv bias name;
the working upstream `default-model-ll` variant is deliberately selected, not
silently labeled as full DFN3. Full-model/other-runtime qualification remains open.

Task-local build tools only, no installer/admin/system PATH changes:

```powershell
# Python packages live in the named per-user tools directory, not globally:
python -m pip install --target "$env:USERPROFILE/.cargo-tools/sticktalk-call-hq/python" meson==1.7.2 ninja==1.13.0 libclang==18.1.1
# Extract official NASM2.16.03 ZIP into that tools directory (URL/hash below).
. ./crates/media/tools/build-env.ps1
# Process-local MSVC2022 environment; ONE short target C:/mhq, outside OneDrive.
# Full static CRT: -C target-feature=+crt-static (no VC-runtime installer/DLL).
cargo test -p sticktalk-media
cargo clippy -p sticktalk-media --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p sticktalk-media --release --examples
cargo test -p sticktalk-media -- --ignored --nocapture
# No devices/windows/sound/network; generated and public FILE audio:
cargo run -p sticktalk-media --release --example voice_quality_bench -- --seconds 10 --clean-wav <mono-48k-PCM16-file>
cargo run -p sticktalk-media --release --example callbench -- --video-fixture --seconds 10
```

Meson/Ninja builds native APM/Abseil into the target, never installs system-wide.
MSVC needs the short target path. libclang18 is used for **bindgen parsing only**,
with the STL compiler-version guard overridden; the actual native DSP is compiled
with supported MSVC2022/C++20. Runtime checks validate generated/native aggregate
sizes/alignments before APM creation. A current libclang19+ is preferable for
packaging; the parser workaround is explicitly documented, not hidden.

[NASM ZIP](https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/win64/nasm-2.16.03-win64.zip),
SHA256 `3ee4782247bcb874378d02f7eab4e294a84d3d15f3f6ee2de2f47a46aa7226e6`.
Build-only; deployed clients do not need NASM, Python, Meson or Ninja. Packaging
must retain dependency licenses; patent/distribution review is a host concern.

See [BENCH.md](BENCH.md) for dated results, workload definitions and real gaps.
Digital loopback excludes hardware/acoustics/network. Current adaptive playout
**also misses the <=80 ms digital latency target**; see the measured result/gap
in BENCH.md. No claim of physical
mouth-to-ear <=80 ms, flawless real Bluetooth/hot-plug, DNSMOS superiority or
"10x Discord" is made without the necessary controlled quality/hardware tests.
