mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    };
    (status, v)
}

/// `/api/behaviour/model-mix` must honor the `from`/`to` window so the
/// Overview's "Invocations by model" tile tracks the dashboard filter.
/// With no window it stays all-time — the unfiltered Behaviour page.
#[tokio::test]
async fn model_mix_respects_the_filter_window() {
    let (pool, _db_dir) = common::fixture_pool();

    // Two opus sessions far apart in time — one old, one recent.
    let old_ms = 1_700_000_000_000;
    let recent_ms = 1_777_000_000_000;
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "old".into(),
            started_at_ms: Some(old_ms),
            model: "claude-opus-4-7".into(),
            input_tokens: 100,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "recent".into(),
            started_at_ms: Some(recent_ms),
            model: "claude-opus-4-7".into(),
            input_tokens: 100,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);

    // No window → all-time → both sessions counted.
    let (status, all) = get_json(router.clone(), "/api/behaviour/model-mix").await;
    assert_eq!(status, StatusCode::OK, "body: {all}");
    assert_eq!(
        all["by_model"][0]["invocations"], 2,
        "all-time model-mix must count both sessions: {all}"
    );

    // Windowed to the recent session only → just that one.
    let (status, win) = get_json(
        router,
        &format!(
            "/api/behaviour/model-mix?from={}&to={}",
            recent_ms,
            recent_ms + 1
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {win}");
    assert_eq!(
        win["by_model"][0]["invocations"], 1,
        "windowed model-mix must count only the in-window session: {win}"
    );
}
