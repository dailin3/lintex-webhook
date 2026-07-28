use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use lintex_webhook::{AppState, app};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn rejects_invalid_run_ids_before_touching_disk() {
    let temp = tempdir().unwrap();
    let state = AppState::new_for_test(
        "secret",
        temp.path().join("config"),
        temp.path().join("runs"),
    );
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/runs/not-a-uuid/log")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
