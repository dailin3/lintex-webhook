#!/usr/bin/env bash
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
installer="$root/deploy/install.sh"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

help="$($installer --help)" || fail "--help must succeed without root"
grep -q -- '--config-repository-url' <<<"$help" \
  || fail "help must document the config repository option"
grep -q -- '--version' <<<"$help" \
  || fail "help must document the release selector"

if "$installer" --version continuous >"$tmpdir/non-root.log" 2>&1; then
  fail "installer must reject non-root execution"
fi
grep -q 'run this script as root' "$tmpdir/non-root.log" \
  || fail "non-root failure must explain how to run the installer"

grep -q 'deploy/install.sh' "$root/.github/workflows/release.yml" \
  || fail "the release bundle must include the installer"

echo "install script contract: ok"
