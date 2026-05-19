mod common;

use andon_lib::{
    diagnostics::Diagnostics,
    otlp::IngestionControl,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// HTTP helper
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

// ---------------------------------------------------------------------------
// 1. get_diagnostics_returns_snapshot_shape
// ---------------------------------------------------------------------------

/// GET /api/diagnostics always returns a valid snapshot even with an empty DB.
#[tokio::test]
async fn get_diagnostics_returns_snapshot_shape() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/diagnostics").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // Required top-level fields from DiagnosticsSnapshot
    assert!(body["start_ms"].is_number(), "start_ms should be present");
    assert!(body["uptime_seconds"].is_number(), "uptime_seconds should be present");
    assert!(body["total_records"].is_number(), "total_records should be present");
    assert!(body["total_payloads"].is_number(), "total_payloads should be present");
    assert!(body["event_counters"].is_object(), "event_counters should be an object");
    assert!(body["transport_counters"].is_object(), "transport_counters should be an object");
    assert!(body["binds"].is_object(), "binds should be present");
    assert!(body["health"].is_object(), "health should be present");
}

// ---------------------------------------------------------------------------
// 2. diagnostics_reflects_seeded_payloads
// ---------------------------------------------------------------------------

/// Share a `Diagnostics` instance between an ingestor and the router so that
/// records ingested before the request are visible in the snapshot.
#[tokio::test]
async fn diagnostics_reflects_seeded_payloads() {
    let (pool, _db_dir) = common::fixture_pool();

    // Create shared instances
    let diagnostics = Diagnostics::default();
    let control = IngestionControl::default();

    // Build ingestor wired to the same Diagnostics
    let ingestor = common::test_ingestor_with(&pool, diagnostics.clone(), control.clone());

    // Build router wired to the same Diagnostics
    let (router, _router_dir) =
        common::test_router_with(&pool, diagnostics.clone(), control.clone());

    // Ingest a payload so the diagnostics counter increments
    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "diag-sess-1")],
        "api_request",
        vec![
            common::kv("model", "claude-sonnet"),
            common::kv_f64("cost_usd", 0.01),
            common::kv_int("input_tokens", 10),
            common::kv_int("output_tokens", 5),
            common::kv_int("duration_ms", 200),
        ],
    );
    ingestor.ingest_logs_v2(payload, "http").expect("ingest");

    let (status, body) = get_json(router, "/api/diagnostics").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // total_records should be >= 1 now
    let total = body["total_records"].as_u64().unwrap_or(0);
    assert!(total >= 1, "expected total_records >= 1, got {body}");

    // last_payload should be present
    assert!(!body["last_payload"].is_null(), "last_payload should not be null after ingestion");
}

// ---------------------------------------------------------------------------
// 3. get_diagnostics_events_returns_events_array
// ---------------------------------------------------------------------------

/// Seed a log event via the ingestor, then GET /api/diagnostics/events and
/// verify the response contains the `events` array with the seeded record.
#[tokio::test]
async fn get_diagnostics_events_returns_events_array() {
    let (pool, _db_dir) = common::fixture_pool();

    // Use test_ingestor (separate Diagnostics is fine — events come from the DB)
    let ingestor = common::test_ingestor(&pool);
    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "diag-events-sess")],
        "tool_use",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/diagnostics/events").await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let events = body["events"].as_array().expect("events key must be an array");
    assert!(!events.is_empty(), "expected at least one event");

    let ev = &events[0];
    assert!(ev["event_name"].is_string(), "event_name should be a string");
    assert!(ev["timestamp"].is_number(), "timestamp should be a number");
    assert!(ev["transport"].is_string(), "transport should be a string");
}

// ---------------------------------------------------------------------------
// 4. get_diagnostics_export_returns_bundle
// ---------------------------------------------------------------------------

/// GET /api/diagnostics/export bundles diagnostics + recent events in one object.
#[tokio::test]
async fn get_diagnostics_export_returns_bundle() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed one event so recent_events is non-empty
    let ingestor = common::test_ingestor(&pool);
    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "diag-export-sess")],
        "session_start",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "http").expect("ingest");

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/diagnostics/export").await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body["generated_at"].is_string(), "generated_at should be a string");
    assert!(body["diagnostics"].is_object(), "diagnostics key should be an object");
    assert!(body["recent_events"].is_array(), "recent_events should be an array");
    assert!(body["db_path"].is_string(), "db_path should be a string");

    let events = body["recent_events"].as_array().unwrap();
    assert!(!events.is_empty(), "recent_events should contain the seeded record");
}
