#!/usr/bin/env bash
# Manual cold failover. Never run two writable primaries.
set -euo pipefail
[[ $EUID == 0 && ${1:-} == --oracle-fenced ]] || { echo 'First stop/fence Oracle; then run sudo bash failover.sh --oracle-fenced' >&2; exit 1; }
mountpoint -q /srv/sticktalk || { echo 'USB disk is not mounted' >&2; exit 1; }
[[ -f /etc/sticktalk/standby.env ]] || { echo 'Prepare standby.env first' >&2; exit 1; }
data=/srv/sticktalk/node
# The owner decrypts the newest .sqlite3.age on a separate trusted computer,
# reapplies deletions, then transfers only that recovery DB to this LUKS volume.
latest=${STICKTALK_RECOVERY_DB:-}
[[ -n $latest && $latest != */* && $latest != *..* && -f $data/backups/$latest ]] || {
  echo 'Set STICKTALK_RECOVERY_DB to the owner-decrypted/reviewed SQLite recovery file in node/backups. Never copy the age private key here.' >&2
  exit 1
}
[[ $(sqlite3 "$data/backups/$latest" 'PRAGMA quick_check;') == ok ]] || { echo 'Backup failed integrity check' >&2; exit 1; }
while IFS= read -r key; do
  [[ $key != *'/'* && $key != *'..'* && -f $data/files/$key ]] || { echo "Missing snapshot blob: $key" >&2; exit 1; }
done < <(sqlite3 "$data/backups/$latest" 'SELECT storage_key FROM uploads UNION SELECT thumbnail_key FROM uploads WHERE thumbnail_key IS NOT NULL;')
systemctl stop sticktalk-storage.service sticktalk-standby.service
if [[ -f $data/sticktalk.sqlite3 ]]; then
  old="$data/pre-failover-$(date -u +%s)"
  mkdir -m 0700 "$old"
  for suffix in '' -wal -shm; do
    [[ ! -f $data/sticktalk.sqlite3$suffix ]] || mv -- "$data/sticktalk.sqlite3$suffix" "$old/"
  done
fi
install -o sticktalk -g sticktalk -m 0600 "$data/backups/$latest" "$data/sticktalk.sqlite3"
sqlite3 "$data/sticktalk.sqlite3" 'DELETE FROM partial_uploads; DELETE FROM upload_reservations; DELETE FROM blob_replicas;'
chown sticktalk:sticktalk "$data/sticktalk.sqlite3"
systemctl start sticktalk-standby.service
echo "Pi standby started from $latest. Only tailnet users can connect; see README for public recovery."
