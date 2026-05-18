mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// get_json helper (mirrors api_overview.rs / api_sessions.rs)
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

/// Unix-ms timestamp for "now" (used as the recent anchor).
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Timestamp `days` full days before now (in ms).
fn days_ago_ms(days: i64) -> i64 {
    now_ms() - days * 24 * 60 * 60 * 1000
}

// ---------------------------------------------------------------------------
// Utility: insert a tool_decision row with an explicit file_path
// ---------------------------------------------------------------------------

fn insert_decision_with_path(
    pool: &std::sync::Arc<andon_lib::db::DbPool>,
    session_id: &str,
    timestamp: i64,
    file_path: &str,
    decision: &str,
    language: &str,
) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO tool_decisions \
         (session_id, timestamp, tool_name, decision, language, file_path) \
         VALUES (?, ?, 'Edit', ?, ?, ?)",
        params![session_id, timestamp, decision, language, file_path],
    )
    .expect("insert tool_decision with file_path");
}

// ---------------------------------------------------------------------------
// 1. files_heatmap_returns_correct_shape_and_values
// ---------------------------------------------------------------------------

/// Seed two files inside the 30-day window and assert the heatmap returns both
/// with correct edit counts and accept_rate values.
#[tokio::test]
async fn files_heatmap_returns_correct_shape_and_values() {
    let (pool, _db_dir) = common::fixture_pool();

    // Anchor: 3 days ago (well inside a 30-day window)
    let ts = days_ago_ms(3);

    // Session A: two files
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "hm-1".into(),
            started_at_ms: Some(ts),
            model: "claude-sonnet".into(),
            files: vec![
                common::FileChange { path: "src/main.rs", added: 10, removed: 2 },
                common::FileChange { path: "src/lib.rs",  added: 5,  removed: 1 },
                // Second edit on main.rs → edit_count should be 2
                common::FileChange { path: "src/main.rs", added: 3,  removed: 0 },
            ],
            // decisions without file_path — these will be grouped under '?' key
            // so they won't join to the file rows; that's intentional here.
            ..Default::default()
        },
    );

    // Add decisions keyed to the same file_path so accept_rate is exercised
    insert_decision_with_path(&pool, "hm-1", ts, "src/main.rs", "accept", "rust");
    insert_decision_with_path(&pool, "hm-1", ts, "src/main.rs", "reject", "rust");
    // lib.rs: 2 accepts → rate 1.0
    insert_decision_with_path(&pool, "hm-1", ts, "src/lib.rs", "accept", "rust");
    insert_decision_with_path(&pool, "hm-1", ts, "src/lib.rs", "accept", "rust");

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/files/heatmap?days=30").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "response must be an array");

    let rows = body.as_array().unwrap();
    // We seeded 2 distinct paths + the '?' group from decisions without path
    // (decisions with file_path join to edits, so only 2 edits-rows expected)
    assert!(rows.len() >= 2, "at least 2 file rows expected, got {}", rows.len());

    let find_file = |path: &str| -> Option<&Value> {
        rows.iter().find(|r| r["file_path"] == path)
    };

    let main_row = find_file("src/main.rs").expect("src/main.rs must appear");
    assert_eq!(main_row["edit_count"], 2, "main.rs edited twice");
    let main_rate = main_row["accept_rate"].as_f64().unwrap();
    // 1 accept, 1 reject → 0.5
    assert!((main_rate - 0.5).abs() < 1e-4, "main.rs accept_rate={main_rate}");

    let lib_row = find_file("src/lib.rs").expect("src/lib.rs must appear");
    assert_eq!(lib_row["edit_count"], 1, "lib.rs edited once");
    let lib_rate = lib_row["accept_rate"].as_f64().unwrap();
    // 2 accepts → 1.0
    assert!((lib_rate - 1.0).abs() < 1e-4, "lib.rs accept_rate={lib_rate}");

    // Snapshot the overall shape (redact variable values)
    insta::assert_json_snapshot!("files_heatmap_shape", &body, {
        "[].edit_count"   => "[count]",
        "[].accept_rate"  => "[rate]",
    });
}

// ---------------------------------------------------------------------------
// 2. files_heatmap_days_param_excludes_old_rows
// ---------------------------------------------------------------------------

/// Rows whose timestamp falls outside the `days` window must not appear.
#[tokio::test]
async fn files_heatmap_days_param_excludes_old_rows() {
    let (pool, _db_dir) = common::fixture_pool();

    // "inside" row: 2 days ago — inside a days=7 window
    let inside_ts = days_ago_ms(2);
    // "outside" row: 40 days ago — outside a days=7 window
    let outside_ts = days_ago_ms(40);

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "hm-win-in".into(),
            started_at_ms: Some(inside_ts),
            model: "claude-sonnet".into(),
            files: vec![common::FileChange { path: "new/file.rs", added: 1, removed: 0 }],
            ..Default::default()
        },
    );

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "hm-win-out".into(),
            started_at_ms: Some(outside_ts),
            model: "claude-sonnet".into(),
            files: vec![common::FileChange { path: "old/file.rs", added: 99, removed: 99 }],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/files/heatmap?days=7").await;

    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().unwrap();

    let paths: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["file_path"].as_str())
        .collect();

    assert!(
        paths.contains(&"new/file.rs"),
        "in-window file must appear; got {paths:?}"
    );
    assert!(
        !paths.contains(&"old/file.rs"),
        "out-of-window file must NOT appear; got {paths:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. files_heatmap_empty_db_returns_empty_array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn files_heatmap_empty_db_returns_empty_array() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/files/heatmap?days=30").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "empty response must still be an array");
    assert_eq!(body.as_array().unwrap().len(), 0, "array must be empty");
}

// ---------------------------------------------------------------------------
// 4. v2_files_returns_correct_shape
// ---------------------------------------------------------------------------

/// Seed one session with file changes and decisions (linked via file_path),
/// then assert the v2/files response has the expected top-level shape and
/// correct totals.
#[tokio::test]
async fn v2_files_returns_correct_shape() {
    let (pool, _db_dir) = common::fixture_pool();

    let ts = days_ago_ms(2);

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "vf-1".into(),
            started_at_ms: Some(ts),
            model: "claude-sonnet".into(),
            files: vec![
                common::FileChange { path: "app/main.ts", added: 20, removed: 5 },
                common::FileChange { path: "app/utils.ts", added: 8, removed: 3 },
            ],
            ..Default::default()
        },
    );

    // Link decisions to file paths so accept_rate is non-zero
    // main.ts: 2 accept, 1 reject → rate = 2/3 ≈ 0.6667
    insert_decision_with_path(&pool, "vf-1", ts, "app/main.ts", "accept", "typescript");
    insert_decision_with_path(&pool, "vf-1", ts, "app/main.ts", "accept", "typescript");
    insert_decision_with_path(&pool, "vf-1", ts, "app/main.ts", "reject", "typescript");
    // utils.ts: 1 accept → rate = 1.0
    insert_decision_with_path(&pool, "vf-1", ts, "app/utils.ts", "accept", "typescript");

    let (router, _router_dir) = common::test_router(&pool);
    // Provide an explicit time window covering the seeded data
    let from = ts - 1000;
    let to = ts + 86_400_000; // +1 day
    let url = format!("/api/v2/files?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    // Top-level shape
    assert!(body["files"].is_array(), "files key must be an array");
    assert!(body["lang_breakdown"].is_array(), "lang_breakdown must be an array");
    assert!(body["totals"].is_object(), "totals must be an object");

    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "expected 2 file rows");

    // Default sort is by edits DESC, both have 1 edit, so order may vary
    let find_file = |path: &str| -> Option<&Value> {
        files.iter().find(|r| r["file_path"] == path)
    };

    let main_row = find_file("app/main.ts").expect("app/main.ts must appear");
    assert_eq!(main_row["edits"], 1);
    assert_eq!(main_row["added"], 20);
    assert_eq!(main_row["removed"], 5);
    let main_rate = main_row["accept_rate"].as_f64().unwrap();
    // 2 accept, 1 reject → 2/3 ≈ 0.6667
    assert!(
        (main_rate - 2.0 / 3.0).abs() < 1e-4,
        "main.ts accept_rate={main_rate}"
    );
    assert_eq!(main_row["decision_count"], 3);

    let utils_row = find_file("app/utils.ts").expect("app/utils.ts must appear");
    assert_eq!(utils_row["edits"], 1);
    let utils_rate = utils_row["accept_rate"].as_f64().unwrap();
    assert!((utils_rate - 1.0).abs() < 1e-4, "utils.ts accept_rate={utils_rate}");

    // totals
    let totals = &body["totals"];
    assert_eq!(totals["files"], 2);
    assert_eq!(totals["edits"], 2);
    assert_eq!(totals["added"], 28);
    assert_eq!(totals["removed"], 8);

    // lang_breakdown — should have an entry for 'typescript'
    let lb = body["lang_breakdown"].as_array().unwrap();
    let ts_entry = lb.iter().find(|e| e["lang"] == "typescript");
    assert!(ts_entry.is_some(), "lang_breakdown must include 'typescript'");
    assert_eq!(ts_entry.unwrap()["edits"], 2);

    insta::assert_json_snapshot!("v2_files_shape", &body, {
        ".files[].last_ts"     => "[ts]",
        ".files[].accept_rate" => "[rate]",
    });
}

// ---------------------------------------------------------------------------
// 5. v2_files_empty_db_returns_empty_array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_files_empty_db_returns_empty_array() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/v2/files").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["files"].is_array(), "files must be an array");
    assert_eq!(body["files"].as_array().unwrap().len(), 0);
    assert_eq!(body["totals"]["files"], 0);
    assert_eq!(body["totals"]["edits"], 0);
}

// ---------------------------------------------------------------------------
// 6. v2_files_time_window_excludes_outside_rows
// ---------------------------------------------------------------------------

/// File changes outside the requested from/to window must not appear.
#[tokio::test]
async fn v2_files_time_window_excludes_outside_rows() {
    let (pool, _db_dir) = common::fixture_pool();

    let inside_ts = days_ago_ms(3);
    let outside_ts = days_ago_ms(90);

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "vf-win-in".into(),
            started_at_ms: Some(inside_ts),
            model: "claude-sonnet".into(),
            files: vec![common::FileChange { path: "recent.rs", added: 1, removed: 0 }],
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "vf-win-out".into(),
            started_at_ms: Some(outside_ts),
            model: "claude-sonnet".into(),
            files: vec![common::FileChange { path: "ancient.rs", added: 99, removed: 99 }],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    // Narrow window: only covers inside_ts
    let from = inside_ts - 1000;
    let to = inside_ts + 86_400_000;
    let url = format!("/api/v2/files?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    let files = body["files"].as_array().unwrap();
    let paths: Vec<&str> = files.iter().filter_map(|r| r["file_path"].as_str()).collect();

    assert!(paths.contains(&"recent.rs"), "in-window file must appear; got {paths:?}");
    assert!(
        !paths.contains(&"ancient.rs"),
        "out-of-window file must NOT appear; got {paths:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. v2_files_sort_by_churn
// ---------------------------------------------------------------------------

/// When sort=churn, files with higher (added+removed) come first.
#[tokio::test]
async fn v2_files_sort_by_churn() {
    let (pool, _db_dir) = common::fixture_pool();

    let ts = days_ago_ms(1);

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "vf-sort".into(),
            started_at_ms: Some(ts),
            model: "claude-sonnet".into(),
            files: vec![
                // low churn: 5 total lines
                common::FileChange { path: "small.rs", added: 3, removed: 2 },
                // high churn: 100 total lines
                common::FileChange { path: "large.rs", added: 60, removed: 40 },
            ],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let from = ts - 1000;
    let to = ts + 86_400_000;
    let url = format!("/api/v2/files?from={from}&to={to}&sort=churn");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    // large.rs has more churn, must come first
    assert_eq!(files[0]["file_path"], "large.rs", "high-churn file must sort first");
    assert_eq!(files[1]["file_path"], "small.rs");
}
