mod common;

use andon_lib::coach::skill;
use andon_lib::settings::CoachSettings;
use rusqlite::params;

#[tokio::test]
async fn discovery_surfaces_threshold_hits() {
    let (pool, _dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    for sid in ["s1", "s2"] {
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - 86400_000]).unwrap();
    }
    for (sid, turn, ts, hash) in [
        ("s1", 0, now - 80_000_000, "h1"),
        ("s1", 1, now - 70_000_000, "h1"),
        ("s2", 0, now - 60_000_000, "h1"),
    ] {
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES (?1, ?2, ?3, 'jsonl', 'package the extension', ?4, 21, 0, 0, 0)",
            params![sid, turn, ts, hash],
        ).unwrap();
    }
    drop(conn);

    skill::discover_all(&pool, &CoachSettings::default()).unwrap();

    let (label, occurrences, sessions): (String, i64, i64) = pool.get().unwrap().query_row(
        "SELECT label, occurrences, session_count FROM skill_opportunities WHERE norm_hash = 'h1' ORDER BY window_end DESC LIMIT 1",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    ).unwrap();
    assert_eq!(occurrences, 3);
    assert_eq!(sessions, 2);
    assert!(label.contains("package"));
}

#[tokio::test]
async fn discovery_idempotent() {
    let (pool, _dir) = common::fixture_pool();
    let cs = CoachSettings::default();
    skill::discover_all(&pool, &cs).unwrap();
    let n1: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| r.get(0)).unwrap();
    skill::discover_all(&pool, &cs).unwrap();
    let n2: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| r.get(0)).unwrap();
    assert_eq!(n1, n2);
}
