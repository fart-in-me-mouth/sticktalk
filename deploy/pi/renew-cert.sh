#!/usr/bin/env bash
set -euo pipefail
umask 077
source /etc/sticktalk/tailscale.env
[[ $STICKTALK_HOST =~ ^[a-z0-9.-]+\.ts\.net$ ]] || exit 1
work=$(mktemp -d /run/sticktalk-cert.XXXXXX)
trap 'rm -f -- "$work/cert" "$work/key"; rmdir -- "$work"' EXIT
tailscale cert --cert-file "$work/cert" --key-file "$work/key" "$STICKTALK_HOST"
install -o root -g caddy -m 0640 "$work/cert" /etc/caddy/sticktalk.crt
install -o root -g caddy -m 0640 "$work/key" /etc/caddy/sticktalk.key
if systemctl is-active --quiet caddy; then systemctl reload caddy; fi
