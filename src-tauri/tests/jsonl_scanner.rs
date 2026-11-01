mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use andon_lib::jsonl::scanner::{self, ScanClock};
use common::{fixture_pool, test_ingestor, test_ingestor_with_control};

const TURN_1: &str = r#"{"type":"assistant","sessionId":"sS","requestId":"req_s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;
const TURN_2: &str = r#"{"type":"assistant","sessionId":"sS","requestId":"req_s2","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;

fn write_transcript(home: &Path, lines: &[&str]) -> PathBuf {
    let proj = home.join("projects").join("p");
    fs::create_dir_all(&proj).unwrap();
    let path = proj.join("session.jsonl");
    fs::write(&path, lines.join("\n")).unwrap();
    path
}

/// A clock far enough ahead that every file has settled.
fn settled() -> ScanClock {
    ScanClock {
        now_ms: i64::MAX / 2,
        settle_ms: scanner::SETTLE_MS,
    }
}

fn session_cost(pool: &Arc<andon_lib::db::DbPool>) -> f64 {
    pool.get()
        .unwrap()
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id='sS'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn scan_ingests_settled_transcript_once() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), &[TURN_1]);

    let first = scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    assert_eq!(first.files_processed, 1);
    assert!((session_cost(&pool) - 3.0).abs() < 1e-9, "1M sonnet-4-6 input = $3");

    let second = scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    assert_eq!(second.files_processed, 0, "an unchanged file is not re-read");
}

#[tokio::test]
async fn scan_skips_transcript_still_being_written() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), &[TURN_1]);

    // "Now" is the moment of writing: the file has not been quiet for the
    // settle window, so OTLP may still be about to claim this session.
    let clock = ScanClock {
        now_ms: andon_lib::jsonl::now_ms(),
        settle_ms: scanner::SETTLE_MS,
    };
    let stats = scanner::scan_once(&pool, &ing, home.path(), clock).await.unwrap();
    assert_eq!(stats.files_processed, 0);
    assert_eq!(session_cost(&pool), 0.0);
}

#[tokio::test]
async fn scan_picks_up_new_turns_in_a_grown_transcript() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), &[TURN_1]);
    scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();

    write_transcript(home.path(), &[TURN_1, TURN_2]);
    let stats = scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    assert_eq!(stats.files_processed, 1, "a changed file is re-read");
    assert!((session_cost(&pool) - 6.0).abs() < 1e-9, "both turns counted once each");
}

#[tokio::test]
async fn scan_does_nothing_while_ingestion_is_paused() {
    let (pool, _g) = fixture_pool();
    let (ing, control) = test_ingestor_with_control(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), &[TURN_1]);

    control.set_paused(true);
    let paused = scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    assert_eq!(paused.files_processed, 0);

    // The file must not be marked as ingested while paused, or resuming would
    // never pick it up.
    control.set_paused(false);
    let resumed = scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    assert_eq!(resumed.files_processed, 1);
    assert!((session_cost(&pool) - 3.0).abs() < 1e-9);
}

#[tokio::test]
async fn scan_records_a_run_only_when_it_ingests() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), &[TURN_1]);

    scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();
    scanner::scan_once(&pool, &ing, home.path(), settled()).await.unwrap();

    let runs: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM jsonl_ingest_runs WHERE kind = 'scan'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(runs, 1, "idle scans do not flood the ingest-run log");
}
