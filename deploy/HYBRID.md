# Optional: Oracle main + private Pi storage

> Optional cloud topology. Most people should self-host instead; see
> [SELF_HOST.md](SELF_HOST.md).

## 9. Connect storage

1. Finish [Pi setup](pi/README.md). Replace example Pi address `100.64.0.2`
   below with its actual Tailscale IPv4. Pi gets no router forwarding, Funnel,
   public DNS record or public Caddy listener.
2. On Oracle install Tailscale, inspect the installer, then join the same tailnet:

   ```bash
   curl -fsSL https://tailscale.com/install.sh -o /tmp/install-tailscale.sh
   less /tmp/install-tailscale.sh
   sh /tmp/install-tailscale.sh
   sudo tailscale up
   tailscale ip -4
   tailscale ping 100.64.0.2
   ```

   Open the login URL yourself. Restrict Tailscale policy to Oracle → Pi TCP
   9400 and owner SSH access; preserve existing administrator access. Friends
   normally need neither Tailscale nor storage access. DERP relay works but
   direct connections are faster. Never put tailnet auth keys in Git.
3. Run `sudo nano /etc/sticktalk.env` on Oracle. Privately copy Pi's **same**
   secret (do not generate another). Add, replacing placeholders:

   ```text
   STICKTALK_STORAGE_URL=http://100.64.0.2:9400
   STICKTALK_STORAGE_SECRET=PASTE_THE_PI_SECRET_HERE
   STICKTALK_CACHE_MB=102400
   STICKTALK_CACHE_FILE_MB=25
   STICKTALK_MIN_FREE_DISK_GB=10
   ```

   HTTP here is inside encrypted Tailscale, never public Internet. Only literal
   tailnet IPv4 or loopback destinations are accepted. HMAC-SHA256 signs method,
   path, timestamp, nonce, offset, length, digest and Range. Persistent nonce
   protection rejects replay after restart. Keep clocks synchronized.
4. Restart and check. The CLI does not automatically source environment files:

   ```bash
   sudo chmod 600 /etc/sticktalk.env
   sudo systemctl restart sticktalk
   sudo bash -c 'set -a; . /etc/sticktalk.env; set +a; exec runuser -u sticktalk -- /usr/local/bin/sticktalk-server --data-dir /var/lib/sticktalk admin storage-status'
   sudo journalctl -u sticktalk -n 30 --no-pager
   ```

5. Resumable chunks stream to Pi with 64 KiB I/O buffers. Oracle retains a
   durable local spool until final hash verification/publication on Pi. This is
   **not spool-free direct-to-Pi ingestion**: concurrent in-progress uploads need
   their full size temporarily available on Oracle under the existing disk guard.
   Published files above 25 MiB leave Oracle. Small files, avatars/icons and
   thumbnails may stay in a configurable 100 GiB size-bounded LRU; the total cap
   can evict any hot file. Only acknowledged replicas may be evicted. Outage
   files are never disposable cache. Eviction runs at publication/maintenance.
6. During Pi outages uploads continue locally while space permits. Cached
   downloads work; Pi-only downloads return **Storage temporarily offline. Please
   retry later.** Resume downloads with Range and uploads from their acknowledged
   offset. Pending files sync in bounded batches with a 60-second retry while
   work is pending (no idle polling). Existing local files migrate the same way.
   Leave the URL
   unset for pure local operation. Do not change storage destinations without
   restoring all files: the DB records which replicas were acknowledged.

## 10. Backups and Oracle reclamation

1. On Oracle, from the repository, install and test the nightly timer:

   ```bash
   sudo install -m 0644 deploy/sticktalk-backup.service deploy/sticktalk-backup.timer /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl start sticktalk-backup.service
   sudo journalctl -u sticktalk-backup -n 30 --no-pager
   sudo systemctl enable --now sticktalk-backup.timer
   systemctl list-timers sticktalk-backup.timer
   ```

   It takes an online SQLite `VACUUM INTO` snapshot without stopping chat,
   integrity-checks it, ensures every referenced blob is on Pi, then streams the
   database. Allow one extra DB copy on Oracle. Failed backups never publish a
   completed snapshot or prune history. Completed snapshots cover the last 14
   days. Deleted completed files remain on Pi for 15 days so retained DBs can restore them.
   Cleanup is bounded/request-driven; prolonged outages can leave older snapshots
   until the next successful backup. Inspect failures; a timer is not proof of
   a verified backup. Keep config/secrets separately in an encrypted admin backup.
2. **Oracle may reclaim idle Always Free VMs.** Do not burn CPU, fake traffic
   or add keepalives. Nightly off-VM backup plus Pi standby are the mitigation.
   Expect up to one day of history loss, longer if backups failed. Files cannot
   reconstruct messages absent from the DB. Pi/HDD failure still loses the sole
   bulk copy: keep an independent encrypted off-site copy if those files matter.
3. Rehearse [one-command Pi failover](pi/README.md#6-one-command-cold-failover).
   Pi stays private: friends need temporary tailnet access, or restore a public
   replacement Oracle VM. DNS/HTTPS/media recovery is deliberate, not automatic.
   Never run two writable primaries. Physical Pi/Linux/tailnet validation remains
   necessary before relying on the standby.

## Encrypted online backups
Set `STICKTALK_BACKUP_RECIPIENT=age1...` in `/etc/sticktalk.env` before enabling
the backup timer. The owner generates the age identity and retains its private
key offline; only the public recipient belongs on Oracle/Pi. Online snapshots
are now `.sqlite3.age`; decrypt on the owner's trusted computer before manual
standby recovery. Never feed ciphertext to SQLite or put the owner's private key
on the standby. Remove legacy plaintext snapshots after a verified encrypted
backup. See `pi/README.md` for LUKS, exact commands and 14-day privacy retention.
