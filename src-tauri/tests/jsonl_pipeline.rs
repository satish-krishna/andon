mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use andon_lib::jsonl;
use common::{fixture_pool, test_ingestor};

fn write_transcript(dir: &Path, slug: &str, lines: &[&str]) {
    let proj = dir.join("projects").join(slug);
    fs::create_dir_all(&proj).unwrap();
    fs::write(proj.join("session.jsonl"), lines.join("\n")).unwrap();
}

#[tokio::test]
async fn backfill_processes_synthetic_session() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "p", &[
        r#"{"type":"user","sessionId":"sess-1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name><command-args>x</command-args>"}]}}"#,
        r#"{"type":"assistant","sessionId":"sess-1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":50,"output_tokens":100},"content":[{"type":"tool_use","id":"u1","name":"Task","input":{"subagent_type":"Explore"}}]}}"#,
    ]);

    let pool_arc = Arc::clone(&pool);
    let stats = jsonl::backfill(&pool_arc, &ing, home.path())
        .await
        .unwrap();
    assert_eq!(stats.files_processed, 1);
    assert_eq!(stats.records_errored, 0);

    let conn = pool.get().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id='sess-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM slash_commands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM subagent_calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn backfill_is_idempotent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    // One turn with tokens + cost so the second run has something to dedup against.
    write_transcript(
        home.path(),
        "x",
        &[
            r#"{"type":"user","sessionId":"sIDP","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
            r#"{"type":"assistant","sessionId":"sIDP","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":200}}}"#,
        ],
    );
    let pool_arc = Arc::clone(&pool);

    let s1 = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let s2 = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    let conn = pool.get().unwrap();
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    let tokens: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    let costs: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1, "session row not duplicated");
    assert_eq!(tokens, 2, "input + output rows; no duplicates from second run");
    assert_eq!(costs, 1, "single cost row");
    assert!(s1.tokens_filled >= 2, "first run filled at least 2 token rows");
    assert_eq!(s2.tokens_filled, 0, "second run filled nothing");
    assert_eq!(s2.cost_filled, 0);
}
