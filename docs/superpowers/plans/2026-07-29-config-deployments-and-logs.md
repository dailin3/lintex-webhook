# Config Deployments And Logs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add config-driven multi-service deployments and persistent remotely accessible run logs, then deploy the complete Login pipeline.

**Architecture:** `lintex-webhook` loads an allowlist from `/opt/lintex-config/services.toml`, updates that repository before each deployment, and executes only the configured script. Each run persists metadata and terminal output under `/var/lib/lintex-webhook/runs`, with authenticated list, detail, plain log, and SSE endpoints.

**Tech Stack:** Rust 2024, Axum, Tokio, serde, TOML, tracing, systemd, Docker Compose, GitHub Actions

## Global Constraints

- Real `.env` files never enter Git.
- Requests cannot select commands or paths.
- All run endpoints require bearer authentication.
- One deployment runs at a time.
- Retain runs for 30 days and at most 500 entries.
- Only version tags create GitHub Releases.

---

### Task 1: Config and persistent run model

- [ ] Write failing tests for TOML loading, allowed/unknown services, run metadata, output persistence, and retention.
- [ ] Implement focused config and run-store modules.
- [ ] Run tests, formatting, and Clippy; commit.

### Task 2: Deployment API and live logs

- [ ] Write failing HTTP tests for `/deploy/:service`, `/runs`, run detail, log download, SSE, auth, failures, and concurrency.
- [ ] Implement repository update, script execution, output fan-out, and authenticated routes.
- [ ] Run tests, formatting, Clippy, and smoke test; commit.

### Task 3: Private config repository and service assets

- [ ] Create local `lintex-config` with Login Compose, safe deploy script, env template, ignore rules, and documentation.
- [ ] Validate Compose and shell syntax, scan for credentials, create private GitHub repository, and push.

### Task 4: Release and server deployment

- [ ] Update systemd assets and docs, tag and publish the next Webhook release.
- [ ] Configure server Deploy Key, clone config into `/opt/lintex-config`, create protected `.env`, install Webhook and systemd.
- [ ] Configure Nginx HTTPS endpoint using an available deployment hostname.

### Task 5: End-to-end CI/CD

- [ ] Update Login workflow endpoint to `/deploy/lintex-login` and configure GitHub Secrets.
- [ ] Trigger CI, verify ACR push, Webhook deployment, container health, persisted logs, and remote log endpoints.
- [ ] Review security, repository cleanliness, and operational documentation.
