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
| `crates/desktop` | Windows desktop app since 2026-11: a thin native window (wry/tao, WebView2) that loads the web app from the server. Installed as `sticktalk.exe`. |
| `crates/client` | **Retired** native desktop app (iced + wgpu). Still in the tree and buildable, but no longer packaged; the web app replaced it. |
| `crates/media` | Used only by the retired native client. Windows media: WGC capture, Media Foundation hardware H.264 encode/decode, AEC3/AGC2, DeepFilterNet3 noise suppression, Opus, NetEQ. Vendored deps live in `vendor/`. |
| `crates/desktop-platform` | Tray, autostart, single instance, window placement, updater/maintenance binaries, packaging. |
| `crates/emoji` | Emoji tables and atlas. |
| `crates/web` | The one user interface: Leptos (Rust → WebAssembly) PWA, styled with Tailwind CSS and shadcn/ui components, behaviors from UI SDK. Served by the server; shown by browsers, phones and the desktop window. |

## Hosting plan (decided 2026-11-14: self-hosted; not yet running for real users)
- **Self-hosted on the owner's own machine** (Windows PC first; Linux/Pi also supported), published with **Tailscale Funnel** at `https://<machine>.<tailnet>.ts.net` (port 443, 8443 or 10000, whichever is free). One command: `deploy/self-host/setup-windows.ps1` (per-user startup task, no admin) or `deploy/self-host/setup-linux.sh` (systemd). Guide: `deploy/SELF_HOST.md`.
- Calls need UDP, which Funnel can't carry: they work between tailnet devices by default, or for everyone with router UDP forwarding (`-Voice Public`).
- Optional alternatives, no longer the plan: Oracle Always Free VM with Caddy + DuckDNS (`deploy/ORACLE.md`), plus a private Pi for storage/backups/standby (`deploy/HYBRID.md`).
- The server is built with `--profile server` (`panic = "unwind"`, so the SFU contains per-peer panics).
- Step-by-step guides: `deploy/SELF_HOST.md`, `HANDOFF.md`, `deploy/README.md` (index), `deploy/RELEASING.md`, `deploy/windows/`.
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
- **AI connector (MCP):** paste `<server>/mcp` into Claude/Cursor/ChatGPT, sign in and approve (read only, or read and send). OAuth 2.1 with dynamic registration and PKCE; 15 tools; connections listed and revocable under Devices. See `docs/MCP.md`.
- **UI:** a full audit against Discord (190 states). Latest owner fixes: one selected channel row with Discord-style fills (no outline boxes), hover subtler than selected, 22px inline / 48px jumbo emoji, borderless composer.

## Open items / next steps
1. **Deploy:** run `deploy/self-host/setup-windows.ps1` on the owner's PC for real, register the owner account, then share the address
2. **Create the real secrets** on the deploying machine. Never commit them. Their locations are in `HANDOFF.md`:
   - KLIPY API key
   - Google OAuth client
   - update-signing key
   - backup encryption key
3. **Build and publish** the installer with the real server URL and signing key.
4. **Phone testing:** cross-engine QA (Chromium, WebKit/iOS engine, Firefox) is merged; the suite passes 3× on all three (`docs/QA_WEB_ENGINES.md`). Real-device testing on iPhone and Android is still needed, especially calls and streams on iPhone Safari, because Windows WebKit has no WebRTC or H.264.
5. **Idle RAM:** desktop idle RAM is about 140 MiB working set, dominated by the GPU driver and DX12. The target was under 100 MiB. The non-fork options are unfinished and NOT in this repo: wgpu backend choice, lazy system-font database, exe size/LTO, working-set trim on minimize. Forking or patching iced/wgpu was declined (maintenance burden).
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
- `docs/MCP.md` (AI assistant connector)
- QA reports: `docs/QA_TEXT_REPORT.md`, `docs/QA_WEB_REPORT.md`, `docs/QA_CALLS_REPORT.md`, `docs/UI_AUDIT_REPORT.md`, `docs/FEATURES3.md`, `docs/HANDOFF_VALIDATION.md`
- `crates/client/SCROLL_ENGINE.md`
