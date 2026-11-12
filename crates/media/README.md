# sticktalk-media

Standalone Windows media proof for StickTalk. No client/server/protocol changes,
networking, audio, or application UI. `motion_fixture` is **only** a disposable
benchmark stimulus, not client UI.

## Public API / integration contract

```rust
use std::time::Duration;
use sticktalk_media::{list_sources, ShareSession, Quality, Decoder};

// Run codec discovery / Decoder on a dedicated MTA media thread, not an STA UI
// thread. ShareSession creates its own MTA capture/encode worker.
let source = list_sources()?.into_iter().find(|s| s.primary).unwrap();
let mut share = ShareSession::start(source, Quality::default(), 8_000_000)?;
let mut decoder = Decoder::new()?;
let encoded = share.recv_timeout(Duration::from_secs(3))?;
for decoded in decoder.decode(encoded)? {
    // Hand decoded.texture + decoded.subresource to a renderer using
    // decoder.device(). Keep the ENTIRE DecodedFrame sample lease alive until
    // GPU rendering completes. Do not keep a growing queue of decoder frames.
}
share.request_keyframe();
share.set_quality(Quality::default(), 6_000_000)?;
share.set_cursor(false);
share.stop()?;
# Ok::<(), sticktalk_media::Error>(())
```

- `list_sources() -> Result<Vec<Source>>`: monitors, primary flag, visible
  non-minimized windows, native size snapshot and opaque OS handle. WGC's actual
  item size supersedes the enumeration snapshot when starting capture.
- `hardware_encoders()`: names of hardware H.264 / HEVC / AV1 MFTs registered on
  this system. Enumeration is availability, **not** proof each device/format
  combination works. Start tries the hardware H.264 candidates with D3D11.
- `Quality { fps: FrameRate, resolution: Resolution }`: 15/30/60 independently
  combined with 480/720/1080/1440/Source. Default **1080p60**. Scaling preserves
  source aspect ratio to even chroma-compatible dimensions; 16:9 480p is 852x480.
  Source uses the exact native size without resizing. **Odd-sized Source is
  explicitly rejected**, never silently scaled/cropped. Choose a scaled preset
  for odd-sized windows. Padding/clean-aperture support is a remaining gap.
- `ShareSession::start(source, quality, bitrate)` requires hardware. For advanced
  controls, `start_with_options(source, ShareOptions)` adds cursor, CBR/peak-VBR,
  GOP length and `EncoderSelection` (`HardwareOnly`, `AllowSoftwareFallback`,
  `SoftwareOnly`). Explicit software use/fallback reports `EncoderKind::Software`
  and `fallback_notice`, caps at 30 fps and a 1280x720 bounding box. No silent CPU
  fallback. Bitrate is bits/sec, validated to 100 kbit/s..100 Mbit/s.
- `recv_timeout` is a blocking stream interface over a **three-frame bounded**
  channel. Integrate from the application's media/network worker, not a UI
  reducer. There is no additional async runtime. Encoded frames are bounded at
  8 MiB each. A slow consumer causes drops and forces a recovery IDR. Fatal errors
  are retained even when the queue is full. `stop()` / Drop joins the worker.
- `EncodedFrame`: Annex-B H.264 access unit, `timestamp` / `duration` in 100 ns
  session-relative units, IDR flag, output size, generation and measured latency.
  IDRs carry SPS/PPS. Hardware output timestamps are checked for reordering.
- `set_quality(quality, bitrate)` coalesces requests and creates the next encoder
  generation with IDR/SPS/PPS. Capture and its D3D device **stay running**.
  The encoder MFT is replaced because vendor dynamic media-type changes are not
  universally supported. There can be a short encoder-only pause; this is not
  a promise of seamless reconfiguration. Decoder renegotiates at the generation's
  first IDR. Control errors arrive on the output stream. Public `size` and
  `effective_quality` are **startup snapshots**; frames carry current settings.
- `request_keyframe`, `set_cursor`, `stats`, `stop` are independent controls.
- `Decoder::new()` / `decode(EncodedFrame) -> Result<Vec<DecodedFrame>>` uses
  Microsoft's synchronous H.264 MFT with DXGI device manager / D3D11VA. It checks
  hardware decoder configurations and requires decoder-bound DXGI surfaces;
  CPU-memory or CPU-upload output is rejected. This MFT is not enumerated as an
  asynchronous hardware *encoder*: its reconstruction is hardware-accelerated.
- `DecodedFrame` retains the MF sample lease and exposes its NV12 texture, array
  subresource, visible size and timestamp. `Decoder::device()` exposes the D3D11
  device for rendering. Renderer integration must arrange same-device use or GPU
  interop/copy explicitly; this crate does not integrate iced or share handles.
- `Decoder::readback_rgba_for_test` is **diagnostic only**. It performs a staging
  readback and CPU NV12-to-RGBA conversion. Never use it for production playback.

## Pipeline and ownership

WGC free-threaded pool (three BGRA textures) → latest captured texture/sample
lease → D3D11 Video Processor (resize + full-range RGB to studio-range BT.709 NV12)
→ hardware MFT via `IMFDXGIDeviceManager` → CPU **compressed bytes only** → decoder
MFT / D3D11VA → NV12 decoder texture/sample lease.

There is no pixel `Map`, staging texture, CPU BGRA conversion, or pixel readback
in hardware capture/encode/decode. Software fallback has its own clearly separate
NV12 staging/I420 path. Hardware encoding is serial low-latency, one outstanding
surface, so the converter never overwrites a surface still being encoded. Async
MFT NeedInput/HaveOutput events are honored; waiting uses bounded 1 ms sleeps,
not spinning. Frame pacing is deadline-based, with no accumulated catch-up queue.
Static source frames can be repeated at the selected pacing rate; statistics
separate source arrivals, repeats, scheduled drops and encoded output.

COM/MF references are apartment-affine and balanced. One lazily-created
**process-lifetime MTA usage cookie** protects windows-rs' cached agile WinRT
activation factories across sequential sessions. It does not keep a capture,
encoder, decoder or GPU device running. This fixes the reproduced second-session
use-after-apartment-teardown crash. No capture/encode/decode work runs after stop.

## Build / benchmark

Requires native MSVC C++ tools/Windows SDK and Windows 10 2004+; measured on
Windows 11 build 26200. Rust stable. OpenH264 is compiled from source by its crate;
no DLL download is required. Package/license/patent distribution review remains a
product packaging responsibility.

```powershell
$env:CARGO_TARGET_DIR = "C:/Users/dwhee/.cargo-target/sticktalk-media"
cargo build -p sticktalk-media --release
cargo test -p sticktalk-media
cargo clippy -p sticktalk-media --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p sticktalk-media --release --examples
cargo run -p sticktalk-media --release --example sharebench -- --list
# Run the fixture in a SEPARATE terminal/process (it closes automatically):
cargo run -p sticktalk-media --release --example motion_fixture -- 180
# Capture primary monitor, hardware encode and hardware decode in this process:
cargo run -p sticktalk-media --release --example sharebench -- --seconds 30 --matrix
# Sender-only budget measurement:
cargo run -p sticktalk-media --release --example sharebench -- --seconds 30 --no-decode
# Additional real hardware / control / fallback smoke tests:
cargo run -p sticktalk-media --release --example sharebench -- --seconds 2 --all-qualities
cargo run -p sticktalk-media --release --example sharebench -- --seconds 6 --reconfigure --vbr --readback
cargo run -p sticktalk-media --release --example sharebench -- --seconds 3 --software --readback
cargo run -p sticktalk-media --release --example sharebench -- --seconds 3 --window "StickTalk GPU benchmark pattern" --resolution source --no-cursor --readback
```

The optional `STICKTALK_MEDIA_TRACE=1` installs a native exception logger **only
inside sharebench**, useful for driver/COM diagnostics. It is never installed by
the library. `--report PATH` appends a measured Markdown table. Read [BENCH.md](BENCH.md)
for measured results, metric definitions, targets and remaining acceptance gaps.
