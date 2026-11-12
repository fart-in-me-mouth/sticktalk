# SFU loopback benchmark

Measured 2026-11-12, Windows 11 Home 10.0.26200, AMD Ryzen 7 5700X
(8 cores / 16 logical processors). Release profile, pinned str0m 0.25.0,
Rust-crypto backend. Not a Raspberry Pi result.

## Reproduce

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-sfu'
cargo run -p sticktalk-sfu --release --example loopback -- --seconds 30
```

A headless child process runs only `Sfu::run`, including the real multiplexed
UDP socket and signaling actor. The parent runs one str0m sender and four
str0m receivers over actual loopback UDP. No browser, OS window, screen capture,
audio device, hardware encoder, or decoder is opened. Windows child creation
explicitly uses CREATE_NO_WINDOW.

Workload: synthetic H.264 FU-A RTP, 16,666 compressed bytes/frame, 60 frame
markers/second, nominal **8 Mbps per source**, 1080p60-class traffic. The payload
is an opaque transport stimulus, **not a decodable 1080p video**. str0m still
performs real ICE, DTLS, SRTP authentication/encryption, feedback, pacing and
RTX. SFU receives one upload and forwards to four independent SRTP sessions.
After three seconds of streaming BWE warmup, measure 30 seconds. The benchmark
sets a 64 Mbps global video cap so four 8 Mbps payload streams plus transport
headers/probes fit. Default application cap remains 32 Mbps.

## Final measured run

- Duration: **30.012 seconds**; 1800 source frames during measurement.
- SFU CPU: **11.662% of one logical core**, kernel + user process CPU / wall time.
- SFU working set: **14.09 MiB**, peak observed **14.09 MiB**.
- Receiver payload throughput: **7.860–7.998 Mbps** each.
- Receiver RTP marker frame rate: **58.88–59.98 fps** each.
- Largest observed RTP/RTX UDP payload: **1046 bytes**, below the 1200-byte cap.
- Global-cap drops: **0** in every reported session.

Raw output:

```text
SFU child PID: 21052
duration=30.012s; SFU CPU=11.662% of one logical core; SFU working set=14.09 MiB; peak observed=14.09 MiB
synthetic H264 RTP: 1920x1080/60-sized workload, 8 Mbps payload, 4 receivers; NO capture/encode/decode; input_frames=1800; warmup=3s
receiver 1: 7.860 Mbps payload, 58.88 fps marker rate, packets=30077, max UDP=1046 bytes
receiver 2: 7.983 Mbps payload, 59.88 fps marker rate, packets=30550, max UDP=1046 bytes
receiver 3: 7.998 Mbps payload, 59.98 fps marker rate, packets=30605, max UDP=1046 bytes
receiver 4: 7.895 Mbps payload, 59.18 fps marker rate, packets=30211, max UDP=1046 bytes
max per-session cap drops=0; configured global video cap=64 Mbps
```

CPU/RAM come from GetProcessTimes/GetProcessMemoryInfo for the **SFU child**,
excluding all five parent peers. CPU is not divided by 16 machine threads.
Working set excludes kernel networking buffers; the SFU requests fixed 256 KiB
send and receive UDP buffers. Parent sockets use the same bounded sizes.
Frame rate counts received RTP marker packets, not decoded/displayed frames.
Receiver differences reflect UDP/BWE/pacing behavior; full zero-loss 8.000 Mbps
at every receiver is not claimed. Measurements are single desktop runs while
the owner/other build tasks may be active, not a stable hardware capacity limit.

The benchmark exposed two bugs that were corrected: duplicate SDP/default TX
SSRCs can starve a rewritten RTP stream's MID-based pacer, and BWE needs an
explicit nonzero desired subscription capacity to recover from idle. Fixed-size
larger UDP buffers avoid small-default-buffer keyframe-burst losses. str0m's RTX
cache size does not bound its send queue; admission is explicitly bounded and
stale queues restart on a keyframe instead of growing indefinitely.

## Validation and remaining acceptance

Passed:

```text
cargo test -p sticktalk-sfu -p sticktalk-proto
cargo clippy -p sticktalk-sfu -p sticktalk-proto --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p sticktalk-sfu --release --example loopback -- --seconds 30
```

SFU coverage: 9 two/three-peer tests and 1 Tokio driver test; the shared proto suite also passes. Actual peers cover audio/camera/screen, mute/deafen/speaking, leave/cache
cleanup, PLI/FIR, deliberate loss repaired by RTX, the aggregate cap and MTU,
remote consent/denial/timeout/revoke/rate/replay/share-stop, RID selection and
keyframe-gated switching, HEVC/AV1 capability intersection and H.264 fallback.
The low-cap test verifies the token-bucket byte inequality including probes/RTX.

Not done here:
- Real Pi 4/ARM or Tailscale direct/DERP runs; no performance extrapolation.
- Real browser/native endpoints and browser packet-size interoperability. The
  current RTP forwarding contract admits <=1060-byte payloads and rejects
  oversized packets; senders must respect that contract. No server-side
  arbitrary-codec RTP repacketizer is provided. This is a functional browser
  interoperability gap, not just missing browser test coverage; see README.md.
- Native Annex-B -> RTP integration, real hardware decode/render and the owner's
  ten-minute 1080p60 game/video acceptance. These numbers do not satisfy that
  capture/display acceptance by themselves, or measure end-to-end latency.
- Automatic RID ladder selection/SVC extraction. Receivers explicitly pick a
  sender-provided RID. AV1 switches conservatively require a new coded sequence.
- Server wiring, persistence/ACL adapter and client remote-control OS injection,
  bounds, local takeback/hotkey/banner, held-key cleanup on transport loss.

Tailscale relaying cannot be inferred from an ICE host candidate; the trusted
host must feed `set_path`/`Command::Path`. Unknown remains unknown otherwise.

Workspace note: the full `cargo fmt --all -- --check` passed earlier in this
run, but its final recheck found concurrently edited, unformatted `proto/tests/wire.rs`
and server files. They were not reformatted by this task. The SFU package and
owned proto files have separate clean formatting checks. Cargo also regenerated
the shared Cargo.lock for the new SFU dependencies; no manual lockfile edits or
server/rest.rs edits were made.
