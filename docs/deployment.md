# Deploy And Update Lintex Webhook

The production Webhook runs on the Aliyun server as a systemd service:

```text
GitHub Release
    -> /usr/local/bin/lintex-webhook
    -> systemd: lintex-webhook.service
    -> 127.0.0.1:9000
    -> Nginx / HTTPS: deploy.dailin.tech
```

Service deployment configuration lives separately in the private
`dailin3/lintex-config` repository, cloned at `/opt/lintex-config`. Updating the
Webhook binary does not overwrite `/etc/lintex-webhook.env`, service `.env`
files, deployment history, or the config checkout.

## Release A Version

Ordinary pushes to `master` run checks and produce an Actions artifact. Only a
semantic `v*` tag publishes a GitHub Release:

```bash
git checkout master
git pull --ff-only
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
git tag -a v0.2.3 -m "Lintex Webhook v0.2.3"
git push origin v0.2.3
```

Wait for the `Build and Release` workflow to finish before updating the server.

## Update Production

Connect to the server, download the updater from the same version being
installed, and run it as root:

```bash
ssh aliyun
curl --fail --show-error --location \
  https://raw.githubusercontent.com/dailin3/lintex-webhook/v0.2.3/deploy/update-webhook.sh \
  --output /tmp/update-webhook.sh
sudo bash /tmp/update-webhook.sh v0.2.3
rm /tmp/update-webhook.sh
```

For future releases, replace `v0.2.3` in both places with the new tag. Once
this updater is present in a release, `latest` may be used as its argument:

```bash
sudo bash /tmp/update-webhook.sh latest
```

The updater:

1. downloads the static Linux binary and checksum from GitHub Releases;
2. verifies SHA-256 before installing anything;
3. installs the matching systemd and tmpfiles definitions from the tag;
4. restarts the service and checks `127.0.0.1:9000/health`;
5. restores the previous binary and unit if the health check fails.

## Verify Production

```bash
sudo systemctl --no-pager --full status lintex-webhook
curl --fail https://deploy.dailin.tech/health
```

Expected health response:

```json
{"status":"ok"}
```

Lifecycle errors are available in journald:

```bash
sudo journalctl -u lintex-webhook --since "15 minutes ago"
```

Deployment terminal output is stored separately under
`/var/lib/lintex-webhook/runs` and retrieved through the authenticated `/runs`
API.

## Update Service Configuration

Changes to Compose files or deployment scripts do not require a Webhook
release. Commit them to the private `lintex-config` repository. The Webhook
runs `git pull --ff-only` before every deployment:

```bash
curl --fail --show-error --request POST \
  --header "Authorization: Bearer $WEBHOOK_TOKEN" \
  https://deploy.dailin.tech/deploy/lintex-login
```

Real `.env` files remain server-only and must never be committed.
