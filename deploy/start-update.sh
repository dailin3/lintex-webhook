#!/usr/bin/env bash
set -Eeuo pipefail

exec systemd-run \
  --unit=lintex-webhook-update \
  --collect \
  --property=Type=exec \
  /usr/local/libexec/lintex-webhook-update continuous
