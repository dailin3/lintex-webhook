#!/usr/bin/env bash
set -Eeuo pipefail

repository="dailin3/lintex-webhook"
version="continuous"
config_repository_url=""
config_directory="/opt/lintex-config"
bundle_directory=""

usage() {
  cat <<'EOF'
Install Lintex Webhook on a systemd-based Linux server.

Usage:
  sudo ./install.sh [options]

Options:
  --version VERSION                 continuous (default), latest, or vX.Y.Z
  --config-repository-url URL       Clone the private lintex-config repository
  --config-directory PATH           Config checkout path (default: /opt/lintex-config)
  --bundle-directory PATH           Install from an already extracted release bundle
  -h, --help                        Show this help

The installer creates /etc/lintex-webhook.env with a random token when that
file does not already exist. Existing tokens, config checkouts, run logs, and
service secrets are preserved when the installer is run again.
EOF
}

while (($#)); do
  case "$1" in
    --version)
      version="${2:?missing value for --version}"
      shift 2
      ;;
    --config-repository-url)
      config_repository_url="${2:?missing value for --config-repository-url}"
      shift 2
      ;;
    --config-directory)
      config_directory="${2:?missing value for --config-directory}"
      shift 2
      ;;
    --bundle-directory)
      bundle_directory="${2:?missing value for --bundle-directory}"
      shift 2
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
  echo "ERROR: run this script as root (for example: sudo ./install.sh)" >&2
  exit 1
fi

for command in curl tar sha256sum systemctl install openssl git visudo sudo; do
  command -v "$command" >/dev/null || {
    echo "ERROR: required command not found: $command" >&2
    exit 1
  }
done
getent group docker >/dev/null || {
  echo "ERROR: Docker must be installed before Lintex Webhook" >&2
  exit 1
}

workdir="$(mktemp -d)"
cleanup() { rm -rf "$workdir"; }
trap cleanup EXIT

if [[ -n "$bundle_directory" ]]; then
  source_directory="$(cd "$bundle_directory" && pwd)"
else
  if [[ "$version" == "latest" ]]; then
    release_url="$(curl --fail --silent --show-error --location \
      --output /dev/null --write-out '%{url_effective}' \
      "https://github.com/${repository}/releases/latest")"
    version="${release_url##*/}"
  fi
  if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && "$version" != "continuous" ]]; then
    echo "ERROR: expected continuous, latest, or vX.Y.Z" >&2
    exit 1
  fi
  release="https://github.com/${repository}/releases/download/${version}"
  curl --fail --silent --show-error --location \
    --output "$workdir/bundle.tar.gz" \
    "$release/lintex-webhook-bundle-linux-x86_64.tar.gz"
  curl --fail --silent --show-error --location \
    --output "$workdir/bundle.tar.gz.sha256" \
    "$release/lintex-webhook-bundle-linux-x86_64.tar.gz.sha256"
  sed -i 's/lintex-webhook-bundle-linux-x86_64.tar.gz/bundle.tar.gz/' \
    "$workdir/bundle.tar.gz.sha256"
  (cd "$workdir" && sha256sum --check bundle.tar.gz.sha256 && tar -xzf bundle.tar.gz)
  source_directory="$workdir"
fi

for file in lintex-webhook lintex-webhook.service lintex-webhook.sudoers \
  lintex-webhook.tmpfiles install.sh uninstall.sh start-update.sh \
  update-webhook.sh; do
  [[ -f "$source_directory/$file" ]] || {
    echo "ERROR: bundle is missing $file" >&2
    exit 1
  }
done
visudo --check --file="$source_directory/lintex-webhook.sudoers"

if ! id lintex-deploy >/dev/null 2>&1; then
  useradd --system --create-home --home-dir /var/lib/lintex-deploy \
    --shell /usr/sbin/nologin --groups docker lintex-deploy
else
  usermod --append --groups docker lintex-deploy
fi

install -d -o root -g root -m 755 /usr/local/libexec
install -d -o lintex-deploy -g docker -m 750 /var/lib/lintex-webhook
install -d -o lintex-deploy -g docker -m 750 /var/lib/lintex-webhook/runs
install -d -o lintex-deploy -g docker -m 750 "$config_directory"

if [[ -n "$config_repository_url" && ! -d "$config_directory/.git" ]]; then
  rmdir "$config_directory"
  sudo -u lintex-deploy git clone "$config_repository_url" "$config_directory"
fi

environment_file=/etc/lintex-webhook.env
if [[ ! -f "$environment_file" ]]; then
  token="$(openssl rand -hex 32)"
  umask 077
  cat >"$environment_file" <<EOF
WEBHOOK_TOKEN=$token
CONFIG_REPOSITORY=$config_directory
LISTEN_ADDR=127.0.0.1:9000
EOF
  unset token
fi

install -o root -g root -m 755 "$source_directory/lintex-webhook" /usr/local/bin/lintex-webhook
install -o root -g root -m 755 "$source_directory/uninstall.sh" /usr/local/sbin/lintex-webhook-uninstall
install -o root -g root -m 644 "$source_directory/lintex-webhook.service" /etc/systemd/system/lintex-webhook.service
install -o root -g root -m 644 "$source_directory/lintex-webhook.tmpfiles" /usr/lib/tmpfiles.d/lintex-webhook.conf
install -o root -g root -m 755 "$source_directory/update-webhook.sh" /usr/local/libexec/lintex-webhook-update
install -o root -g root -m 755 "$source_directory/start-update.sh" /usr/local/libexec/lintex-webhook-start-update
install -o root -g root -m 440 "$source_directory/lintex-webhook.sudoers" /etc/sudoers.d/lintex-webhook

systemd-tmpfiles --create /usr/lib/tmpfiles.d/lintex-webhook.conf
systemctl daemon-reload
systemctl enable --now lintex-webhook
curl --fail --silent --show-error --retry 10 --retry-delay 1 \
  --retry-connrefused http://127.0.0.1:9000/health >/dev/null

echo "Lintex Webhook is installed and healthy"
echo "Environment: $environment_file"
echo "Configuration repository: $config_directory"
