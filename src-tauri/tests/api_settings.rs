mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 1. get_settings_returns_defaults
// ---------------------------------------------------------------------------

/// Fresh tempdir — GET /api/settings should return the default AppSettings
/// shape with forwarder.enabled = false and timeout_ms = 2000.
#[tokio::test]
async fn get_settings_returns_defaults() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/settings").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    assert_eq!(body["version"], json!(1), "version field mismatch");
    let fwd = &body["forwarder"];
    assert_eq!(fwd["enabled"], json!(false), "forwarder.enabled default should be false");
    assert_eq!(fwd["timeout_ms"], json!(2000), "forwarder.timeout_ms default should be 2000");
    assert_eq!(fwd["endpoint"], json!(""), "forwarder.endpoint default should be empty");
    assert!(fwd["headers"].is_object(), "forwarder.headers should be an object");
}

// ---------------------------------------------------------------------------
// 2. put_forwarder_round_trip
// ---------------------------------------------------------------------------

/// PUT new forwarder settings, then GET /api/settings and verify the value
/// was persisted — both in the response body of PUT and the subsequent GET.
#[tokio::test]
async fn put_forwarder_round_trip() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let new_settings = json!({
        "enabled": true,
        "endpoint": "http://otel.example.com:4318",
        "timeout_ms": 5000,
        "headers": {
            "x-custom-header": "test-value"
        }
    });

    // PUT and check response
    let (put_status, put_body) = put_json(router.clone(), "/api/settings/forwarder", new_settings).await;
    assert_eq!(put_status, StatusCode::OK, "PUT body: {put_body}");
    assert_eq!(put_body["enabled"], json!(true));
    assert_eq!(put_body["endpoint"], json!("http://otel.example.com:4318"));
    assert_eq!(put_body["timeout_ms"], json!(5000));
    assert_eq!(put_body["headers"]["x-custom-header"], json!("test-value"));

    // GET and verify persistence
    let (get_status, get_body) = get_json(router, "/api/settings").await;
    assert_eq!(get_status, StatusCode::OK, "GET body: {get_body}");
    let fwd = &get_body["forwarder"];
    assert_eq!(fwd["enabled"], json!(true), "persisted enabled");
    assert_eq!(fwd["endpoint"], json!("http://otel.example.com:4318"), "persisted endpoint");
    assert_eq!(fwd["timeout_ms"], json!(5000), "persisted timeout_ms");
    assert_eq!(fwd["headers"]["x-custom-header"], json!("test-value"), "persisted header");
}
