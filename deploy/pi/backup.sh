#!/usr/bin/env bash
# Consistent SQLite + uploads, no live-WAL copy. Run by the root timer.
set -euo pipefail
umask 077
source /etc/sticktalk/backup.env
: "${BACKUP_DEST:?Set the off-device rsync destination}"
: "${BACKUP_KEY:?Set the private SSH key file}"
: "${BACKUP_RECIPIENT:?Set the owner age public recipient (age1...)}"
[[ $BACKUP_RECIPIENT == age1* ]] || { echo 'Invalid age public recipient.' >&2; exit 1; }
command -v age >/dev/null
[[ $(findmnt -n -o SOURCE /srv/sticktalk) == /dev/mapper/sticktalk-data ]] || { echo 'Encrypted volume is not mounted; refusing backup.' >&2; exit 1; }
[[ $BACKUP_DEST =~ ^[a-zA-Z0-9_.@:/-]+$ && $BACKUP_DEST != -* && $BACKUP_KEY =~ ^/[a-zA-Z0-9_./-]+$ ]] || exit 1
mountpoint -q /srv/sticktalk || { echo 'SSD missing; refusing backup.' >&2; exit 1; }
exec 9>/run/lock/sticktalk-backup.lock
flock -n 9 || exit 0
# Current private-node layout, with compatibility for an older Pi-only server.
if [[ -f /srv/sticktalk/node/storage.sqlite3 ]]; then
  data=/srv/sticktalk/node
  database=storage.sqlite3
  services=(sticktalk-storage sticktalk-standby)
else
  data=/srv/sticktalk/data
  database=sticktalk.sqlite3
  services=(sticktalk)
fi
[[ -f $data/$database ]] || { echo 'Missing database; refusing backup.' >&2; exit 1; }
root=/srv/sticktalk/backups
install -d -m 0700 "$root"
# Reusable local snapshot: allow space for one extra copy on the SSD.
snapshot=$root/snapshot
install -d -m 0700 "$snapshot/files" "$snapshot/updates"
active_services=()
for service in "${services[@]}"; do
  if systemctl is-active --quiet "$service"; then active_services+=("$service"); fi
done
restart() {
  local result=$?
  for service in "${active_services[@]}"; do
    systemctl start "$service" || { echo 'BACKUP: restart failed' >&2; exit 1; }
  done
  exit "$result"
}
trap restart EXIT
systemctl stop "${services[@]}"
rm -f -- "$snapshot/sticktalk.sqlite3" "$snapshot/storage.sqlite3"
sqlite3 "$data/$database" ".backup '$snapshot/$database.new'"
[[ $(sqlite3 -readonly "$snapshot/$database.new" 'PRAGMA quick_check;') == ok ]]
rsync -a --delete "$data/files/" "$snapshot/files/"
if [[ -d $data/updates ]]; then rsync -a --delete "$data/updates/" "$snapshot/updates/"; fi
if [[ -d $data/backups ]]; then
  install -d -m 0700 "$snapshot/replica-backups"
  rsync -a --delete "$data/backups/" "$snapshot/replica-backups/"
fi
mv -- "$snapshot/$database.new" "$snapshot/$database"
for service in "${active_services[@]}"; do systemctl start "$service"; done
trap - EXIT
# Only ciphertext leaves the encrypted data volume. The private key is NEVER here.
archives=$root/encrypted
install -d -m 0700 "$archives"
archive=$archives/$(date -u +%Y%m%dT%H%M%SZ).tar.age
tar -C "$snapshot" -cf - . | age -r "$BACKUP_RECIPIENT" -o "$archive.tmp"
mv -- "$archive.tmp" "$archive"
# Snapshot is regenerable; all persistent backups are encrypted.
rm -rf -- "$snapshot"
# 14-day retention; --delete mirrors expiry to the dedicated remote directory.
find "$archives" -maxdepth 1 -type f -name '*.tar.age' -mmin +20160 -delete
rsync -a --delete -e "ssh -i $BACKUP_KEY -o BatchMode=yes -o StrictHostKeyChecking=yes" "$archives/" "$BACKUP_DEST/"
echo 'Encrypted StickTalk off-device backup completed (14-day retention).'
