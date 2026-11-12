# StickTalk on your Raspberry Pi

This is the primary hosting path. The old `deploy/` Oracle instructions are the
fallback. You need a **Pi 4 with at least 2 GB RAM**, its proper power supply,
a wired network connection if possible, an SD card for the operating system, and
a **USB SSD** for chat data. Leave it powered on. No router port forwarding.

## 1. Put the operating system on the SD card

Use Raspberry Pi Imager on your own computer. Choose **Raspberry Pi OS Lite
(64-bit), Bookworm or newer**, not the 32-bit version. In Imager's settings set a
username, password, hostname (`sticktalk` is fine), Wi-Fi if needed, and enable
SSH with your SSH public key. Write the **SD card**, not the SSD. Boot the Pi and
plug in the SSD. Commands below go in an SSH terminal connected to the Pi.
`uname -m` must print `aarch64`.

## 2. Connect Tailscale

Follow [Tailscale's Raspberry Pi instructions](https://tailscale.com/download/linux/rpi).
Sign the Pi into **your own** Tailscale account. Enable MagicDNS and HTTPS
certificates in that account's DNS settings. Note the Pi's full DNS name, such as
`sticktalk.friendly-tailnet.ts.net` (your name will be different).

Run `sudo tailscale up --ssh` after installation. Use the Tailscale admin console
to share this **one device** with each friend. Each friend installs Tailscale,
uses their own account and accepts the share. They do not need your account
password. Keep SSH access restricted to the owner/deployer in your tailnet policy;
friends need TCP 443 for chat and UDP 50000–50100 for media. Node sharing does not grant administrative SSH
access. Test the exact policy with a friend's separate account.

Do **not** enable Funnel or add router port forwarding. Caddy below binds only to
the Pi's `100.x` Tailscale address; the chat process binds only to localhost.

## 3. Put the data on the SSD, not the SD card

Have the person helping you prepare an **ext4-formatted USB SSD**. Formatting a
disk erases it: this kit deliberately never runs a format command. If it contains
anything important, stop and back it up. Don't guess a `/dev/sd...` device name.

On the Pi, run `lsblk -f`. Find the SSD partition by its **size, label and USB
connection**. Copy its UUID (the long value in the UUID column).

```sh
sudo mkdir -p /srv/sticktalk
sudo nano /etc/fstab
```

Add this line, replacing `YOUR-SSD-UUID` with that actual UUID:

```text
UUID=YOUR-SSD-UUID /srv/sticktalk ext4 defaults,nofail,noatime 0 2
```

Save, then run `sudo mount /srv/sticktalk` and `findmnt /srv/sticktalk`.
It must show your SSD partition, not the root/SD-card filesystem. Leave at least
10 GiB free plus space for the local backup snapshot (one extra copy of uploads).
The service refuses to start if this mount is missing; it will not silently put
data back on the SD card. If an SSD mount fails, chat stays offline until fixed.

## 4. Build on the Windows development computer

Never compile on the 2 GB Pi. From the project folder in PowerShell:

```powershell
./deploy/pi/build.ps1 -InstallTools
```

This installs pinned Zig 0.13.0 and cargo-zigbuild 0.24.1 via Python when requested,
cross-compiles and **links** the release ARM64 executable, then checks its ELF
architecture. Python 3, Rust and Visual Studio C++ build tools must already exist.
Later builds can omit `-InstallTools`. Build and Zig cache output stays outside
OneDrive. Target: `aarch64-unknown-linux-gnu.2.36` (Bookworm glibc baseline).

For the first install, copy the binary and kit. Replace the username and DNS name:

```powershell
ssh piuser@sticktalk.friendly-tailnet.ts.net 'mkdir -p ~/sticktalk-setup'
scp C:/Users/dwhee/.cargo-target/sticktalk-server/aarch64-unknown-linux-gnu/release/sticktalk-server piuser@sticktalk.friendly-tailnet.ts.net:sticktalk-setup/
scp -r ./deploy/pi piuser@sticktalk.friendly-tailnet.ts.net:sticktalk-setup/
```

SSH asks you to verify the Pi's identity the first time. Compare its fingerprint
using the Pi owner, rather than turning host-key checks off.

## 5. Install on the Pi

On a **dedicated** Pi (this replaces `/etc/caddy/Caddyfile`):

```sh
cd ~/sticktalk-setup
sudo bash pi/install.sh sticktalk.friendly-tailnet.ts.net
```

The script installs Caddy/SQLite/rsync, creates an unprivileged service account,
requests HTTPS using **`tailscale cert`**, and starts chat. It does not format
disks. The certificate is publicly trusted; certificate transparency reveals the
Pi's DNS name, not your messages. A daily timer renews certificate files and
reloads Caddy; these files do **not** renew automatically by themselves.
See [Tailscale HTTPS guidance](https://tailscale.com/docs/how-to/set-up-https-certificates).

Settings live in `/etc/sticktalk/server.env` (owner-readable only). Chat data lives
in `/srv/sticktalk/data`: SQLite database/WAL, `files/`, and signed `updates/`.
Leave upload size and per-user quota at `0` for unlimited; the disk floor still
applies. A local snapshot backup needs extra space beyond this floor. Never copy
only a running database's `.sqlite3` file: recent messages may still be in WAL.

Useful checks:

```sh
systemctl --no-pager status sticktalk caddy
curl https://sticktalk.friendly-tailnet.ts.net/api/v1/health
systemctl list-timers 'sticktalk-*'
sudo journalctl -u sticktalk -n 50 --no-pager
df -h /srv/sticktalk
```

Logs are volatile, limited to 64 MiB in RAM (128 MiB if persistent storage is
later enabled), and do not contain chat bodies or auth tokens. The journald drop-in
changes global host limits. Export diagnostic logs before reboot if needed.

Your friends enter `https://sticktalk.friendly-tailnet.ts.net` in StickTalk while
connected to Tailscale. The release script sets `STICKTALK_DEFAULT_SERVER` at
build time, so the distributed desktop app can prefill this address.

## 6. Turn on a nightly backup to a second computer

A backup on the same SSD is **not** a disaster backup. Ask your helper to set up
a second Tailscale Linux PC/Oracle VM with SSH, rsync, a dedicated `backup` user,
and a private destination directory. A Windows destination can use an SSH/rsync
environment configured by its owner. Oracle object storage is an alternative,
but this kit's shipped transport is rsync over SSH, not an object-storage client.

On the Pi, generate a dedicated root-owned SSH key for backups (not your release
signing key), authorize its public key on that destination with no port forwarding,
verify/pin the destination SSH host key as root, and test a BatchMode connection.
Keep the private key mode 0600. Then:

```sh
sudo install -m 0600 ~/sticktalk-setup/pi/backup.env.example /etc/sticktalk/backup.env
sudo nano /etc/sticktalk/backup.env
sudo systemctl start sticktalk-backup.service
sudo journalctl -u sticktalk-backup -n 30 --no-pager
sudo systemctl enable --now sticktalk-backup.timer
```

Set the real destination/key path in the file first. The script briefly stops
chat, uses SQLite `.backup`, checks the database, rsyncs matching uploads and update
artifacts into a reusable SSD snapshot, restarts chat even after local failure,
then sends a dated copy off-device. First/large backups can mean noticeable chat
downtime. Remote transfer errors fail the service and leave the local snapshot.
The remote parent directory must already exist. Configure destination retention
(for example seven daily sets plus four weekly sets); the script never deletes
remote history. Backups contain private messages, credentials and files: keep them
private and use encrypted storage at the destination. Keep `/etc/sticktalk` and
SSH keys in a separate encrypted admin backup. The signing private key stays offline.

**Try a restore before relying on this:** stop chat; move the existing data aside
on the SSD; restore one dated set's database, `files/` and `updates/` into
`/srv/sticktalk/data`; set ownership recursively to `sticktalk:sticktalk`; run
SQLite `PRAGMA quick_check;`; start chat; verify old messages and attachments from
a friend's account. Restore a matching set, not today's files with last week's DB.
The kit has not been exercised on your physical Pi or backup destination yet.

## 7. Updates and account help

For subsequent server deployments on Windows:

```powershell
./deploy/pi/build.ps1
./deploy/pi/deploy.ps1 -Destination piuser@sticktalk.friendly-tailnet.ts.net
```

Keep the previous binary before upgrading if you need rollback. Database migrations
are forward-only: restore a compatible full backup if rolling back schema changes.
Client installer/signing instructions are in `tools/release/README.md`.

Admin commands run **locally on the Pi**, not from a public API:

```sh
sudo -u sticktalk /usr/local/bin/sticktalk-server --data-dir /srv/sticktalk/data admin list-users
sudo -u sticktalk /usr/local/bin/sticktalk-server --data-dir /srv/sticktalk/data admin reset-password alice
sudo -u sticktalk /usr/local/bin/sticktalk-server --data-dir /srv/sticktalk/data admin disable-user alice
sudo -u sticktalk /usr/local/bin/sticktalk-server --data-dir /srv/sticktalk/data admin enable-user alice
```

Send the printed reset code privately to the right person after verifying their
identity. It expires in 30 minutes and works once through the reset-password API.
Issuing a code/disable revokes all their sessions. Existing sockets are disconnected
at the next housekeeping tick (at most 60 seconds); mutations reject revoked
sessions immediately. Re-enabling does not restore sessions. Never paste a code
or publishing token into a public channel.

## Voice, video, screen share and remote control

The server embeds the str0m SFU; no separate daemon or Caddy UDP proxy is needed.
The installer binds media to `tailscale ip -4`. Existing installations must add
these settings to `/etc/sticktalk/server.env` and restart `sticktalk`:

```sh
STICKTALK_VOICE_BIND_IP=100.64.0.2 # replace with this Pi's actual address
STICKTALK_VOICE_UDP_PORT_START=50000
STICKTALK_VOICE_UDP_PORT_END=50100
STICKTALK_VOICE_EGRESS_BPS=32000000
STICKTALK_VOICE_MAX_DATAGRAM_BYTES=1232
```

Allow **UDP 50000–50100** from authorized friends to this Pi in the Tailscale
policy, alongside TCP 443. For example, the media grant's `ip` entry is
`["udp:50000-50100"]`; use your existing approved source/destination selectors.
If a host firewall is enabled, permit that UDP range on `tailscale0` only
(e.g. `ufw allow in on tailscale0 proto udp to any port 50000:50100`). Do not
open it on the public interface or router. One UDP socket multiplexes all calls
and chooses the first available port in the range; do not allow only the first
port unless the configured range is also one port.

`--voice-advertised-ip` / `STICKTALK_VOICE_ADVERTISED_IP` can pin the ICE address.
Without it the server discovers an assigned IPv4 **100.64/10** interface address,
then falls back to the HTTP bind IP. An unspecified advertised IP fails startup;
set it explicitly on non-Tailscale/wildcard HTTP hosts. IPv6 is supported with
explicit matching `--voice-bind-ip` and advertised IP. No STUN/TURN or router
forwarding is needed when every participant can reach the Pi through Tailscale.
Caddy carries HTTPS/WS only; WebRTC's DTLS/SRTP protects the media UDP hop.

The default UDP ceiling is **1232 bytes**, allowing IPv6 UDP on a 1280-byte
Tailscale MTU (IPv4 permits 1252). Standard browser-sized ~1200-byte RTP packets
now pass; the old 1060-byte compressed-payload rejection is removed. The limit
includes RTP extensions, SRTP authentication and RTX. Native packetizers should
leave headroom (<=1180 compressed bytes for the tested profile). Larger or
unusual extension sets still hit the final wire-size guard; there is no codec
repacketization or IP fragmentation. Do not raise the ceiling above your path's
allowance. The configurable range is 1200–1472 bytes.

The default aggregate outgoing video cap is **32 Mbps**, including headers,
SRTP, retransmissions and probes. Audio/control/handshakes are separate. The SFU
requests sender bitrate reductions and hard-limits final video egress. A home
uplink needs more than 8 Mbps per 8 Mbps viewer after transport overhead; tune
the cap below the available upload bandwidth, not the Pi's nominal Ethernet rate.

Test direct and **DERP-relayed** paths with `tailscale ping` / `tailscale status`.
Automatic trusted DERP telemetry is not wired in: protocol path remains `unknown`,
never guessed `direct` from an ICE host candidate. Physical Pi performance,
real-browser playback and the 10-minute 1080p60 capture/display acceptance still
need hardware validation. Remote-control consent/grant/input enforcement is on
the server; capture, playback and OS input injection remain client duties.

Build tool reference: [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild).
