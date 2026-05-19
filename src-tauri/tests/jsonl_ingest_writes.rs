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
fn dedups_token_usage_against_otlp_within_window() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 10000, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    // JSONL turn at the same timestamp must NOT duplicate the existing OTLP row.
    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(),
        ts: 10_000,
        model: "claude-opus-4-7".into(),
        input: 500,
        output: 0,
        cache_create: 0,
        cache_read: 0,
    }];
    let (tokens_filled, _) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id='s1' AND model='claude-opus-4-7' AND token_type='input'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "JSONL must not duplicate the OTLP row");
    assert_eq!(tokens_filled, 0);
}

#[test]
fn gap_fills_when_otlp_partial() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    // OTLP captured only the first turn at t=100ms.
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 100, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    // JSONL has two turns: the captured one + a later gap turn at t=10_000ms.
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 500,
            output: 0,
            cache_create: 0,
            cache_read: 0,
        },
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            ts: 10_000,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 2000,
            cache_create: 0,
            cache_read: 50,
        },
    ];
    let (tokens_filled, _) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();

    let conn = pool.get().unwrap();
    // Original OTLP row preserved.
    let otlp_count: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=100 AND token_type='input'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(otlp_count, 500);

    // Gap-turn input/output/cacheRead all written.
    let gap_input: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='input'",
            [], |r| r.get(0),
        )
        .unwrap();
    let gap_output: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='output'",
            [], |r| r.get(0),
        )
        .unwrap();
    let gap_cache_read: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='cacheRead'",
            [], |r| r.get(0),
        )
        .unwrap();
    assert_eq!(gap_input, 1000);
    assert_eq!(gap_output, 2000);
    assert_eq!(gap_cache_read, 50);

    // No cacheCreation row (count was 0, skipped).
    let cache_create_n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='cacheCreation'",
            [], |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cache_create_n, 0);

    // 3 rows filled.
    assert_eq!(tokens_filled, 3);

    // data_source flipped from 'otlp' to 'mixed' once JSONL contributed.
    let data_source: String = conn
        .query_row(
            "SELECT data_source FROM sessions WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(data_source, "mixed");
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
fn gap_fills_cost_when_otlp_partial() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
         VALUES ('s1', 100, 'claude-opus-4-7', 0.01)",
        [],
    )
    .unwrap();
    drop(conn);

    let events = vec![
        // Overlap with the OTLP row — must NOT duplicate.
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.01,
        },
        // Gap — must be written.
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            ts: 10_000,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
        },
    ];
    let (_, cost_filled) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    assert_eq!(cost_filled, 1);

    let conn = pool.get().unwrap();
    let total: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((total - 0.06).abs() < 1e-9);
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
