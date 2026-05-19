mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::Value;
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

async fn post_empty(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
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
// 1. repo_backfill_scans_sessions_with_null_repo_root
// ---------------------------------------------------------------------------

/// Seed a session whose file_changes paths live inside a real git repo.
/// POST /api/repo/backfill should return scanned >= 1 and may return updated > 0
/// when git is available and the path is discoverable.
#[tokio::test]
async fn repo_backfill_scans_sessions_with_null_repo_root() {
    let (pool, _db_dir) = common::fixture_pool();

    // Create a real git repo so repo_inference can resolve it.
    let git_dir = common::init_temp_repo(&["initial commit"]);
    let abs_file = git_dir.path().join("f0.txt");

    // Insert a session with repo_root NULL (the default after a plain INSERT).
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('backfill-1', 1_700_000_000_000)",
            [],
        )
        .unwrap();
        // Seed a file_change with an absolute path inside the temp git repo.
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed) \
             VALUES ('backfill-1', 1_700_000_000_000, ?1, 5, 2)",
            params![abs_file.to_string_lossy().as_ref()],
        )
        .unwrap();
    }

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = post_empty(router, "/api/repo/backfill").await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    // scanned should reflect the session we inserted (repo_root IS NULL)
    let scanned = body["scanned"].as_u64().unwrap_or(0);
    assert!(scanned >= 1, "expected scanned >= 1, got {body}");
    // updated is best-effort; just verify it is a non-negative integer
    assert!(body["updated"].is_number(), "updated should be a number");
}

// ---------------------------------------------------------------------------
// 2. list_repos_after_backfill
// ---------------------------------------------------------------------------

/// After a successful backfill that writes repo_root, GET /api/repos should
/// return at least one repo entry.
#[tokio::test]
async fn list_repos_after_backfill() {
    let (pool, _db_dir) = common::fixture_pool();

    let git_dir = common::init_temp_repo(&["initial commit"]);
    let abs_file = git_dir.path().join("f0.txt");
    let repo_root_str = git_dir.path().to_string_lossy().into_owned();

    // Insert a session and manually set repo_root so we don't depend on git
    // inference succeeding in CI (git might not be configured).
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at, repo_root) VALUES ('listrepos-1', 1_700_000_000_000, ?1)",
            params![&repo_root_str],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed) \
             VALUES ('listrepos-1', 1_700_000_000_000, ?1, 1, 0)",
            params![abs_file.to_string_lossy().as_ref()],
        )
        .unwrap();
    }

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/repos").await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let arr = body.as_array().expect("repos response should be an array");
    assert!(!arr.is_empty(), "expected at least one repo after seeding");

    let first = &arr[0];
    assert!(first["key"].is_string(), "repo entry should have a key field");
    assert!(first["label"].is_string(), "repo entry should have a label field");
    assert!(first["session_count"].as_i64().unwrap_or(0) >= 1, "session_count should be >= 1");
}

// ---------------------------------------------------------------------------
// 3. top_repos_aggregation
// ---------------------------------------------------------------------------

/// Seed two sessions with cost entries in different repos, then verify
/// GET /api/overview/top-repos returns entries ordered by cost (highest first).
#[tokio::test]
async fn top_repos_aggregation() {
    let (pool, _db_dir) = common::fixture_pool();

    let now_ms: i64 = 1_700_000_000_000;
    let window_ms: i64 = 7 * 24 * 60 * 60 * 1000; // 7 days
    let from = now_ms - window_ms;
    let to = now_ms + 1000;

    {
        let conn = pool.get().unwrap();

        // Session A — more expensive
        conn.execute(
            "INSERT INTO sessions (session_id, started_at, repo_root, repo_remote) \
             VALUES ('top-repo-a', ?1, '/repos/alpha', 'https://github.com/org/alpha')",
            params![now_ms - 1000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES ('top-repo-a', ?1, 'claude-sonnet', 0.80)",
            params![now_ms - 1000],
        )
        .unwrap();

        // Session B — cheaper
        conn.execute(
            "INSERT INTO sessions (session_id, started_at, repo_root, repo_remote) \
             VALUES ('top-repo-b', ?1, '/repos/beta', 'https://github.com/org/beta')",
            params![now_ms - 2000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES ('top-repo-b', ?1, 'claude-haiku', 0.10)",
            params![now_ms - 2000],
        )
        .unwrap();
    }

    let (router, _router_dir) = common::test_router(&pool);
    let url = format!("/api/overview/top-repos?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let arr = body.as_array().expect("top-repos should be an array");
    assert!(!arr.is_empty(), "expected at least one entry");

    // First entry should be the most expensive repo
    let first = &arr[0];
    assert_eq!(
        first["key"].as_str().unwrap_or(""),
        "https://github.com/org/alpha",
        "highest-cost repo should be first"
    );
    let cost = first["cost_usd"].as_f64().unwrap_or(0.0);
    assert!((cost - 0.80).abs() < 1e-6, "cost_usd should be ~0.80, got {cost}");
    assert!(first["spark"].is_array(), "spark field should be present");
}
