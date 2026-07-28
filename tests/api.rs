use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use lintex_webhook::{AppState, app};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _temp: TempDir,
    state: AppState,
}

impl Fixture {
    fn new(script: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("config");
        let service_dir = config_dir.join("lintex-login");
        let runs_dir = temp.path().join("runs");
        fs::create_dir_all(&service_dir).unwrap();
        let script_path = service_dir.join("deploy.sh");
        fs::write(&script_path, script).unwrap();
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            config_dir.join("services.toml"),
            format!(
                r#"
[services.lintex-login]
display_name = "Lintex Login"
working_directory = "{}"
deploy_script = "{}"
"#,
                service_dir.display(),
                script_path.display()
            ),
        )
        .unwrap();
        Self {
            state: AppState::new_for_test("secret", config_dir, runs_dir),
            _temp: temp,
        }
    }
}

fn request(method: &str, uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

async fn json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn wait_finished(router: &axum::Router, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let response = router
                .clone()
                .oneshot(request("GET", &format!("/runs/{id}"), Some("secret")))
                .await
                .unwrap();
            let body = json(response).await;
            if body["status"] != "running" {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn health_is_public_but_run_routes_are_protected() {
    let fixture = Fixture::new("#!/bin/sh\nexit 0\n");
    let router = app(fixture.state.clone());
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", "/health", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        router
            .oneshot(request("GET", "/runs", None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn unknown_service_is_not_found() {
    let fixture = Fixture::new("#!/bin/sh\nexit 0\n");
    let router = app(fixture.state.clone());
    assert_eq!(
        router
            .oneshot(request("POST", "/deploy/unknown", Some("secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn successful_run_persists_metadata_and_terminal_output() {
    let fixture = Fixture::new("#!/bin/sh\necho hello\necho warning >&2\n");
    let router = app(fixture.state.clone());
    let response = router
        .clone()
        .oneshot(request("POST", "/deploy/lintex-login", Some("secret")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let id = json(response).await["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let metadata = wait_finished(&router, &id).await;
    assert_eq!(metadata["status"], "succeeded");
    assert_eq!(metadata["exit_code"], 0);

    let response = router
        .clone()
        .oneshot(request("GET", &format!("/runs/{id}/log"), Some("secret")))
        .await
        .unwrap();
    let log = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(log.contains("$ git -C"));
    assert!(log.contains("hello"));
    assert!(log.contains("warning"));

    let list = json(
        router
            .oneshot(request("GET", "/runs", Some("secret")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_run_records_exit_code_and_releases_lock() {
    let fixture = Fixture::new("#!/bin/sh\necho broken >&2\nexit 17\n");
    let router = app(fixture.state.clone());
    let first = router
        .clone()
        .oneshot(request("POST", "/deploy/lintex-login", Some("secret")))
        .await
        .unwrap();
    let id = json(first).await["request_id"].as_str().unwrap().to_owned();
    let metadata = wait_finished(&router, &id).await;
    assert_eq!(metadata["status"], "failed");
    assert_eq!(metadata["exit_code"], 17);
    assert_eq!(
        router
            .oneshot(request("POST", "/deploy/lintex-login", Some("secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
}

#[tokio::test]
async fn concurrent_run_is_rejected() {
    let fixture = Fixture::new("#!/bin/sh\nsleep 1\n");
    let router = app(fixture.state.clone());
    assert_eq!(
        router
            .clone()
            .oneshot(request("POST", "/deploy/lintex-login", Some("secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        router
            .oneshot(request("POST", "/deploy/lintex-login", Some("secret")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
}

#[test]
fn fixture_paths_are_absolute() {
    assert!(Path::new("/opt/lintex-config").is_absolute());
}
