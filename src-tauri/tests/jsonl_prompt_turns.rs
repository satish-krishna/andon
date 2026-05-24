mod common;

use andon_lib::jsonl::reconciler::Coverage;
use andon_lib::jsonl::reducer::{DerivedEvent, ReduceOptions, Reducer};
use common::{fixture_pool, test_ingestor};

/// Test the full reducer → ingest_derived → prompt_turns pipeline.
/// Uses the direct `ingest_derived` API (same pattern as jsonl_ingest_writes.rs)
/// so the test exercises the writer without needing a real file on disk.
#[test]
fn jsonl_ingest_writes_prompt_turns_with_flags() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    // Seed a session row for the FK constraint.
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 1)",
            [],
        )
        .unwrap();
    }

    // Build the PromptTurn event via the reducer so norm_hash, length, and
    // flags are computed the same way production code computes them.
    let jsonl_line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Refactor @src/foo.rs — must be idempotent."}]}}"#;

    let record = andon_lib::jsonl::record::parse_line(jsonl_line).expect("parse");
    let mut reducer = Reducer::with_options(ReduceOptions {
        constraint_keywords: vec!["must".into(), "should".into()],
    });
    let events = reducer.reduce(&record);

    // The reducer may emit SessionLifecycle + PromptTurn; find the PromptTurn.
    let prompt_turn = events
        .iter()
        .find(|e| matches!(e, DerivedEvent::PromptTurn { .. }))
        .expect("expected at least one PromptTurn event");
    let DerivedEvent::PromptTurn {
        ref text,
        has_constraint,
        has_file_ref,
        length,
        ..
    } = prompt_turn
    else {
        unreachable!()
    };
    // Sanity-check the reducer output before writing to DB.
    assert!(has_constraint, "reducer should detect 'must' as constraint keyword");
    assert!(has_file_ref, "reducer should detect '@src/foo.rs' as file ref");
    assert!(*length > 0, "length must be positive");

    // Write to DB.
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    // Query prompt_turns and verify the row was written with correct values.
    let (stored_text, stored_has_constraint, stored_has_file_ref, stored_length): (
        String,
        i64,
        i64,
        i64,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT text, has_constraint, has_file_ref, length \
             FROM prompt_turns WHERE session_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();

    assert_eq!(stored_text, *text);
    assert_eq!(stored_has_constraint, 1, "has_constraint should be 1");
    assert_eq!(stored_has_file_ref, 1, "has_file_ref should be 1");
    assert_eq!(stored_length, *length, "stored length matches reducer output");
}

/// Re-ingesting the same PromptTurn must not create a duplicate row
/// (INSERT OR IGNORE on the (session_id, turn_index) primary key).
#[test]
fn jsonl_prompt_turns_reingest_is_idempotent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s2', 1)",
            [],
        )
        .unwrap();
    }

    let events = vec![DerivedEvent::PromptTurn {
        session_id: "s2".into(),
        request_id: Some("req_a".into()),
        turn_index: 0,
        ts_ms: 1_000,
        text: "hello world".into(),
        norm_hash: "abc".into(),
        command: None,
        length: 11,
        has_file_ref: false,
        has_code: false,
        has_constraint: false,
    }];

    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM prompt_turns WHERE session_id='s2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second ingest must not duplicate the prompt_turn row");
}
