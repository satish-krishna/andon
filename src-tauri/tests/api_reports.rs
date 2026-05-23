mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
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

/// Send a POST with an empty body and return parsed JSON.
async fn post_empty(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

// ---------------------------------------------------------------------------
// 1. v2_kpis_returns_expected_shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_kpis_returns_expected_shape() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed one session with cost and tokens so the response is non-trivial
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "kpis-1".into(),
            started_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            cost_usd: 1.25,
            input_tokens: 500,
            output_tokens: 250,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/kpis").await;

    assert_eq!(status, StatusCode::OK);

    // Must have the top-level keys
    assert!(body["window"].is_object(), "window key missing");
    assert!(body["cost"].is_object(), "cost key missing");
    assert!(body["sessions"].is_object(), "sessions key missing");
    assert!(body["tokens"].is_object(), "tokens key missing");

    // Cost current must be >= 1.25
    let cost_current = body["cost"]["current"].as_f64().unwrap_or(0.0);
    assert!(
        cost_current >= 1.25 - 1e-9,
        "expected cost current >= 1.25, got {cost_current}"
    );

    // Sessions current must be >= 1
    let sessions_current = body["sessions"]["current"].as_i64().unwrap_or(0);
    assert!(sessions_current >= 1, "expected sessions >= 1, got {sessions_current}");

    insta::assert_json_snapshot!("v2_kpis_shape", &body, {
        ".window.from"           => "[ts]",
        ".window.to"             => "[ts]",
        ".window.label"          => "[label]",
        ".cost.current"          => "[cost]",
        ".cost.previous"         => "[cost]",
        ".cost.delta_pct"        => "[pct]",
        ".cost.projected_eom"    => "[cost]",
        ".cost.day_of_month"     => "[day]",
        ".cost.days_in_month"    => "[days]",
        ".sessions.current"      => "[n]",
        ".sessions.previous"     => "[n]",
        ".sessions.delta_pct"    => "[pct]",
        ".sessions.pace"         => "[n]",
        ".tokens.input.current"  => "[n]",
        ".tokens.input.previous" => "[n]",
        ".tokens.input.delta_pct" => "[pct]",
        ".tokens.output.current" => "[n]",
        ".tokens.output.previous" => "[n]",
        ".tokens.output.delta_pct" => "[pct]",
        ".tokens.cache_read.current"   => "[n]",
        ".tokens.cache_create.current" => "[n]",
    });
}

// ---------------------------------------------------------------------------
// 2. v2_tape_returns_expected_shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_tape_returns_expected_shape() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed a session in the current month
    let now_ms = chrono::Utc::now().timestamp_millis();
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "tape-1".into(),
            started_at_ms: Some(now_ms),
            cost_usd: 0.75,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/tape").await;

    assert_eq!(status, StatusCode::OK);

    // Top-level shape
    assert!(body["month"].is_string(), "month key missing");
    assert!(body["days_in_month"].is_number(), "days_in_month key missing");
    assert!(body["current"].is_array(), "current key must be array");
    assert!(body["previous"].is_array(), "previous key must be array");

    // current array length matches days_in_month
    let days_in_month = body["days_in_month"].as_u64().unwrap();
    let current = body["current"].as_array().unwrap();
    assert_eq!(
        current.len() as u64,
        days_in_month,
        "current array length must match days_in_month"
    );

    // At least one day has the seeded cost
    let total_cost: f64 = current.iter().filter_map(|v| v.as_f64()).sum();
    assert!(
        total_cost >= 0.75 - 1e-9,
        "total cost in current tape should be >= 0.75, got {total_cost}"
    );

    insta::assert_json_snapshot!("v2_tape_shape", &body, {
        ".month"         => "[month]",
        ".days_in_month" => "[days]",
        ".today_day"     => "[day]",
        ".current"       => "[current_vals]",
        ".previous"      => "[prev_vals]",
    });
}

// ---------------------------------------------------------------------------
// 3. get_report_returns_not_exists_for_unseen_session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_report_returns_not_exists_for_unseen_session() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/sessions/no-such-session/report").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["exists"], false, "report should not exist yet");
    assert!(body["path"].is_string(), "path key must be present");
    assert!(body["generated_at"].is_null(), "generated_at must be null for non-existent report");
}

// ---------------------------------------------------------------------------
// 4. generate_report_writes_file_and_get_report_detects_it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn generate_report_writes_file_and_get_report_detects_it() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed a proper session with data so the report template renders
    let now_ms = chrono::Utc::now().timestamp_millis();
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "rpt-gen-1".into(),
            started_at_ms: Some(now_ms),
            ended_at_ms: Some(now_ms + 60_000),
            cost_usd: 0.10,
            input_tokens: 100,
            output_tokens: 50,
            model: "claude-sonnet".into(),
            decisions: vec![("accept", "rust")],
            ..Default::default()
        },
    );

    let (router, router_dir) = common::test_router(&pool);

    // Before generation: exists == false
    let (status, before) = get_json(router.clone(), "/api/sessions/rpt-gen-1/report").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["exists"], false);

    // Generate the report via POST
    let (post_status, post_body) = post_empty(router.clone(), "/api/sessions/rpt-gen-1/report").await;
    assert_eq!(post_status, StatusCode::OK, "generate_report POST failed: {post_body}");
    assert_eq!(post_body["exists"], true, "post response must have exists=true");
    assert!(post_body["generated_at"].is_number(), "generated_at must be a timestamp");

    // Verify the file exists on disk
    let reports_dir = router_dir.path().join("reports");
    let html_path = reports_dir.join("rpt-gen-1.html");
    assert!(
        html_path.exists(),
        "HTML report file should exist at {}", html_path.display()
    );

    // After generation: GET returns exists == true
    let (status2, after) = get_json(router, "/api/sessions/rpt-gen-1/report").await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(after["exists"], true, "GET after generation should show exists=true");
    assert!(after["generated_at"].is_number(), "generated_at must be set after generation");

    insta::assert_json_snapshot!("get_report_exists_shape", &after, {
        ".path"         => "[path]",
        ".generated_at" => "[ts]",
    });
}

// ---------------------------------------------------------------------------
// 5. open_report_returns_not_found_when_report_missing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn open_report_returns_not_found_when_report_missing() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = post_empty(router, "/api/sessions/nonexistent-session/report/open").await;

    // Handler always returns 200 (no HTTP error codes for this route), but ok == false
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["ok"], false,
        "open_report should return ok=false when file doesn't exist: {body}"
    );
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "error message should indicate 'not found', got: {body}"
    );
}

// ---------------------------------------------------------------------------
// 6. open_report_returns_ok_when_report_exists
//    We generate the report first via POST, then call /open.
//    On Windows the `cmd /C start` is launched but we cannot block on it;
//    the important thing is the validation step returns ok=true.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "shells out to the OS to open a browser; would hang or pop UI in CI/dev"]
async fn open_report_returns_ok_when_report_exists() {
    let (pool, _db_dir) = common::fixture_pool();

    let now_ms = chrono::Utc::now().timestamp_millis();
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "rpt-open-1".into(),
            started_at_ms: Some(now_ms),
            cost_usd: 0.05,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);

    // First generate the report so the file exists
    let (gen_status, _) = post_empty(router.clone(), "/api/sessions/rpt-open-1/report").await;
    assert_eq!(gen_status, StatusCode::OK, "report generation failed");

    // Now open it — we only assert validation logic (ok == true), not that a
    // browser window actually opened.
    let (status, body) = post_empty(router, "/api/sessions/rpt-open-1/report/open").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["ok"], true,
        "open_report should return ok=true when file exists: {body}"
    );
}

// ---------------------------------------------------------------------------
// 7. reports_index_returns_empty_when_no_reports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reports_index_returns_empty_when_no_reports() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/sessions/reports/index").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["session_ids"].is_array(), "session_ids key must be an array");
    assert_eq!(
        body["session_ids"].as_array().unwrap().len(),
        0,
        "should be empty when no reports have been generated"
    );
}

// ---------------------------------------------------------------------------
// 8. reports_index_lists_generated_reports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reports_index_lists_generated_reports() {
    let (pool, _db_dir) = common::fixture_pool();

    let now_ms = chrono::Utc::now().timestamp_millis();
    for i in 1..=3 {
        common::seed_session(
            &pool,
            &common::SeedOpts {
                session_id: format!("idx-session-{i}"),
                started_at_ms: Some(now_ms + i * 1000),
                cost_usd: 0.01 * i as f64,
                model: "claude-sonnet".into(),
                ..Default::default()
            },
        );
    }

    let (router, _router_dir) = common::test_router(&pool);

    // Generate reports for two of the three sessions
    let (s1, _) = post_empty(router.clone(), "/api/sessions/idx-session-1/report").await;
    let (s2, _) = post_empty(router.clone(), "/api/sessions/idx-session-2/report").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);

    let (status, body) = get_json(router, "/api/sessions/reports/index").await;

    assert_eq!(status, StatusCode::OK);
    let ids = body["session_ids"].as_array().expect("session_ids must be array");
    assert_eq!(ids.len(), 2, "two reports should be listed, got {}", ids.len());

    let id_strs: Vec<&str> = ids.iter().filter_map(|v| v.as_str()).collect();
    assert!(id_strs.contains(&"idx-session-1"), "idx-session-1 should be in index");
    assert!(id_strs.contains(&"idx-session-2"), "idx-session-2 should be in index");
    assert!(!id_strs.contains(&"idx-session-3"), "idx-session-3 should NOT be in index");

    insta::assert_json_snapshot!("reports_index_shape", &body, {
        ".session_ids" => insta::sorted_redaction(),
    });
}

// ---------------------------------------------------------------------------
// 9. v2_cache_efficiency_returns_expected_shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_cache_efficiency_returns_expected_shape() {
    let (pool, _db_dir) = common::fixture_pool();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cache-1".into(),
            started_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            model: "claude-opus-4-7".into(),
            input_tokens: 1_000_000,
            output_tokens: 200_000,
            cache_read_tokens: 1_000_000,
            cache_create_tokens: 1_000_000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/cache-efficiency").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["tokens"].is_object(), "tokens key missing");
    assert!(body["savings"].is_object(), "savings key missing");

    // hit_ratio = cacheRead / (input + cacheCreation + cacheRead)
    //           = 1e6 / (1e6 + 1e6 + 1e6) = 0.3333...
    let hr = body["hit_ratio"].as_f64().unwrap();
    assert!((hr - 0.3333).abs() < 0.01, "hit_ratio was {hr}");

    // opus: net = gross 13.50 - creation overhead 3.75 = 9.75
    let net = body["savings"]["net"].as_f64().unwrap();
    assert!((net - 9.75).abs() < 1e-6, "net was {net}");

    assert_eq!(body["tokens"]["cache_read"].as_i64().unwrap(), 1_000_000);
    assert_eq!(body["unpriced_cache_tokens"].as_i64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// 10. v2_model_efficiency_buckets_by_dominant_family
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_model_efficiency_buckets_by_dominant_family() {
    let (pool, _db_dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "me-opus".into(),
            started_at_ms: Some(now),
            model: "claude-opus-4-7".into(),
            output_tokens: 1000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "me-haiku".into(),
            started_at_ms: Some(now),
            model: "claude-haiku-4-5".into(),
            output_tokens: 500,
            cost_usd: 1.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/model-efficiency").await;

    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("response must be an array");
    assert_eq!(rows.len(), 2, "expected one row per family, got {}", rows.len());

    // sorted by total cost desc -> opus first
    assert_eq!(rows[0]["family"], "opus");
    assert_eq!(rows[0]["sessions"].as_i64().unwrap(), 1);
    assert!((rows[0]["total_cost_usd"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    assert!((rows[0]["cost_per_session"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    // cost per 1k output = 5.0 / 1000 * 1000 = 5.0
    assert!((rows[0]["cost_per_1k_output"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    assert_eq!(rows[1]["family"], "haiku");
}

// ---------------------------------------------------------------------------
// 11. v2_model_efficiency_respects_model_filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_model_efficiency_respects_model_filter() {
    let (pool, _db_dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "mf-opus".into(),
            started_at_ms: Some(now),
            model: "claude-opus-4-7".into(),
            output_tokens: 1000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "mf-haiku".into(),
            started_at_ms: Some(now),
            model: "claude-haiku-4-5".into(),
            output_tokens: 500,
            cost_usd: 1.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/model-efficiency?models=opus").await;

    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("response must be an array");
    assert_eq!(rows.len(), 1, "model filter should leave only the opus family");
    assert_eq!(rows[0]["family"], "opus");
}

// ---------------------------------------------------------------------------
// 12. v2_cache_efficiency_respects_model_filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_cache_efficiency_respects_model_filter() {
    let (pool, _db_dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cf-opus".into(),
            started_at_ms: Some(now),
            model: "claude-opus-4-7".into(),
            input_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            cache_create_tokens: 1_000_000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cf-haiku".into(),
            started_at_ms: Some(now),
            model: "claude-haiku-4-5".into(),
            input_tokens: 2_000_000,
            cache_read_tokens: 2_000_000,
            cache_create_tokens: 2_000_000,
            cost_usd: 1.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/cache-efficiency?models=opus").await;

    assert_eq!(status, StatusCode::OK);
    // Only the opus session's cache tokens should be counted (haiku excluded).
    assert_eq!(body["tokens"]["cache_read"].as_i64().unwrap(), 1_000_000);
    assert_eq!(body["tokens"]["cache_create"].as_i64().unwrap(), 1_000_000);
    // net savings = opus only: gross 13.50 - creation overhead 3.75 = 9.75
    let net = body["savings"]["net"].as_f64().unwrap();
    assert!((net - 9.75).abs() < 1e-6, "net was {net}");
}

// ---------------------------------------------------------------------------
// 13. jsonl_subagent_record_upserts_is_subagent_flag
// ---------------------------------------------------------------------------

#[test]
fn jsonl_subagent_record_upserts_is_subagent_flag() {
    use andon_lib::jsonl::reducer::DerivedEvent;
    use andon_lib::jsonl::reconciler::Coverage;
    let (pool, _db_dir) = common::fixture_pool();

    // Pre-seed a cost_entries / token_usage row with is_subagent=0 by hand,
    // simulating a row OTLP wrote first.
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().expect("conn");
    conn.execute(
        "INSERT INTO cost_entries (session_id, request_id, timestamp, model, cost_usd, is_subagent)
         VALUES ('s1', 'req_x', ?, 'claude-opus-4-7', 1.0, 0)",
        rusqlite::params![now],
    ).expect("insert pre-existing cost row");
    conn.execute(
        "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count, is_subagent)
         VALUES ('s1', 'req_x', ?, 'claude-opus-4-7', 'input', 100, 0)",
        rusqlite::params![now],
    ).expect("insert pre-existing token row");
    drop(conn);

    // Now re-ingest the same request_id via the JSONL path with is_subagent=true.
    let ingestor = common::test_ingestor(&pool);
    let events = vec![
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: now,
            model: "claude-opus-4-7".into(),
            cost_usd: 1.0,
            is_subagent: true,
        },
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: now,
            model: "claude-opus-4-7".into(),
            input: 100,
            output: 0,
            cache_create: 0,
            cache_read: 0,
            is_subagent: true,
        },
    ];
    ingestor.ingest_derived(&events, Coverage::JsonlOnly).expect("ingest");

    // The rows should now be is_subagent=1.
    let conn = pool.get().expect("conn");
    let cost_flag: i64 = conn.query_row(
        "SELECT is_subagent FROM cost_entries WHERE request_id = 'req_x'",
        [], |r| r.get(0),
    ).expect("query cost");
    let token_flag: i64 = conn.query_row(
        "SELECT is_subagent FROM token_usage WHERE request_id = 'req_x' AND token_type = 'input'",
        [], |r| r.get(0),
    ).expect("query token");
    assert_eq!(cost_flag, 1, "cost row should be upserted to is_subagent=1");
    assert_eq!(token_flag, 1, "token row should be upserted to is_subagent=1");
}
