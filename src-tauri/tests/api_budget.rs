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
