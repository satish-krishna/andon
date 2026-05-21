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
