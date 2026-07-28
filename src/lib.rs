use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Mutex,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use uuid::Uuid;

const RETENTION_DAYS: i64 = 30;
const MAX_RUNS: usize = 500;

#[derive(Clone)]
pub struct AppState {
    token: Arc<[u8]>,
    config_repository: PathBuf,
    services_config: PathBuf,
    runs_directory: PathBuf,
    deploying: Arc<AtomicBool>,
    update_repository: bool,
}

impl AppState {
    pub fn new(
        token: impl AsRef<[u8]>,
        config_repository: PathBuf,
        services_config: PathBuf,
        runs_directory: PathBuf,
    ) -> Self {
        Self {
            token: Arc::from(token.as_ref()),
            config_repository,
            services_config,
            runs_directory,
            deploying: Arc::new(AtomicBool::new(false)),
            update_repository: true,
        }
    }

    pub fn new_for_test(
        token: impl AsRef<[u8]>,
        config_repository: PathBuf,
        runs_directory: PathBuf,
    ) -> Self {
        let services_config = config_repository.join("services.toml");
        Self {
            token: Arc::from(token.as_ref()),
            config_repository,
            services_config,
            runs_directory,
            deploying: Arc::new(AtomicBool::new(false)),
            update_repository: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("WEBHOOK_TOKEN is required")]
    MissingToken,
    #[error("invalid LISTEN_ADDR: {0}")]
    InvalidListenAddr(#[from] std::net::AddrParseError),
}

pub struct AppConfig {
    pub listen_addr: SocketAddr,
    pub token: String,
    pub config_repository: PathBuf,
    pub services_config: PathBuf,
    pub runs_directory: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let token = env::var("WEBHOOK_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        let config_repository = PathBuf::from(
            env::var_os("CONFIG_REPOSITORY").unwrap_or_else(|| "/opt/lintex-config".into()),
        );
        let services_config = PathBuf::from(
            env::var_os("SERVICES_CONFIG")
                .unwrap_or_else(|| config_repository.join("services.toml").into_os_string()),
        );
        let runs_directory = PathBuf::from(
            env::var_os("RUNS_DIRECTORY").unwrap_or_else(|| "/var/lib/lintex-webhook/runs".into()),
        );
        let listen_addr = env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9000".into())
            .parse()?;
        Ok(Self {
            listen_addr,
            token,
            config_repository,
            services_config,
            runs_directory,
        })
    }
}

#[derive(Clone, Deserialize)]
struct Service {
    display_name: String,
    working_directory: PathBuf,
    deploy_script: PathBuf,
}

#[derive(Deserialize)]
struct ServicesFile {
    services: HashMap<String, Service>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    pub id: String,
    pub service: String,
    pub display_name: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub elapsed_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/deploy/{service}", post(deploy))
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/log", get(get_log))
        .route("/runs/{id}/stream", get(stream_log))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok"}))
}

async fn deploy(
    State(state): State<AppState>,
    AxumPath(service_name): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state.token) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let services = match load_services(&state.services_config).await {
        Ok(v) => v,
        Err(e) => return api_error_owned(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let Some(service) = services.services.get(&service_name).cloned() else {
        return api_error(StatusCode::NOT_FOUND, "unknown service");
    };
    if !service.working_directory.is_absolute()
        || !service.deploy_script.is_absolute()
        || !service
            .working_directory
            .starts_with(&state.config_repository)
        || !service.deploy_script.starts_with(&state.config_repository)
    {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "invalid service paths");
    }
    if state
        .deploying
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return api_error(StatusCode::CONFLICT, "deployment already running");
    }

    let id = Uuid::now_v7().to_string();
    let metadata = RunMetadata {
        id: id.clone(),
        service: service_name,
        display_name: service.display_name.clone(),
        status: RunStatus::Running,
        started_at: Utc::now(),
        finished_at: None,
        elapsed_ms: None,
        exit_code: None,
    };
    if let Err(e) = create_run(&state.runs_directory, &metadata).await {
        state.deploying.store(false, Ordering::Release);
        return api_error_owned(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let response_id = id.clone();
    tokio::spawn(async move {
        if let Err(e) = execute_run(&state, service, metadata).await {
            error!(run_id=%id, error=%e, "deployment task failed");
        }
        state.deploying.store(false, Ordering::Release);
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"status":"accepted", "request_id":response_id})),
    )
        .into_response()
}

async fn list_runs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state.token) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match read_all_runs(&state.runs_directory).await {
        Ok(runs) => Json(runs).into_response(),
        Err(e) => api_error_owned(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn get_run(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state.token) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if Uuid::parse_str(&id).is_err() {
        return api_error(StatusCode::BAD_REQUEST, "invalid run id");
    }
    match read_metadata(&state.runs_directory, &id).await {
        Ok(run) => Json(run).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            api_error(StatusCode::NOT_FOUND, "run not found")
        }
        Err(e) => api_error_owned(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn get_log(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state.token) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if Uuid::parse_str(&id).is_err() {
        return api_error(StatusCode::BAD_REQUEST, "invalid run id");
    }
    match fs::read_to_string(run_dir(&state.runs_directory, &id).join("output.log")).await {
        Ok(log) => (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            log,
        )
            .into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            api_error(StatusCode::NOT_FOUND, "run not found")
        }
        Err(e) => api_error_owned(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn stream_log(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state.token) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if Uuid::parse_str(&id).is_err() {
        return api_error(StatusCode::BAD_REQUEST, "invalid run id");
    }
    if read_metadata(&state.runs_directory, &id).await.is_err() {
        return api_error(StatusCode::NOT_FOUND, "run not found");
    }
    let runs = state.runs_directory.clone();
    let output = stream! {
        let mut sent = 0usize;
        loop {
            if let Ok(content) = fs::read_to_string(run_dir(&runs,&id).join("output.log")).await
                && content.len() > sent {
                let chunk = content[sent..].to_owned(); sent = content.len();
                yield Ok::<Event,std::convert::Infallible>(Event::default().event("log").data(chunk));
            }
            match read_metadata(&runs,&id).await { Ok(run) if run.status != RunStatus::Running => { yield Ok(Event::default().event("done").data(serde_json::to_string(&run).unwrap_or_default())); break; }, Err(_) => break, _ => {} }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    Sse::new(output)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

async fn execute_run(
    state: &AppState,
    service: Service,
    mut metadata: RunMetadata,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let started = std::time::Instant::now();
    let log = Arc::new(Mutex::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(run_dir(&state.runs_directory, &metadata.id).join("output.log"))
            .await?,
    ));
    let mut result = Ok(0);
    if state.update_repository {
        append_line(
            &log,
            "system",
            &format!(
                "$ git -C {} pull --ff-only",
                state.config_repository.display()
            ),
        )
        .await?;
        result = run_command(
            Command::new("git")
                .arg("-C")
                .arg(&state.config_repository)
                .args(["pull", "--ff-only"]),
            log.clone(),
        )
        .await;
    } else {
        append_line(
            &log,
            "system",
            "$ git -C <config> pull --ff-only (skipped in test)",
        )
        .await?;
    }
    if matches!(result, Ok(0)) {
        append_line(
            &log,
            "system",
            &format!("$ {}", service.deploy_script.display()),
        )
        .await?;
        result = run_command(
            Command::new(&service.deploy_script).current_dir(&service.working_directory),
            log.clone(),
        )
        .await;
    }
    metadata.finished_at = Some(Utc::now());
    metadata.elapsed_ms = Some(started.elapsed().as_millis() as u64);
    match result {
        Ok(0) => {
            metadata.status = RunStatus::Succeeded;
            metadata.exit_code = Some(0);
        }
        Ok(code) => {
            metadata.status = RunStatus::Failed;
            metadata.exit_code = Some(code);
        }
        Err(e) => {
            append_line(&log, "error", &e.to_string()).await?;
            metadata.status = RunStatus::Failed;
            metadata.exit_code = Some(-1);
        }
    }
    write_metadata(&state.runs_directory, &metadata).await?;
    cleanup_runs(&state.runs_directory).await?;
    info!(run_id=%metadata.id,status=?metadata.status,elapsed_ms=?metadata.elapsed_ms,"deployment finished");
    Ok(())
}

async fn run_command(
    command: &mut Command,
    log: Arc<Mutex<tokio::fs::File>>,
) -> Result<i32, std::io::Error> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_log = log.clone();
    let err_log = log.clone();
    let out = tokio::spawn(async move { copy_lines(stdout, "stdout", out_log).await });
    let err = tokio::spawn(async move { copy_lines(stderr, "stderr", err_log).await });
    let status = child.wait().await?;
    out.await.map_err(std::io::Error::other)??;
    err.await.map_err(std::io::Error::other)??;
    Ok(status.code().unwrap_or(-1))
}

async fn copy_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    stream: &str,
    log: Arc<Mutex<tokio::fs::File>>,
) -> std::io::Result<()> {
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        append_line(&log, stream, &line).await?;
    }
    Ok(())
}
async fn append_line(
    log: &Arc<Mutex<tokio::fs::File>>,
    stream: &str,
    line: &str,
) -> std::io::Result<()> {
    let mut file = log.lock().await;
    file.write_all(format!("{} [{}] {}\n", Utc::now().to_rfc3339(), stream, line).as_bytes())
        .await?;
    file.flush().await
}

async fn load_services(
    path: &Path,
) -> Result<ServicesFile, Box<dyn std::error::Error + Send + Sync>> {
    Ok(toml::from_str(&fs::read_to_string(path).await?)?)
}
fn run_dir(base: &Path, id: &str) -> PathBuf {
    base.join(id)
}
async fn create_run(base: &Path, run: &RunMetadata) -> std::io::Result<()> {
    fs::create_dir_all(run_dir(base, &run.id)).await?;
    write_metadata(base, run).await
}
async fn write_metadata(base: &Path, run: &RunMetadata) -> std::io::Result<()> {
    let directory = run_dir(base, &run.id);
    let temporary = directory.join("metadata.json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(run).map_err(std::io::Error::other)?,
    )
    .await?;
    fs::rename(temporary, directory.join("metadata.json")).await
}
async fn read_metadata(base: &Path, id: &str) -> std::io::Result<RunMetadata> {
    serde_json::from_slice(&fs::read(run_dir(base, id).join("metadata.json")).await?)
        .map_err(std::io::Error::other)
}
async fn read_all_runs(base: &Path) -> std::io::Result<Vec<RunMetadata>> {
    let mut runs = Vec::new();
    let mut entries = match fs::read_dir(base).await {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(runs),
        Err(e) => return Err(e),
    };
    while let Some(entry) = entries.next_entry().await? {
        if let Ok(run) = read_metadata(base, &entry.file_name().to_string_lossy()).await {
            runs.push(run);
        }
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    Ok(runs)
}
async fn cleanup_runs(base: &Path) -> std::io::Result<()> {
    let runs = read_all_runs(base).await?;
    let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
    for (index, run) in runs.iter().enumerate() {
        if index >= MAX_RUNS || run.started_at < cutoff {
            let _ = fs::remove_dir_all(run_dir(base, &run.id)).await;
        }
    }
    Ok(())
}

fn authorized(headers: &HeaderMap, expected: &[u8]) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| bool::from(v.as_bytes().ct_eq(expected)))
}
fn api_error(status: StatusCode, message: &'static str) -> Response {
    api_error_owned(status, message.to_owned())
}
fn api_error_owned(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(serde_json::json!({"status":"error","message":message})),
    )
        .into_response()
}
