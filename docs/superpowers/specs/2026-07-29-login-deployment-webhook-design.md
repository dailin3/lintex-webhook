# Lintex Login Deployment Webhook Design

## Purpose

Build a small Rust HTTP service that deploys only `lintex-login` after GitHub
Actions has pushed a new image to Aliyun ACR. The service runs directly on the
Docker host and invokes one administrator-controlled deployment script.

The first release intentionally does not support multiple projects, arbitrary
commands, a database, a job queue, deployment history, or automatic rollback.

## Deployment Flow

1. A push to `master` starts the `lintex-login` GitHub Actions workflow.
2. GitHub Actions builds the image and pushes `latest` and commit-SHA tags to
   Aliyun ACR.
3. After the push succeeds, GitHub Actions sends `POST /deploy` with a bearer
   token to the webhook.
4. The webhook authenticates the request and rejects it if another deployment
   is running.
5. The webhook starts the fixed deployment script asynchronously and returns
   `202 Accepted` with a request ID.
6. The script runs `docker compose pull`, then
   `docker compose up -d --remove-orphans`, followed by an HTTP health check.

## HTTP Interface

### `GET /health`

Returns `200 OK` when the webhook process is running. This endpoint does not
require authentication and does not inspect the deployed application.

### `POST /deploy`

Requires `Authorization: Bearer <WEBHOOK_TOKEN>`.

- `202 Accepted`: deployment started.
- `401 Unauthorized`: token is absent or invalid.
- `409 Conflict`: a deployment is already running.
- `500 Internal Server Error`: the deployment process could not be started.

The request body is ignored. Callers cannot supply a command, project name,
image, Compose path, or script path.

## Runtime Configuration

The service reads these environment variables:

- `WEBHOOK_TOKEN`: required secret used to authenticate deployment requests.
- `DEPLOY_SCRIPT`: required absolute path to the administrator-controlled
  deployment script.
- `LISTEN_ADDR`: optional socket address; defaults to `127.0.0.1:9000`.
- `RUST_LOG`: optional log filter; defaults to `lintex_webhook=info,tower_http=info`.

Secrets are never committed. An example environment file contains names and
safe placeholders only.

## Logging

The service writes structured logs to standard output so systemd/journald can
collect them. Each deployment receives a request ID. Logs include:

- request ID and remote address;
- authentication success or rejection without the token value;
- deployment accepted or rejected because one is already running;
- script start, completion status, and elapsed time;
- captured script stdout and stderr, line by line, with the request ID;
- process-launch and unexpected runtime errors.

The fixed deployment script prints a short message before each stage: registry
pull, Compose update, and application health check. It must not print registry
credentials or environment-file contents.

The service keeps only the current deployment lock in memory. Persistent log
storage, rotation, and retention are delegated to journald.

## Concurrency And Failure Handling

An atomic in-memory lock allows one deployment at a time. The lock is released
after success, script failure, or process-launch failure. A failed deployment is
logged but does not terminate the webhook process.

The deployment script stops at the first failed command. If `docker compose
pull`, `docker compose up`, or the health check fails, the script exits nonzero
and the webhook records the failure. Automatic rollback is outside the first
release; the immutable commit-SHA image remains available for manual rollback.

## Security

- Listen on loopback by default and place HTTPS and any public exposure behind
  the server's existing reverse proxy.
- Authenticate with a long random token stored in GitHub Actions secrets and a
  root-readable systemd environment file.
- Compare bearer tokens without exposing them in logs.
- Execute only the configured script, with no shell input from the request.
- Run under a dedicated system user that has only the Docker access required
  for deployment.
- Apply request-body and timeout limits at the reverse proxy.

## Repository Contents

- Rust application using Axum and Tokio.
- Unit and HTTP integration tests.
- `deploy-login.sh.example` showing the fixed Compose deployment flow.
- Example `docker-compose.yml` and environment file for `lintex-login`.
- Example systemd service unit.
- README with build, installation, GitHub Actions, logging, and rollback steps.
- MIT license so the repository can be public.

## Testing

Tests cover:

- health endpoint success;
- missing, malformed, and incorrect bearer tokens;
- a valid request starts the configured script;
- concurrent deployment requests return `409 Conflict`;
- deployment lock is released after script success or failure;
- secrets do not appear in application-generated logs where practical to test.

Before release, run formatting, Clippy with warnings denied, all tests, and a
local smoke test with a harmless temporary deployment script.

## Integration Changes

After the webhook is deployed, `lintex-login` gains:

- a production Compose example using the Aliyun ACR image;
- a GitHub Actions step that calls the webhook only after image push succeeds;
- documentation for `DEPLOY_WEBHOOK_URL` and `DEPLOY_WEBHOOK_TOKEN` secrets.

These integration changes are separate from the webhook binary and do not put
server credentials in either repository.
