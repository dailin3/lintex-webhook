use std::{
    env,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    token: Arc<[u8]>,
    runner: Arc<dyn ScriptRunner>,
    deploying: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(token: impl AsRef<[u8]>, runner: Arc<dyn ScriptRunner>) -> Self {
        Self {
            token: Arc::from(token.as_ref()),
            runner,
            deploying: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error("failed to start deployment script: {0}")]
    Start(#[source] std::io::Error),
    #[error("failed to read deployment output: {0}")]
    Output(#[source] std::io::Error),
    #[error("deployment script exited with status {0}")]
    Failed(i32),
}

#[async_trait]
pub trait ScriptRunner: Send + Sync {
    async fn run(&self, request_id: String) -> Result<(), RunError>;
}

#[derive(Clone)]
pub struct ProcessScriptRunner {
    script: PathBuf,
}

impl ProcessScriptRunner {
    pub fn new(script: PathBuf) -> Self {
        Self { script }
    }
}

#[async_trait]
impl ScriptRunner for ProcessScriptRunner {
    async fn run(&self, request_id: String) -> Result<(), RunError> {
        let mut child = Command::new(&self.script)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(RunError::Start)?;

        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let stdout_id = request_id.clone();
        let stderr_id = request_id.clone();

        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await.map_err(RunError::Output)? {
                info!(request_id = %stdout_id, stream = "stdout", message = %line, "deployment output");
            }
            Ok::<(), RunError>(())
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Some(line) = lines.next_line().await.map_err(RunError::Output)? {
                warn!(request_id = %stderr_id, stream = "stderr", message = %line, "deployment output");
            }
            Ok::<(), RunError>(())
        });

        let status = child.wait().await.map_err(RunError::Output)?;
        stdout_task.await.expect("stdout task does not panic")?;
        stderr_task.await.expect("stderr task does not panic")?;

        if status.success() {
            Ok(())
        } else {
            Err(RunError::Failed(status.code().unwrap_or(-1)))
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("WEBHOOK_TOKEN is required")]
    MissingToken,
    #[error("DEPLOY_SCRIPT is required")]
    MissingScript,
    #[error("invalid LISTEN_ADDR: {0}")]
    InvalidListenAddr(#[from] std::net::AddrParseError),
}

pub struct AppConfig {
    pub listen_addr: SocketAddr,
    pub token: String,
    pub deploy_script: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let token = env::var("WEBHOOK_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        let deploy_script = env::var_os("DEPLOY_SCRIPT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(ConfigError::MissingScript)?;
        let listen_addr = env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9000".to_owned())
            .parse()?;

        Ok(Self {
            listen_addr,
            token,
            deploy_script,
        })
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/deploy", post(deploy))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<StatusResponse> {
    Json(StatusResponse { status: "ok" })
}

async fn deploy(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state.token) {
        warn!("deployment request rejected: invalid authorization");
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    if state
        .deploying
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        warn!("deployment request rejected: deployment already running");
        return api_error(StatusCode::CONFLICT, "deployment already running");
    }

    let request_id = Uuid::now_v7().to_string();
    let response_id = request_id.clone();
    info!(request_id = %request_id, "deployment accepted");

    tokio::spawn(async move {
        let started = Instant::now();
        info!(request_id = %request_id, "deployment started");
        let result = state.runner.run(request_id.clone()).await;
        state.deploying.store(false, Ordering::Release);

        match result {
            Ok(()) => {
                info!(request_id = %request_id, elapsed_ms = started.elapsed().as_millis(), "deployment completed")
            }
            Err(error) => {
                error!(request_id = %request_id, elapsed_ms = started.elapsed().as_millis(), error = %error, "deployment failed")
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(AcceptedResponse {
            status: "accepted",
            request_id: response_id,
        }),
    )
        .into_response()
}

fn authorized(headers: &HeaderMap, expected: &[u8]) -> bool {
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };

    value.as_bytes().ct_eq(expected).into()
}

fn api_error(status: StatusCode, message: &'static str) -> Response {
    (
        status,
        Json(ErrorResponse {
            status: "error",
            message,
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct AcceptedResponse {
    status: &'static str,
    request_id: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    status: &'static str,
    message: &'static str,
}
