mod common;

use andon_lib::jsonl::reconciler::Coverage;
use andon_lib::jsonl::reducer::DerivedEvent;
use common::{fixture_pool, test_ingestor};

#[test]
fn writes_slash_and_subagent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::SlashCommand {
            session_id: "s1".into(),
            ts: 100,
            name: "review".into(),
            arg_count: 1,
        },
        DerivedEvent::SubAgentCall {
            parent_id: "s1".into(),
            child_id: Some("c".into()),
            subagent_type: Some("Explore".into()),
            started_at: 110,
        },
    ];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let conn = pool.get().unwrap();
    let cmd: String = conn
        .query_row(
            "SELECT command_name FROM slash_commands WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cmd, "review");
    let st: String = conn
        .query_row(
            "SELECT subagent_type FROM subagent_calls WHERE parent_session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(st, "Explore");
}

#[test]
fn otlp_coverage_skips_jsonl_cost_and_tokens() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    // An OTLP token row exists for this session — it is OTLP-covered.
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 100, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: 9000,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 2000,
            cache_create: 0,
            cache_read: 0,
            is_subagent: false,
        },
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: 9000,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.42,
            is_subagent: false,
        },
    ];
    let (tokens_written, cost_written, _) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    assert_eq!((tokens_written, cost_written), (0, 0));

    let conn = pool.get().unwrap();
    let tok: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='s1'", [], |r| r.get(0))
        .unwrap();
    let cost: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tok, 1, "only the original OTLP token row remains");
    assert_eq!(cost, 0, "no JSONL cost written for an OTLP-covered session");
}

#[test]
fn jsonl_only_writes_cost_and_tokens_with_request_id() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s2".into(),
            request_id: "req_y".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 500,
            cache_create: 0,
            cache_read: 0,
            is_subagent: false,
        },
        DerivedEvent::CostEntry {
            session_id: "s2".into(),
            request_id: "req_y".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
            is_subagent: false,
        },
    ];
    let (tokens_written, cost_written, _) =
        ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    assert_eq!(tokens_written, 2, "input + output rows");
    assert_eq!(cost_written, 1);

    let conn = pool.get().unwrap();
    let rid: String = conn
        .query_row("SELECT request_id FROM cost_entries WHERE session_id='s2'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rid, "req_y");
}

#[test]
fn writes_tool_decisions_for_jsonl_only_session() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![DerivedEvent::ToolCall {
        session_id: "s1".into(),
        ts: 100,
        tool_name: "Read".into(),
        file_path: Some("a.rs".into()),
        model: Some("claude-opus-4-7".into()),
    }];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let conn = pool.get().unwrap();
    let (src, model): (String, String) = conn
        .query_row(
            "SELECT source, model FROM tool_decisions WHERE session_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(src, "jsonl");
    assert_eq!(model, "claude-opus-4-7");
}

#[test]
fn jsonl_reingest_is_idempotent_via_request_id() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s3".into(),
            request_id: "req_z".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 500,
            cache_create: 0,
            cache_read: 0,
            is_subagent: false,
        },
        DerivedEvent::CostEntry {
            session_id: "s3".into(),
            request_id: "req_z".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
            is_subagent: false,
        },
    ];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let (t2, c2, _) = ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    assert_eq!((t2, c2), (0, 0), "second ingest writes nothing");

    let conn = pool.get().unwrap();
    let cost: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='s3'", [], |r| r.get(0))
        .unwrap();
    let tok: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='s3'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost, 1);
    assert_eq!(tok, 2);
}

#[test]
fn dedups_slash_commands_on_repeat() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![DerivedEvent::SlashCommand {
        session_id: "s1".into(),
        ts: 100,
        name: "review".into(),
        arg_count: 1,
    }];

    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM slash_commands WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second call must not duplicate slash_command");
}

#[test]
fn dedups_subagent_calls_on_repeat() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![DerivedEvent::SubAgentCall {
        parent_id: "s1".into(),
        child_id: None,
        subagent_type: Some("Explore".into()),
        started_at: 100,
    }];

    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM subagent_calls WHERE parent_session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second call must not duplicate subagent_call");
}
