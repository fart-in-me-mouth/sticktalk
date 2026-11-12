#!/usr/bin/env bash
# Consistent SQLite + uploads, no live-WAL copy. Run by the root timer.
set -euo pipefail
umask 077
source /etc/sticktalk/backup.env
: "${BACKUP_DEST:?Set the off-device rsync destination}"
: "${BACKUP_KEY:?Set the private SSH key file}"
[[ $BACKUP_DEST =~ ^[a-zA-Z0-9_.@:/-]+$ && $BACKUP_DEST != -* && $BACKUP_KEY =~ ^/[a-zA-Z0-9_./-]+$ ]] || exit 1
mountpoint -q /srv/sticktalk || { echo 'SSD missing; refusing backup.' >&2; exit 1; }
exec 9>/run/lock/sticktalk-backup.lock
flock -n 9 || exit 0
data=/srv/sticktalk/data
root=/srv/sticktalk/backups
install -d -m 0700 "$root"
# Reusable local snapshot: allow space for one extra copy on the SSD.
snapshot=$root/snapshot
install -d -m 0700 "$snapshot/files" "$snapshot/updates"
was_active=$(systemctl is-active sticktalk || true)
restart() {
  local result=$?
  if [[ $was_active == active ]]; then systemctl start sticktalk || { echo 'BACKUP: restart failed' >&2; exit 1; }; fi
  exit "$result"
}
trap restart EXIT
systemctl stop sticktalk
sqlite3 "$data/sticktalk.sqlite3" ".backup '$snapshot/sticktalk.sqlite3.new'"
[[ $(sqlite3 "$snapshot/sticktalk.sqlite3.new" 'PRAGMA quick_check;') == ok ]]
rsync -a --delete "$data/files/" "$snapshot/files/"
if [[ -d $data/updates ]]; then rsync -a --delete "$data/updates/" "$snapshot/updates/"; fi
mv -- "$snapshot/sticktalk.sqlite3.new" "$snapshot/sticktalk.sqlite3"
if [[ $was_active == active ]]; then systemctl start sticktalk; fi
trap - EXIT
# Dated remote sets; destination owner must configure retention and permissions.
rsync -a -e "ssh -i $BACKUP_KEY -o BatchMode=yes -o StrictHostKeyChecking=yes" "$snapshot/" "$BACKUP_DEST/$(date -u +%Y%m%dT%H%M%SZ)/"
echo 'StickTalk off-device backup completed.'
