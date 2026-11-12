# Phase 2 (voice / video / screen share) — hard requirements from the owner

## Screen share: minimum 1080p @ 60 fps, and still light on the sharer's PC
- **Capture**: Windows Graphics Capture API (window or monitor), frames stay on the GPU (D3D11 textures) — no CPU copies / readback.
- **Encode**: hardware encoder only by default — NVENC (NVIDIA), AMF (AMD), Quick Sync (Intel), reached via Media Foundation (or vendor SDKs if MF can't hit 60 fps). Zero-copy texture → encoder. CPU software encoding (openh264) is a fallback only, and drops to 720p30 automatically with a notice.
- **Codec**: H.264 High as the baseline (every GPU can decode it in hardware); negotiate AV1 or HEVC when all viewers support hardware decode. Viewers use hardware decode (D3D11VA / Media Foundation) and render the decoded texture directly.
- **User-selectable quality (independent pickers, all free, no paywall)** shown in the share popup like a settings menu with radio rows:
  - Frame rate: **15 / 30 / 60 FPS**
  - Resolution: **480p / 720p / 1080p / 1440p / Source**
  - Every combination is allowed. **Default: 1080p + 60 FPS.** Remember the last choice. The user can change it mid-stream without restarting the share.
- **Bitrate**: scales with the choice (about 1.5 Mbps at 480p30 up to 6–10 Mbps at 1080p60 and 12–16 Mbps at 1440p60; **Source** = the captured window/monitor's native resolution with no scaling, e.g. 4K at about 20–30 Mbps at 60 FPS, and the UI shows the actual resolution next to the option, e.g. "Source (2560×1440)"), adaptive via congestion control (REMB/TWCC). Content hint: "motion" (games) vs "detail" (text/code).
- **Audio**: optional system/app audio capture (WASAPI loopback), Opus 48 kHz stereo.
- **Server**: str0m SFU forwards packets without transcoding — server CPU stays tiny. Oracle Always Free gives 10 TB/month egress, so roughly 4 viewers × 8 Mbps is fine.
- **Budget on the sharer's PC at 1080p60 with a hardware encoder**: < 5% CPU, < 150 MB extra RAM. Measure it and document the numbers.
- **Acceptance test**: share a 1080p60 game/video for 10 minutes to 2 viewers. Show an fps/bitrate/dropped-frames overlay (toggleable). It must hold ≥ 58 fps average with no growing latency.

## Voice quality: must beat Discord (owner: "best on the market")
- **Noise suppression:** use **DeepFilterNet3** (Rust: `deep_filter` / libDF, MIT/Apache), which runs real-time on the CPU. Modes: Off / Standard (RNNoise-class, lightest) / **High quality (DeepFilterNet, default on capable CPUs)**. Auto-select based on a quick CPU benchmark at first run.
- **Echo cancellation and AGC:** use **WebRTC AudioProcessing (AEC3 + AGC2 + high-pass)** through `webrtc-audio-processing` bindings, the industry standard. Prefer it over home-grown DSP.
- **Codec:**
  - Opus 48 kHz with in-band FEC, DTX, and an adaptive bitrate of 32–128 kbps.
  - The default is 64 kbps mono voice.
  - A **"Studio / Music" mode**: stereo, 128–256 kbps, all processing off, for music and streaming instruments.
  - Packet-loss concealment.
  - An **adaptive jitter buffer** (NetEQ-style, with time-stretching) to minimize latency on good links and stay smooth on bad ones.
- **Device handling (must be flawless):**
  - Enumerate input and output devices with friendly names.
  - **Hot-plug and default-device change events (`IMMNotificationClient`)**, so the app follows the Windows default automatically unless the user pins a device.
  - Survive device unplug mid-call without dropping the call: fall back, and toast "Switched to <device>".
  - Bluetooth headsets: avoid accidentally forcing hands-free (HFP) mode. Use the A2DP output when the mic comes from another device, and explain the tradeoff in settings.
  - Exclusive-mode is off. Communications ducking is off by default.
- **Settings UX:**
  - Mic test with a live level meter and loopback playback ("Let's Check"). This is the only time a meter animates, and only while it's open.
  - Input sensitivity: automatic, or a manual slider over the live meter.
  - Per-user volume (0–200%) and per-user mute that persist.
  - Push-to-talk with a release delay. Global hotkeys work in fullscreen games.
  - Attenuate other apps while others speak: an option, off by default.
- **Measure and report:**
  - CPU% per mode on this PC.
  - Mouth-to-ear latency on loopback, with a target of 80 ms or less on a good network.
  - Voice quality: PESQ/POLQA-style objective scores, or DNSMOS for noise suppression if available.

## Voice / video (also Phase 2)
- Opus voice with noise suppression (RNNoise-class, `nnnoiseless`), echo cancellation, voice activity detection (VAD) + push-to-talk, per-user volume, mute/deafen.
- Camera video 720p30 by default, using the hardware encode path.
- No audio or video work at all while the user is not in a call.

## Remote control during screen share (owner requirement)
Viewers can control the sharer's computer during a stream, like TeamViewer. **Only with the sharer's explicit consent every time.**
- **Off by default.** It's only possible while sharing a full monitor or a single window. Input is limited to that monitor or window's bounds.
- **Request flow:** a viewer clicks "Request control" → the sharer gets a prompt: "<name> wants to control your screen. [Allow] [Deny]". The prompt has no remember option and times out after 30 s, which counts as Deny. Only one controller at a time.
- **While control is active:**
  - The sharer sees a persistent, always-on-top banner: "<name> is controlling your screen — Stop (Ctrl+Alt+Shift+S)".
  - That global hotkey revokes control instantly. Any movement of the sharer's own physical mouse also takes control back. Stopping the stream ends control.
  - Everyone else in the call sees "<name> is controlling".
- **Transport:** input events (mouse move, button, wheel, key down/up) go viewer → SFU → sharer over a WebRTC **data channel**, never the chat WebSocket. Mouse coordinates are normalized 0..1 to the shared area. The channel is rate-limited and capped at 1000 events/s.
- **Injection:** use `SendInput` on Windows. It can't touch the UAC/secure desktop, and that's acceptable. Release all held keys and buttons on revoke or disconnect so nothing gets "stuck".
- **Security:**
  - The server enforces that only the user the sharer approved can send input for that session.
  - The approval is a session-bound grant, invalidated on revoke.
  - Clients ignore input events without a valid grant.
  - Log grant/revoke events, not the keystrokes themselves.
- **Viewer side:** in control mode, the viewer's stream view captures mouse and keyboard while focused. Esc held for 1 s releases capture locally. Latency target: under 50 ms added on top of the video path.
