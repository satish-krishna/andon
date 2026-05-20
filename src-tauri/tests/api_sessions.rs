mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// get_json helper (mirrors api_overview.rs)
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
// Time helpers
// ---------------------------------------------------------------------------

/// Returns a unix-ms timestamp N days ago relative to a fixed anchor so tests
/// are deterministic. We use 2025-01-15 noon UTC as the anchor.
fn anchor_ms() -> i64 {
    // 2025-01-15 12:00:00 UTC in unix ms
    1_736_942_400_000
}

fn ms_ago(days: i64) -> i64 {
    anchor_ms() - days * 24 * 60 * 60 * 1000
}

// ---------------------------------------------------------------------------
// 1. list_sessions_returns_all_rows_when_no_filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_sessions_returns_all_rows_when_no_filter() {
    let (pool, _db_dir) = common::fixture_pool();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "ls-1".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 1.0,
            input_tokens: 200,
            output_tokens: 100,
            model: "claude-sonnet".into(),
            decisions: vec![("accept", "rust"), ("reject", "rust")],
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "ls-2".into(),
            started_at_ms: Some(ms_ago(1)),
            cost_usd: 0.5,
            input_tokens: 50,
            output_tokens: 25,
            model: "claude-haiku".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    // Provide an explicit wide window so current-month default doesn't drop old rows
    let url = format!("/api/sessions?from=0&to={}", anchor_ms() + 1000);
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "body must be an array");

    let sessions = body.as_array().unwrap();
    assert_eq!(sessions.len(), 2, "expected 2 sessions, got {}", sessions.len());

    // Latest first: ls-1 has the higher started_at (anchor_ms > ms_ago(1))
    assert_eq!(sessions[0]["session_id"], "ls-1", "newest session must be first");
    assert_eq!(sessions[1]["session_id"], "ls-2");

    // Verify specific values for ls-1
    assert_eq!(sessions[0]["tokens_input"], 200);
    assert_eq!(sessions[0]["tokens_output"], 100);
    assert_eq!(sessions[0]["accepts"], 1);
    assert_eq!(sessions[0]["rejects"], 1);

    insta::assert_json_snapshot!("list_sessions_shape", &sessions[0], {
        ".started_at" => "[ts]",
        ".ended_at"   => "[ts]",
        ".cost_usd"   => "[cost]",
    });
}

// ---------------------------------------------------------------------------
// 2. list_sessions_from_to_filter_excludes_outside_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_sessions_from_to_filter_excludes_outside_rows() {
    let (pool, _db_dir) = common::fixture_pool();

    let inside_ms = anchor_ms();
    let before_ms = ms_ago(10);
    let after_ms = anchor_ms() + 10 * 24 * 60 * 60 * 1000;

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "filt-inside".into(),
            started_at_ms: Some(inside_ms),
            cost_usd: 1.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "filt-before".into(),
            started_at_ms: Some(before_ms),
            cost_usd: 2.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "filt-after".into(),
            started_at_ms: Some(after_ms),
            cost_usd: 3.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    // Window that includes only inside_ms
    let from = inside_ms - 1000;
    let to = inside_ms + 1000;
    let url = format!("/api/sessions?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);
    let sessions = body.as_array().unwrap();
    assert_eq!(sessions.len(), 1, "only the in-window session should be returned");
    assert_eq!(sessions[0]["session_id"], "filt-inside");
}

// ---------------------------------------------------------------------------
// 3. list_sessions_limit_is_honored
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_sessions_limit_is_honored() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed 5 sessions spread across 5 different milliseconds
    for i in 0..5_i64 {
        common::seed_session(
            &pool,
            &common::SeedOpts {
                session_id: format!("limit-{i}"),
                started_at_ms: Some(anchor_ms() + i * 1000),
                model: "claude-sonnet".into(),
                ..Default::default()
            },
        );
    }

    let (router, _router_dir) = common::test_router(&pool);
    let from = anchor_ms() - 1000;
    let to = anchor_ms() + 10_000;
    let url = format!("/api/sessions?from={from}&to={to}&limit=3");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);
    let sessions = body.as_array().unwrap();
    assert_eq!(sessions.len(), 3, "limit=3 must cap the result to 3 rows");

    // ORDER BY started_at DESC → limit-4 (highest started_at) should be first
    assert_eq!(sessions[0]["session_id"], "limit-4");
}

// ---------------------------------------------------------------------------
// 4. session_detail_returns_correct_data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_detail_returns_correct_data() {
    let (pool, _db_dir) = common::fixture_pool();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "detail-1".into(),
            started_at_ms: Some(anchor_ms()),
            ended_at_ms: Some(anchor_ms() + 60_000),
            cost_usd: 2.5,
            input_tokens: 400,
            output_tokens: 200,
            model: "claude-opus".into(),
            decisions: vec![
                ("accept", "typescript"),
                ("accept", "typescript"),
                ("reject", "typescript"),
            ],
            files: vec![common::FileChange {
                path: "src/main.ts",
                added: 10,
                removed: 3,
            }],
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/sessions/detail-1").await;

    assert_eq!(status, StatusCode::OK);

    // Top-level shape keys
    assert!(body["session"].is_object(), "session key must exist");
    assert!(body["cost_by_model"].is_array(), "cost_by_model must be an array");
    assert!(body["tokens_by_type"].is_array(), "tokens_by_type must be an array");
    assert!(body["tool_decisions"].is_array(), "tool_decisions must be an array");
    assert!(body["files"].is_array(), "files must be an array");
    assert!(body["active_time_seconds"].is_number(), "active_time_seconds must be a number");

    // session summary fields
    let s = &body["session"];
    assert_eq!(s["session_id"], "detail-1");
    assert_eq!(s["tokens_input"], 400);
    assert_eq!(s["tokens_output"], 200);
    assert_eq!(s["accepts"], 2);
    assert_eq!(s["rejects"], 1);

    // cost_by_model — one entry for claude-opus
    let cbm = body["cost_by_model"].as_array().unwrap();
    assert_eq!(cbm.len(), 1);
    assert_eq!(cbm[0]["key"], "claude-opus");

    // tokens_by_type
    let tbt = body["tokens_by_type"].as_array().unwrap();
    let input_row = tbt.iter().find(|r| r["key"] == "input");
    let output_row = tbt.iter().find(|r| r["key"] == "output");
    assert!(input_row.is_some(), "tokens_by_type must have an 'input' entry");
    assert!(output_row.is_some(), "tokens_by_type must have an 'output' entry");
    assert!(
        (input_row.unwrap()["value"].as_f64().unwrap() - 400.0).abs() < 1e-9,
        "input token count"
    );

    // tool_decisions — 3 rows, ordered by timestamp ASC
    let td = body["tool_decisions"].as_array().unwrap();
    assert_eq!(td.len(), 3);

    // files
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["file_path"], "src/main.ts");
    assert_eq!(files[0]["lines_added"], 10);
    assert_eq!(files[0]["lines_removed"], 3);

    insta::assert_json_snapshot!("session_detail_shape", &body, {
        ".session.started_at"        => "[ts]",
        ".session.ended_at"          => "[ts]",
        ".session.cost_usd"          => "[cost]",
        ".cost_by_model[].value"     => "[cost]",
        ".tokens_by_type[].value"    => "[tokens]",
        ".tool_decisions[].timestamp"=> "[ts]",
        ".active_time_seconds"       => "[secs]",
    });
}

// ---------------------------------------------------------------------------
// 5. session_detail_404_for_missing_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_detail_404_for_missing_id() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, _body) = get_json(router, "/api/sessions/does-not-exist").await;

    assert_eq!(status, StatusCode::NOT_FOUND, "missing session must return 404");
}

// ---------------------------------------------------------------------------
// 6. v2_sessions_returns_sessions_and_coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_sessions_returns_sessions_and_coverage() {
    let (pool, _db_dir) = common::fixture_pool();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "v2-1".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 1.5,
            input_tokens: 300,
            output_tokens: 150,
            model: "claude-sonnet".into(),
            decisions: vec![("accept", "rust"), ("abort", "rust")],
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "v2-2".into(),
            started_at_ms: Some(ms_ago(1)),
            cost_usd: 0.25,
            model: "claude-haiku".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let from = ms_ago(5);
    let to = anchor_ms() + 1000;
    let url = format!("/api/v2/sessions?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    // Top-level shape: { sessions: [...], coverage: { total, with_repo } }
    assert!(body["sessions"].is_array(), "sessions key must be an array");
    assert!(body["coverage"].is_object(), "coverage key must be an object");

    let sessions = body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2, "both sessions should be returned");

    // Latest first (ORDER BY started_at DESC)
    assert_eq!(sessions[0]["session_id"], "v2-1");
    assert_eq!(sessions[1]["session_id"], "v2-2");

    // v2 schema has extra fields compared to v1
    let s0 = &sessions[0];
    assert!(s0["aborts"].is_number(), "v2 must include 'aborts' field");
    assert!(s0["duration_seconds"].is_number(), "v2 must include 'duration_seconds' field");
    assert!(s0["api_calls"].is_number(), "v2 must include 'api_calls' field");
    assert!(s0["decisions"].is_number(), "v2 must include 'decisions' field");
    assert_eq!(s0["accepts"], 1);
    assert_eq!(s0["aborts"], 1);

    // coverage
    let cov = &body["coverage"];
    assert_eq!(cov["total"], 2);

    insta::assert_json_snapshot!("v2_sessions_shape", &body, {
        ".sessions[].started_at"       => "[ts]",
        ".sessions[].ended_at"         => "[ts]",
        ".sessions[].cost_usd"         => "[cost]",
        ".sessions[].duration_seconds" => "[secs]",
    });
}

// ---------------------------------------------------------------------------
// 7. v2_sessions_from_to_filter_excludes_outside_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_sessions_from_to_filter_excludes_outside_rows() {
    let (pool, _db_dir) = common::fixture_pool();

    let inside_ms = anchor_ms();
    let outside_ms = ms_ago(60); // 60 days ago — well outside any window

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "v2-filt-in".into(),
            started_at_ms: Some(inside_ms),
            cost_usd: 1.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "v2-filt-out".into(),
            started_at_ms: Some(outside_ms),
            cost_usd: 9.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let from = inside_ms - 1000;
    let to = inside_ms + 1000;
    let url = format!("/api/v2/sessions?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    let sessions = body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "only in-window session must be returned");
    assert_eq!(sessions[0]["session_id"], "v2-filt-in");

    // coverage.total must match the same window
    assert_eq!(body["coverage"]["total"], 1);
}

// ---------------------------------------------------------------------------
// 8. v2_sessions_limit_is_honored
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_sessions_limit_is_honored() {
    let (pool, _db_dir) = common::fixture_pool();

    for i in 0..5_i64 {
        common::seed_session(
            &pool,
            &common::SeedOpts {
                session_id: format!("v2-lim-{i}"),
                started_at_ms: Some(anchor_ms() + i * 1000),
                model: "claude-sonnet".into(),
                ..Default::default()
            },
        );
    }

    let (router, _router_dir) = common::test_router(&pool);
    let from = anchor_ms() - 1000;
    let to = anchor_ms() + 10_000;
    let url = format!("/api/v2/sessions?from={from}&to={to}&limit=2");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);
    let sessions = body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2, "limit=2 must cap the result to 2 rows");

    // ORDER BY started_at DESC → v2-lim-4 should be first
    assert_eq!(sessions[0]["session_id"], "v2-lim-4");
}

// ---------------------------------------------------------------------------
// 9. cost_source_reflects_request_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cost_source_reflects_request_id() {
    let (pool, _db_dir) = common::fixture_pool();

    // OTLP session: seed_session inserts a cost row with request_id = NULL.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-otlp".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 1.0,
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );

    // JSONL session: seeded with no cost row, then a cost row carrying a
    // non-NULL request_id is inserted directly (as JSONL ingest would).
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-jsonl".into(),
            started_at_ms: Some(anchor_ms()),
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO cost_entries (session_id, request_id, timestamp, model, cost_usd) \
             VALUES ('src-jsonl', 'req_abc', 0, 'claude-opus-4-7', 0.5)",
            [],
        )
        .unwrap();
    }

    // No-cost session: seed_session with cost_usd = 0.0 inserts no cost row.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-none".into(),
            started_at_ms: Some(anchor_ms()),
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );

    let from = ms_ago(5);
    let to = anchor_ms() + 1000;

    // --- v2 list endpoint ---
    let (router, _rd) = common::test_router(&pool);
    let (status, body) = get_json(router, &format!("/api/v2/sessions?from={from}&to={to}")).await;
    assert_eq!(status, StatusCode::OK);
    let pick = |id: &str| -> Value {
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["session_id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(pick("src-otlp")["cost_source"], "otlp");
    assert_eq!(pick("src-jsonl")["cost_source"], "jsonl");
    assert!(
        pick("src-none")["cost_source"].is_null(),
        "a session with no cost rows has a null cost_source"
    );

    // --- detail endpoint exposes the same field ---
    let (router2, _rd2) = common::test_router(&pool);
    let (status2, detail) = get_json(router2, "/api/sessions/src-jsonl").await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(detail["session"]["cost_source"], "jsonl");
}
