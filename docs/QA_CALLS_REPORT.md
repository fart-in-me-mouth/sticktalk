# Calls QA — 2026-11-13

Branch: `t3/calls-qa-polish`. **Launch qualification is incomplete; do not read a unit-test pass as physical-device or full desktop-UI acceptance.**

## Changes

- SFU tracks original-media backlog separately from padding/RTX, admits wire output before dequeue, bounds queues using bitrate/RTT, shares actual wire budget fairly per viewer, and reserves encoder headroom. The global 32 Mbps wire cap is unchanged. Whole-RTC deferral is limited to 10 ms to protect audio/control.
- Viewer control requests now have a distinct, scoped pending state and expiry. Real shared-window mouse clicks can activate the target; keyboard/wheel retain foreground checks. Release-on-revoke remains fail closed.
- Call controls, picker tabs, quality popover, participant grids/rings/badges, own-stream controls, empty/solo states and camera tiles polished. Deterministic headless screenshots cover 1–9 participants and minimum window size.

## Feature → status

| Feature | Status / evidence |
|---|---|
| Two native peers, real local server, Opus/SRTP/SCTP | PASS: 75/75 file-Opus packets, correlation 1.000; speaking and guild voice state verified. These are native Peer endpoints, not two manually operated iced windows. |
| Join/leave; mute/deafen | PASS at native protocol/unit seams and two independent headless Chromium processes; browser media teardown checked. |
| Switching channels while connected | Server routing/permission tests pass; complete real-desktop UX not qualified. |
| Global PTT; activity threshold | Hotkey arbitration/settings/reducer tests and headless settings reviewed; physical global-hotkey use in a second real iced client not qualified. |
| Speaking, local volume/mute, moderator mute/deafen/move/disconnect | Native/server tests cover permissions/state; full audible desktop interaction not qualified. |
| Noise suppression / AEC / AGC | Settings and DSP tests pass; optimized synthetic mode timing/CPU measured below. No acoustic echo or objective speech-quality qualification. |
| Device switching/hotplug/default follow | Simulated recovery/default/pinned/fallback tests and real IMM watcher registration pass. Physical unplug/replug mid-call not performed. |
| Network drop/reconnect | Real chat/socket replay and voice transport fail-closed tests pass; long real-audio network-drop/rejoin not qualified. |
| Cameras | Two headless Chromium processes decode H.264 fake-camera frames. Synthetic native HW encode/decode passes. **No physical cameras enumerate on this PC**, so native camera hardware acceptance is blocked. |
| 1–9 tiles, focus/fullscreen/pop-out | Headless visual fixtures and ownership/visibility teardown tests pass; real OS fullscreen/pop-out interaction not qualified. |
| Multiple simultaneous streams | PASS: two native screen publications delivered in both directions through real SFU. |
| Preview thumbnails / application audio | Bounded preview lifecycle tests pass; process-loopback activation/stop passes, but captured zero packets from a silent owned process. Audible application-stream E2E not qualified. |
| Stream stats | Headless overlay reviewed; measurement harness records decoded frames, timing, CPU, working set and private bytes. |
| Viewer count/list | **NOT DONE:** no authoritative per-stream viewer projection in current protocol; roster members were not misrepresented as viewers. |
| Request → consent on each grant → deny/allow → input → revoke → end-share | PASS through real local server/native peers and actual SendInput into an owned secondary-monitor window. Fresh request/consent/grant IDs and auto-revoke checked. Single control→OS key sample 13.941 ms. |
| Emergency hotkey/banner revoke | Dry-run hotkey precedence, immediate hook gate, held-input release and banner UI pass. Full physical hotkey/banner interaction across two real iced clients remains unqualified. |
| Join/leave sounds | Dry mocked effects only; no sound played on owner’s PC. |
| Two/three real desktop GUI clients plus web together | **NOT COMPLETED:** native transport/process acceptance and separate web acceptance are not equivalent to this requested mixed-GUI scenario. |

## Video acceptance

<!-- ACCEPTANCE_TABLE -->

Each case has two hardware-decoding viewers, 8 s warmup, and then the stated measured interval. FPS is decoder output, not renderer presentation. Drift is change in receive-vs-RTP timing offset, **not glass-to-glass latency**. Minimum FPS is the minimum complete one-second bin. Source fixture is 2560×1440, not 4K. WGC captures an owned moving D3D11 window on the secondary monitor, never personal desktop pixels. Moving-WGC runs reject static-repeat substitutes. Preset gates scale the 60 fps thresholds proportionally; 60 fps targets are average ≥58, minimum ≥45, absolute drift <20 ms, zero encoder reconfigurations.

Raw case metrics, binary hash/commit provenance, scripts and archived logs: `crates/client/bench/qa-calls-2026-11-13/`. Failed short pre-fix diagnostics are retained rather than hidden.

## Voice / resource measurements

| Measurement | Result | Limit |
|---|---|---|
| Digital clean marker p50/p95, 10 ms packets | Off 25.417/25.438 ms; Standard 35.417/35.417; High 35.417/35.417 | 16/16 markers each; excludes devices/network/acoustics |
| Digital clean marker p50/p95, 20 ms packets | Off 35.417/35.438; Standard 45.417/45.417; High 45.417/45.438 ms | Not mouth-to-ear |
| Deterministic 5% loss + 0/20/30 ms jitter | High/20 ms matches 15/16, p50 65.333, p95 85.417 ms | **Coverage gate FAIL**; diagnostic only |
| Synthetic talk CPU, Off / Standard / High | 0.273 / 0.410 / 1.045% whole machine | Own process CPU; background builds present, not exclusive-PC baseline |
| Synthetic silence CPU, Off / Standard / High | 0.059 / 0.049 / 0.059% whole machine | Same limitation |
| WASAPI muted idle, no remotes, 10.4 s | 0.0094% machine, 0.1503% one core; working set 56.06 MiB | Real endpoint, silent output; not entire idle app |
| DSP working set / private | Off 50.57/6.96, Standard 50.8/6.12, High 90.99/46.39 MiB | Synthetic bench processes |
| All processors dropped, 5 s | 0% measured CPU; working set/private 48.38/4.88 MiB | Process-level measurement |
| Synthetic NV12 720p30 → HW codec, 10 s | 300 frames, 30.00 fps; CPU 0.0879% machine, WS 77.53 MiB | Not camera capture |
| Native GPU import | D3D11→NT handle/fence→DX12/iced pixel check passes; zero production CPU pixel copies | Integration fixture, not full UI presentation |

No acoustic mouth-to-ear measurement, PESQ/DNSMOS, competitor comparison or audible live conversational acceptance was obtained. “Better than Discord” audio quality is therefore **not established**.

## Web evidence

Two independent headless Chromium processes, fake media devices and a real loopback native server exchanged audio and decoded 17 remote H.264 frames each (audio packets 205/206). Mute/deafen/leave released publications, subscriptions and tracks. This used the other web agent’s prebuilt bundle revision `9afc59fa41f8d85ff553`, not a claim that this branch’s unbuilt web sources are qualified. No edits to `crates/web`.

## Validation / remaining blockers

<!-- VALIDATION -->

- Full four-crate test command reaches an unrelated existing server upload reservation assertion (`audit_tests::http_admission_and_global_upload_reservation_are_bounded`, line 402, left 1/right 0). Reproduced in two suite runs; isolated rerun passes. Unrelated upload code was not changed to hide this race.
- Full desktop GUI E2E, physical camera/hotplug, real emergency hotkey exercise, audible application audio, true mouth-to-ear, objective voice quality and viewer count/list remain launch blockers or explicit qualification gaps.
- All test listeners bind loopback. Owned test windows/processes closed; no browser/OAuth/private data sent outside this PC, no push, no UAC.
