#!/usr/bin/env bash
set -Eeuo pipefail

repository="dailin3/lintex-webhook"
version="${1:-latest}"
workdir="$(mktemp -d)"
binary=/usr/local/bin/lintex-webhook
unit=/etc/systemd/system/lintex-webhook.service
tmpfiles=/usr/lib/tmpfiles.d/lintex-webhook.conf

cleanup() {
  rm -rf "$workdir"
}
trap cleanup EXIT

if [[ "${EUID}" -ne 0 ]]; then
  echo "ERROR: run this script with sudo" >&2
  exit 1
fi

if [[ "$version" == "latest" ]]; then
  release_url="$(curl --fail --silent --show-error --location \
    --output /dev/null --write-out '%{url_effective}' \
    "https://github.com/${repository}/releases/latest")"
  version="${release_url##*/}"
fi

if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "ERROR: expected a release version such as v0.2.1" >&2
  exit 1
fi

release="https://github.com/${repository}/releases/download/${version}"
source="https://raw.githubusercontent.com/${repository}/${version}/deploy"

echo "Updating Lintex Webhook to ${version}"
curl --fail --silent --show-error --location \
  --output "$workdir/webhook.tar.gz" \
  "$release/lintex-webhook-linux-x86_64.tar.gz"
curl --fail --silent --show-error --location \
  --output "$workdir/webhook.tar.gz.sha256" \
  "$release/lintex-webhook-linux-x86_64.tar.gz.sha256"

sed -i 's/lintex-webhook-linux-x86_64.tar.gz/webhook.tar.gz/' \
  "$workdir/webhook.tar.gz.sha256"
(
  cd "$workdir"
  sha256sum --check webhook.tar.gz.sha256
  tar -xzf webhook.tar.gz
)

curl --fail --silent --show-error --location \
  --output "$workdir/lintex-webhook.service" \
  "$source/lintex-webhook.service"
curl --fail --silent --show-error --location \
  --output "$workdir/lintex-webhook.tmpfiles" \
  "$source/lintex-webhook.tmpfiles"

cp --preserve=mode,ownership "$binary" "$workdir/lintex-webhook.previous"
cp --preserve=mode,ownership "$unit" "$workdir/lintex-webhook.service.previous"

install -o root -g root -m 755 "$workdir/lintex-webhook" "$binary"
install -o root -g root -m 644 "$workdir/lintex-webhook.service" "$unit"
install -o root -g root -m 644 "$workdir/lintex-webhook.tmpfiles" "$tmpfiles"
systemd-tmpfiles --create "$tmpfiles"
systemctl daemon-reload

if systemctl restart lintex-webhook \
  && curl --fail --silent --show-error --retry 10 --retry-delay 1 \
    --retry-connrefused \
    http://127.0.0.1:9000/health >/dev/null; then
  echo "Lintex Webhook ${version} is healthy"
  exit 0
fi

echo "ERROR: health check failed; restoring the previous version" >&2
install -o root -g root -m 755 "$workdir/lintex-webhook.previous" "$binary"
install -o root -g root -m 644 \
  "$workdir/lintex-webhook.service.previous" "$unit"
systemctl daemon-reload
systemctl restart lintex-webhook
exit 1
