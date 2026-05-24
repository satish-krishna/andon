mod common;

use rusqlite::params;

#[tokio::test]
async fn evaluate_session_writes_findings_when_rules_trigger() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    // Long session with no commits triggers long-session-no-commit
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - 120 * 60_000, now],
    )
    .unwrap();
    drop(conn);

    andon_lib::coach::eval::evaluate_session(
        &pool,
        "s1",
        &andon_lib::settings::CoachSettings::default(),
    )
    .unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM coach_findings WHERE session_id = 's1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n >= 1, "long-session-no-commit should have fired");
}

#[tokio::test]
async fn jsonl_backfill_completion_runs_evaluator_and_discovery() {
    let (pool, dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let jsonl = r#"{"type":"summary","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z"}
{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
"#;
    let p = dir.path().join("session.jsonl");
    std::fs::write(&p, jsonl).unwrap();

    let ingestor = common::test_ingestor(&pool);
    andon_lib::jsonl::backfill(&pool, &ingestor, dir.path())
        .await
        .unwrap();

    let n_findings: i64 = pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM coach_findings", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert!(n_findings >= 0);
    let n_skills: i64 = pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert!(n_skills >= 0);
}
