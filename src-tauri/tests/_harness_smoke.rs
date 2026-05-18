mod common;

#[test]
fn fixture_pool_applies_migrations_and_supports_wal() {
    let (pool, _guard) = common::fixture_pool();
    let conn = pool.get().expect("checkout connection");

    // WAL mode must be active (in-memory would not support it).
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("read journal_mode");
    assert_eq!(mode.to_lowercase(), "wal");

    // Migrations create the sessions table.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
            [],
            |r| r.get(0),
        )
        .expect("count sessions table");
    assert_eq!(count, 1);
}

#[test]
fn seed_session_inserts_session_and_related_rows() {
    let (pool, _guard) = common::fixture_pool();
    let opts = common::SeedOpts {
        session_id: "sess-1".into(),
        model: "claude-opus-4-5-20251001".into(),
        input_tokens: 100,
        output_tokens: 50,
        cost_usd: 0.42,
        decisions: vec![("accept", "rust"), ("reject", "rust")],
        ..Default::default()
    };
    common::seed_session(&pool, &opts);

    let conn = pool.get().unwrap();
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(session_count, 1);

    let token_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(token_rows, 2, "one row per token_type");

    let cost: f64 = conn
        .query_row("SELECT cost_usd FROM cost_entries WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert!((cost - 0.42).abs() < 1e-9);

    let decisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM tool_decisions WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(decisions, 2);
}

#[test]
fn sample_sum_metric_builds_a_single_point() {
    let rm = common::sample_sum_metric(
        vec![common::kv("session.id", "s1")],
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-opus-4-5-20251001")],
        1.23,
    );
    assert_eq!(rm.len(), 1);
}
