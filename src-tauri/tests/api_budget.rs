mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
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

async fn put_json(router: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    use axum::http::Method;
    let req = Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    };
    (status, v)
}

#[tokio::test]
async fn v2_kpis_includes_budget_block() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/v2/kpis").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let budget = &body["cost"]["budget"];
    assert!(budget.is_object(), "cost.budget must be present: {body}");
    assert_eq!(budget["monthly_usd"], json!(0.0), "default budget is 0");
    assert_eq!(
        budget["status"], json!("neutral"),
        "status is neutral with no budget set"
    );
}

#[tokio::test]
async fn put_budget_round_trip() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (put_status, put_body) =
        put_json(router.clone(), "/api/settings/budget", json!({ "monthly_usd": 150.0 })).await;
    assert_eq!(put_status, StatusCode::OK, "PUT body: {put_body}");
    assert_eq!(put_body["monthly_usd"], json!(150.0));

    let (get_status, get_body) = get_json(router, "/api/settings").await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(
        get_body["budget"]["monthly_usd"], json!(150.0),
        "budget must persist into /api/settings"
    );
}

#[tokio::test]
async fn put_budget_rejects_negative() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) =
        put_json(router, "/api/settings/budget", json!({ "monthly_usd": -5.0 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// `projected_eom` is a month-end figure: it must reflect the true
/// month-to-date spend, never the dashboard's period filter. Two requests
/// with different `from`/`to` windows must report the same projection.
#[tokio::test]
async fn v2_kpis_projection_is_independent_of_filter_window() {
    use chrono::{Datelike, Local, TimeZone};

    let (pool, _db_dir) = common::fixture_pool();

    let now = Local::now();
    let now_ms = now.timestamp_millis();
    // Two cost rows a second apart — both in the current month.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cost-a".into(),
            started_at_ms: Some(now_ms - 1_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 40.0,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cost-b".into(),
            started_at_ms: Some(now_ms - 2_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 25.0,
            ..Default::default()
        },
    );

    let (router, _dir) = common::test_router(&pool);

    let month_start = Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("valid month start")
        .timestamp_millis();

    // FULL window captures both rows; NARROW captures only the newer one.
    let (_s1, full) = get_json(
        router.clone(),
        &format!("/api/v2/kpis?from={month_start}&to={}", now_ms + 1),
    )
    .await;
    let (_s2, narrow) = get_json(
        router,
        &format!("/api/v2/kpis?from={}&to={}", now_ms - 1_500, now_ms + 1),
    )
    .await;

    // Precondition: the filter genuinely changes the windowed cost.
    assert_ne!(
        full["cost"]["current"], narrow["cost"]["current"],
        "the two windows must capture different cost for this test to mean anything"
    );
    // The fix: the month-end projection must NOT depend on the filter window.
    assert_eq!(
        full["cost"]["projected_eom"], narrow["cost"]["projected_eom"],
        "projected_eom must be the month-to-date projection regardless of filter"
    );
    // ...and neither does the session pace.
    assert_eq!(
        full["sessions"]["pace"], narrow["sessions"]["pace"],
        "session pace must be the month-to-date pace regardless of filter"
    );
}
