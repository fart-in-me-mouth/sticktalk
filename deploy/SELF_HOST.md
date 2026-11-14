# Host StickTalk yourself

This is the recommended way to run StickTalk: on a computer you own. That can be
your Windows PC, a Linux box or a Raspberry Pi, published on the internet with
**Tailscale Funnel**. You don't need a cloud VM, a domain, router port forwarding
or a certificate: Tailscale gives the machine a free `https://<machine>.<tailnet>.ts.net`
address with a real TLS certificate. One script builds, installs, keeps the server
running and prints your addresses, including the [MCP connector](../docs/MCP.md) URL for AI
assistants.

What you get, and the trade-offs:

- **Text chat, uploads, the web/phone app and the MCP connector** work for anyone on the
  internet who has the address.
- **Voice and video calls** need UDP, which Funnel cannot carry. With the default
  `-Voice Tailnet`, calls work between devices on your Tailscale network: invite
  friends to your tailnet, or [share the machine](https://tailscale.com/kb/1084/sharing)
  with them. To let anyone call, forward UDP to your PC instead (see
  [Calls for everyone](#calls-for-everyone-optional)).
- **The server runs while your computer is on.** On Windows it starts when you sign in.
  On Linux it is a systemd service and starts at boot.
- **Registration is open**: anyone with the address can create an account. Share it only
  with friends.

## Windows (this is the main path)

### 1. One-time prerequisites

Use a normal, non-administrator PowerShell.

1. **Tailscale**: install from <https://tailscale.com/download/windows> and sign in.
2. **Funnel for your tailnet**, once, in the [Tailscale admin console](https://login.tailscale.com/admin):
   - DNS → enable **MagicDNS** and **HTTPS Certificates**.
   - Access controls → allow Funnel for your machine. The policy needs a
     `nodeAttrs` entry with `"attr": ["funnel"]`; the console offers to add it.
     See [Tailscale Funnel](https://tailscale.com/kb/1223/funnel).
3. **Build tools**, from [HANDOFF.md](../HANDOFF.md#3-prepare-windows-1011-x64):
   - Git;
   - Rust (rustup, MSVC);
   - Visual Studio 2022 Build Tools with *Desktop development with C++* (Microsoft
     installs it machine-wide; an administrator may need to approve it once);
   - Node 22+ (only for the web app; skip it with `-SkipWeb`).

### 2. Install and start

In the repository folder:

```powershell
powershell -ExecutionPolicy Bypass -File deploy/self-host/setup-windows.ps1
```

The first run builds the server and the web app (tens of minutes); later runs are quick.
The script then:

1. installs StickTalk to `%LOCALAPPDATA%\StickTalk\Server`: `bin`, the `web` app, `data`
   for your database and uploads, and `logs`;
2. registers a hidden **startup task**, *StickTalk Server*, that starts it whenever you
   sign in to Windows and restarts it if it crashes. No administrator rights and no
   console window;
3. runs `tailscale funnel` on the first free Tailscale HTTPS port, sets
   `STICKTALK_PUBLIC_URL` to match, and checks that the public address answers;
4. prints your addresses:

```text
StickTalk is self-hosted.
  Web app / desktop client server address: https://your-pc.your-tailnet.ts.net
  MCP connector URL for AI assistants:     https://your-pc.your-tailnet.ts.net/mcp
```

Funnel only offers ports **443, 8443 and 10000**. The script never takes a port another
service already uses. If 443 is taken (for example by T3 Code's own `tailscale serve`),
your address gets a port, such as `https://desktop-b09gtso.humpback-salak.ts.net:10000`.
That address works the same way in browsers, the desktop client and AI apps.

### 3. Use it

- Open the web address in a browser, select **Register**, and create your account.
  Phones can install the web app from that page.
- In the desktop client, enter the same address as the server.
- Send friends the address.
- AI assistants: paste the `/mcp` URL into Claude (Settings › Connectors › *Add custom
  connector*), Claude Code (`claude mcp add --transport http sticktalk <URL>/mcp`),
  Cursor or ChatGPT. Sign in and select **Allow**. See [docs/MCP.md](../docs/MCP.md).

### Manage it

| To | Run |
|---|---|
| Update after `git pull` | `powershell -ExecutionPolicy Bypass -File deploy/self-host/setup-windows.ps1` (same options as before) |
| Stop / start | `Stop-ScheduledTask 'StickTalk Server'` / `Start-ScheduledTask 'StickTalk Server'` |
| See the log | `Get-Content "$env:LOCALAPPDATA\StickTalk\Server\logs\server.log" -Tail 50 -Wait` |
| Tailnet only (not public) | setup with `-Exposure Tailnet` |
| Remove (keeps data) | `powershell -ExecutionPolicy Bypass -File deploy/self-host/uninstall-windows.ps1` |
| Remove everything | add `-DeleteData` to the uninstall command |

Re-running setup is safe. It stops the old server, installs the new build, keeps
your data, and reuses the same Tailscale port, so your address doesn't change.

Other options:

- `-SkipWeb`: desktop clients only, no Node needed.
- `-SkipBuild`: reconfigure without rebuilding.
- `-Port`: the local port, 8080 by default.
- `-HttpsPort 443|8443|10000`: choose the public port yourself.
- `-NoStartupTask`: start once, without a startup task.

Optional server settings (Google sign-in, GIFs, upload limits, push) go in
`%LOCALAPPDATA%\StickTalk\Server\server.env`, one `KEY=VALUE` per line. It uses the same
names as `/etc/sticktalk.env` in [ORACLE.md](ORACLE.md), and setup never overwrites the
file. Secrets stay in your user profile; never commit them.

## Linux or Raspberry Pi

Use 64-bit Debian, Ubuntu or Raspberry Pi OS (Bookworm or newer) with systemd.
Install Tailscale (`curl -fsSL https://tailscale.com/install.sh | sh`, then
`sudo tailscale up`) and enable Funnel as in step 2 above. Then, as your normal user:

```bash
# On a capable Linux machine: builds here (needs Rust + build-essential, clang, cmake, python3).
bash deploy/self-host/setup-linux.sh

# On a Raspberry Pi: cross-build on Windows (deploy/pi/build.ps1) and copy the binary
# and, optionally, a built web app, then:
bash deploy/self-host/setup-linux.sh --binary ./sticktalk-server --web ./web-dist
```

The script:

- installs `/usr/local/bin/sticktalk-server` and the hardened `sticktalk` systemd
  service, with data in `/var/lib/sticktalk`;
- writes the managed keys into `/etc/sticktalk.env`, keeping any other settings you added;
- configures `tailscale funnel`;
- prints the same addresses as the Windows script.

`--exposure`, `--https-port`, `--voice` and `--public-url` match the Windows options.
`--uninstall` stops the service and removes the Tailscale route, and keeps the data.
Backups: see [ORACLE.md § Backups](ORACLE.md#6-backups-sqlite-backup-plus-files);
`sudo bash deploy/backup.sh` works on any systemd host.

## Calls for everyone (optional)

Funnel carries only HTTPS. For calls with people who aren't on your tailnet, the SFU
needs a public UDP address:

1. Forward **UDP 50000–50100** on your router to this computer.
2. Find your public IPv4 address. A dynamic home IP changes; a DuckDNS name helps you
   track it, but StickTalk needs the IP itself.
3. Re-run setup with `-Voice Public -VoiceAdvertisedIp <your public IPv4>`, or
   `--voice public --voice-advertised-ip …` on Linux. On Windows, allow
   `sticktalk-server.exe` through Windows Defender Firewall for UDP when asked. That
   prompt needs an administrator.

No TURN relay is bundled, so very restrictive networks may still fail to connect.

## Alternatives to Funnel

- **Tailnet only** (`-Exposure Tailnet`): the same `ts.net` address, but only your
  Tailscale devices can reach it. This is the most private option, and calls work with
  the default `-Voice Tailnet`.
- **Your own domain:**
  - Run Caddy with a DuckDNS (or any) domain, forwarding TCP 80/443 on your router; or
  - run a Cloudflare Tunnel (`cloudflared`) to `http://127.0.0.1:8080`.

  Either way, pass the resulting origin with `-PublicUrl https://chat.example.org`, or
  `--public-url` on Linux. The `deploy/Caddyfile` template works on any host.
- **A cloud VM:** [ORACLE.md](ORACLE.md) covers an Always Free Oracle VM.
  [HYBRID.md](HYBRID.md) adds a private Pi for storage and backups. Both are optional.

## How it fits together, and security notes

- The server listens only on `127.0.0.1`. Tailscale connects to it locally, so no Windows
  Firewall prompt appears for chat. The voice UDP socket is the only exception: it binds
  to your Tailscale or public address.
- Tailscale terminates HTTPS at your machine. Tailscale relays Funnel traffic but can't
  read it.
- `STICKTALK_PUBLIC_URL` must be exactly the address people use, port included. The
  scripts set it. It is the OAuth issuer and MCP resource for AI connectors and the
  origin for browser sign-in. If the address changes, re-run setup and reconnect AI
  apps.
- `*.ts.net` names appear in public certificate logs, so treat the address as
  discoverable, not secret. Use strong passwords. Consider `-Exposure Tailnet` if only
  your own devices need access.
- **Back up** `data` (the SQLite database and uploads) with the server stopped, or use
  SQLite `.backup`. It contains private messages and password hashes, so keep backups
  encrypted.
