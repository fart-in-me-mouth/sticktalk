# Private Pi storage and cold standby

Oracle is the public main server. Pi 4 (2 GB+, 64-bit Bookworm, wired Ethernet,
powered ~5 TB USB HDD) is private storage, backup target and standby. Friends
normally connect to Oracle without Tailscale. **Never expose Pi publicly.**

## 1. Install Tailscale

On Pi over SSH, run:

```bash
sudo apt-get update
sudo apt-get install -y sqlite3 ca-certificates curl cryptsetup age rsync
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

Use the **LUKS2 encrypted data volume** in "Protect the SSD and backups against
theft" below before installing the service. Do not add a plain ext4 USB partition
to fstab. That section has exact identification, backup, unlock and recovery
commands; formatting is only for a verified EMPTY partition.

`findmnt /srv/sticktalk` must show `/dev/mapper/sticktalk-data`, not an unencrypted
USB partition or the SD card. Keep 10 GiB free plus backup/retention space. The
backup script refuses a different mount. See the manual-unlock versus unattended
keyfile tradeoff below before deciding how the Pi should reboot.

## 3. Build and install

On Windows, in the repository, use the existing ARM64 cross-build helper; do
not compile on the 2 GB Pi or copy a newer Ubuntu-glibc binary to Bookworm:

```powershell
./deploy/pi/build.ps1 -InstallTools
ssh piuser@100.64.0.2 'mkdir -p ~/sticktalk-setup'
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PWD 'target' }
scp "$target/aarch64-unknown-linux-gnu/server/sticktalk-server" piuser@100.64.0.2:sticktalk-setup/
scp -r ./deploy/pi piuser@100.64.0.2:sticktalk-setup/
```

Replace username/IP and verify the SSH fingerprint with the owner. On Pi:

```bash
cd ~/sticktalk-setup
id sticktalk || sudo useradd --system --user-group --home-dir /srv/sticktalk --shell /usr/sbin/nologin sticktalk
sudo install -d -m 0700 /etc/sticktalk
# The encrypted mount root is 0700: the service must be able to traverse it.
sudo chown sticktalk:sticktalk /srv/sticktalk
sudo install -d -o sticktalk -g sticktalk -m 0700 /srv/sticktalk/node
sudo install -m 0755 sticktalk-server /usr/local/bin/sticktalk-server
sudo install -m 0644 pi/sticktalk-storage.service pi/sticktalk-standby.service /etc/systemd/system/
sudo install -m 0700 pi/failover.sh /usr/local/sbin/sticktalk-failover
sudo install -m 0700 pi/backup.sh /usr/local/sbin/sticktalk-backup
sudo install -m 0644 pi/sticktalk-backup.service pi/sticktalk-backup.timer /etc/systemd/system/
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
   # First decrypt/review a completed .sqlite3.age on your trusted recovery PC,
   # then transfer recovery.sqlite3 to /srv/sticktalk/node/backups on LUKS.
   sudo STICKTALK_RECOVERY_DB=recovery.sqlite3 /usr/local/sbin/sticktalk-failover --oracle-fenced
   ```

   The helper requires that explicitly selected decrypted/reviewed snapshot,
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

## Protect the SSD and backups against theft (required)

Tailscale protects the network, **not a stolen drive**. Use LUKS2 for the Pi's
**data volume**, and age for every off-device backup. The Pi can read live data
while unlocked. Anyone with root access then can too. A keyfile on the same Pi
allows unattended reboot but does not protect against theft of both Pi and SSD;
the recommended plan is manual passphrase unlock after reboot.

### 1. Prepare an empty data partition (one-time, destructive)

**Do not run the format commands on a disk containing your only data copy.**
First make and test an encrypted backup below. Identify the external SSD by its
model, serial and size, not by a guessed `/dev/sda`. These commands run on the Pi,
not your Windows PC. Install tools, then list disks:

```sh
sudo apt-get update
sudo apt-get install cryptsetup age sqlite3 rsync
lsblk -o NAME,SIZE,MODEL,SERIAL,FSTYPE,MOUNTPOINTS
ls -l /dev/disk/by-id/
```

Replace `REPLACE_WITH_EMPTY_SSD_PARTITION` with the stable **partition** identifier
from that listing (usually ending in `-part1`). Ask for help if uncertain. Never
select the Pi boot/root SD partition. This erases the selected partition:

```sh
DATA=/dev/disk/by-id/REPLACE_WITH_EMPTY_SSD_PARTITION
readlink -f "$DATA"
lsblk -o NAME,SIZE,MODEL,FSTYPE,MOUNTPOINTS "$DATA"
# On an existing installation only: stop its storage/standby services first.
# sudo systemctl stop sticktalk-storage sticktalk-standby
sudo cryptsetup luksFormat --type luks2 "$DATA"
# Choose a long unique passphrase; save it in your password manager OFF the Pi.
sudo cryptsetup open "$DATA" sticktalk-data
sudo mkfs.ext4 -L sticktalk-data /dev/mapper/sticktalk-data
sudo mkdir -p /srv/sticktalk
sudo mount /dev/mapper/sticktalk-data /srv/sticktalk
sudo chmod 700 /srv/sticktalk
sudo cryptsetup luksHeaderBackup "$DATA" --header-backup-file /root/sticktalk-luks-header.img
sudo cryptsetup luksUUID "$DATA"
```

Keep the printed UUID and a copy of the header backup **offline** with the owner;
the header is not a decryption key, but a damaged header can make data unrecoverable.
For an existing installation, restore the private node archive into
`/srv/sticktalk/node`, and restore the
service's ownership (`sudo chown -R sticktalk:sticktalk /srv/sticktalk/node`).
Older Pi-only servers use `/srv/sticktalk/data` instead. Add to
`/etc/fstab` (not a second plaintext mount) using `sudoedit /etc/fstab`:

```text
/dev/mapper/sticktalk-data /srv/sticktalk ext4 defaults,noauto 0 2
```

After every reboot, unlock and start manually (replace the UUID):

```sh
sudo cryptsetup open /dev/disk/by-uuid/REPLACE_WITH_LUKS_UUID sticktalk-data
sudo mount /srv/sticktalk
sudo systemctl start sticktalk-storage
findmnt /srv/sticktalk
sudo cryptsetup status sticktalk-data
```

Do not start the old `sticktalk` primary or `sticktalk-standby` during a normal
storage-node reboot. Standby is only started after fencing Oracle in section 6.
On a fresh installation continue with section 3 after mounting; the account and
services do not exist yet.

**Optional unattended plan:** keep the passphrase as recovery and add a root-only
keyfile on the Pi's SD card. This protects a removed SSD, not the whole stolen Pi:

```sh
sudo install -d -m 700 /etc/sticktalk
sudo sh -c 'umask 077; head -c 64 /dev/urandom > /etc/sticktalk/data.key'
sudo cryptsetup luksAddKey /dev/disk/by-uuid/REPLACE_WITH_LUKS_UUID /etc/sticktalk/data.key
sudoedit /etc/crypttab
# Add: sticktalk-data UUID=REPLACE_WITH_LUKS_UUID /etc/sticktalk/data.key luks
sudoedit /etc/fstab
# Replace noauto with defaults for the mapper entry (do not duplicate it).
```

Keep a separate recovery passphrase; losing both it and the keyfile loses the data.
Never put the keyfile or recovery passphrase in the backup recipient configuration.

### 2. Owner-held encrypted backups (14-day retention)

On the **owner's separate trusted computer** with age installed, create a key:

```sh
age-keygen -o sticktalk-backup-key.txt
age-keygen -y sticktalk-backup-key.txt
```

Save the `AGE-SECRET-KEY-...` file in your password manager and a second offline
location. Put **only** the public `age1...` line on the Pi, using
`sudoedit /etc/sticktalk/backup.env`:

```text
BACKUP_RECIPIENT=age1REPLACE_WITH_OWNER_PUBLIC_RECIPIENT
BACKUP_DEST=backupuser@backuphost:/dedicated/sticktalk-encrypted
BACKUP_KEY=/root/.ssh/sticktalk_backup
```

That remote directory must be dedicated to StickTalk: the script mirrors expiry
with `rsync --delete`. Existing remote plaintext snapshots from older versions are
**not** secured retroactively: delete them after verifying an encrypted restore.
The hybrid setup in step 3 installs the backup helper and timer without running
the old Pi-primary installer. Create the dedicated destination and a root-only
SSH key for its backup account, verify the remote host fingerprint out of band,
and add it to root's known_hosts (StrictHostKeyChecking remains enabled). Set
`BACKUP_KEY` to that private key's absolute path. Then test and enable the timer:

```sh
sudo chmod 600 /etc/sticktalk/backup.env
sudo systemctl daemon-reload
sudo systemctl start sticktalk-backup.service
sudo journalctl -u sticktalk-backup -n 50 --no-pager
sudo systemctl enable --now sticktalk-backup.timer
systemctl list-timers sticktalk-backup.timer
```

Check the destination contains `.tar.age` files only. The script refuses an
unencrypted mount or missing recipient; it never sends a plaintext snapshot.

Test a restore on the owner's trusted computer (not into the live server):

```sh
mkdir -m 700 restore-test
age -d -i sticktalk-backup-key.txt -o snapshot.tar SNAPSHOT.tar.age
tar -xf snapshot.tar -C restore-test
sqlite3 -readonly restore-test/storage.sqlite3 'PRAGMA integrity_check;'
# For an older Pi-only archive, use restore-test/sticktalk.sqlite3 instead.
# Expect: ok. Verify a few attachments and the replica-backups/*.sqlite3.age files.
# To validate a chat snapshot too, replace TIMESTAMP with the newest actual name:
age -d -i sticktalk-backup-key.txt -o restored-chat.sqlite3 restore-test/replica-backups/backup-TIMESTAMP.sqlite3.age
sqlite3 -readonly restored-chat.sqlite3 'PRAGMA integrity_check;'
# Expect: ok. Remove these plaintext test copies after checking.
```

Both scripts retain encrypted snapshots for **14 days**, removing expired remote
copies on the next successful sync. An offline/unreachable backup host cannot be
pruned until it reconnects; the owner must delete expired copies there manually.
Account/message deletion affects live storage immediately, but historical copies
remain decryptable by the owner until expiry. Do not restore erased accounts from
old snapshots; reapply deletions before serving a restored backup. Backups are not
cryptographic per-user deletion, and SSD secure erasure is not guaranteed.

References: https://www.kernel.org/doc/html/latest/admin-guide/device-mapper/dm-crypt.html
and https://github.com/FiloSottile/age#usage .

Oracle's online `admin backup` also encrypts the SQLite snapshot before sending
it to Pi. Put `STICKTALK_BACKUP_RECIPIENT=age1...` (public key only) in Oracle's
`/etc/sticktalk.env`. Missing/invalid recipients fail closed. New snapshots are
`backup-TIMESTAMP.sqlite3.age`. Decrypt on the owner's trusted computer before
manual standby restore; do not copy the private identity to either server. Old
plaintext `.sqlite3` snapshots must be removed after testing the encrypted path.

For failover, place the owner-decrypted and deletion-reviewed database under
`/srv/sticktalk/node/backups/recovery.sqlite3`, fence Oracle, then run:
`sudo STICKTALK_RECOVERY_DB=recovery.sqlite3 bash failover.sh --oracle-fenced`.
The script no longer silently chooses an older plaintext snapshot. Remove this
recovery copy after installing it into the encrypted live data directory.
