use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use lintex_webhook::{AppState, RunError, ScriptRunner, app};
use serde_json::Value;
use tokio::sync::{Notify, oneshot};
use tower::ServiceExt;

type DeploymentResultReceiver = oneshot::Receiver<Result<(), RunError>>;

#[derive(Clone, Default)]
struct ControlledRunner {
    calls: Arc<Mutex<Vec<String>>>,
    started: Arc<Notify>,
    result: Arc<Mutex<Option<DeploymentResultReceiver>>>,
}

impl ControlledRunner {
    fn immediate(result: Result<(), RunError>) -> Self {
        let (sender, receiver) = oneshot::channel();
        sender.send(result).expect("receiver remains alive");
        Self {
            result: Arc::new(Mutex::new(Some(receiver))),
            ..Self::default()
        }
    }

    fn blocking() -> (Self, oneshot::Sender<Result<(), RunError>>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                result: Arc::new(Mutex::new(Some(receiver))),
                ..Self::default()
            },
            sender,
        )
    }

    async fn wait_started(&self) {
        tokio::time::timeout(Duration::from_secs(1), self.started.notified())
            .await
            .expect("deployment should start");
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("calls lock").len()
    }
}

#[async_trait]
impl ScriptRunner for ControlledRunner {
    async fn run(&self, request_id: String) -> Result<(), RunError> {
        self.calls.lock().expect("calls lock").push(request_id);
        self.started.notify_one();
        let receiver = self
            .result
            .lock()
            .expect("result lock")
            .take()
            .expect("one configured deployment result");
        receiver.await.expect("test controls deployment result")
    }
}

fn test_app(runner: ControlledRunner) -> axum::Router {
    app(AppState::new("test-secret", Arc::new(runner)))
}

fn deploy_request(token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri("/deploy");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).expect("valid request")
}

#[tokio::test]
async fn health_returns_ok() {
    let response = test_app(ControlledRunner::immediate(Ok(())))
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({ "status": "ok" }));
}

#[tokio::test]
async fn deploy_rejects_missing_or_invalid_tokens() {
    let runner = ControlledRunner::immediate(Ok(()));
    let router = test_app(runner.clone());

    for request in [deploy_request(None), deploy_request(Some("wrong"))] {
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(runner.call_count(), 0);
}

#[tokio::test]
async fn valid_deploy_returns_request_id_and_starts_runner() {
    let runner = ControlledRunner::immediate(Ok(()));
    let response = test_app(runner.clone())
        .oneshot(deploy_request(Some("test-secret")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "accepted");
    assert!(body["request_id"].as_str().is_some_and(|id| !id.is_empty()));
    runner.wait_started().await;
    assert_eq!(runner.call_count(), 1);
}

#[tokio::test]
async fn concurrent_deploy_returns_conflict() {
    let (runner, finish) = ControlledRunner::blocking();
    let router = test_app(runner.clone());

    assert_eq!(
        router
            .clone()
            .oneshot(deploy_request(Some("test-secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    runner.wait_started().await;

    assert_eq!(
        router
            .clone()
            .oneshot(deploy_request(Some("test-secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    finish.send(Ok(())).unwrap();
}

#[tokio::test]
async fn deployment_lock_is_released_after_runner_failure() {
    let runner = ControlledRunner::immediate(Err(RunError::Failed(17)));
    let router = test_app(runner.clone());

    assert_eq!(
        router
            .clone()
            .oneshot(deploy_request(Some("test-secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    runner.wait_started().await;

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let response = router
                .clone()
                .oneshot(deploy_request(Some("test-secret")))
                .await
                .unwrap();
            if response.status() == StatusCode::ACCEPTED {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lock should be released after failure");
}
