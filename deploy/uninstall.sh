#!/usr/bin/env bash
set -Eeuo pipefail

purge=false

usage() {
  cat <<'EOF'
Uninstall Lintex Webhook from a systemd-based Linux server.

Usage:
  sudo ./uninstall.sh [--purge]

Options:
  --purge     Also delete /etc/lintex-webhook.env, deployment run logs, and
              the lintex-deploy system user.
  -h, --help  Show this help.

The private /opt/lintex-config checkout, service .env files, Docker containers,
images, volumes, and databases are always preserved.
EOF
}

while (($#)); do
  case "$1" in
    --purge)
      purge=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${EUID}" -ne 0 ]]; then
  echo "ERROR: run this script as root (for example: sudo ./uninstall.sh)" >&2
  exit 1
fi

systemctl disable --now lintex-webhook.service 2>/dev/null || true
systemctl stop lintex-webhook-update.service 2>/dev/null || true

files=(
  /usr/local/bin/lintex-webhook
  /usr/local/sbin/lintex-webhook-uninstall
  /usr/local/libexec/lintex-webhook-update
  /usr/local/libexec/lintex-webhook-start-update
  /etc/systemd/system/lintex-webhook.service
  /usr/lib/tmpfiles.d/lintex-webhook.conf
  /etc/sudoers.d/lintex-webhook
)
for file in "${files[@]}"; do
  [[ ! -e "$file" ]] || unlink "$file"
done
systemctl daemon-reload
systemctl reset-failed lintex-webhook.service 2>/dev/null || true

if [[ "$purge" == true ]]; then
  [[ ! -e /etc/lintex-webhook.env ]] || unlink /etc/lintex-webhook.env
  if [[ -d /var/lib/lintex-webhook ]]; then
    find /var/lib/lintex-webhook -depth -delete
  fi
  if id lintex-deploy >/dev/null 2>&1; then
    userdel lintex-deploy
  fi
fi

echo "Lintex Webhook has been uninstalled"
if [[ "$purge" == false ]]; then
  echo "Preserved /etc/lintex-webhook.env and /var/lib/lintex-webhook"
fi
echo "Preserved /opt/lintex-config and all Docker service data"
