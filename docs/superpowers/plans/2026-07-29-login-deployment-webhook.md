# Login Deployment Webhook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and document a small Rust webhook that securely deploys `lintex-login` through one fixed script.

**Architecture:** An Axum router owns authentication and a single in-memory deployment lock. A small runner abstraction launches the configured script in the background and streams process output through tracing; systemd and journald own process supervision and log retention.

**Tech Stack:** Rust 2024, Axum, Tokio, tracing, serde, UUID, subtle, tower HTTP tests

## Global Constraints

- Expose only `GET /health` and `POST /deploy`.
- Accept no caller-controlled deployment parameters.
- Permit one deployment at a time.
- Never log bearer tokens or credentials.
- Keep the first release single-project and storage-free.

---

### Task 1: HTTP contract and deployment lifecycle

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Test: `tests/api.rs`

**Interfaces:**
- `AppConfig::from_env() -> Result<AppConfig, ConfigError>` loads runtime settings.
- `app(AppState) -> Router` creates the HTTP application.
- `ScriptRunner::run(request_id) -> Future<Output = Result<(), RunError>>` runs the fixed script.

- [ ] Write API tests for health, missing/invalid/valid tokens, concurrent deployment, and lock release after success/failure.
- [ ] Run `cargo test` and confirm tests fail because the application API is missing.
- [ ] Implement the minimal router, constant-time token check, lock, background task, script runner, and configuration loader.
- [ ] Run `cargo test` and confirm all API tests pass.
- [ ] Run `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Commit the working Rust service.

### Task 2: Server deployment assets and documentation

**Files:**
- Create: `.env.example`
- Create: `.gitignore`
- Create: `deploy/deploy-login.sh.example`
- Create: `deploy/lintex-webhook.service`
- Create: `deploy/docker-compose.login.yml`
- Create: `LICENSE`
- Create: `README.md`

**Interfaces:**
- The script is invoked without arguments and uses fixed server paths.
- The systemd unit reads `/etc/lintex-webhook.env`.

- [ ] Add safe example files with no credentials.
- [ ] Document installation, reverse proxy expectations, GitHub secrets, logs, and manual rollback.
- [ ] Validate shell syntax, Compose syntax where available, secret scans, and repository status.
- [ ] Commit deployment assets and documentation.

### Task 3: Connect lintex-login CI

**Files:**
- Modify: `/Users/lin/Documents/lintex-login/.github/workflows/docker.yml`
- Create: `/Users/lin/Documents/lintex-login/compose.yaml`
- Modify: `/Users/lin/Documents/lintex-login/.env.example`
- Modify: `/Users/lin/Documents/lintex-login/README.md`

**Interfaces:**
- GitHub Actions calls `${DEPLOY_WEBHOOK_URL}` with `${DEPLOY_WEBHOOK_TOKEN}` only after image push succeeds.
- Compose runs the fixed Aliyun ACR `latest` image and reads runtime secrets from `.env.production`.

- [ ] Add the production Compose definition and document runtime variables.
- [ ] Add a curl deployment step with explicit failure and timeout behavior.
- [ ] Validate YAML, run existing application checks, inspect the diff for secrets, and commit.

### Task 4: Final verification

**Files:**
- Verify all files changed by Tasks 1-3.

- [ ] Run Rust format, Clippy, tests, and a local smoke test using a harmless temporary script.
- [ ] Run lintex-login lint/build checks and inspect both git diffs.
- [ ] Re-evaluate security, logging, simplicity, and document remaining server-side setup.
