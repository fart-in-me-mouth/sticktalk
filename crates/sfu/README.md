# sticktalk-sfu

Embeddable Phase 2 library, now hosted by `crates/server` (see its `voice` module).
Uses pinned str0m 0.25.0 RTP mode with ICE/DTLS/SRTP/SCTP, Opus passthrough,
independent camera/screen publications, receiver subscriptions/RID selection,
per-hop TWCC/REMB/NACK/RTX and upstream PLI/FIR. No codec/capture/device/UI work.

## Embedding API

```rust,no_run
use std::sync::Arc;
use sticktalk_sfu::{Config, Sfu, Command, SignalOut, proto::*};

// A fast in-memory ACL projection supplied by the host. Never trust a
// user/session/channel identity copied out of a client-controlled body.
let acl = Arc::new(|session: VoiceSession, channel: ChannelId| {
    // Replace with current authenticated-session + voice-channel membership ACL.
    let _ = (session, channel);
    false
});
let sfu = Sfu::new(Config {
    bind_ip: "127.0.0.1".parse().unwrap(),
    advertised_ip: "127.0.0.1".parse().unwrap(),
    ports: 50000..=50100,
    video_egress_bps: 32_000_000,
    ..Config::default()
}, acl)?;
// On the host's existing Tokio runtime (no extra runtime/task per participant):
let (commands, received_commands) = tokio::sync::mpsc::channel::<Command>(128);
let (signals, received_signals) = tokio::sync::mpsc::channel::<SignalOut>(128);
// await sfu.run(received_commands, signals) in a host-owned task;
// route received_signals to the corresponding authenticated existing WS.
# Ok::<(), sticktalk_sfu::Error>(())
```

- `Sfu::new(config, Arc<dyn Authorizer>)` validates configuration and binds **one
  nonblocking UDP socket** to the first free port in the inclusive range. Nothing
  starts in the background. Development defaults are loopback-only; configure a
  literal advertised deployment IP explicitly, never a wildcard address.
- `handle_signal(session, nonce, VoiceSignal, Instant)` or
  `handle_envelope(session, VoicePayload, Instant)` mutate the sans-IO engine;
  `drain()` yields `Out::Signal` and `Out::Datagram`. Drain after **each operation**.
- `handle_datagram(source, bytes, Instant)`, `next_timeout()`,
  `handle_timeout(Instant)` support an alternative host-owned event loop.
  `local_addr()` is the advertised endpoint. The bound socket is owned by the
  built-in driver; for a custom I/O harness, the engine still reserves that port.
- `run(self, Receiver<Command>, Sender<SignalOut>)` consumes the engine and
  integrates UDP + signaling + exact str0m deadlines with `tokio::select!`.
  Commands: Signal, Disconnect, Refresh, Move, trusted Path telemetry, Shutdown. The UDP socket has fixed 256 KiB send/receive buffers. No empty-call
  ticker or media work. Output backpressure fails the actor closed so the host can revoke its ACL and
  restart, rather than losing cleanup notifications or accumulating unbounded signaling/media. Internal queues cap at 4096 items;
  callers must treat Backpressure as overload and fail closed/restart the actor.
- `disconnect(session)` is idempotent. The host **must call it** on WS loss,
  logout, kick, channel deletion or revoked access. It removes routes, RTP caches,
  ICE/crypto state and control grants. It also invalidates requests/grants held
  by other peers for that connection. IDs must be unique for each call lifetime, including same-WS leave/rejoin.
- `set_path(session, MediaPath)` is trusted telemetry, not a client assertion.
  Tailscale DERP is below UDP/ICE and cannot be inferred from ICE host candidates.
- `encode_signal` / `encode_event` build the original four-field VoicePayload
  envelope, with named-map MessagePack typed bodies. Complete normative wire
  catalog, limits and client obligations: `docs/PROTOCOL.md` section 10.

### Negotiation / client work

Join with mandatory H.264 capability. Client is always SDP offerer, sends its
complete publication set, and receives an Answer. For each received source,
first negotiate a recvonly transceiver, then Subscribe its MID to the source
MID + optional RID. A receive slot cannot also publish. Publications/participants
are full replacement events. Reconnect starts fresh; voice/grants are not replayed.
Advanced codecs require every call participant's explicit support **and hardware
decode**; an incompatible join stops those packets and requests H.264 fallback.
Profiles/formats are matched exactly, never transcoded. Native `sticktalk-media`
Annex-B output still needs client-side RTP packetization to <=1180-byte payloads for the tested RTP extension profile,
ICE/signaling integration, and receiver depacketization/hardware decode.

RTP payloads use shared Arc slices, with per-receiver MID/PT/SSRC/sequence
rewriting; original source ordering is retained. New subscriptions/layer switches
use fresh SSRCs and keyframe gating. H.264 IDR/FU-A/STAP-A, HEVC IRAP/FU and AV1
new-coded-sequence detection are supported. AV1 switches conservatively wait for
a new sequence; arbitrary AV1 intra frames without N are not inspected. Request
an encoder restart/new-sequence keyframe when switching AV1 layers. No temporal
SVC extraction or automatic RID ladder algorithm: receiver policy picks a RID.

### Resource/network policy

- Final video UDP egress (including RTX, SSRC-zero probes, headers and SRTP tags)
  obeys the global byte bucket. 100 ms burst, minimum one max-size datagram.
  Fair per-subscription budgets + minimum receiver BWE (10% overhead headroom) are signaled upstream via
  REMB and Bitrate; hard dropping is the safety net for noncompliant senders.
- **All SRTP UDP payloads <=`max_datagram_bytes`** (default 1232), including RTX.
  Actual incoming and rewritten encrypted outgoing datagrams are checked; there
  is no fixed 1060-byte compressed-payload cutoff. Standard browser-sized RTP
  passes. Oversized media is dropped/counted, not fragmented/repacketized.
  Configure 1200–1472 bytes according to the actual path (Tailscale MTU 1280:
  IPv6 UDP <=1232, IPv4 <=1252).
- Audio/control/RTCP/ICE are outside the video cap. Host connection/datagram rate
  controls remain necessary for non-video abuse. Unpaced audio is not sacrificed
  to a congested screen share. Pacer queue admission is bounded to 128 packets/128 KiB and rejects an oldest queued age over 250 ms; overflow discards the queue and restarts on a keyframe with a fresh SSRC;
  RTX caches are 256 packets/video stream, 32/audio, one second.
- Stats are per-session and event-driven ~1 Hz while in a call. Bitrate reports
  actual emitted UDP bytes (not cap-rejected writes). Marker FPS is a transport
  measure, not decoded/rendered FPS; RTT/loss/path remain unknown until measured.

### Remote control: fail closed

Only the sharer can approve a pending 30-second request, while a monitor/window
share exists and the requester subscribes to it. One controller at a time, one
random session-bound grant; no remember option. Binary ordered reliable channel
`sticktalk-control-v1`, 512-byte messages, 1000 events/server-second. Wrong
sessions/grants cannot inject; matching-grant replay, invalid input, rate excess
or blocked SCTP revokes control (do not silently drop key-up). Revoke sends
ReleaseAll on SCTP plus ControlRevoked on WS. Unsubscribe, re-offer, stream stop,
data-channel loss and disconnect invalidate grants. Only grant/revoke metadata
is logged, never input.

**Native client responsibilities remain essential:** ignore stale-grant queued
input, release held keys/buttons on either revoke message or any disconnect,
limit normalized input to the real captured area, implement SendInput, physical
mouse takeback, Ctrl+Alt+Shift+S, persistent banner and viewer capture/Esc policy.
The SFU cannot inspect the sharer's actual desktop or inject OS input.

## Validation / benchmark

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-sfu'
cargo test -p sticktalk-sfu -p sticktalk-proto
cargo clippy -p sticktalk-sfu -p sticktalk-proto --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p sticktalk-sfu --release --example loopback -- --seconds 30
```

Ten in-process tests connect two/three **real str0m peers** over a deterministic
packet harness. They cover actual ICE/DTLS/SRTP/SCTP, audio/camera/screen receive,
mute/deafen/speaking, leave/cache cleanup, PLI/FIR, induced loss recovered by RTX,
MTU/cap and measured stats, consent/timeout/revoke/controller identity, the 1000
input limit/replay, layer selection/switch gating, HEVC/AV1 and H.264 fallback.
Two Tokio driver tests cover schema errors, clean shutdown and fail-closed
output backpressure. Full-server HTTP/WS/UDP peer tests live in `crates/server/tests/voice.rs`. Proto tests cover legacy envelopes and all added JSON/MessagePack shapes.

`loopback` is headless. A **separate child process** runs the SFU and real UDP
socket; one sender + four receivers run in the parent. The child is explicitly
created with CREATE_NO_WINDOW on Windows. The workload is synthetic H.264 RTP
sized as 8 Mbps, 60 frame markers/sec (1080p60-class), not actual captured video.
No encode/decode/capture/device/browser runs. CPU is the child process's
kernel+user CPU divided by wall time, as % of **one logical core**, not diluted
by machine core count. RAM is its working set; parent peer RAM/CPU are excluded.
Windows uses GetProcessTimes/GetProcessMemoryInfo; Linux uses /proc (USER_HZ=100).
Measurement follows a 3-second streaming warmup for BWE. The benchmark uses a 64 Mbps video cap so 4 x 8 Mbps payload + headers/probes fits.
See BENCH.md for the measured run and acceptance gaps.

**Browser packet-size admission fixed:** real-str0m tests forward 1180-byte
compressed RTP payloads (~1226 bytes encrypted UDP), including through the full
server. The incoming datagram and final rewritten SRTP/RTX must fit the configured
ceiling. There is no codec-aware repacketizer; unusual extension sets/large RTP
still drop at that boundary. Real-browser SDP/playback remains unvalidated.

Not yet validated: physical Pi 4/ARM performance, real native/browser endpoints,
real H.264 hardware stream playback, Tailscale direct/DERP network conditions,
and the 10-minute capture/display acceptance. Server migrations, channel ACL projection, WS routing and guild-wide state are
implemented in `crates/server`; client OS-control features remain separate.

The integrated host implements `Authorizer::permissions`, `send_audio`,
`receive_audio`, `relocated_channel` and `generation` over its synchronous
session-bound role projection. The closure adapter preserves the standalone
join-only API and grants all publication rights to joined sessions; hosts that
need roles must implement the full policy interface. Increment generation before
acknowledging ACL/mute/move changes, enqueue Refresh (Move for a room change),
and fail closed if the command queue is full. Generation invalidates queued UDP
output, including a driver batch. Offers, forwarding and encrypted RTP/RTX egress
recheck track-specific permissions. Unmapped/withdrawn SSRCs are never sent.
