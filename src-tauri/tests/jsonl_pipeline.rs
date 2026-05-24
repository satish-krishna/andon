mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use andon_lib::{jsonl, settings::CoachSettings};
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
    let stats = jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default())
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
            r#"{"type":"assistant","sessionId":"sIDP","requestId":"req_idp","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":200}}}"#,
        ],
    );
    let pool_arc = Arc::clone(&pool);

    let s1 = jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();
    let s2 = jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();

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
    assert!(s1.tokens_written >= 2, "first run filled at least 2 token rows");
    assert_eq!(s2.tokens_written, 0, "second run filled nothing");
    assert_eq!(s2.cost_written, 0);
    assert_eq!(s1.sessions_added, 1, "first run inserts the session");
    assert_eq!(s2.sessions_added, 0, "re-running adds no new session");
}

#[tokio::test]
async fn ingest_one_unreadable_file_is_counted() {
    // A transcript path that can't be opened must surface as an ingest error,
    // not a silent records_processed=0 success that hides the failure.
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("never-written.jsonl");

    let pool_arc = Arc::clone(&pool);
    let stats = jsonl::ingest_one(&pool_arc, &ing, &missing).await.unwrap();
    assert_eq!(stats.records_errored, 1, "unreadable transcript counted as an error");

    let conn = pool.get().unwrap();
    let errs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jsonl_errors WHERE error_kind='io_error'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(errs, 1, "I/O failure logged to jsonl_errors");
}

#[tokio::test]
async fn backfill_counts_multi_record_request_once() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    // One API call (req_M) split across three assistant records, each carrying
    // the identical usage — exactly how Claude Code writes transcripts.
    write_transcript(home.path(), "m", &[
        r#"{"type":"user","sessionId":"sM","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"text","text":"x"}]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t2","name":"Grep","input":{}}]}}"#,
    ]);

    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();
    jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();

    let conn = pool.get().unwrap();
    // 1M input tokens of opus-4-7 = $15.00, counted exactly once despite 3 records
    // and 2 backfill runs.
    let cost: f64 = conn
        .query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id='sM'", [], |r| r.get(0))
        .unwrap();
    assert!((cost - 15.0).abs() < 1e-6, "expected $15.00, got {cost}");
    let cost_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sM'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_rows, 1, "one cost row per API call");
    let input_tokens: i64 = conn
        .query_row("SELECT COALESCE(SUM(count), 0) FROM token_usage WHERE session_id='sM' AND token_type='input'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(input_tokens, 1_000_000, "input tokens counted once");
}

#[tokio::test]
async fn reingest_of_grown_transcript_adds_new_turns() {
    // A JSONL-only session must stay JSONL-only across re-ingests: the second
    // backfill picks up the new turn (req_g2) without duplicating req_g1.
    // Guards the `coverage_for` `request_id IS NULL` refinement from Task 4.
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();

    write_transcript(home.path(), "g", &[
        r#"{"type":"user","sessionId":"sG","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
    ]);
    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();

    // The session resumes — the transcript file grows a second API call.
    write_transcript(home.path(), "g", &[
        r#"{"type":"user","sessionId":"sG","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g2","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
    ]);
    jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();

    let conn = pool.get().unwrap();
    let cost_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sG'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_rows, 2, "new turn ingested, old turn not duplicated");
}

#[tokio::test]
async fn backfill_records_api_call_count() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "c", &[
        r#"{"type":"user","sessionId":"sC","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_1","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_2","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
    ]);

    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path(), &CoachSettings::default()).await.unwrap();

    let conn = pool.get().unwrap();
    let api_calls: i64 = conn
        .query_row("SELECT api_calls FROM session_jsonl_calls WHERE session_id='sC'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(api_calls, 2, "two distinct requestIds across three records");
}
