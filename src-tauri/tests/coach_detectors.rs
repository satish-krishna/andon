mod common;

use andon_lib::coach::{engine, rules::Window};
use rusqlite::params;

fn seed_prompt_turn(pool: &andon_lib::db::DbPool, session_id: &str, turn: i64, ts: i64, text: &str, norm_hash: &str) {
    pool.get().unwrap().execute(
        "INSERT INTO prompt_turns
           (session_id, turn_index, ts, source, text, norm_hash,
            length, has_file_ref, has_code, has_constraint)
         VALUES (?1, ?2, ?3, 'jsonl', ?4, ?5, ?6, 0, 0, 0)",
        params![session_id, turn, ts, text, norm_hash, text.chars().count() as i64],
    ).unwrap();
}

fn enable_only(pool: &andon_lib::db::DbPool, ids: &[&str]) {
    let conn = pool.get().unwrap();
    conn.execute("UPDATE coach_rules SET enabled = 0", []).unwrap();
    for id in ids {
        conn.execute("UPDATE coach_rules SET enabled = 1 WHERE id = ?1", params![id]).unwrap();
    }
}

#[tokio::test]
async fn repeated_prompts_fires_at_three_hits() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    drop(conn);

    // Three identical hashes within one session -> trigger.
    seed_prompt_turn(&pool, "s1", 0, now - 800, "package the extension", "h1");
    seed_prompt_turn(&pool, "s1", 1, now - 600, "Package the extension", "h1");
    seed_prompt_turn(&pool, "s1", 2, now - 400, "package the EXTENSION", "h1");

    enable_only(&pool, &["repeated-prompts"]);

    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();

    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'repeated-prompts'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 1, "exactly one finding per session, not per hit");
}

#[tokio::test]
async fn repeated_prompts_skips_below_threshold() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();

    seed_prompt_turn(&pool, "s1", 0, now - 800, "a", "h1");
    seed_prompt_turn(&pool, "s1", 1, now - 600, "a", "h1");

    enable_only(&pool, &["repeated-prompts"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();

    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'repeated-prompts'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn lazy_prompting_fires_when_third_are_short() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    for i in 0..15 {
        let text = if i < 5 { "fix bug" } else { "Refactor authentication middleware to use JWT with rotation" };
        seed_prompt_turn(&pool, "s1", i, now - (1000 - i*10), text, &format!("h{}", i));
    }
    enable_only(&pool, &["lazy-prompting"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'lazy-prompting'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn low_constraint_usage_fires_below_twenty_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    // 6 turns, only 1 has constraint -> 16.7% < 20% -> trigger
    for i in 0..6 {
        pool.get().unwrap().execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES ('s1', ?1, ?2, 'jsonl', 'x', ?3, 1, 0, 0, ?4)",
            params![i, now - (1000 - i*10), format!("h{}", i), if i == 0 { 1 } else { 0 }],
        ).unwrap();
    }
    enable_only(&pool, &["low-constraint-usage"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'low-constraint-usage'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
