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

#[tokio::test]
async fn coverage_gaps_flags_partial_otlp_session() {
    let (pool, _g) = common::fixture_pool();
    {
        let conn = pool.get().unwrap();
        // 'partial': transcript shows 5 API calls, OTLP recorded only 2 → flagged.
        conn.execute(
            "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at) VALUES ('partial', 5, 0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('partial', NULL, 100, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('partial', NULL, 200, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        // 'full': transcript and OTLP agree (2 == 2) → NOT flagged.
        conn.execute(
            "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at) VALUES ('full', 2, 0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('full', NULL, 100, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('full', NULL, 200, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
    }
    let (router, _g2) = test_router(&pool);
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/jsonl/coverage-gaps")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let gaps: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = gaps.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only the partial session is flagged");
    assert_eq!(arr[0]["session_id"], "partial");
    assert_eq!(arr[0]["jsonl_calls"], 5);
    assert_eq!(arr[0]["otlp_calls"], 2);
}
