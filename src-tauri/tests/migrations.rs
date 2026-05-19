mod common;

#[test]
fn migrations_are_idempotent() {
    let (pool, _g) = common::fixture_pool();
    // fixture_pool already runs migrations once via andon_lib::db::init.
    // Running migrations again must not error.
    let mut conn = pool.get().expect("checkout connection for second migration run");
    andon_lib::db::migrations::apply(&mut conn).expect("second migration run");
}

#[test]
fn schema_matches_documented_tables() {
    let (pool, _g) = common::fixture_pool();
    let conn = pool.get().expect("checkout connection for schema check");
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare schema query");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("query tables")
        .filter_map(Result::ok)
        .collect();

    for expected in [
        "active_time",
        "cost_entries",
        "file_changes",
        "git_activity",
        "log_events",
        "metrics_raw",
        "schema_version",
        "sessions",
        "token_usage",
        "tool_decisions",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing table {expected}; have {names:?}"
        );
    }
}
