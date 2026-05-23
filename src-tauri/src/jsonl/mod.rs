//! JSONL transcript ingestion. See docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md.

pub mod record;
pub mod pricing;
pub mod reducer;
pub mod parser;
pub mod walker;
pub mod reconciler;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rusqlite::params;

use crate::db::DbPool;
use crate::otlp::ingestor::Ingestor;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IngestStats {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    /// Session rows newly inserted by this run; re-ingesting an existing
    /// session contributes 0.
    pub sessions_added: i64,
    pub tokens_written: i64,
    pub cost_written: i64,
    pub duration_ms: i64,
}

#[tracing::instrument(skip(pool, ingestor))]
pub async fn backfill(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    claude_home: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "backfill", started_at)?;
    let mut stats = IngestStats::default();
    let files = walker::enumerate(claude_home);
    stats.files_processed = files.len() as i64;
    for path in &files {
        match ingest_one_inner(pool, ingestor, path).await {
            Ok(s) => {
                stats.records_processed += s.records_processed;
                stats.records_errored += s.records_errored;
                stats.sessions_added += s.sessions_added;
                stats.tokens_written += s.tokens_written;
                stats.cost_written += s.cost_written;
            }
            Err(e) => {
                tracing::error!(?path, error = ?e, "jsonl ingest failed");
                stats.records_errored += 1;
            }
        }
    }
    stats.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &stats)?;
    Ok(stats)
}

#[tracing::instrument(skip(pool, ingestor))]
pub async fn ingest_one(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    transcript_path: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "session_end", started_at)?;
    let mut s = ingest_one_inner(pool, ingestor, transcript_path).await?;
    s.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &s)?;
    Ok(s)
}

async fn ingest_one_inner(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    path: &Path,
) -> Result<IngestStats> {
    use std::panic::AssertUnwindSafe;
    let path_owned = path.to_path_buf();
    let pool_clone = Arc::clone(pool);
    let pool_for_ing = Arc::clone(pool);
    let control = ingestor.control.clone();
    let diag = ingestor.diagnostics.clone();

    let result = tokio::task::spawn_blocking(move || {
        let fresh_ing = Ingestor::new(pool_for_ing.clone(), control, diag);
        let mut stats = IngestStats {
            files_processed: 1,
            ..Default::default()
        };
        let mut reducer = reducer::Reducer::new();
        let mut events_by_session: std::collections::HashMap<String, Vec<reducer::DerivedEvent>> =
            std::collections::HashMap::new();

        let scan = parser::for_each_record(&path_owned, |r| {
            stats.records_processed += 1;
            match r {
                Ok(rec) => {
                    let res = std::panic::catch_unwind(AssertUnwindSafe(|| reducer.reduce(&rec)));
                    match res {
                        Ok(events) => {
                            for ev in events {
                                if let Some(sid) = event_session_id(&ev) {
                                    events_by_session.entry(sid).or_default().push(ev);
                                }
                            }
                        }
                        Err(_) => {
                            log_jsonl_error(
                                &pool_clone,
                                &path_owned,
                                0,
                                parser::ErrKind::ReducerPanic.as_str(),
                                "reducer panicked",
                                None,
                            );
                            stats.records_errored += 1;
                        }
                    }
                }
                Err(e) => {
                    log_jsonl_error(
                        &pool_clone,
                        &e.file,
                        e.line_no,
                        e.kind.as_str(),
                        &e.msg,
                        e.cc_version.as_deref(),
                    );
                    stats.records_errored += 1;
                }
            }
            true
        });
        // A transcript that can't be opened or read at all is an ingest
        // failure, not a silent success — log it and count it like a per-line
        // error so `records_errored` and the Diagnostics page reflect reality.
        if let Err(e) = scan {
            log_jsonl_error(
                &pool_clone,
                &path_owned,
                0,
                parser::ErrKind::Io.as_str(),
                &format!("transcript unreadable: {e}"),
                None,
            );
            stats.records_errored += 1;
        }

        for (sid, events) in events_by_session {
            let cov = reconciler::coverage_for(&pool_clone, &sid)
                .unwrap_or(reconciler::Coverage::JsonlOnly);
            match fresh_ing.ingest_derived(&events, cov) {
                Ok((tokens, cost, sessions)) => {
                    stats.tokens_written += tokens;
                    stats.cost_written += cost;
                    // `sessions` is the affected-row count of the
                    // SessionLifecycle `INSERT OR IGNORE`: 1 for a genuinely
                    // new session, 0 when re-running over an existing one.
                    stats.sessions_added += sessions;
                }
                Err(e) => tracing::error!(sid, error = ?e, "ingest_derived failed"),
            }
            let api_calls = events
                .iter()
                .filter_map(|e| match e {
                    reducer::DerivedEvent::TokenUsage { request_id, .. } => Some(request_id.clone()),
                    _ => None,
                })
                .collect::<std::collections::HashSet<_>>()
                .len() as i64;
            record_jsonl_calls(&pool_clone, &sid, api_calls);
        }
        Ok::<_, anyhow::Error>(stats)
    })
    .await??;

    Ok(result)
}

fn event_session_id(ev: &reducer::DerivedEvent) -> Option<String> {
    use reducer::DerivedEvent::*;
    match ev {
        SessionLifecycle { session_id, .. }
        | TokenUsage { session_id, .. }
        | CostEntry { session_id, .. }
        | ToolCall { session_id, .. }
        | SlashCommand { session_id, .. } => Some(session_id.clone()),
        SubAgentCall { parent_id, .. } => Some(parent_id.clone()),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn insert_run(pool: &Arc<DbPool>, kind: &str, started_at: i64) -> Result<i64> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO jsonl_ingest_runs (kind, started_at) VALUES (?1, ?2)",
        params![kind, started_at],
    )?;
    Ok(conn.last_insert_rowid())
}

fn finalise_run(pool: &Arc<DbPool>, id: i64, s: &IngestStats) -> Result<()> {
    pool.get()?.execute(
        "UPDATE jsonl_ingest_runs SET ended_at = ?1, files_processed = ?2,
                                       records_processed = ?3, records_errored = ?4
         WHERE id = ?5",
        params![
            now_ms(),
            s.files_processed,
            s.records_processed,
            s.records_errored,
            id
        ],
    )?;
    Ok(())
}

fn record_jsonl_calls(pool: &Arc<DbPool>, session_id: &str, api_calls: i64) {
    let Ok(conn) = pool.get() else { return };
    let _ = conn.execute(
        "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET api_calls = ?2, updated_at = ?3",
        params![session_id, api_calls, now_ms()],
    );
}

fn log_jsonl_error(
    pool: &Arc<DbPool>,
    path: &Path,
    line_no: usize,
    kind: &str,
    msg: &str,
    cc_version: Option<&str>,
) {
    let Ok(conn) = pool.get() else { return };
    let _ = conn.execute(
        "INSERT INTO jsonl_errors (jsonl_path, line_no, error_kind, error_msg, cc_version, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            path.display().to_string(),
            line_no as i64,
            kind,
            msg,
            cc_version,
            now_ms()
        ],
    );
}
