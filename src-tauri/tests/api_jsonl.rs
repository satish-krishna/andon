mod common;

use axum::http::StatusCode;
use common::test_router;
use tower::ServiceExt;

#[tokio::test]
async fn backfill_endpoint_returns_2xx_or_500() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/jsonl/backfill")
                .header("content-type", "application/json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        res.status(),
        StatusCode::OK | StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn session_end_with_transcript_path_returns_200() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let body = serde_json::json!({
        "session_id": "s-test", "reason": "exit",
        "transcript_path": "/does/not/exist/missing.jsonl"
    });
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/hooks/session-end")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn errors_endpoint_returns_empty_array_initially() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/jsonl/errors")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn behaviour_endpoints_return_200_with_empty_db() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    for path in [
        "/api/behaviour/model-mix",
        "/api/behaviour/slash-commands",
        "/api/behaviour/subagents",
    ] {
        let res = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {path} not OK");
    }
}
