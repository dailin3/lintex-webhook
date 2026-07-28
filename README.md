# Lintex Webhook

A small, allowlist-based Rust deployment webhook. It updates a private
configuration repository, runs one configured deployment script, and stores
CI-style terminal output for remote inspection.

## API

`GET /health` is public. Every other endpoint requires
`Authorization: Bearer <token>`.

- `POST /deploy/:service` accepts a deployment and returns its `request_id`.
- `GET /runs` lists recent runs.
- `GET /runs/:id` returns metadata and exit status.
- `GET /runs/:id/log` returns the complete terminal log.
- `GET /runs/:id/stream` streams `log` and `done` SSE events.

Only one deployment runs at a time. Run data is retained for 30 days, up to
500 runs. Tokens, environment contents, and registry credentials are never
written to deployment logs.

## Configuration

Create `/etc/lintex-webhook.env` from `.env.example`. Services are allowlisted
in `/opt/lintex-config/services.toml`:

```toml
[services.lintex-login]
display_name = "Lintex Login"
working_directory = "/opt/lintex-config/lintex-login"
deploy_script = "/opt/lintex-config/lintex-login/deploy.sh"
```

Both paths must be absolute and remain inside `CONFIG_REPOSITORY`. Before each
deployment the webhook runs `git pull --ff-only` in that repository.

## Build and release

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Pushes to `master` produce an Actions artifact. Only `v*` tags create a GitHub
Release with a static Linux x86_64 binary and SHA-256 checksum.

## Server installation

1. Create the `lintex-deploy` system user and add it to the Docker group.
2. Clone the private config repository at `/opt/lintex-config` with a read-only
   GitHub Deploy Key.
3. Put real service secrets in ignored `.env` files beside each Compose file.
4. Install the binary at `/usr/local/bin/lintex-webhook`.
5. Install `deploy/lintex-webhook.service` and
   `deploy/lintex-webhook.tmpfiles`, then enable the service.
6. Reverse proxy `127.0.0.1:9000` through HTTPS; never expose it directly.

Example log request:

```bash
curl -H "Authorization: Bearer $WEBHOOK_TOKEN" \
  https://deploy.example.com/runs/$RUN_ID/log
```

## License

MIT
