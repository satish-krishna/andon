mod common;

use andon_lib::coach::{rules::Window, score};
use rusqlite::params;

#[tokio::test]
async fn worked_example_three_detectors_one_high_triggers_67() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    pool.get().unwrap().execute("UPDATE coach_rules SET enabled = 0", []).unwrap();
    pool.get().unwrap().execute(
        "UPDATE coach_rules SET enabled = 1 WHERE id IN ('long-session-no-commit', 'late-night-coding', 'abandon-sessions')",
        [],
    ).unwrap();

    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 100)", []).unwrap();
    pool.get().unwrap().execute(
        "INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
         VALUES ('long-session-no-commit', 's1', 100, '{}')", []).unwrap();

    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "hygiene", &win).unwrap();
    // 3 enabled detectors, 1 high triggers: penalty=12, maxPenalty=36, score=67
    assert_eq!(s.score, Some(67));
    assert_eq!(s.status, "needs-improvement");
    assert_eq!(s.triggered_count, 1);
}

#[tokio::test]
async fn empty_practice_returns_null_status_na() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    pool.get().unwrap().execute("UPDATE coach_rules SET enabled = 0 WHERE practice = 'tool'", []).unwrap();
    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "tool", &win).unwrap();
    assert_eq!(s.score, None);
    assert_eq!(s.status, "n/a");
}

#[tokio::test]
async fn clean_practice_scores_100() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "prompt", &win).unwrap();
    assert_eq!(s.score, Some(100));
    assert_eq!(s.status, "good");
    assert_eq!(s.triggered_count, 0);
}

#[tokio::test]
async fn wow_pct_correct_signed_integer() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let day = 86_400_000i64;
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 14*day]).unwrap();
    // Last 7d: 10 findings; prior 7d: 8 findings → wow = +25
    for i in 0..10 {
        conn.execute("INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
                      VALUES ('lazy-prompting', 's1', ?1, '{}')", params![now - (1+i) * 3600_000]).unwrap();
    }
    for i in 0..8 {
        conn.execute("INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
                      VALUES ('lazy-prompting', 's1', ?1, '{}')", params![now - 7*day - (1+i) * 3600_000]).unwrap();
    }
    drop(conn);
    let wow = score::trends_wow(&pool, "prompt", now).unwrap();
    assert_eq!(wow, 25);
}

#[tokio::test]
async fn wow_pct_returns_zero_when_prev_is_zero() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let wow = score::trends_wow(&pool, "prompt", now).unwrap();
    assert_eq!(wow, 0);
}
