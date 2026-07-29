# Lintex Webhook

Lintex Webhook is a small, allowlist-based deployment service. It pulls the
private `lintex-config` repository, runs one configured deployment script, and
stores CI-style terminal output for remote inspection.

```text
service CI -> POST /deploy/:service -> lintex-webhook
                                      -> git pull lintex-config
                                      -> run the allowlisted deploy.sh
                                      -> store logs and exit status

webhook master -> GitHub Actions -> continuous release -> POST /update
                                                    -> update the complete
                                                       Webhook runtime bundle
```

The Webhook repository owns only its own binary, systemd unit, sudoers rule,
tmpfiles rule, installer, and updater. Compose files, service deployment
scripts, and service-specific configuration belong in the private
`lintex-config` repository.

## API

`GET /health` is public. Every other endpoint requires:

```http
Authorization: Bearer <WEBHOOK_TOKEN>
```

| Endpoint | Purpose |
| --- | --- |
| `POST /deploy/:service` | Start an allowlisted service deployment |
| `POST /update` | Start an asynchronous Webhook self-update |
| `GET /runs` | List recent deployment runs |
| `GET /runs/:id` | Read run metadata and exit status |
| `GET /runs/:id/log` | Read the complete terminal log |
| `GET /runs/:id/stream` | Stream `log` and `done` SSE events |

Only one service deployment runs at a time. Run data is retained for 30 days,
up to 500 runs. Tokens, environment contents, and registry credentials are not
written to deployment logs.

## Install On A New Server

The server must be x86_64 Linux with systemd, Docker, Git, curl, OpenSSL,
`sha256sum`, `tar`, `sudo`, and `visudo`. The installer checks these
prerequisites, but it does not install them. Install Docker from Docker's
official repository before running the installer; the Docker daemon must be
running and a `docker` group must exist.

For example, on Debian or Ubuntu, install the non-Docker tools with:

```bash
sudo apt update
sudo apt install -y git curl openssl coreutils tar sudo
```

Nginx (or another reverse proxy), DNS, and TLS certificates are also separate
server prerequisites. The installer does not configure public networking.

Download the `continuous` release bundle on a trusted workstation, verify it,
and copy it to the server:

```bash
mkdir -p lintex-webhook-install
gh release download continuous \
  --repo dailin3/lintex-webhook \
  --dir lintex-webhook-install
cd lintex-webhook-install
sha256sum --check lintex-webhook-bundle-linux-x86_64.tar.gz.sha256
tar -xzf lintex-webhook-bundle-linux-x86_64.tar.gz
scp -r . your-server:~/lintex-webhook-install/
```

Run the idempotent installer on the server:

```bash
ssh your-server
sudo ~/lintex-webhook-install/install.sh \
  --bundle-directory ~/lintex-webhook-install
```

The installer:

1. creates or updates the `lintex-deploy` system user;
2. preserves an existing `/etc/lintex-webhook.env` and otherwise generates a
   random 256-bit Webhook token;
3. installs the binary, systemd unit, tmpfiles rule, sudoers rule, updater, and
   update launcher;
4. enables and starts `lintex-webhook.service`;
5. verifies `http://127.0.0.1:9000/health`.

It is safe to run the installer again. It does not overwrite an existing
Webhook token, config checkout, deployment logs, or service secrets.

By default, this installs only the Webhook runtime. It creates
`/opt/lintex-config` as an empty directory, but it does **not** clone the
private `lintex-config` repository because GitHub authentication must be
configured first.

The installer can also download a public release directly when the server has
reliable GitHub access:

```bash
sudo ./install.sh --version continuous
```

Use `--version latest` for the newest formal release or `--version vX.Y.Z` for
a pinned release.

### Install The Private Config Repository

`CONFIG_REPOSITORY` is the contract between the Webhook and the private Config
repository. Its default and recommended value is `/opt/lintex-config`, as set
in `/etc/lintex-webhook.env`. Keep the checkout there unless there is a clear
server-level reason to move it. If it is moved, update `CONFIG_REPOSITORY` and
restart `lintex-webhook.service`; changing only the clone location will break
deployments.

Add a read-only GitHub Deploy Key for `dailin3/lintex-config` to the new server,
then either let the installer clone it:

```bash
sudo ./install.sh \
  --bundle-directory ~/lintex-webhook-install \
  --config-repository-url git@github.com:dailin3/lintex-config.git
```

Or clone it manually as the deployment user after installing the Webhook:

```bash
sudo -u lintex-deploy git clone \
  git@github.com:dailin3/lintex-config.git \
  /opt/lintex-config
```

The expected service registry is `/opt/lintex-config/services.toml`:

```toml
[services.lintex-login]
display_name = "Lintex Login"
working_directory = "/opt/lintex-config/lintex-login"
deploy_script = "/opt/lintex-config/lintex-login/deploy.sh"
```

Both paths must be absolute and remain inside `CONFIG_REPOSITORY`. Before each
deployment, the Webhook runs `git pull --ff-only` in that repository. It only
executes the selected service's `deploy.sh` after the Config pull succeeds.

Real service `.env` files stay on the server and must not be committed. Create
them from the corresponding `.env.example` files in `lintex-config` and set
permissions appropriate for `lintex-deploy` and Docker.

### Add HTTPS

Reverse proxy `https://deploy.dailin.tech` to `127.0.0.1:9000`. Never expose
port 9000 directly to the Internet. Configure TLS, then verify:

```bash
curl --fail https://deploy.dailin.tech/health
```

Configure the Webhook repository's GitHub Actions settings:

```text
Repository variable:
DEPLOY_WEBHOOK_URL=https://deploy.dailin.tech

Repository secret:
DEPLOY_WEBHOOK_TOKEN=<WEBHOOK_TOKEN from /etc/lintex-webhook.env>
```

Do not print the token in terminal logs or put it in Git.

## Move To Another Server

A server migration has four kinds of state. Move or recreate all four:

| State | Location | Migration action |
| --- | --- | --- |
| Webhook runtime | `/usr/local`, systemd, sudoers | Recreate with `install.sh` |
| Webhook identity | `/etc/lintex-webhook.env` | Securely copy it or generate a new token |
| Deployment configuration | `/opt/lintex-config` | Clone again with a new read-only Deploy Key |
| Service secrets/data | ignored `.env`, volumes, databases | Back up and restore per service |

Recommended migration order:

1. Install Docker and the Webhook on the new server.
2. Clone `lintex-config` and restore every server-only service `.env`.
3. Restore persistent Docker volumes and databases. The Webhook does not back
   these up.
4. Install Nginx or another reverse proxy and obtain TLS certificates.
5. Deploy each service on the new server and verify it through the server IP or
   a temporary hostname.
6. Lower DNS TTL, move `deploy.dailin.tech`, `login.dailin.tech`, and other
   service records to the new server, then verify HTTPS.
7. If the Webhook token changed, update `DEPLOY_WEBHOOK_TOKEN` in every caller
   and in the Webhook repository's self-update workflow.
8. Keep the old server available until OAuth callbacks, service deployment,
   logs, and persistent data have all been verified.

To preserve deployment history, copy `/var/lib/lintex-webhook/runs`. It is
optional; losing it does not prevent future deployments.

For Login specifically, remember that Nginx must allow Supabase's session
cookies in callback responses:

```nginx
proxy_buffer_size 16k;
proxy_buffers 8 16k;
proxy_busy_buffers_size 32k;
```

Without these buffers, a successful OAuth callback can appear as `502 Bad
Gateway` with `upstream sent too big header` in the Nginx error log.

## Daily Use

### Deploy A Service

Service repositories build their own images. Their CI calls the Webhook after
publishing an image:

```bash
curl --fail --show-error --request POST \
  --header "Authorization: Bearer $WEBHOOK_TOKEN" \
  https://deploy.dailin.tech/deploy/lintex-login
```

The Webhook pulls `lintex-config`, resolves the allowlisted service, executes
its `deploy.sh`, and returns a `request_id`.

### Read Deployment Logs

```bash
curl --fail \
  --header "Authorization: Bearer $WEBHOOK_TOKEN" \
  https://deploy.dailin.tech/runs

curl --fail \
  --header "Authorization: Bearer $WEBHOOK_TOKEN" \
  https://deploy.dailin.tech/runs/$RUN_ID/log
```

Webhook process and self-update logs remain in systemd:

```bash
sudo journalctl -u lintex-webhook --since "30 minutes ago"
sudo journalctl -u lintex-webhook-update --since "30 minutes ago"
```

### Change Service Deployment Configuration

Edit the private `lintex-config` repository. Commit Compose files, deployment
scripts, and non-secret configuration there. The next `/deploy/:service` call
pulls the change automatically. Keep real `.env` files server-only.

### Update The Webhook

An ordinary push to `master` runs tests, builds a static Linux binary,
publishes the complete `continuous` bundle, and calls production `/update`.
The server installs the entire bundle, including systemd, sudoers, tmpfiles,
and operational scripts. It health-checks the new process and restores the
previous runtime files if the check fails.

Formal `vX.Y.Z` tags create historical GitHub Releases but are not required for
normal deployment.

Verify the automatic update with:

```bash
sudo systemctl status lintex-webhook --no-pager
sudo journalctl -u lintex-webhook-update --since "15 minutes ago"
curl --fail https://deploy.dailin.tech/health
```

The `/update` endpoint is only for changes to the `lintex-webhook` repository.
Do not call it after changing Login or another business service.

### Choose The Correct Trigger

There are two independent CI triggers:

| Repository changed | CI action after a successful build | Server action |
| --- | --- | --- |
| `lintex-webhook` | `POST /update` | Replace and restart the complete Webhook runtime bundle |
| A business service such as `lintex-login` | `POST /deploy/lintex-login` | Pull `lintex-config` and execute that service's `deploy.sh` |

A typical service deployment script in private `lintex-config` runs:

```bash
docker compose pull
docker compose up -d --remove-orphans
```

The service CI must call `/deploy/:service` only after its new image has been
successfully pushed. The Webhook itself does not build images and does not need
to understand the service repository.

## Uninstall

The installer places the uninstaller on the server. Run:

```bash
sudo lintex-webhook-uninstall
```

The default uninstall removes only the Webhook runtime:

- binary and self-update scripts;
- systemd unit;
- sudoers and tmpfiles rules.

It preserves `/etc/lintex-webhook.env`, `/var/lib/lintex-webhook`, the complete
`/opt/lintex-config` checkout, service `.env` files, Docker containers, images,
volumes, and databases. This makes reinstalling or moving the Webhook safer.

To also remove the Webhook token, run history, and `lintex-deploy` system user:

```bash
sudo lintex-webhook-uninstall --purge
```

Even `--purge` never deletes `lintex-config` or Docker service data. Those
belong to the deployed services and require separate, explicit removal.

## Local Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
tests/install.sh
cargo build --release
```

## License

MIT
