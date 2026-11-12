#!/usr/bin/env bash
# Run with sudo. Stops only StickTalk, and always attempts to restore its state.
set -euo pipefail
umask 077
[[ $EUID == 0 ]] || { echo 'Run with sudo.' >&2; exit 1; }
data=/var/lib/sticktalk
root=/var/backups/sticktalk
[[ -f $data/sticktalk.sqlite3 && -d $data/files ]] || { echo 'Missing server data.' >&2; exit 1; }
was_active=$(systemctl is-active sticktalk.service || true)
restart() {
  local status=$?
  if [[ $was_active == active ]]; then
    systemctl start sticktalk.service || { echo 'BACKUP: failed to restart StickTalk.' >&2; exit 1; }
  fi
  exit "$status"
}
trap restart EXIT
systemctl stop sticktalk.service
install -d -m 0700 "$root"
destination=$(mktemp -d "$root/$(date -u +%Y%m%dT%H%M%SZ).XXXXXX")
# SQLite's backup API includes committed WAL state; files are quiescent too.
sqlite3 "$data/sticktalk.sqlite3" ".backup '$destination/sticktalk.sqlite3'"
[[ $(sqlite3 "$destination/sticktalk.sqlite3" 'PRAGMA quick_check;') == ok ]]
cp -a -- "$data/files" "$destination/files"
printf 'Backup complete: %s\n' "$destination"
