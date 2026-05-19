/// Integration tests for the aggregation math in `src/reports/model.rs`.
///
/// We feed fixed seed data into an isolated SQLite fixture and assert
/// computed outputs exactly. Template rendering is NOT tested here —
/// template changes should not break these assertions.
mod common;

use rusqlite::params;
use std::sync::Arc;

use andon_lib::reports::model::ReportData;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Seed an `active_time` row directly (not covered by `seed_session`).
fn insert_active_time(pool: &Arc<andon_lib::db::DbPool>, session_id: &str, kind: &str, seconds: f64) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO active_time (session_id, timestamp, seconds, kind) VALUES (?, 0, ?, ?)",
        params![session_id, seconds, kind],
    )
    .expect("insert active_time");
}

/// Seed a cost entry with an explicit model name.
fn insert_cost(pool: &Arc<andon_lib::db::DbPool>, session_id: &str, model: &str, cost: f64) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES (?, 0, ?, ?)",
        params![session_id, model, cost],
    )
    .expect("insert cost_entries");
}

/// Seed a token_usage row directly.
fn insert_token(
    pool: &Arc<andon_lib::db::DbPool>,
    session_id: &str,
    model: &str,
    token_type: &str,
    count: i64,
) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES (?, 0, ?, ?, ?)",
        params![session_id, model, token_type, count],
    )
    .expect("insert token_usage");
}

/// Seed a tool_decision row with an optional file_path.
fn insert_decision(
    pool: &Arc<andon_lib::db::DbPool>,
    session_id: &str,
    tool: &str,
    decision: &str,
    language: Option<&str>,
    file_path: Option<&str>,
) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision, language, file_path) \
         VALUES (?, 0, ?, ?, ?, ?)",
        params![session_id, tool, decision, language, file_path],
    )
    .expect("insert tool_decisions");
}

/// Seed a file_change row.
fn insert_file_change(
    pool: &Arc<andon_lib::db::DbPool>,
    session_id: &str,
    file_path: &str,
    added: i64,
    removed: i64,
) {
    let conn = pool.get().expect("checkout connection");
    conn.execute(
        "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed) \
         VALUES (?, 0, ?, ?, ?)",
        params![session_id, file_path, added, removed],
    )
    .expect("insert file_changes");
}

// ---------------------------------------------------------------------------
// 1. cost_rollup_sums_all_entries
// ---------------------------------------------------------------------------

#[test]
fn cost_rollup_sums_all_entries() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "cost-rollup-1";

    // Session row required for ReportData::load
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_cost(&pool, sid, "claude-sonnet", 0.10);
    insert_cost(&pool, sid, "claude-sonnet", 0.05);
    insert_cost(&pool, sid, "claude-opus",   0.30);

    let data = ReportData::load(&pool, sid).expect("load");

    // 0.10 + 0.05 + 0.30 = 0.45  rounded to 4 decimal places
    assert_eq!(data.cost_usd, 0.45, "cost_usd rollup incorrect: {}", data.cost_usd);
}

// ---------------------------------------------------------------------------
// 2. cost_rollup_rounds_to_four_decimal_places
// ---------------------------------------------------------------------------

#[test]
fn cost_rollup_rounds_to_four_decimal_places() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "cost-round-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    // 1/3 repeating — should be rounded to 4 dp
    insert_cost(&pool, sid, "claude-sonnet", 1.0 / 3.0);

    let data = ReportData::load(&pool, sid).expect("load");

    // (0.3333... * 10000).round() / 10000 == 0.3333
    assert_eq!(data.cost_usd, 0.3333, "expected 0.3333, got {}", data.cost_usd);
}

// ---------------------------------------------------------------------------
// 3. token_rollup_groups_by_type
// ---------------------------------------------------------------------------

#[test]
fn token_rollup_groups_by_type() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "token-rollup-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_token(&pool, sid, "claude-sonnet", "input",        500);
    insert_token(&pool, sid, "claude-sonnet", "input",        300);  // second batch
    insert_token(&pool, sid, "claude-sonnet", "output",       200);
    insert_token(&pool, sid, "claude-sonnet", "cache_read",   100);
    insert_token(&pool, sid, "claude-sonnet", "cache_create",  50);

    let data = ReportData::load(&pool, sid).expect("load");

    let find = |key: &str| -> i64 {
        data.tokens.iter().find(|kv| kv.key == key).map(|kv| kv.value).unwrap_or(0)
    };

    assert_eq!(find("input"), 800, "input tokens should sum to 800");
    assert_eq!(find("output"), 200, "output tokens should be 200");
    assert_eq!(find("cache_read"), 100, "cache_read tokens should be 100");
    assert_eq!(find("cache_create"), 50, "cache_create tokens should be 50");
}

// ---------------------------------------------------------------------------
// 4. accept_rate_pure_math
// ---------------------------------------------------------------------------

/// The `rate` function: accepted / (accepted + rejected + aborted), rounded to 4 dp.

#[test]
fn accept_rate_all_accepted() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "ar-all-accept";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    for _ in 0..4 {
        insert_decision(&pool, sid, "Edit", "accept", None, None);
    }

    let data = ReportData::load(&pool, sid).expect("load");
    assert_eq!(data.accept_rate, 1.0, "all-accept rate should be 1.0, got {}", data.accept_rate);
}

#[test]
fn accept_rate_mixed_decisions() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "ar-mixed";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    // 3 accept, 1 reject, 1 abort  →  3/5 = 0.6
    for _ in 0..3 { insert_decision(&pool, sid, "Edit", "accept", None, None); }
    insert_decision(&pool, sid, "Edit", "reject", None, None);
    insert_decision(&pool, sid, "Edit", "abort",  None, None);

    let data = ReportData::load(&pool, sid).expect("load");
    assert_eq!(data.accept_rate, 0.6, "expected 0.6, got {}", data.accept_rate);
}

#[test]
fn accept_rate_zero_when_no_decisions() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "ar-empty";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    let data = ReportData::load(&pool, sid).expect("load");
    assert_eq!(data.accept_rate, 0.0, "empty decisions → rate 0.0, got {}", data.accept_rate);
}

// ---------------------------------------------------------------------------
// 5. cost_by_model_split
// ---------------------------------------------------------------------------

#[test]
fn cost_by_model_split_per_model() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "model-split-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_cost(&pool, sid, "claude-opus",   0.80);
    insert_cost(&pool, sid, "claude-opus",   0.20);
    insert_cost(&pool, sid, "claude-haiku",  0.10);
    insert_cost(&pool, sid, "claude-sonnet", 0.40);

    let data = ReportData::load(&pool, sid).expect("load");

    let find = |key: &str| -> f64 {
        data.cost_by_model.iter().find(|kv| kv.key == key).map(|kv| kv.value).unwrap_or(-1.0)
    };

    // opus: 0.80+0.20 = 1.00 — highest so it comes first (ORDER BY 2 DESC)
    assert!((find("claude-opus")   - 1.00).abs() < 1e-9, "opus cost wrong: {}", find("claude-opus"));
    assert!((find("claude-sonnet") - 0.40).abs() < 1e-9, "sonnet cost wrong");
    assert!((find("claude-haiku")  - 0.10).abs() < 1e-9, "haiku cost wrong");

    // Verify ordering: opus first, then sonnet, then haiku
    let keys: Vec<&str> = data.cost_by_model.iter().map(|kv| kv.key.as_str()).collect();
    assert_eq!(keys, vec!["claude-opus", "claude-sonnet", "claude-haiku"]);
}

// ---------------------------------------------------------------------------
// 6. tokens_by_type_mirrors_token_list
// ---------------------------------------------------------------------------

#[test]
fn tokens_by_type_mirrors_token_list_as_float() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "tok-float-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_token(&pool, sid, "claude-sonnet", "input",  1000);
    insert_token(&pool, sid, "claude-sonnet", "output",  500);

    let data = ReportData::load(&pool, sid).expect("load");

    // tokens_by_type is derived from tokens — values are the same but as f64
    let tok_input = data.tokens.iter().find(|kv| kv.key == "input").map(|kv| kv.value);
    let tbt_input = data.tokens_by_type.iter().find(|kv| kv.key == "input").map(|kv| kv.value as i64);

    assert_eq!(tok_input, tbt_input, "tokens_by_type must mirror tokens as f64");
}

// ---------------------------------------------------------------------------
// 7. active_time_rollup
// ---------------------------------------------------------------------------

#[test]
fn active_time_sums_user_and_cli_separately() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "active-time-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_active_time(&pool, sid, "user", 120.0);
    insert_active_time(&pool, sid, "user",  30.0);
    insert_active_time(&pool, sid, "cli",   60.0);

    let data = ReportData::load(&pool, sid).expect("load");

    assert_eq!(data.active_user_seconds, 150.0, "user seconds wrong: {}", data.active_user_seconds);
    assert_eq!(data.active_cli_seconds,   60.0, "cli seconds wrong: {}",  data.active_cli_seconds);
}

// ---------------------------------------------------------------------------
// 8. duration_seconds — ended_at path vs active_time fallback
// ---------------------------------------------------------------------------

#[test]
fn duration_seconds_uses_ended_at_when_present() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "dur-ended-1";

    // started_at = 1000 ms, ended_at = 61000 ms → 60 s
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES (?, 1000, 61000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    let data = ReportData::load(&pool, sid).expect("load");

    assert_eq!(data.duration_seconds, 60.0, "expected 60 s from ended_at, got {}", data.duration_seconds);
}

#[test]
fn duration_seconds_falls_back_to_active_time_when_no_ended_at() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "dur-active-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    insert_active_time(&pool, sid, "user", 45.0);
    insert_active_time(&pool, sid, "cli",  15.0);

    let data = ReportData::load(&pool, sid).expect("load");

    // No ended_at → duration = user + cli = 60.0
    assert_eq!(data.duration_seconds, 60.0, "expected fallback duration 60 s, got {}", data.duration_seconds);
}

// ---------------------------------------------------------------------------
// 9. file_changes_rollup_and_per_file_accept_rate
// ---------------------------------------------------------------------------

#[test]
fn file_changes_grouped_by_path_with_accept_rate() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "file-agg-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    // Two batches of changes to the same file
    insert_file_change(&pool, sid, "src/lib.rs", 10, 5);
    insert_file_change(&pool, sid, "src/lib.rs", 20, 3);

    // Two accepted, two rejected → per-file accept rate = 0.5
    insert_decision(&pool, sid, "Edit", "accept", Some("rust"), Some("src/lib.rs"));
    insert_decision(&pool, sid, "Edit", "accept", Some("rust"), Some("src/lib.rs"));
    insert_decision(&pool, sid, "Edit", "reject", Some("rust"), Some("src/lib.rs"));
    insert_decision(&pool, sid, "Edit", "reject", Some("rust"), Some("src/lib.rs"));

    let data = ReportData::load(&pool, sid).expect("load");

    let file = data.files.iter().find(|f| f.file_path == "src/lib.rs")
        .expect("src/lib.rs must be in files list");

    assert_eq!(file.added,   30, "added lines wrong: {}", file.added);
    assert_eq!(file.removed,  8, "removed lines wrong: {}", file.removed);
    assert_eq!(file.accept_rate, 0.5, "per-file accept_rate wrong: {}", file.accept_rate);
}

// ---------------------------------------------------------------------------
// 10. decisions list is ordered by timestamp ASC
// ---------------------------------------------------------------------------

#[test]
fn decisions_ordered_by_timestamp_ascending() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "dec-order-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 1000)",
        params![sid],
    )
    .unwrap();
    // Insert out-of-order timestamps
    conn.execute(
        "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES (?, 3000, 'Edit', 'accept')",
        params![sid],
    ).unwrap();
    conn.execute(
        "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES (?, 1000, 'Edit', 'reject')",
        params![sid],
    ).unwrap();
    conn.execute(
        "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES (?, 2000, 'Edit', 'abort')",
        params![sid],
    ).unwrap();
    drop(conn);

    let data = ReportData::load(&pool, sid).expect("load");

    assert_eq!(data.decisions.len(), 3, "expected 3 decisions");
    assert_eq!(data.decisions[0].timestamp, 1000);
    assert_eq!(data.decisions[1].timestamp, 2000);
    assert_eq!(data.decisions[2].timestamp, 3000);
    assert_eq!(data.decisions[0].decision, "reject");
    assert_eq!(data.decisions[1].decision, "abort");
    assert_eq!(data.decisions[2].decision, "accept");
}

// ---------------------------------------------------------------------------
// 11. empty session returns zero-value aggregates (no panics)
// ---------------------------------------------------------------------------

#[test]
fn empty_session_returns_zero_aggregates() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "empty-session-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES (?, 5000)",
        params![sid],
    )
    .unwrap();
    drop(conn);

    let data = ReportData::load(&pool, sid).expect("load");

    assert_eq!(data.cost_usd,            0.0);
    assert_eq!(data.accept_rate,         0.0);
    assert_eq!(data.active_user_seconds, 0.0);
    assert_eq!(data.active_cli_seconds,  0.0);
    assert!(data.tokens.is_empty(),         "no token rows → empty vec");
    assert!(data.cost_by_model.is_empty(),  "no cost rows → empty vec");
    assert!(data.files.is_empty(),          "no file_changes → empty vec");
    assert!(data.decisions.is_empty(),      "no decisions → empty vec");
}

// ---------------------------------------------------------------------------
// 12. session metadata is surfaced correctly
// ---------------------------------------------------------------------------

#[test]
fn session_metadata_fields_are_populated() {
    let (pool, _dir) = common::fixture_pool();
    let sid = "meta-1";

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions \
         (session_id, started_at, ended_at, service_version, host_arch, os_type, terminal_type) \
         VALUES (?, 1000, 61000, '1.2.3', 'aarch64', 'linux', 'iterm2')",
        params![sid],
    )
    .unwrap();
    drop(conn);

    let data = ReportData::load(&pool, sid).expect("load");

    assert_eq!(data.session_id,       sid);
    assert_eq!(data.started_at,       1000);
    assert_eq!(data.ended_at,         Some(61000));
    assert_eq!(data.service_version.as_deref(), Some("1.2.3"));
    assert_eq!(data.host_arch.as_deref(),       Some("aarch64"));
    assert_eq!(data.os_type.as_deref(),         Some("linux"));
    assert_eq!(data.terminal_type.as_deref(),   Some("iterm2"));
}

// ---------------------------------------------------------------------------
// 13. multiple sessions are isolated — no cross-contamination
// ---------------------------------------------------------------------------

#[test]
fn cost_is_isolated_per_session() {
    let (pool, _dir) = common::fixture_pool();

    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s-a', 1000)", []).unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s-b', 2000)", []).unwrap();
    drop(conn);

    insert_cost(&pool, "s-a", "claude-sonnet", 1.00);
    insert_cost(&pool, "s-b", "claude-sonnet", 0.50);

    let data_a = ReportData::load(&pool, "s-a").expect("load s-a");
    let data_b = ReportData::load(&pool, "s-b").expect("load s-b");

    assert_eq!(data_a.cost_usd, 1.00, "session A cost wrong");
    assert_eq!(data_b.cost_usd, 0.50, "session B cost wrong");
}
