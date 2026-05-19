mod common;

// ---------------------------------------------------------------------------
// claude_code.session.count
// ---------------------------------------------------------------------------

/// session.count is a no-op handler but must not error and must upsert a
/// sessions row when session.id is present.
#[test]
fn session_count_metric_upserts_session_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-count")],
        "claude_code.session.count",
        vec![],
        1.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id = 'sess-count'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "session row should be upserted for session.count metric");
}

// ---------------------------------------------------------------------------
// claude_code.lines_of_code.count
// ---------------------------------------------------------------------------

#[test]
fn lines_of_code_count_added_inserts_file_changes_added() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-loc")],
        "claude_code.lines_of_code.count",
        vec![
            common::kv("type", "added"),
            common::kv("file.path", "src/main.rs"),
        ],
        42.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (added, removed): (i64, i64) = conn
        .query_row(
            "SELECT lines_added, lines_removed FROM file_changes \
             WHERE session_id = 'sess-loc' AND file_path = 'src/main.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(added, 42);
    assert_eq!(removed, 0);
}

#[test]
fn lines_of_code_count_removed_inserts_file_changes_removed() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-loc-rm")],
        "claude_code.lines_of_code.count",
        vec![
            common::kv("type", "removed"),
            common::kv("file.path", "src/lib.rs"),
        ],
        10.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (added, removed): (i64, i64) = conn
        .query_row(
            "SELECT lines_added, lines_removed FROM file_changes \
             WHERE session_id = 'sess-loc-rm' AND file_path = 'src/lib.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(added, 0);
    assert_eq!(removed, 10);
}

// ---------------------------------------------------------------------------
// claude_code.pull_request.count
// ---------------------------------------------------------------------------

#[test]
fn pull_request_count_inserts_git_activity_pull_request() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-pr")],
        "claude_code.pull_request.count",
        vec![],
        3.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (activity, count): (String, i64) = conn
        .query_row(
            "SELECT activity, count FROM git_activity WHERE session_id = 'sess-pr'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(activity, "pull_request");
    assert_eq!(count, 3);
}

// ---------------------------------------------------------------------------
// claude_code.commit.count
// ---------------------------------------------------------------------------

#[test]
fn commit_count_inserts_git_activity_commit() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-commit")],
        "claude_code.commit.count",
        vec![],
        5.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (activity, count): (String, i64) = conn
        .query_row(
            "SELECT activity, count FROM git_activity WHERE session_id = 'sess-commit'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(activity, "commit");
    assert_eq!(count, 5);
}

// ---------------------------------------------------------------------------
// claude_code.cost.usage
// ---------------------------------------------------------------------------

#[test]
fn cost_usage_metric_inserts_cost_entry() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-cost")],
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-opus-4-5-20251001")],
        2.50,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (model, cost): (String, f64) = conn
        .query_row(
            "SELECT model, cost_usd FROM cost_entries WHERE session_id = 'sess-cost'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(model, "claude-opus-4-5-20251001");
    assert!((cost - 2.50).abs() < 1e-9, "cost_usd mismatch: {cost}");
}

// ---------------------------------------------------------------------------
// claude_code.token.usage  (vary token_type)
// ---------------------------------------------------------------------------

fn token_usage_test(session_id: &str, token_type_attr: &str, expected_type: &str) {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", session_id)],
        "claude_code.token.usage",
        vec![
            common::kv("type", token_type_attr),
            common::kv("model", "claude-haiku"),
        ],
        100.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (token_type, count): (String, i64) = conn
        .query_row(
            "SELECT token_type, count FROM token_usage WHERE session_id = ?1",
            rusqlite::params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(token_type, expected_type, "token_type mismatch");
    assert_eq!(count, 100);
}

#[test]
fn token_usage_input_type() {
    token_usage_test("sess-tok-input", "input", "input");
}

#[test]
fn token_usage_output_type() {
    token_usage_test("sess-tok-output", "output", "output");
}

#[test]
fn token_usage_cache_read_type() {
    token_usage_test("sess-tok-cache-read", "cacheRead", "cacheRead");
}

#[test]
fn token_usage_cache_creation_type() {
    token_usage_test("sess-tok-cache-creation", "cacheCreation", "cacheCreation");
}

// ---------------------------------------------------------------------------
// claude_code.code_edit_tool.decision  (vary decision and language)
// ---------------------------------------------------------------------------

fn decision_test(session_id: &str, decision: &str, language: &str) {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", session_id)],
        "claude_code.code_edit_tool.decision",
        vec![
            common::kv("tool", "Edit"),
            common::kv("decision", decision),
            common::kv("language", language),
        ],
        1.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (tool, dec, lang): (String, String, String) = conn
        .query_row(
            "SELECT tool_name, decision, language FROM tool_decisions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(tool, "Edit");
    assert_eq!(dec, decision, "decision mismatch");
    assert_eq!(lang, language, "language mismatch");
}

#[test]
fn code_edit_tool_decision_accept() {
    decision_test("sess-dec-accept", "accept", "rust");
}

#[test]
fn code_edit_tool_decision_reject() {
    decision_test("sess-dec-reject", "reject", "typescript");
}

#[test]
fn code_edit_tool_decision_abort() {
    decision_test("sess-dec-abort", "abort", "python");
}

// ---------------------------------------------------------------------------
// claude_code.active_time.total
// ---------------------------------------------------------------------------

#[test]
fn active_time_total_inserts_active_time_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-at")],
        "claude_code.active_time.total",
        vec![common::kv("type", "user")],
        120.5,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (seconds, kind): (f64, String) = conn
        .query_row(
            "SELECT seconds, kind FROM active_time WHERE session_id = 'sess-at'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((seconds - 120.5).abs() < 1e-9, "seconds mismatch: {seconds}");
    assert_eq!(kind, "user");
}

#[test]
fn active_time_total_defaults_kind_to_user_when_type_absent() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // Send with no "type" attribute — handler defaults to "user".
    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-at-default")],
        "claude_code.active_time.total",
        vec![],
        60.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let kind: String = conn
        .query_row(
            "SELECT kind FROM active_time WHERE session_id = 'sess-at-default'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "user", "default kind should be 'user'");
}

// ---------------------------------------------------------------------------
// Extra: unknown metric lands in metrics_raw
// ---------------------------------------------------------------------------

#[test]
fn unknown_metric_lands_in_metrics_raw() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "sess-raw")],
        "claude_code.something.new",
        vec![common::kv("custom_attr", "hello")],
        7.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM metrics_raw \
             WHERE session_id = 'sess-raw' AND metric_name = 'claude_code.something.new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "unknown metric should produce exactly one metrics_raw row");
}

// ---------------------------------------------------------------------------
// Extra: missing session_id does not panic
// ---------------------------------------------------------------------------

/// A cost.usage payload with no resource attributes (no session.id) must not
/// panic or return an error — the row is silently skipped.
#[test]
fn missing_session_id_does_not_panic() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // No resource attrs at all.
    let payload = common::sample_sum_metric(
        vec![],
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-sonnet")],
        1.0,
    );
    // Must not panic; must return Ok.
    ingestor
        .ingest_metrics_v2(payload, "test")
        .expect("ingest should succeed even with no session.id");

    // No cost_entries row should have been created.
    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "no cost row should be inserted without a session_id");
}
