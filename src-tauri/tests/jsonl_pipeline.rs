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
    write_transcript(
        home.path(),
        "x",
        &[
            r#"{"type":"user","sessionId":"sIDP","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        ],
    );
    let pool_arc = Arc::clone(&pool);
    let _ = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let _ = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let conn = pool.get().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id='sIDP'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second run must not duplicate the session row");
}
