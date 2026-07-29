#!/usr/bin/env bash
set -Eeuo pipefail

repository="dailin3/lintex-webhook"
version="${1:-continuous}"
workdir="$(mktemp -d)"
binary=/usr/local/bin/lintex-webhook
unit=/etc/systemd/system/lintex-webhook.service
tmpfiles=/usr/lib/tmpfiles.d/lintex-webhook.conf
updater=/usr/local/libexec/lintex-webhook-update
launcher=/usr/local/libexec/lintex-webhook-start-update
sudoers=/etc/sudoers.d/lintex-webhook

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

if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && "$version" != "continuous" ]]; then
  echo "ERROR: expected continuous or a release version such as v0.2.4" >&2
  exit 1
fi

release="https://github.com/${repository}/releases/download/${version}"
echo "Updating Lintex Webhook to ${version}"
curl --fail --silent --show-error --location \
  --output "$workdir/webhook.tar.gz" \
  "$release/lintex-webhook-bundle-linux-x86_64.tar.gz"
curl --fail --silent --show-error --location \
  --output "$workdir/webhook.tar.gz.sha256" \
  "$release/lintex-webhook-bundle-linux-x86_64.tar.gz.sha256"

sed -i 's/lintex-webhook-bundle-linux-x86_64.tar.gz/webhook.tar.gz/' \
  "$workdir/webhook.tar.gz.sha256"
(
  cd "$workdir"
  sha256sum --check webhook.tar.gz.sha256
  tar -xzf webhook.tar.gz
)

cp --preserve=mode,ownership "$binary" "$workdir/lintex-webhook.previous"
cp --preserve=mode,ownership "$unit" "$workdir/lintex-webhook.service.previous"
for file in "$tmpfiles" "$updater" "$launcher" "$sudoers"; do
  if [[ -f "$file" ]]; then
    cp --preserve=mode,ownership "$file" "$workdir/$(basename "$file").previous"
  fi
done

install -d -o root -g root -m 755 /usr/local/libexec
install -o root -g root -m 755 "$workdir/lintex-webhook" "$binary"
install -o root -g root -m 644 "$workdir/lintex-webhook.service" "$unit"
install -o root -g root -m 644 "$workdir/lintex-webhook.tmpfiles" "$tmpfiles"
install -o root -g root -m 755 "$workdir/update-webhook.sh" "$updater"
install -o root -g root -m 755 "$workdir/start-update.sh" "$launcher"
visudo --check --file="$workdir/lintex-webhook.sudoers"
install -o root -g root -m 440 "$workdir/lintex-webhook.sudoers" "$sudoers"
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
for file in "$tmpfiles" "$updater" "$launcher" "$sudoers"; do
  previous="$workdir/$(basename "$file").previous"
  if [[ -f "$previous" ]]; then
    cp --preserve=mode,ownership "$previous" "$file"
  else
    rm -f "$file"
  fi
done
systemctl daemon-reload
systemctl restart lintex-webhook
exit 1
