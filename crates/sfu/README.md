# sticktalk-sfu

Embeddable Phase 2 library, now hosted by `crates/server` (see its `voice` module).
Uses pinned str0m 0.25.0 RTP mode with ICE/DTLS/SRTP/SCTP, Opus passthrough,
independent camera/screen publications, receiver subscriptions/RID selection,
per-hop TWCC/REMB/NACK/RTX and upstream PLI/FIR. No codec/capture/device/UI work.

## Sparse-codec robustness / deployment requirement

The High-only renegotiation panic was reproduced in str0m 0.25.0 at
`format/payload_params.rs::Claimed::assert_claim_once` (`Pt locked multiple
times: 112`). Our old configuration included both High level 3.1 (`64001f`)
and High level 5.1 (`640033`). str0m treats levels as negotiable when matching
H.264 **profiles**; a sparse recvonly/sendrecv offer lets both local entries claim the
same remote PT. `rtc_config()` now configures one entry per profile and
packetization mode, with a single High entry at level 5.1. No client codec
ladder is required. The socket-free upstream reproduction intentionally
panics; the actual SFU regression answers both initial and sparse re-offers.
No dependency fork/upgrade or SDP capability fabrication is involved.

The input policy rejects ambiguous bundled PT/profile/RTX mappings, invalid
MID/RID identifiers and non-UDP/unspecified/multicast ICE endpoints before
str0m mutation. MessagePack requires full consumption and depth <=32. SDP is
bounded before parsing (64 KiB, 2048 lines, 2048 bytes/line); there are at most
33 lifetime MIDs, 128 lifetime signaled SSRCs, and 128 retained ICE candidates
per peer (embedded and trickled together). These are conservative policy
limits, not a claim to accept every SDP extension or arbitrary codec. Offers
that fail str0m negotiation terminate that peer because acceptance may have
partially mutated its session. The SFU is always answerer; client answers
are invalid signaling variants, not another negotiation entry point.

Signaling, UDP input, timeout and data-channel event failures are contained
per peer. A fatal peer gets a generic correlated `Unavailable` error **before**
`Left`, so the host can deliver it before removing the lease. Its routes,
grants and RTC are discarded; other peers/rooms keep running. Logs contain
session identity only, never SDP, panic payloads or upstream SDP errors. The
panic hook suppresses payloads only inside guarded SFU operations and preserves
the prior hook outside them. This is a last-resort boundary, not validation.

**Build servers with unwinding enabled**, including dependencies:

```powershell
$env:CARGO_PROFILE_RELEASE_PANIC = 'unwind'
cargo build --profile server -p sticktalk-server
```

The workspace's existing default release profile uses `panic=abort`, which
cannot implement an in-process error boundary. `Sfu::new` rejects such builds
with typed `PanicStrategy` **before binding a socket** rather than silently
providing false isolation. This does not change the client's abort profile.
OOM/process termination cannot be recovered by an unwind boundary.

### Additional quotas

| Resource | Default enforcement |
|---|---|
| Peers | 64 globally, 16/room (existing configurable caps) |
| Published tracks | 4/peer, 48/room; room moves also checked |
| Video encrypted UDP egress | 32 Mbps/peer, 64 Mbps/room, existing 32 Mbps global; each has 100 ms burst, at least one MTU packet |
| ICE candidates | 128/peer lifetime, 1024/room retained; duplicates do not consume another slot |
| Offers, including invalid attempts | rolling 30/peer/minute, 120/room/minute |
| Control messages | 512 bytes, named MessagePack, grant/session/replay validation, 1000 accepted inputs/server-second/grant |
| Coordinates | unsigned 16-bit inclusive [0,65535]; out-of-range/negative wire values rejected; wheel [-4096,4096] |

Audio/control are deliberately outside the **video** bitrate buckets. Bitrate
feedback is capped per peer/room too, but final encrypted wire enforcement
remains authoritative, including RTX. Room quota maps disappear when empty.
Each viewer's video bucket is additionally paced at its current fair global/
room allocation, capped by the configured peer ceiling. Changing an allocation
never refills tokens. The encoder receives the slowest subscribed BWE ceiling
with 25% wire headroom for RTP/SRTP, repairs and bursts (two 1440p60 viewers:
12 Mbps payload each within the unchanged 32 Mbps aggregate wire limit).

Original-media backlog is tracked until encrypted egress, independently of
str0m's probe-padding/RTX snapshot. Its byte bound follows bitrate times a
100-250 ms two-RTT horizon, with a 256 KiB IDR floor, a 2 MiB/2048-packet hard
ceiling, and a 250 ms age limit. A full wire bucket defers RTC draining to a
one-shot token-refill deadline instead of popping and discarding an original.
Whole-RTC deferral is limited to 10 ms: an extremely low video quota must not
stall voice, RTCP or remote-control traffic for a full-packet refill. Existing
permission checks and generation fencing remain at encrypted egress.

Lifetime MID/SSRC/ICE limits intentionally require a fresh join when exhausted;
repeated offers cannot grow str0m's retained session state indefinitely.

### Robustness test modes

Seven `prop_*` suites default to **3072 cases each** (21,504 total), without
forking, UI, devices or non-loopback sockets. They cover SDP offers and answers,
ICE, MessagePack control/coordinate parsing, control grants/rate/replay/bounds,
layer selection, and stateful renegotiation sequences. Both arbitrary and
structured valid/mutated inputs are exercised. CI can run the normal crate test
command. `robustness-ci.toml` optimizes dependency arithmetic/parsing only;
SFU/server code and dependency debug assertions remain enabled. Each stateful
case has fresh RTC/ICE state; parser tests share a DTLS certificate, not a
session. The longer local mode (6144 cases/suite, 43,008 total) is:

```powershell
$env:CARGO_TARGET_DIR = '$env:USERPROFILE/.cargo-target/sticktalk-sfu-robust'
$env:CARGO_INCREMENTAL = '0'
$env:PROPTEST_CASES = '6144'
cargo --config crates/sfu/robustness-ci.toml test -p sticktalk-sfu --lib prop_ -- --test-threads=1
```

Deterministic tests cover quota edges, a forced peer panic across two rooms,
real encrypted per-peer/room cap enforcement, the existing global cap/1232-byte
MTU paths, and host WS error/Left delivery while another real SRTP call survives.

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
$env:CARGO_TARGET_DIR = '$env:USERPROFILE/.cargo-target/sticktalk-sfu'
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

### Robustness validation — 2026-11-12

- CI mode (`CI=true`, `PROPTEST_CASES=3072`): 134 tests passed across SFU/server;
  seven property suites, **21,504 cases**. Existing global egress cap, RTX and
  1232-byte MTU tests passed, as did the real encrypted peer/room caps.
- Local long mode (`PROPTEST_CASES=6144`): seven suites passed, **43,008 cases**.
  Total completed property cases: **64,512**. All sockets were loopback-only.
- Commands passed (one dedicated target directory, removed after validation):
  `cargo --config crates/sfu/robustness-ci.toml test -p sticktalk-sfu -p sticktalk-server --no-fail-fast`,
  `cargo --config crates/sfu/robustness-ci.toml test -p sticktalk-sfu --lib prop_ -- --test-threads=1`,
  `cargo clippy -p sticktalk-sfu -p sticktalk-server --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`.
