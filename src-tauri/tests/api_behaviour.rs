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

    // tool_decisions rows with a non-NULL `model` so the by_model_tool half of
    // the response — and the window filter applied to it — is exercised too.
    {
        let conn = pool.get().expect("checkout connection");
        for (session_id, ts) in [("old", old_ms), ("recent", recent_ms)] {
            conn.execute(
                "INSERT INTO tool_decisions \
                 (session_id, timestamp, tool_name, decision, model) \
                 VALUES (?, ?, 'Edit', 'accept', 'claude-opus-4-7')",
                rusqlite::params![session_id, ts],
            )
            .expect("insert tool_decisions");
        }
    }

    let (router, _router_dir) = common::test_router(&pool);

    // No window → all-time → both sessions counted on each half of the
    // response (by_model from token_usage, by_model_tool from tool_decisions).
    let (status, all) = get_json(router.clone(), "/api/behaviour/model-mix").await;
    assert_eq!(status, StatusCode::OK, "body: {all}");
    assert_eq!(
        all["by_model"][0]["invocations"], 2,
        "all-time model-mix must count both sessions: {all}"
    );
    assert_eq!(
        all["by_model_tool"][0]["count"], 2,
        "all-time model-mix must count both tool decisions: {all}"
    );

    // Windowed to the recent session only → just that one, on each half.
    let (status, win) = get_json(
        router.clone(),
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
    assert_eq!(
        win["by_model_tool"][0]["count"], 1,
        "windowed model-mix must count only the in-window tool decision: {win}"
    );

    // A lone `from` bound is applied on its own — not ignored as all-time:
    // the recent session onward, excluding the old one, on each half.
    let (status, from_only) =
        get_json(router, &format!("/api/behaviour/model-mix?from={recent_ms}")).await;
    assert_eq!(status, StatusCode::OK, "body: {from_only}");
    assert_eq!(
        from_only["by_model"][0]["invocations"], 1,
        "a from-only window must exclude the old session: {from_only}"
    );
    assert_eq!(
        from_only["by_model_tool"][0]["count"], 1,
        "a from-only window must exclude the old tool decision: {from_only}"
    );
}
