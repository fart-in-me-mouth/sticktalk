# StickTalk: project status and context

Last updated: 2026-11-14. Read this first, then `HANDOFF.md` (build and deploy guide) and `AGENTS.md` (rules for AI coding agents).

## What it is
StickTalk is a Discord-style chat app for a group of friends, written in Rust. It has servers, channels, roles, DMs, voice and video calls, and screen share. The goal is to be "10x better than Discord":
- smoother scrolling
- best-in-class voice quality
- 1080p60+ screen share
- low CPU and RAM usage

Priorities, in order:
1. quality and smoothness
2. efficiency
3. polished UI matching or beating Discord
4. security

It must stay **100% free** to host.

## Architecture
| Crate | What |
|---|---|
| `crates/proto` | Wire types shared by server and clients. Changes must be additive and wire-compatible. |
| `crates/server` | axum + SQLite (sqlx, WAL). REST + WebSocket, auth, permissions, uploads, FTS5 search, blob storage (local or remote storage node), encrypted backups. |
| `crates/sfu` | WebRTC SFU on str0m: voice, camera and screen-share forwarding, pacing, BWE, keyframe handling. |
| `crates/client-core` | Platform-neutral client logic; compiles to native and wasm. |
| `crates/client` | Windows desktop app. iced + wgpu, custom smooth-scroll engine (`src/scroll/`), modular `ui/`, headless screenshot mode (`--mock --screenshots <dir>`). |
| `crates/media` | Windows media: WGC capture, Media Foundation hardware H.264 encode/decode, AEC3/AGC2, DeepFilterNet3 noise suppression, Opus, NetEQ. Vendored deps live in `vendor/`. |
| `crates/desktop-platform` | Tray, autostart, single instance, window placement, updater/maintenance binaries, packaging. |
| `crates/emoji` | Emoji tables and atlas. |
| `crates/web` | Leptos CSR PWA for phones. |

## Hosting plan (decided; not deployed yet)
- **Oracle Cloud Always Free A1 VM:** main public server (API, SFU, DB, web PWA), behind Caddy with a DuckDNS domain.
- **Friend's Raspberry Pi 4** (5 TB HDD, LUKS-encrypted, reached over Tailscale):
  - bulk file storage node (`sticktalk-server storage --listen <tailscale-ip>`, HMAC-signed requests)
  - nightly age-encrypted backups with 14-day retention
  - cold standby
- The server is built with `--profile server` (`panic = "unwind"`, so the SFU contains per-peer panics).
- Step-by-step guides: `HANDOFF.md`, `deploy/README.md`, `deploy/pi/README.md`, `deploy/RELEASING.md`, `deploy/windows/`.

## Where it is now: feature-complete apart from deployment
Done, merged into `master`, and tested (608 workspace tests, 0 failures; clippy `-D warnings` clean; fmt clean):
- **Accounts:** register/login, Google sign-in (nonce + reauth), sessions, password change revokes other sessions, account deletion (fast; other devices land on login).
- **Servers and permissions:** servers, channels, categories, roles, permission overrides, invites, bans, audit log.
- **Social:** friends, DMs, group DMs.
- **Messages:** send/edit/delete/reply, mentions, reactions, pins, markdown, typing indicators, unreads, jump to message.
- **Media in chat:** unlimited file size (quotas: 3 concurrent uploads, 100 GB/day/user), resumable uploads, streamed video over 128 MiB, click-to-zoom image viewer.
- **Pickers and customization:** GIF picker (KLIPY), emoji picker, custom server emoji, full-history server search (FTS5), slowmode, server folders.
- **Voice:** best-in-class DSP, about 35 ms digital latency, PTT/VAD, device hotplug.
- **Video:** cameras; screen share in 15/30/60 fps × 480p/720p/1080p/1440p/Source. Measured: **60 fps to 2 viewers at 1080p, 1440p and Source, drift under 5 ms, 0 encoder reconfigurations.**
- **Remote control:** requires explicit consent every time, can be revoked instantly; input latency about 14 ms.
- **Desktop distribution:** per-user NSIS installer (no admin, about 51 MB), signed auto-update with rollback, crash recovery, tray, window placement memory.
- **Phone PWA:** Discord-mobile-style navigation, long-press sheet, settings/admin parity, voice + stream viewing, offline shell.
- **Security:** review in `docs/SECURITY_REVIEW.md`; fixes S1–S10 applied.
- **UI:** a full audit against Discord (190 states). Latest owner fixes: one selected channel row with Discord-style fills (no outline boxes), hover subtler than selected, 22px inline / 48px jumbo emoji, borderless composer.

## Open items / next steps
1. **Deploy:**
   - Oracle VM + DuckDNS + Caddy
   - Pi: storage node, LUKS, backups, Tailscale; then rehearse a failover
2. **Create the real secrets** on the deploying machine. Never commit them. Their locations are in `HANDOFF.md`:
   - KLIPY API key
   - Google OAuth client
   - update-signing key
   - backup encryption key
3. **Build and publish** the installer with the real server URL and signing key.
4. **Phone testing:** real-device testing on iPhone and Android hasn't been done; only emulation so far. An agent was running cross-engine tests (WebKit/Firefox) on branch work that may not be merged yet. Check for branches named like `t3/*webkit*` or `t3/*engine*`.
5. **Idle RAM:** desktop idle RAM is about 140 MiB working set, dominated by the GPU driver and DX12. The target was under 100 MiB. An agent was exploring non-fork options (wgpu backend choice, font database, exe size, trim on minimize); check for a `t3/*ram*` branch. Forking or patching iced/wgpu was declined (maintenance burden).
6. **Claude UI pass:** once everything works end to end, the owner wants Claude to personally rework UI details. Current UI taste notes:
   - Discord-like neutral gray with a mint accent
   - filled rows, no outline boxes
   - "server" wording
7. **Later features:** threads, events, stickers, soundboard, game activity, AutoMod, link embeds, bots.

## Working conventions
- **AI agents:** read `AGENTS.md`. One cargo target dir per task with a 10 GB disk budget and `CARGO_INCREMENTAL=0`. Bind tests to loopback only. No UAC prompts. Never pop windows on the owner's primary monitor (use headless screenshots or `STICKTALK_AGENT_WINDOW=secondary`).
- **Workspace checks:** `cargo test --workspace --exclude webrtc-audio-processing` (the vendored APM tests don't build on MSVC). Also `cargo clippy --workspace --exclude webrtc-audio-processing --all-targets -- -D warnings` and `cargo fmt --all -- --check`.
- **Builds:** server `cargo build -p sticktalk-server --profile server`; desktop `cargo build --release -p sticktalk`; web: see `crates/web/README.md`.
- **Moving the code:** the repo is portable (no machine-specific paths). `deploy/make-handoff-bundle.ps1` makes a git bundle for transfer.
- **Benchmark evidence** (`crates/*/bench*`, `evidence/`): historical measurements from the original dev PC.

## Key docs
- `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`
- `docs/UI_SPEC.md`, `docs/UI_PARITY.md`
- `docs/PHASE2_REQUIREMENTS.md`, `docs/GOOGLE_LOGIN.md`
- QA reports: `docs/QA_TEXT_REPORT.md`, `docs/QA_WEB_REPORT.md`, `docs/QA_CALLS_REPORT.md`, `docs/UI_AUDIT_REPORT.md`, `docs/FEATURES3.md`, `docs/HANDOFF_VALIDATION.md`
- `crates/client/SCROLL_ENGINE.md`
