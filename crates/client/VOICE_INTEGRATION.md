# Calls2 video, picker and remote-control integration

Round-2 evidence: [`bench/calls2-2026-11-12/REPORT.md`](bench/calls2-2026-11-12/REPORT.md).
**End-to-end release acceptance is still failing:** viewers 12.837/12.950 fps
and +166/+170 ms drift; sharer CPU 0.633%, WS +101.85 MiB. Do not label this
launch-ready. The round-1 status below is historical; its missing GPU bridge, camera chooser,
monitor application-audio chooser and disabled native-control statements are
superseded by this section. Audio/settings remain owned by the parallel work.

- Decoded NV12 stays on the GPU: D3D11 plane shader → immutable shared BGRA
  texture + shared fence → DX12/wgpu iced primitive. Decoder textures need a
  GPU copy before plane sampling; there is no production streaming readback.
  This requires a DX12 renderer on the same adapter. Other adapters/backends
  fail closed; cross-adapter and software display fallback are not qualified.
- The sender explicitly declares desired BWE capacity (without a bandwidth
  floor), so an idle-to-video transition can probe upward. Unsent video is
  bounded independently of the RTX cache; congestion drops dependent frames
  and requests a fresh IDR rather than accumulating latency and RAM.
- Screen playback, local/remote camera tiles and source previews use the bridge.
  Picker snapshots refresh only while open, with measured tiny CPU fallback.
  Logical window/channel/focused-view visibility unsubscribes video layers and
  discards decoders; per-pixel grid occlusion is not tracked.
  Focus/fullscreen/pop-out share the call transport; the stats overlay is optional.
- Picker includes monitor/window tabs, explicit application process audio,
  camera selection and persisted 1080p60-default quality. Quality updates the
  active share without republishing. Source labels show the selected real size.
- Native injection, passive revoke hooks and a no-activate always-on-top Stop
  banner are one fail-closed safety unit. Real mouse input closes an atomic
  injection gate before notifying an independent release thread. Stop/disconnect
  signal this thread before a potentially full media queue. All held keys and
  buttons are released; retired grants cannot re-enable injection.
- Input uses USB HID scan-code construction, physical PMv2 surface bounds,
  negative-origin virtual-desktop mapping and pixel-center absolute coordinates.
  Window focus is restricted to the consented HWND; monitor keyboard input
  requires the foreground window to remain wholly inside the shared monitor.
  Viewer capture requires a local video click and ends on focus loss or Esc held
  for one second. Consent expires after 30 seconds and controller state is shared.
- **Safety:** no real SendInput, microphone, camera, sounds, notifications or
  visible windows were used in qualification. Input construction and recording
  mock tests are not live OS integration acceptance. Native banner placement,
  physical hotkeys/takeback, games and DPI behavior still require a controlled
  manual test on a dedicated machine, never the owner's active PC.

---

## Historical round-1 status

# Native call integration — implementation status

This is a **partial Phase 2 integration**, not launch acceptance. No real
microphone/camera, input injection, visible window, notification or sound is
used by the automated qualification.

## Boundaries

- `client-core::voice`: bounded ephemeral call/guild projection, server gates,
  reconnect/Ready/Resumed fresh rejoin, per-user settings, consent/timeout model,
  strict versioned named-map signaling. Voice requests use the existing single
  chat WS driver, never durable command receipts or reconnect replay.
- `client::voice::peer`: real str0m offer/answer, ICE/DTLS/SRTP/SCTP, complete
  publication replacement, lazy reusable recvonly slots before subscription,
  bounded H.264 Annex-B/FU-A/STAP-A transport, audio levels and keyframe requests.
  Total media m-lines are capped at 32; inactive slots are reused by media kind.
- `client::voice::native`: explicit joined-call ownership of VoiceSession,
  hardware ShareSession/CameraSession, optional process-only LoopbackSession,
  remote audio mixing, hardware video decode, device fallback toasts, bitrate
  hints and live share quality changes. Nothing starts on import or when idle.
  Bounded command/event queues fail closed; sockets bind loopback by default.
  `STICKTALK_CLIENT_VOICE_BIND_IP` permits an explicitly chosen unicast local
  address; wildcard, multicast, broadcast and hostname binds are rejected.
- `voice_ui` + `ui/{call,share_picker,stream_view,voice_settings}` are additive
  main-window hooks for the parallel shell/settings work. Voice rows, speaking
  rings, LIVE flags, connected panel, call controls and local audio context menu
  are driven by this projection. Preferences are bounded and validated, with
  debounced disk persistence; mock screenshots never touch real preferences.
- Picker thumbnails are one-shot 160-pixel WGC snapshots, at most 32 sources,
  250 ms/source timeout, bounded staging readback. Capture resources close before
  the results reach the UI. This readback is **not** in the streaming path.
- Passive Windows low-level keyboard/mouse hooks use their own message queue,
  never a visible window and never swallow game input. PTT release delay is
  applied in the media engine. Ctrl+Shift+M/D and Ctrl+Alt+Shift+S are supported.

## Deliberately incomplete / fail closed

1. **No GPU surface-to-iced renderer bridge.** The receiver assembles and
   hardware-decodes video, but drops the bounded surface leases after decode.
   It must not be represented as working video display. Focused stream UI says
   display is unavailable. Camera preview, rendered video tiles and fullscreen
   playback are not implemented.
   Focused-view audio controls currently change the publisher's local mix,
   not an independent stream-audio fader; stats are transport-level.
2. **Production remote control is disabled.** Consent/rate/sequence/surface
   bounds and held-key release are tested with a dry-run injector. The native
   driver does not call SendInput; incoming control causes revoke. Allow is not
   offered until a persistent always-on-top Stop banner and native surface-bound
   injector can be installed as one safety unit. Viewer input capture/Esc-release
   are also not implemented. Never enable injection without those safeguards.
3. Remote/Tailscale operation needs an explicitly selected reachable local
   address (`STICKTALK_CLIENT_VOICE_BIND_IP`) and deployment acceptance; only
   loopback was tested. No wildcard bind or unsafe automatic interface
   enumeration is hidden in tests.
4. Voice settings model and live Configure binding exist. Full device dropdowns,
   live mic-test meter/loopback UI, PTT rebinding UI, split-button menus and final
   settings parity remain for integration with the parallel settings agent.
   Studio profile changes require rejoining; the engine rejects silent channel
   layout changes. Headless tests never start the native device driver.
5. Process loopback is wired for window shares (PID from HWND); a monitor share
   with Include Audio asks for an application instead of capturing everything
   silently. Monitor process selection and a camera device chooser remain.
6. Join/leave/mute cues have a desktop-platform policy hook, unset by default.
   No sounds are played by this work. Physical Bluetooth/hot-plug, game-focus hotkey behavior,
   acoustic AEC and mouth-to-ear latency still need controlled manual acceptance.
7. The 60-second FILE video test is decode/transport qualification, **not** a
   10-minute moving desktop/game, two-viewer encode/capture/render acceptance.
   Digital voice measurements exclude WASAPI, acoustics and network.
8. The SFU's str0m 0.25 answerer can panic on renegotiation when a sparse
   H.264 High-only offer makes two local profile levels claim one remote PT.
   The client works around this by offering the SFU's full H.264 profile ladder
   and explicitly sends the negotiated High/5.1 payload type. This normal-client
   workaround is not server-side hardening against malicious SDP; the server
   owner should fix that upstream/validation boundary separately.
9. Actual desktop idle working set/private bytes, physical call CPU, acoustic
   mouth-to-ear latency and DNSMOS/PESQ were not measured. The included numbers
   are explicitly file-only DSP or whole synthetic transport/decode harnesses.
10. **The <=80 ms mouth-to-ear requirement is not met/qualified by this work.**
    Off/Standard synthetic digital tone delay improved substantially, but the
    speech-window diagnostic failed its correlation-coverage gate (52/109
    voiced windows, below 70%). Adaptive time stretching makes a single global
    fixed-lag speech estimate unreliable. Full DFN3 also suppresses the tone,
    making its 154.667 ms tone result unsuitable as speech qualification. A
    robust speech timing harness and controlled acoustic/device measurement
    remain required. See `bench/voice-2026-11-12/REPORT.md` and raw evidence.

## Reproduce safely

Use ONE task target outside OneDrive; delete it after final validation:

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-client-calls'
cargo check -p sticktalk-media
cargo test -p sticktalk-client-core -p sticktalk-media -p sticktalk
cargo clippy -p sticktalk-client-core -p sticktalk-media -p sticktalk --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test -p sticktalk --test voice -- --nocapture
cargo test -p sticktalk --test voice -- --ignored --nocapture
cargo run -p sticktalk-media --release --example voice_quality_bench -- --seconds 3 --ten-ms
cargo run -p sticktalk-media --release --example dfn_full_probe -- <full-model-tar.gz>
cargo run -p sticktalk -- --mock --screenshots <output-dir>
```

Tests instantiate two native endpoint objects, not two GUI processes. The
file-audio test checks actual server voice projection/speaking and received
Opus correlation, then exchanges a real SCTP control grant/key/revoke using
only the dry-run injector and verifies held-key release. The explicit ignored
hardware test generates a 1080p60
NV12 fixture, hardware-encodes it into a temporary FILE, then transports that
file through the real local server and hardware-decodes for 60 seconds.
