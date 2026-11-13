# Private Pi storage and cold standby

Oracle is the public main server. Pi 4 (2 GB+, 64-bit Bookworm, wired Ethernet,
powered ~5 TB USB HDD) is private storage, backup target and standby. Friends
normally connect to Oracle without Tailscale. **Never expose Pi publicly.**

## 1. Install Tailscale

On Pi over SSH, run:

```bash
sudo apt-get update
sudo apt-get install -y sqlite3 ca-certificates curl
curl -fsSL https://tailscale.com/install.sh -o /tmp/install-tailscale.sh
less /tmp/install-tailscale.sh
sh /tmp/install-tailscale.sh
sudo tailscale up
tailscale ip -4
```

Open the login URL yourself. Save Pi's `100.x` IPv4. Join Oracle to the same
tailnet. In Tailscale's policy allow only Oracle → Pi TCP 9400 and owner → Pi SSH;
retain existing admin access. Never forward router ports or enable Funnel.
No Caddy is needed for storage. For an old Pi-only installation first run:

```bash
sudo systemctl disable --now sticktalk caddy
```

## 2. Mount the HDD

Ask your helper for an ext4 USB disk. **Do not format existing data.** Run
`lsblk -f`; identify the partition by capacity, label and USB connection.
Copy its UUID, then run `sudo mkdir -p /srv/sticktalk` and `sudo nano /etc/fstab`.
Add, replacing the placeholder with the actual UUID:

```text
UUID=YOUR-HDD-UUID /srv/sticktalk ext4 defaults,nofail,noatime 0 2
```

Run `sudo mount /srv/sticktalk`, `findmnt /srv/sticktalk` and `df -h /srv/sticktalk`.
It must show the USB partition, not the SD card. Keep 10 GiB free plus backup and
deleted-file retention space. The service refuses startup without this mount.

## 3. Build and install

On Windows, in the repository, use the existing ARM64 cross-build helper; do
not compile on the 2 GB Pi or copy a newer Ubuntu-glibc binary to Bookworm:

```powershell
./deploy/pi/build.ps1 -InstallTools
ssh piuser@100.64.0.2 'mkdir -p ~/sticktalk-setup'
scp C:/Users/dwhee/.cargo-target/sticktalk-server/aarch64-unknown-linux-gnu/release/sticktalk-server piuser@100.64.0.2:sticktalk-setup/
scp -r ./deploy/pi piuser@100.64.0.2:sticktalk-setup/
```

Replace username/IP and verify the SSH fingerprint with the owner. On Pi:

```bash
cd ~/sticktalk-setup
id sticktalk || sudo useradd --system --user-group --home-dir /srv/sticktalk --shell /usr/sbin/nologin sticktalk
sudo install -d -m 0700 /etc/sticktalk
sudo install -d -o sticktalk -g sticktalk -m 0700 /srv/sticktalk/node
sudo install -m 0755 sticktalk-server /usr/local/bin/sticktalk-server
sudo install -m 0644 pi/sticktalk-storage.service pi/sticktalk-standby.service /etc/systemd/system/
sudo install -m 0700 pi/failover.sh /usr/local/sbin/sticktalk-failover
```

Do **not** run the old `install.sh`; it installs a Pi-only primary.

## 4. Generate the secret and start storage

Generate the secret only on Pi:

```bash
sudo sh -c 'umask 077; printf "STICKTALK_STORAGE_SECRET=" > /etc/sticktalk/storage.env; head -c 32 /dev/urandom | base64 >> /etc/sticktalk/storage.env'
sudo nano /etc/sticktalk/storage.env
```

Add `STICKTALK_STORAGE_LISTEN=100.64.0.2:9400`, using Pi's actual address.
Privately copy the **same secret line** to Oracle's `/etc/sticktalk.env`.
Never paste it in Git, chat, logs or screenshots. Then:

```bash
sudo chmod 600 /etc/sticktalk/storage.env
sudo systemctl daemon-reload
sudo systemctl enable --now sticktalk-storage
systemctl --no-pager status sticktalk-storage
sudo journalctl -u sticktalk-storage -n 30 --no-pager
ss -ltn | grep 9400
```

The listener must show Pi's `100.x` address only, never `0.0.0.0`. HMAC protects
chunk PUT/finalize/ranged GET/DELETE/stat/free-space; replay protection survives
restarts. Keep clocks synchronized. The service streams with bounded buffers
and has a 512 MiB memory limit. There is no unauthenticated health endpoint;
check from Oracle with `admin storage-status` as described in the main guide.

## 5. Prepare standby now

Run `sudo nano /etc/sticktalk/standby.env` and add (replace every IP):

```text
STICKTALK_DATA_DIR=/srv/sticktalk/node
STICKTALK_BIND=100.64.0.2:8080
STICKTALK_PUBLIC_URL=http://100.64.0.2:8080
STICKTALK_VOICE_BIND_IP=100.64.0.2
STICKTALK_VOICE_ADVERTISED_IP=100.64.0.2
STICKTALK_MIN_FREE_DISK_GB=10
STICKTALK_MAX_UPLOAD_MB=0
```

Copy Google client ID, update public key and other needed main-server settings
privately. **Do not set a storage URL**: standby uses local `files/`. Run
`sudo chmod 600 /etc/sticktalk/standby.env`. Do not enable/start standby yet.
Set up and test the main guide's nightly backup timer. Keep environment files,
tailnet access and signing settings separately in an encrypted admin backup.

## 6. One-command cold failover

1. **Fence Oracle first.** Stop `sticktalk` and power Oracle off or revoke its
   public ingress. A lost SSH connection is not proof it stopped. Never run two
   writable primaries.
2. On Pi run:

   ```bash
   sudo /usr/local/sbin/sticktalk-failover --oracle-fenced
   ```

   The helper selects the latest completed snapshot using storage metadata,
   validates the database and referenced files, stops storage, preserves any old
   standby DB/WAL, restores the snapshot and starts the full server. Resumable
   upload sessions are discarded; re-upload unfinished files. Expect history
   loss since the last successful nightly snapshot.
3. Check `systemctl status sticktalk-standby` and logs. Pi stays private. For
   temporary native-client access explicitly grant trusted friends tailnet access
   to Pi TCP 8080 and UDP 50000–50100. Tailscale encrypts that HTTP traffic.
   Browser media needs HTTPS/secure contexts: configure tailnet-only Caddy HTTPS
   separately if needed. Never open public Pi ports.
4. Prefer restoring a replacement public Oracle VM. Fence Pi standby first,
   online-backup its now-current DB, privately copy that DB and any new local
   files to Oracle, restart Pi storage, restore Oracle and update DuckDNS. Do
   not overwrite newer Pi conversations with an old Oracle DB. Failback is manual.

Oracle can reclaim idle Always Free VMs. Nightly off-VM backups plus this standby
are the mitigation; **no CPU-burning keepalives**. The HDD is still the only bulk
copy, not protection from HDD failure. Rehearse on physical Pi/Linux/tailnet
equipment before relying on this procedure.
