# Lintex Webhook

A small Rust webhook that deploys `lintex-login` after its Docker image is
pushed to Aliyun ACR.

## API

- `GET /health` returns `{"status":"ok"}`.
- `POST /deploy` requires `Authorization: Bearer <token>` and returns `202`.
- A second deployment while one is running returns `409`.

The `202` response means the deployment was accepted. Script failures happen
asynchronously and are reported in the service logs.

The webhook accepts no deployment arguments. It runs only the script configured
with `DEPLOY_SCRIPT`.

## Build and test

```bash
cargo build --release
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Download

Every push to `master` produces a Linux x86_64 binary in GitHub Actions
Artifacts. Version tags such as `v0.1.0` also publish the archive and SHA-256
checksum on the [Releases page](https://github.com/dailin3/lintex-webhook/releases).

The release binary is statically linked with musl, so Rust is not required on
the server.

```bash
tar -xzf lintex-webhook-linux-x86_64.tar.gz
sudo install -m 755 lintex-webhook /usr/local/bin/lintex-webhook
```

## Server installation

For the complete current deployment procedure, including Docker, systemd,
Nginx, HTTPS, and GitHub Actions secrets, follow
[`docs/deploy-v0.1.0.md`](docs/deploy-v0.1.0.md).

1. Create a `lintex-deploy` system user and add it to the Docker group.
2. Install `target/release/lintex-webhook` at `/usr/local/bin/lintex-webhook`.
3. Copy `deploy/deploy-login.sh.example` to `/opt/lintex-login/deploy.sh`, make
   it executable, and put the Login Compose file in the same directory.
4. Create `/opt/lintex-login/.env.production` with runtime secrets.
5. Create `/etc/lintex-webhook.env` from `.env.example`, set a random token,
   and restrict it with `chmod 600`.
6. Install `deploy/lintex-webhook.service` under `/etc/systemd/system/`, then
   run `systemctl daemon-reload && systemctl enable --now lintex-webhook`.

Keep the service on `127.0.0.1:9000`. Expose it through an HTTPS reverse proxy
and limit access where practical.

Generate a token with:

```bash
openssl rand -hex 32
```

## GitHub Actions

Add `DEPLOY_WEBHOOK_URL` and `DEPLOY_WEBHOOK_TOKEN` as repository Actions
secrets. Call the webhook only after the image push succeeds:

```bash
curl --fail --show-error --max-time 15 \
  -X POST \
  -H "Authorization: Bearer $DEPLOY_WEBHOOK_TOKEN" \
  "$DEPLOY_WEBHOOK_URL"
```

## Logs

The service emits JSON logs with request IDs, deployment duration, script
stdout/stderr, and exit status. Tokens are never logged.

```bash
journalctl -u lintex-webhook -f
```

## Manual rollback

Change the image tag in the server Compose file from `latest` to a known commit
SHA tag, then run:

```bash
cd /opt/lintex-login
docker compose pull
docker compose up -d
```

## License

MIT
