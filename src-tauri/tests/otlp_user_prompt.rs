mod common;

#[test]
fn user_prompt_body_is_persisted_to_log_events() {
    let (pool, _db_dir) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // Seed the session so the FK constraint doesn't block.
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 1)",
            [],
        )
        .unwrap();
    }

    let logs = common::sample_export_logs_with_body(
        vec![common::kv("session.id", "s1")],
        "user_prompt",
        "tell me about wizards",
        vec![common::kv_int("prompt_length", 21)],
    );
    ingestor.ingest_logs_v2(logs, "grpc").expect("ingest");

    let body: Option<String> = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT body FROM log_events WHERE event_name='user_prompt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        body.as_deref(),
        Some("tell me about wizards"),
        "body must be persisted post-amendment, not redacted"
    );
}
