# Deploy Lintex Webhook v0.1.0

This guide installs the current single-service webhook on an Ubuntu or Debian
x86_64 server. It deploys `lintex-login` from Aliyun ACR and exposes the
webhook through Nginx at `https://deploy.dailin.tech`.

The current flow is:

```mermaid
flowchart LR
    A[Push to GitHub master] --> B[GitHub Actions]
    B --> C[Build and push image to ACR]
    C --> D[POST deploy.dailin.tech/deploy]
    D --> E[lintex-webhook]
    E --> F[deploy.sh]
    F --> G[docker compose pull]
    G --> H[docker compose up -d]
    H --> I[Health check]
```

Do not configure an Aliyun ACR image push trigger for this version. GitHub
Actions calls the webhook after the image push succeeds, so an ACR trigger
would start the same deployment twice.

## 1. Check Docker

Docker Engine and Docker Compose must already be installed:

```bash
docker --version
docker compose version
```

Install them before continuing if either command is unavailable.

## 2. Create the deployment user

Create a service account and give it access to Docker:

```bash
sudo useradd \
  --system \
  --no-create-home \
  --shell /usr/sbin/nologin \
  lintex-deploy

sudo usermod -aG docker lintex-deploy
```

Membership in the Docker group is effectively privileged access to the host.
Keep the webhook token private and expose the service only through HTTPS.

## 3. Download and install the webhook

Download the release and checksum:

```bash
cd /tmp

wget https://github.com/dailin3/lintex-webhook/releases/download/v0.1.0/lintex-webhook-linux-x86_64.tar.gz
wget https://github.com/dailin3/lintex-webhook/releases/download/v0.1.0/lintex-webhook-linux-x86_64.tar.gz.sha256

sha256sum --check lintex-webhook-linux-x86_64.tar.gz.sha256
```

The checksum command must report:

```text
lintex-webhook-linux-x86_64.tar.gz: OK
```

Install the binary:

```bash
tar -xzf lintex-webhook-linux-x86_64.tar.gz

sudo install \
  -o root \
  -g root \
  -m 755 \
  lintex-webhook \
  /usr/local/bin/lintex-webhook
```

Running `/usr/local/bin/lintex-webhook` without configuration should report
that `WEBHOOK_TOKEN` is required. That confirms the binary can run.

## 4. Create the Login deployment files

Create the current deployment directory:

```bash
sudo mkdir -p /opt/lintex-login
sudo chown -R lintex-deploy:docker /opt/lintex-login
```

Create `/opt/lintex-login/compose.yaml`:

```yaml
services:
  login:
    image: crpi-5k1h8laml9223z4c.cn-beijing.personal.cr.aliyuncs.com/lintex/login:latest
    restart: unless-stopped
    env_file:
      - .env.production
    ports:
      - "127.0.0.1:3000:3000"
```

Create `/opt/lintex-login/.env.production` with the real runtime secret:

```env
TURNSTILE_SECRET=replace-with-the-real-turnstile-secret
```

Protect it:

```bash
sudo chown lintex-deploy:docker /opt/lintex-login/.env.production
sudo chmod 600 /opt/lintex-login/.env.production
```

If anonymous pulls from the ACR repository are disabled, authenticate once as
the deployment user:

```bash
sudo -u lintex-deploy docker login \
  --username=dailin1104 \
  crpi-5k1h8laml9223z4c.cn-beijing.personal.cr.aliyuncs.com
```

Enter the ACR access credential password at the prompt. Do not put it in the
Compose file or deployment script.

## 5. Create and test the deployment script

Create `/opt/lintex-login/deploy.sh`:

```bash
#!/usr/bin/env bash
set -Eeuo pipefail

cd /opt/lintex-login

echo '$ docker compose pull'
docker compose pull

echo '$ docker compose up -d --remove-orphans'
docker compose up -d --remove-orphans

echo '$ curl --fail http://127.0.0.1:3000/auth/login'
curl --fail --silent --show-error \
  --retry 10 \
  --retry-delay 2 \
  http://127.0.0.1:3000/auth/login \
  >/dev/null

echo 'Deployment completed'
```

Set ownership and permissions, then run it once manually:

```bash
sudo chown lintex-deploy:docker /opt/lintex-login/deploy.sh
sudo chmod 750 /opt/lintex-login/deploy.sh
sudo -u lintex-deploy /opt/lintex-login/deploy.sh
```

Verify the container and application:

```bash
docker compose -f /opt/lintex-login/compose.yaml ps
curl -I http://127.0.0.1:3000/auth/login
```

Do not continue until the manual deployment succeeds.

## 6. Configure the webhook

Generate a random bearer token:

```bash
openssl rand -hex 32
```

Create `/etc/lintex-webhook.env`, using the generated value:

```env
WEBHOOK_TOKEN=replace-with-the-generated-token
DEPLOY_SCRIPT=/opt/lintex-login/deploy.sh
LISTEN_ADDR=127.0.0.1:9000
RUST_LOG=lintex_webhook=info,tower_http=info
```

Protect the configuration:

```bash
sudo chown root:root /etc/lintex-webhook.env
sudo chmod 600 /etc/lintex-webhook.env
```

Keep the token available for step 10. Never commit it to Git.

## 7. Install the systemd service

Create `/etc/systemd/system/lintex-webhook.service`:

```ini
[Unit]
Description=Lintex Login deployment webhook
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
User=lintex-deploy
Group=docker
EnvironmentFile=/etc/lintex-webhook.env
ExecStart=/usr/local/bin/lintex-webhook
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=/opt/lintex-login

[Install]
WantedBy=multi-user.target
```

Enable and start it:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now lintex-webhook
sudo systemctl status lintex-webhook
```

Check the local endpoint:

```bash
curl http://127.0.0.1:9000/health
```

Expected response:

```json
{"status":"ok"}
```

For the current version, follow service and deployment logs with:

```bash
sudo journalctl -u lintex-webhook -f
```

## 8. Test the deployment endpoint locally

Use the same token stored in `/etc/lintex-webhook.env`:

```bash
curl --fail --show-error \
  --request POST \
  --header "Authorization: Bearer replace-with-the-generated-token" \
  http://127.0.0.1:9000/deploy
```

Expected response:

```json
{"status":"accepted","request_id":"..."}
```

Watch the logs and confirm `docker compose pull`, `docker compose up`, and the
health check complete successfully:

```bash
sudo journalctl -u lintex-webhook -f
```

## 9. Configure DNS, HTTPS, and Nginx

Create a DNS record for `deploy.dailin.tech` pointing to the server. Configure
a valid TLS certificate before exposing the deployment endpoint.

Create an Nginx server block such as
`/etc/nginx/sites-available/deploy.dailin.tech`:

```nginx
server {
    listen 443 ssl http2;
    server_name deploy.dailin.tech;

    ssl_certificate /etc/letsencrypt/live/deploy.dailin.tech/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/deploy.dailin.tech/privkey.pem;

    location = /health {
        proxy_pass http://127.0.0.1:9000/health;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }

    location = /deploy {
        client_max_body_size 1k;
        proxy_connect_timeout 5s;
        proxy_read_timeout 30s;

        proxy_pass http://127.0.0.1:9000/deploy;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header Authorization $http_authorization;
    }

    location / {
        return 404;
    }
}
```

Enable and validate the configuration. Adjust these commands if the server
uses a different Nginx layout:

```bash
sudo ln -s \
  /etc/nginx/sites-available/deploy.dailin.tech \
  /etc/nginx/sites-enabled/deploy.dailin.tech

sudo nginx -t
sudo systemctl reload nginx
```

Test HTTPS from outside the server:

```bash
curl https://deploy.dailin.tech/health

curl --fail --show-error \
  --request POST \
  --header "Authorization: Bearer replace-with-the-generated-token" \
  https://deploy.dailin.tech/deploy
```

Do not expose port `9000` publicly. The webhook should continue listening only
on `127.0.0.1`.

## 10. Add GitHub Actions secrets

Open the Login repository Actions secrets page:

<https://github.com/dailin3/lintex-login/settings/secrets/actions>

Add these repository secrets:

| Secret | Value |
| --- | --- |
| `DEPLOY_WEBHOOK_URL` | `https://deploy.dailin.tech/deploy` |
| `DEPLOY_WEBHOOK_TOKEN` | The token from `/etc/lintex-webhook.env` |

The next push to the `lintex-login` `master` branch will:

1. Build the Docker image.
2. Push `latest` and the commit SHA tag to Aliyun ACR.
3. Call the HTTPS webhook.
4. Pull the new image on the server.
5. Recreate the Login container.
6. Run the Login health check.

The workflow skips the deployment step until both secrets exist.

## Planned configuration layout

The current `v0.1.0` release uses one `DEPLOY_SCRIPT`. A future multi-service
version will move deployment configuration to the agreed layout:

```text
/opt/lintex-config/
├── services.toml
├── lintex-login/
│   ├── compose.yaml
│   ├── deploy.sh
│   └── secrets.env.sops
└── lintex-api/
    ├── compose.yaml
    ├── deploy.sh
    └── secrets.env.sops
```

Do not migrate to this layout while following the current guide. It is recorded
here as the next design target.

