#!/usr/bin/env bash
# Dedicated Pi only. Never formats disks or changes router ports.
set -euo pipefail
[[ $EUID == 0 && $# == 1 ]] || { echo 'Usage: sudo bash install.sh chat.your-tailnet.ts.net' >&2; exit 1; }
host=$1
[[ $host =~ ^[a-z0-9.-]+\.ts\.net$ ]] || { echo 'Use the full Tailscale HTTPS name.' >&2; exit 1; }
[[ $(uname -m) == aarch64 ]] || { echo 'Install 64-bit Raspberry Pi OS.' >&2; exit 1; }
mountpoint -q /srv/sticktalk || { echo 'Mount the USB SSD at /srv/sticktalk first.' >&2; exit 1; }
[[ -f ./sticktalk-server ]] || { echo 'Copy the cross-built binary here first.' >&2; exit 1; }
command -v tailscale >/dev/null || { echo 'Install and connect Tailscale first.' >&2; exit 1; }
ip=$(tailscale ip -4)
[[ $ip =~ ^100\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 1
apt-get update
apt-get install -y caddy sqlite3 rsync
id sticktalk &>/dev/null || useradd --system --home-dir /srv/sticktalk/data --shell /usr/sbin/nologin sticktalk
install -d -o sticktalk -g sticktalk -m 0700 /srv/sticktalk/data
install -d -o root -g root -m 0700 /etc/sticktalk
install -o root -g root -m 0755 ./sticktalk-server /usr/local/bin/sticktalk-server.new
mv /usr/local/bin/sticktalk-server.new /usr/local/bin/sticktalk-server
kit=$(cd -- "$(dirname -- "$0")" && pwd)
if [[ ! -e /etc/sticktalk/server.env ]]; then
  install -m 0600 "$kit/server.env.example" /etc/sticktalk/server.env
  sed -i -e "s|https://REPLACE.your-tailnet.ts.net|https://$host|" -e "s|REPLACE_TAILSCALE_IP|$ip|" /etc/sticktalk/server.env
fi
printf 'STICKTALK_HOST=%s\nSTICKTALK_TAILSCALE_IP=%s\n' "$host" "$ip" >/etc/sticktalk/tailscale.env
chmod 0600 /etc/sticktalk/tailscale.env
install -m 0755 "$kit/renew-cert.sh" /usr/local/sbin/sticktalk-renew-cert
install -m 0755 "$kit/backup.sh" /usr/local/sbin/sticktalk-backup
for unit in sticktalk.service sticktalk-cert.service sticktalk-cert.timer sticktalk-backup.service sticktalk-backup.timer; do
  install -m 0644 "$kit/$unit" "/etc/systemd/system/$unit"
done
install -d -m 0755 /etc/systemd/journald.conf.d
install -m 0644 "$kit/journald.conf" /etc/systemd/journald.conf.d/sticktalk.conf
/usr/local/sbin/sticktalk-renew-cert
sed -e "s/{\$STICKTALK_HOST}/$host/g" -e "s/{\$STICKTALK_TAILSCALE_IP}/$ip/g" "$kit/Caddyfile" >/etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile
systemctl daemon-reload
systemctl restart systemd-journald
systemctl enable --now sticktalk sticktalk-cert.timer
systemctl enable caddy
systemctl restart caddy
echo 'Installed. Configure /etc/sticktalk/backup.env before enabling the backup timer.'
