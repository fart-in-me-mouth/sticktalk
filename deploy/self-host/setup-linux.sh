#!/usr/bin/env bash
# StickTalk self-hosting on Linux or 64-bit Raspberry Pi OS: install, run as a
# systemd service, publish with Tailscale Funnel. Re-run to update. Run as your
# normal user (sudo is used for system steps). See deploy/SELF_HOST.md.
set -euo pipefail
usage() {
  cat <<'EOF'
Usage: deploy/self-host/setup-linux.sh [options]
  --binary PATH        Prebuilt sticktalk-server (e.g. from deploy/pi/build.ps1); default: build here
  --web DIR            Prebuilt web app (web-dist) to serve; default: none (desktop clients only)
  --port N             Local port (default 8080)
  --exposure MODE      funnel (public, default) | tailnet (your devices) | local
  --https-port N       443 | 8443 | 10000 (default: first one not used by another service)
  --public-url URL     Your own https origin instead of Tailscale (Caddy, Cloudflare Tunnel...)
  --voice MODE         tailnet (default) | public | local; calls need UDP, which Funnel can't carry
  --voice-advertised-ip IP   Public IPv4 for --voice public (forward UDP 50000-50100 here)
  --uninstall          Stop and disable the service and remove its Tailscale route (keeps data)
EOF
}
BINARY="" WEB="" PORT=8080 EXPOSURE=funnel HTTPS_PORT=0 PUBLIC_URL="" VOICE=tailnet VOICE_IP="" UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY=$2; shift 2 ;;
    --web) WEB=$2; shift 2 ;;
    --port) PORT=$2; shift 2 ;;
    --exposure) EXPOSURE=$2; shift 2 ;;
    --https-port) HTTPS_PORT=$2; shift 2 ;;
    --public-url) PUBLIC_URL=${2%/}; shift 2 ;;
    --voice) VOICE=$2; shift 2 ;;
    --voice-advertised-ip) VOICE_IP=$2; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
case "$EXPOSURE" in funnel|tailnet|local) ;; *) echo "--exposure must be funnel, tailnet or local" >&2; exit 2 ;; esac
case "$VOICE" in tailnet|public|local) ;; *) echo "--voice must be tailnet, public or local" >&2; exit 2 ;; esac
case "$HTTPS_PORT" in 0|443|8443|10000) ;; *) echo "--https-port must be 443, 8443 or 10000" >&2; exit 2 ;; esac
[[ "$PORT" =~ ^[0-9]+$ ]] || { echo "--port must be a number" >&2; exit 2; }
REPO=$(cd "$(dirname "$0")/../.." && pwd)
BACKEND="http://127.0.0.1:$PORT"
say() { printf '==> %s\n' "$*"; }

TS_HOST="" TS_IP=""
# Sets TS_HOST/TS_IP; returns non-zero when Tailscale is missing or disconnected.
tailscale_info() {
  command -v tailscale >/dev/null || { echo "Tailscale is not installed (https://tailscale.com/download/linux)." >&2; return 1; }
  read -r TS_HOST TS_IP < <(tailscale status --json | python3 -c '
import json, sys
s = json.load(sys.stdin)
if s.get("BackendState") != "Running": sys.exit("Tailscale is not connected; run: sudo tailscale up")
ips = [ip for ip in s["Self"].get("TailscaleIPs", []) if "." in ip]
print(s["Self"]["DNSName"].rstrip("."), ips[0] if ips else "-")') || true
  [ -n "$TS_HOST" ]
}
# The backend this machine's install last published (from its managed env file).
PREV_BIND=$(sudo sed -n 's/^STICKTALK_BIND=//p' /etc/sticktalk.env 2>/dev/null | tail -1 || true)
PREV_BACKEND=${PREV_BIND:+http://$PREV_BIND}
if [ "$UNINSTALL" = 0 ] && { [ "$EXPOSURE" != local ] || [ "$VOICE" = tailnet ]; }; then
  tailscale_info || exit 1
fi
# Prints the HTTPS port to use (ours if already configured, else the first free
# Funnel port), or "remove" lists for uninstall. Never picks a port in use.
serve_ports() {
  sudo tailscale serve status --json 2>/dev/null | python3 -c '
import json, sys
mode, host, backend, requested = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
raw = sys.stdin.read().strip()
cfg = json.loads(raw) if raw else {}
def ours(p):
    h = (cfg.get("Web") or {}).get(f"{host}:{p}", {}).get("Handlers", {})
    return list(h) == ["/"] and h["/"].get("Proxy") == backend
if mode == "remove":
    print(" ".join(str(p) for p in (443, 8443, 10000) if ours(p))); sys.exit()
used = set((cfg.get("TCP") or {}).keys())
cands = [requested] if requested else [443, 8443, 10000]
for p in cands:
    if ours(p): print(p); sys.exit()
for p in cands:
    if str(p) not in used: print(p); sys.exit()
sys.exit("Tailscale HTTPS ports %s are used by other services; free one or pass --public-url" % cands)' "$@"
}

remove_routes() { # backend
  for p in $(serve_ports remove "$TS_HOST" "$1" 0); do
    sudo tailscale funnel --https="$p" off >/dev/null 2>&1 || true
    sudo tailscale serve --https="$p" off >/dev/null 2>&1 || true
    say "Removed Tailscale HTTPS port $p"
  done
}
if [ "$UNINSTALL" = 1 ]; then
  sudo systemctl disable --now sticktalk 2>/dev/null || true
  say "Stopped and disabled the sticktalk service. Data kept in /var/lib/sticktalk."
  if tailscale_info; then remove_routes "${PREV_BACKEND:-$BACKEND}"
  else echo "Warning: Tailscale routes not changed (see 'tailscale serve status')." >&2; fi
  exit 0
fi
# Routes this install published that the new settings no longer use.
STALE=""
[ -n "$PREV_BACKEND" ] && [ "$PREV_BACKEND" != "$BACKEND" ] && STALE="$PREV_BACKEND"
{ [ -n "$PUBLIC_URL" ] || [ "$EXPOSURE" = local ]; } && STALE="$STALE $BACKEND"
if [ -n "${STALE// }" ]; then
  if [ -n "$TS_HOST" ] || tailscale_info 2>/dev/null; then
    for b in $STALE; do remove_routes "$b"; done
  fi
fi

TS_PORT=""
if [ -n "$PUBLIC_URL" ]; then
  [[ "$PUBLIC_URL" =~ ^https://[^/?#]+$ ]] || { echo "--public-url must be an https origin like https://chat.example.org" >&2; exit 2; }
elif [ "$EXPOSURE" = local ]; then
  PUBLIC_URL=$BACKEND
else
  TS_PORT=$(serve_ports select "$TS_HOST" "$BACKEND" "$HTTPS_PORT")
  if [ "$TS_PORT" = 443 ]; then PUBLIC_URL="https://$TS_HOST"; else PUBLIC_URL="https://$TS_HOST:$TS_PORT"; fi
fi
case "$VOICE" in
  tailnet) [ "$TS_IP" != "-" ] && [ -n "$TS_IP" ] || { echo "No Tailscale IPv4; use --voice local or public" >&2; exit 1; }
           VOICE_BIND=$TS_IP; VOICE_ADV=$TS_IP ;;
  public) [ -n "$VOICE_IP" ] || { echo "--voice public needs --voice-advertised-ip" >&2; exit 2; }
          VOICE_BIND=0.0.0.0; VOICE_ADV=$VOICE_IP ;;
  local) VOICE_BIND=127.0.0.1; VOICE_ADV=127.0.0.1 ;;
esac
say "Public address: $PUBLIC_URL"

if [ -z "$BINARY" ]; then
  command -v cargo >/dev/null || { echo "Install Rust (https://rustup.rs) or pass --binary (cross-build: deploy/pi/build.ps1)." >&2; exit 1; }
  say "Building the server (first build takes a while; on a Pi prefer --binary)"
  (cd "$REPO" && CARGO_INCREMENTAL=0 cargo build --locked --profile server -p sticktalk-server --bin sticktalk-server)
  BINARY="${CARGO_TARGET_DIR:-$REPO/target}/server/sticktalk-server"
fi
[ -x "$BINARY" ] || { echo "No executable server at $BINARY" >&2; exit 1; }

say "Installing the service"
id sticktalk >/dev/null 2>&1 || sudo useradd --system --home-dir /var/lib/sticktalk --shell /usr/sbin/nologin sticktalk
# New inode + rename: never overwrite a running executable.
sudo install -o root -g root -m 0755 "$BINARY" /usr/local/bin/sticktalk-server.new
sudo mv /usr/local/bin/sticktalk-server.new /usr/local/bin/sticktalk-server
WEB_LINE=""
if [ -n "$WEB" ]; then
  [ -f "$WEB/index.html" ] || { echo "--web must be a built web-dist directory" >&2; exit 1; }
  # Keep earlier hashed assets: open tabs may still request them.
  sudo mkdir -p /opt/sticktalk/web && sudo cp -R "$WEB/." /opt/sticktalk/web/
  WEB_LINE="STICKTALK_WEB_ROOT=/opt/sticktalk/web"
fi
sudo install -o root -g root -m 0644 "$REPO/deploy/sticktalk.service" /etc/systemd/system/sticktalk.service
# Update only the keys this script manages; keep any secrets you added yourself.
sudo touch /etc/sticktalk.env
sudo sed -i -E '/^(STICKTALK_BIND|STICKTALK_PUBLIC_URL|STICKTALK_WEB_ROOT|STICKTALK_VOICE_BIND_IP|STICKTALK_VOICE_ADVERTISED_IP)=/d' /etc/sticktalk.env
printf '%s\n' "STICKTALK_BIND=127.0.0.1:$PORT" "STICKTALK_PUBLIC_URL=$PUBLIC_URL" \
  "STICKTALK_VOICE_BIND_IP=$VOICE_BIND" "STICKTALK_VOICE_ADVERTISED_IP=$VOICE_ADV" $WEB_LINE | sudo tee -a /etc/sticktalk.env >/dev/null
sudo chown root:sticktalk /etc/sticktalk.env && sudo chmod 0640 /etc/sticktalk.env
sudo systemctl daemon-reload
sudo systemctl enable sticktalk >/dev/null
sudo systemctl restart sticktalk
for _ in $(seq 60); do curl -fsS "$BACKEND/api/v1/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "$BACKEND/api/v1/health" >/dev/null || { echo "Server did not start: sudo journalctl -u sticktalk -n 50" >&2; exit 1; }

if [ -n "$TS_PORT" ]; then
  VERB=funnel; [ "$EXPOSURE" = tailnet ] && VERB=serve
  say "Configuring Tailscale $VERB on HTTPS port $TS_PORT"
  sudo tailscale funnel --https="$TS_PORT" off >/dev/null 2>&1 || true
  sudo tailscale serve --https="$TS_PORT" off >/dev/null 2>&1 || true
  sudo tailscale "$VERB" --bg --https="$TS_PORT" "$BACKEND"
fi
echo
echo "StickTalk is self-hosted."
echo "  Web app / desktop client server address: $PUBLIC_URL"
echo "  MCP connector URL for AI assistants:     $PUBLIC_URL/mcp"
echo "  Logs: sudo journalctl -u sticktalk -f     Data: /var/lib/sticktalk"
if [ "$PUBLIC_URL" != "$BACKEND" ] && [ "$EXPOSURE" != tailnet ]; then echo "  Registration is open: share the address only with friends."; fi
echo "  Stop/remove: $0 --uninstall"
