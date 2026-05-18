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
