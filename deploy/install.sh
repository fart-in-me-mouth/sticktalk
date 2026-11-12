#!/usr/bin/env bash
# Run as the ordinary Ubuntu login user, from the repository root.
# Does not create cloud resources, change firewall rules, or replace existing configs.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ $EUID == 0 ]]; then echo 'Run as your ordinary SSH user, not root (sudo is requested as needed).' >&2; exit 1; fi
if [[ $(uname -m) != aarch64 ]]; then echo 'Notice: target deployment is ARM64; building for this machine.' >&2; fi
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates curl git sqlite3 iptables-persistent
if ! command -v cargo >/dev/null && [[ ! -x $HOME/.cargo/bin/cargo ]]; then
  # No predictable shared-/tmp executable: another local user could replace it
  # or plant a symlink between download and execution.
  rustup_script=$(mktemp)
  trap 'rm -f -- "$rustup_script"' EXIT
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$rustup_script"
  sh "$rustup_script" -y --profile minimal --default-toolchain stable
  rm -f -- "$rustup_script"
  trap - EXIT
fi
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/.cache/sticktalk-server-target"
cargo build --locked --release -p sticktalk-server
if ! id sticktalk >/dev/null 2>&1; then sudo useradd --system --user-group --home-dir /var/lib/sticktalk --shell /usr/sbin/nologin sticktalk; fi
sudo install -d -o sticktalk -g sticktalk -m 0700 /var/lib/sticktalk
# Atomic executable replacement while the old server still runs.
sudo install -o root -g root -m 0755 "$CARGO_TARGET_DIR/release/sticktalk-server" /usr/local/bin/sticktalk-server.new
sudo mv -f /usr/local/bin/sticktalk-server.new /usr/local/bin/sticktalk-server
sudo install -o root -g root -m 0644 deploy/sticktalk.service /etc/systemd/system/sticktalk.service
if [[ ! -f /etc/sticktalk.env ]]; then
  printf '%s\n' 'STICKTALK_PUBLIC_URL=https://YOUR-NAME.duckdns.org' | sudo tee /etc/sticktalk.env >/dev/null
  sudo chmod 0600 /etc/sticktalk.env
fi
if ! command -v caddy >/dev/null; then
  sudo apt-get install -y debian-keyring debian-archive-keyring apt-transport-https gnupg
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | sudo gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt | sudo tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
  sudo chmod 0644 /usr/share/keyrings/caddy-stable-archive-keyring.gpg /etc/apt/sources.list.d/caddy-stable.list
  sudo apt-get update
  sudo apt-get install -y caddy
fi
sudo systemctl daemon-reload
sudo systemctl enable sticktalk
printf '\nInstalled. Existing server was NOT restarted. Next:\n'
printf '  1. Set your hostname in /etc/sticktalk.env and deploy/Caddyfile.\n'
printf '  2. Follow deploy/README.md for BOTH firewalls, DuckDNS, and HTTPS.\n'
printf '  3. sudo systemctl restart sticktalk\n'
