mod common;

use andon_lib::coach::{engine, rules::Window};

#[tokio::test]
async fn evaluate_window_runs_without_errors_on_empty_db() {
    let (pool, _dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: now - 30 * 86400_000, to_ms: now, models: None };

    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).expect("evaluate_window");

    let n: i64 = pool.get().unwrap()
        .query_row("SELECT COUNT(*) FROM coach_findings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
