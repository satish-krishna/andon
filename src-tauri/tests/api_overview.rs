mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// get_json helper
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
// Helper: millisecond timestamp for today at midnight (local time)
// ---------------------------------------------------------------------------

fn today_midnight_ms() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    let start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists");
    Local
        .from_local_datetime(&start)
        .single()
        .map(|dt| dt.timestamp_millis())
        .expect("local midnight is unambiguous")
}

/// Timestamp in the middle of today (noon local time).
fn today_noon_ms() -> i64 {
    today_midnight_ms() + 12 * 60 * 60 * 1000
}

/// Timestamp 25 hours before midnight today → clearly yesterday.
fn yesterday_noon_ms() -> i64 {
    today_midnight_ms() - 13 * 60 * 60 * 1000
}

// ---------------------------------------------------------------------------
// 1. overview_today_aggregates_only_today_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overview_today_aggregates_only_today_rows() {
    let (pool, _db_dir) = common::fixture_pool();

    // Today row
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "today-1".into(),
            started_at_ms: Some(today_noon_ms()),
            cost_usd: 0.5,
            input_tokens: 100,
            output_tokens: 50,
            model: "claude-sonnet".into(),
            decisions: vec![("accept", "rust"), ("reject", "rust")],
            ..Default::default()
        },
    );

    // Yesterday row — must NOT appear in today's totals
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "yesterday-1".into(),
            started_at_ms: Some(yesterday_noon_ms()),
            cost_usd: 9.9,
            input_tokens: 9000,
            output_tokens: 9000,
            model: "claude-sonnet".into(),
            decisions: vec![("accept", "rust"), ("accept", "rust"), ("accept", "rust")],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/overview/today").await;

    assert_eq!(status, StatusCode::OK);

    // Snapshot the shape
    insta::assert_json_snapshot!("overview_today_shape", &body, {
        ".cost_usd" => "[cost]",
        ".sessions" => "[sessions]",
        ".accept_rate" => "[rate]",
        ".tokens_input" => "[tokens_input]",
        ".tokens_output" => "[tokens_output]",
    });

    // Numeric assertions — only today row values
    assert_eq!(body["sessions"], 1);
    assert!(
        (body["cost_usd"].as_f64().unwrap() - 0.5).abs() < 1e-9,
        "cost_usd should be 0.5, got {}",
        body["cost_usd"]
    );
    assert_eq!(body["tokens_input"], 100);
    assert_eq!(body["tokens_output"], 50);

    // accept_rate: 1 accept, 1 reject, 0 aborts → 1/2 = 0.5
    let rate = body["accept_rate"].as_f64().unwrap();
    assert!((rate - 0.5).abs() < 1e-9, "accept_rate should be 0.5, got {rate}");
}

// ---------------------------------------------------------------------------
// 2. cost_by_day_groups_by_day_and_model
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cost_by_day_groups_by_day_and_model() {
    let (pool, _db_dir) = common::fixture_pool();

    let today = today_midnight_ms();
    let day_ms = 24 * 60 * 60 * 1000_i64;

    // Today, model A: 1.0
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cbd-1".into(),
            started_at_ms: Some(today + 1000),
            cost_usd: 1.0,
            model: "model-a".into(),
            ..Default::default()
        },
    );
    // Yesterday, model A: 0.5
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cbd-2".into(),
            started_at_ms: Some(today - day_ms + 1000),
            cost_usd: 0.5,
            model: "model-a".into(),
            ..Default::default()
        },
    );
    // Yesterday, model B: 2.0
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cbd-3".into(),
            started_at_ms: Some(today - day_ms + 2000),
            cost_usd: 2.0,
            model: "model-b".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/overview/cost-by-day?days=7").await;

    assert_eq!(status, StatusCode::OK);

    // Shape: { days: [...], series: [ { name, values }, ... ] }
    assert!(body["days"].is_array(), "days must be an array");
    assert!(body["series"].is_array(), "series must be an array");

    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 7, "7 day labels");

    let series = body["series"].as_array().unwrap();
    // model-a and model-b should appear
    let model_a = series.iter().find(|s| s["name"] == "model-a");
    let model_b = series.iter().find(|s| s["name"] == "model-b");
    assert!(model_a.is_some(), "series should contain model-a");
    assert!(model_b.is_some(), "series should contain model-b");

    let a_values = model_a.unwrap()["values"].as_array().unwrap();
    assert_eq!(a_values.len(), 7, "model-a must have 7 values");

    // Last element = today, second-to-last = yesterday
    let today_idx = a_values.len() - 1;
    let yesterday_idx = a_values.len() - 2;

    let a_today = a_values[today_idx].as_f64().unwrap();
    let a_yesterday = a_values[yesterday_idx].as_f64().unwrap();
    assert!((a_today - 1.0).abs() < 1e-9, "model-a today should be 1.0, got {a_today}");
    assert!((a_yesterday - 0.5).abs() < 1e-9, "model-a yesterday should be 0.5, got {a_yesterday}");

    let b_values = model_b.unwrap()["values"].as_array().unwrap();
    let b_yesterday = b_values[yesterday_idx].as_f64().unwrap();
    assert!((b_yesterday - 2.0).abs() < 1e-9, "model-b yesterday should be 2.0, got {b_yesterday}");

    insta::assert_json_snapshot!("cost_by_day_shape", &body["series"][0]["name"]);
}

// ---------------------------------------------------------------------------
// 3. tokens_by_day_splits_by_type
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tokens_by_day_splits_by_type() {
    let (pool, _db_dir) = common::fixture_pool();

    let today = today_midnight_ms();

    // 300 input tokens + 150 output tokens today
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "tbd-1".into(),
            started_at_ms: Some(today + 1000),
            input_tokens: 300,
            output_tokens: 150,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/overview/tokens-by-day?days=3").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["days"].is_array());
    assert!(body["series"].is_array());

    let series = body["series"].as_array().unwrap();
    let input_series = series.iter().find(|s| s["name"] == "input");
    let output_series = series.iter().find(|s| s["name"] == "output");

    assert!(input_series.is_some(), "series must include 'input'");
    assert!(output_series.is_some(), "series must include 'output'");

    let input_vals = input_series.unwrap()["values"].as_array().unwrap();
    let output_vals = output_series.unwrap()["values"].as_array().unwrap();

    // Today is last element in a 3-day window
    let today_idx = input_vals.len() - 1;
    let input_today = input_vals[today_idx].as_f64().unwrap();
    let output_today = output_vals[today_idx].as_f64().unwrap();

    assert!((input_today - 300.0).abs() < 1e-9, "input today={input_today}");
    assert!((output_today - 150.0).abs() < 1e-9, "output today={output_today}");

    insta::assert_json_snapshot!("tokens_by_day_shape", {
        ".days" => "[days]",
        ".series[].values" => "[values]",
    }, &body);
}

// ---------------------------------------------------------------------------
// 4. accept_by_language_computes_ratio
// ---------------------------------------------------------------------------

#[tokio::test]
async fn accept_by_language_computes_ratio() {
    let (pool, _db_dir) = common::fixture_pool();

    // Rust: 3 accept, 1 reject, 0 abort  → rate = 3/4 = 0.75,  total = 4
    // Python: 1 accept, 0 reject, 1 abort → rate = 1/2 = 0.5,  total = 2
    // Go:    0 accept, 2 reject, 0 abort  → rate = 0/2 = 0.0,  total = 2
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "abl-1".into(),
            started_at_ms: Some(today_noon_ms()),
            model: "claude-sonnet".into(),
            decisions: vec![
                ("accept", "rust"),
                ("accept", "rust"),
                ("accept", "rust"),
                ("reject", "rust"),
                ("accept", "python"),
                ("abort", "python"),
                ("reject", "go"),
                ("reject", "go"),
            ],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/overview/accept-by-language").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "response must be an array");

    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 3, "three languages expected");

    // Helper: find by language name
    let find_lang = |lang: &str| {
        rows.iter().find(|r| r["language"] == lang).cloned()
    };

    let rust_row = find_lang("rust").expect("rust row present");
    let python_row = find_lang("python").expect("python row present");
    let go_row = find_lang("go").expect("go row present");

    // totals
    assert_eq!(rust_row["total"], 4);
    assert_eq!(python_row["total"], 2);
    assert_eq!(go_row["total"], 2);

    // accept_rate — rounded to 4 decimal places by the handler
    let rust_rate = rust_row["accept_rate"].as_f64().unwrap();
    let python_rate = python_row["accept_rate"].as_f64().unwrap();
    let go_rate = go_row["accept_rate"].as_f64().unwrap();

    assert!((rust_rate - 0.75).abs() < 1e-4, "rust rate={rust_rate}");
    assert!((python_rate - 0.5).abs() < 1e-4, "python rate={python_rate}");
    assert!((go_rate - 0.0).abs() < 1e-9, "go rate={go_rate}");

    insta::assert_json_snapshot!("accept_by_language_shape", {
        "[].accept_rate" => "[rate]",
        "[].total" => "[total]",
    }, &body);
}

// ---------------------------------------------------------------------------
// 5. active_time_today_splits_by_kind
// ---------------------------------------------------------------------------

#[tokio::test]
async fn active_time_today_splits_by_kind() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed a session so the session_id FK is valid (sessions table is referenced)
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "att-1".into(),
            started_at_ms: Some(today_noon_ms()),
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    // Insert active_time rows directly — seed_session does not cover these
    let ts = today_noon_ms();
    {
        let conn = pool.get().expect("checkout connection");
        conn.execute(
            "INSERT INTO active_time (session_id, timestamp, seconds, kind) VALUES (?, ?, ?, ?)",
            params!["att-1", ts, 120.0_f64, "user"],
        )
        .expect("insert user active_time");
        conn.execute(
            "INSERT INTO active_time (session_id, timestamp, seconds, kind) VALUES (?, ?, ?, ?)",
            params!["att-1", ts, 45.0_f64, "cli"],
        )
        .expect("insert cli active_time");
        // Also insert a yesterday row — should NOT appear in today's totals
        let yesterday_ts = yesterday_noon_ms();
        conn.execute(
            "INSERT INTO active_time (session_id, timestamp, seconds, kind) VALUES (?, ?, ?, ?)",
            params!["att-1", yesterday_ts, 999.0_f64, "user"],
        )
        .expect("insert yesterday user active_time");
    }

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/overview/active-time/today").await;

    assert_eq!(status, StatusCode::OK);

    insta::assert_json_snapshot!("active_time_today_shape", {
        ".user_seconds" => "[user_seconds]",
        ".cli_seconds" => "[cli_seconds]",
    }, &body);

    let user_s = body["user_seconds"].as_f64().unwrap();
    let cli_s = body["cli_seconds"].as_f64().unwrap();

    assert!((user_s - 120.0).abs() < 1e-9, "user_seconds={user_s}");
    assert!((cli_s - 45.0).abs() < 1e-9, "cli_seconds={cli_s}");

    // Total should exclude yesterday's 999 s
    let total = user_s + cli_s;
    assert!((total - 165.0).abs() < 1e-9, "total={total}");
}
