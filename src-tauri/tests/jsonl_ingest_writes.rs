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
fn skips_token_usage_when_otlp_covered() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(),
        ts: 100,
        model: "claude-opus-4-7".into(),
        input: 10,
        output: 20,
        cache_create: 0,
        cache_read: 0,
    }];
    ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    let conn = pool.get().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n, 0,
        "JSONL must not write token_usage for OTLP-covered sessions"
    );
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
