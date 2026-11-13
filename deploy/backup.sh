#!/usr/bin/env bash
# Run with sudo. Stops only StickTalk, and always attempts to restore its state.
set -euo pipefail
umask 077
source /etc/sticktalk/backup.env
: "${BACKUP_RECIPIENT:?Set the owner age public recipient (age1...)}"
[[ $BACKUP_RECIPIENT == age1* ]] || exit 1
command -v age >/dev/null
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
tar -C "$destination" -cf - . | age -r "$BACKUP_RECIPIENT" -o "$destination.tar.age.tmp"
mv -- "$destination.tar.age.tmp" "$destination.tar.age"
rm -rf -- "$destination"
find "$root" -maxdepth 1 -type f -name '*.tar.age' -mmin +20160 -delete
printf 'Encrypted backup complete: %s\n' "$destination.tar.age"
