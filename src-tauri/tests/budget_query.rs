mod common;

use andon_lib::db::queries::month_to_date_cost;

#[test]
fn month_to_date_cost_sums_cost_entries_in_window() {
    let (pool, _dir) = common::fixture_pool();

    // Two sessions with cost inside the window, one before it.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "in-1".into(),
            started_at_ms: Some(1_700_000_000_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 12.50,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "in-2".into(),
            started_at_ms: Some(1_700_000_500_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 7.50,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "before-window".into(),
            started_at_ms: Some(1_699_000_000_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 99.0,
            ..Default::default()
        },
    );
    // Exactly at to_ms — the window is half-open [from, to), so this is excluded.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "at-upper-bound".into(),
            started_at_ms: Some(1_700_001_000_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 88.0,
            ..Default::default()
        },
    );

    let conn = pool.get().expect("checkout connection");
    let total = month_to_date_cost(&conn, 1_700_000_000_000, 1_700_001_000_000)
        .expect("query month_to_date_cost");

    // Only the two in-window rows count; the before-window and at-upper-bound
    // rows are excluded by the half-open window.
    assert!((total - 20.0).abs() < 1e-9, "expected 20.0, got {total}");
}
