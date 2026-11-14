# Running a StickTalk server

**Start here: [SELF_HOST.md](SELF_HOST.md).** Host StickTalk on your own Windows PC,
Linux box or Raspberry Pi. Tailscale Funnel gives it a public HTTPS address, and one
setup script installs it, keeps it running and prints the web, desktop and
[MCP connector](../docs/MCP.md) addresses.

| Guide | Use it for |
|---|---|
| [SELF_HOST.md](SELF_HOST.md) | **Recommended.** Windows (`self-host/setup-windows.ps1`) and Linux/Pi (`self-host/setup-linux.sh`) with Tailscale Funnel; calls; alternatives |
| [ORACLE.md](ORACLE.md) | Optional cloud VM with Caddy + DuckDNS. Also the reference for server settings: [configuration](ORACLE.md#configuration), [Google sign-in](ORACLE.md#optional-turn-on-continue-with-google-about-five-minutes), [GIFs](ORACLE.md#optional-gifs-provider-decision-and-setup-checked-2026-11-12), [backups](ORACLE.md#6-backups-sqlite-backup-plus-files), [voice](ORACLE.md#phase-2-voice-embedded-sfu), [upload limits](ORACLE.md#upload-abuse-controls-and-privacy) |
| [BROWSER.md](BROWSER.md) | Web/PWA hosting details, cookies, Google web credentials, push |
| [HYBRID.md](HYBRID.md), [pi/README.md](pi/README.md) | Optional: a private Raspberry Pi for bulk storage, encrypted backups and cold standby behind a cloud primary |
| [RELEASING.md](RELEASING.md) | Building and publishing the Windows desktop installer |
