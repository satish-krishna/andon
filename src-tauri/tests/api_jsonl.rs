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

#[tokio::test(flavor = "multi_thread")]
async fn session_end_persists_and_reports_after_fire_and_forget_response() {
    // The session-end hook is fire-and-forget: the handler returns immediately
    // and persists / renders in a detached task so a cancelled hook `curl` can't
    // drop the work. Verify ended_at lands and the report is written even though
    // neither happened before the HTTP response.
    use rusqlite::OptionalExtension;

    let (pool, _g) = common::fixture_pool();
    let (router, dir) = test_router(&pool);
    let body = serde_json::json!({ "session_id": "s-end", "reason": "exit" });
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

    let report_path = dir.path().join("reports").join("s-end.html");
    let mut ended_at: Option<i64> = None;
    for _ in 0..100 {
        ended_at = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT ended_at FROM sessions WHERE session_id = 's-end'",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .unwrap()
            .flatten()
        };
        if ended_at.is_some() && report_path.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(ended_at.is_some(), "ended_at should be persisted by the detached task");
    assert!(report_path.exists(), "report should be rendered by the detached task");
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
async fn model_mix_counts_api_calls_not_token_rows() {
    // Each API call writes several token_usage rows (input/output/cache…).
    // "Invocations by model" must count distinct calls, not rows.
    let (pool, _g) = common::fixture_pool();
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "s-mix".into(),
            model: "claude-opus-4-7".into(),
            input_tokens: 100,
            output_tokens: 200,
            ..Default::default()
        },
    );
    let (router, _g2) = test_router(&pool);
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/behaviour/model-mix")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let opus = json["by_model"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["model"] == "claude-opus-4-7")
        .expect("opus row present");
    assert_eq!(opus["invocations"], 1, "two token rows from one call count once");
    assert_eq!(opus["sessions"], 1);
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
