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
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();

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
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();

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
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
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
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'low-constraint-usage'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// E4: long-session-no-commit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn long_session_no_commit_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let two_hours = 120 * 60 * 1000;
    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - two_hours, now],
    ).unwrap();
    enable_only(&pool, &["long-session-no-commit"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'long-session-no-commit'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn long_session_with_commit_does_not_fire() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let two_hours = 120 * 60 * 1000;
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - two_hours, now],
    ).unwrap();
    conn.execute(
        "INSERT INTO git_activity (session_id, timestamp, activity, count) VALUES ('s1', ?1, 'commit', 1)",
        params![now - 1000],
    ).unwrap();
    enable_only(&pool, &["long-session-no-commit"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'long-session-no-commit'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

// ---------------------------------------------------------------------------
// E5: late-night-coding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn late_night_fires_with_five_sessions() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    use chrono::{TimeZone, Local};
    for d in 0..5 {
        let dt = Local.with_ymd_and_hms(2026, 5, 10 + d, 2, 0, 0).unwrap();
        let ms = dt.timestamp_millis();
        pool.get().unwrap().execute(
            "INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![format!("late-{}", d), ms],
        ).unwrap();
    }
    enable_only(&pool, &["late-night-coding"]);
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: 0, to_ms: now + 86400_000, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'late-night-coding'",
        [], |r| r.get(0)).unwrap();
    assert!(n >= 1, "should fire at least once for 5 late-night sessions");
}

// ---------------------------------------------------------------------------
// E6: abandon-sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn speed_accept_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 60_000]).unwrap();
    for i in 0..5i64 {
        let base = now - 50_000 + i*10_000;
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added) VALUES ('s1', ?1, 'a.rs', 25)",
            params![base]).unwrap();
        conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES ('s1', ?1, 'Edit', 'accept')",
            params![base + 100]).unwrap();
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES ('s1', ?1, ?2, 'jsonl', 'go on', ?3, 5, 0, 0, 0)",
            params![i, base + 5_000, format!("h{}", i)]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["speed-accept"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'speed-accept'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn no_slash_commands_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - 45*60_000, now],
    ).unwrap();
    enable_only(&pool, &["no-slash-commands"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'no-slash-commands'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn abandon_sessions_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    for i in 0..3i64 {
        let sid = format!("aban-{}", i);
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - (i+1) * 60_000]).unwrap();
        conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES (?1, ?2, 'Edit', 'reject')",
            params![sid, now - (i+1) * 60_000]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["abandon-sessions"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'abandon-sessions'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// E9: model-diversity (continuous)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn model_diversity_score_four_models_is_100() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();
    for m in ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5", "claude-other"] {
        conn.execute("INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES ('s1', ?1, ?2, 0.1)",
            params![now-500, m]).unwrap();
    }
    drop(conn);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    let score = andon_lib::coach::rules::score_model_diversity(&pool, &win).unwrap();
    assert_eq!(score, 100);
}

#[tokio::test]
async fn model_diversity_score_two_models_is_50() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();
    for m in ["claude-opus-4-7", "claude-sonnet-4-6"] {
        conn.execute("INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES ('s1', ?1, ?2, 0.1)",
            params![now-500, m]).unwrap();
    }
    drop(conn);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    let score = andon_lib::coach::rules::score_model_diversity(&pool, &win).unwrap();
    assert_eq!(score, 50);
}

// ---------------------------------------------------------------------------
// E10: cache-hit-starvation (context, binary)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_hit_starvation_fires_below_ten_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();

    // 25 turns, each with 5000 input + 100 cacheRead + 50 cacheCreation
    // cacheRate = 2500 / (2500 + 1250 + 125000) ≈ 2% < 10% → trigger
    for i in 0..25 {
        let t = now - 1000 + i*100;
        for (kind, count) in [("input", 5000i64), ("cacheRead", 100), ("cacheCreation", 50)] {
            conn.execute(
                "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES ('s1', ?1, 'm', ?2, ?3)",
                params![t, kind, count]).unwrap();
        }
    }
    drop(conn);
    enable_only(&pool, &["cache-hit-starvation"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'cache-hit-starvation'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// E11: low-spec-rate (context, binary)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn low_spec_rate_fires_when_below_twenty_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    // 6 sessions producing code (file_changes rows). Only 1 has a spec-driven first turn.
    for i in 0..6i64 {
        let sid = format!("s{}", i);
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - 1000 - i*100]).unwrap();
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added) VALUES (?1, ?2, 'a.rs', 5)",
            params![sid, now - 900 - i*100]).unwrap();
        let text = if i == 0 { "spec: must do thing" } else { "just go" };
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES (?1, 0, ?2, 'jsonl', ?3, ?4, ?5, 0, 0, 0)",
            params![sid, now - 950 - i*100, text, format!("h{}", i), text.chars().count() as i64]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["low-spec-rate"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'low-spec-rate'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
