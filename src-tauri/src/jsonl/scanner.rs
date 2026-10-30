//! Background JSONL scanner.
//!
//! Keeps cost and token data flowing when Andon receives no OTLP — e.g. an
//! organization's managed settings route Claude Code telemetry to their own
//! collector — and when the `SessionEnd` hook cannot run (managed
//! `allowManagedHooksOnly`). Transcripts are always written locally, so a
//! periodic scan of `~/.claude/projects` is the one source that no setting can
//! redirect.
//!
//! A transcript is ingested once it has been quiet for [`SETTLE_MS`]: OTLP
//! metrics export on a ~60 s cadence, so waiting lets the reconciler see an
//! OTLP-covered session as such before JSONL would otherwise claim it and
//! double-count. Per-file stamps in `jsonl_file_state` mean an unchanged file
//! is never re-read, and a pricing-table bump re-ingests (and reprices) all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::params;

use super::{finalise_run, ingest_one_inner, insert_run, now_ms, pricing, walker, IngestStats};
use crate::db::DbPool;
use crate::otlp::ingestor::Ingestor;

/// How often the scanner looks for changed transcripts.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(2 * 60);

/// How long a transcript must be unmodified before it is ingested.
pub const SETTLE_MS: i64 = 5 * 60 * 1000;

/// The scanner's notion of time, injected so tests control settling.
#[derive(Debug, Clone, Copy)]
pub struct ScanClock {
    pub now_ms: i64,
    pub settle_ms: i64,
}

impl ScanClock {
    pub fn system() -> Self {
        Self {
            now_ms: now_ms(),
            settle_ms: SETTLE_MS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime_ms: i64,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestedStamp {
    pub stamp: FileStamp,
    pub pricing_version: i64,
}

/// Whether a transcript should be (re-)ingested now.
pub fn is_due(
    stamp: FileStamp,
    last: Option<IngestedStamp>,
    clock: ScanClock,
    pricing_version: i64,
) -> bool {
    if clock.now_ms.saturating_sub(stamp.mtime_ms) < clock.settle_ms {
        return false;
    }
    match last {
        None => true,
        Some(l) => l.stamp != stamp || l.pricing_version != pricing_version,
    }
}

/// Run the scanner forever, one pass every [`SCAN_INTERVAL`]. The first pass
/// runs immediately, which also backfills history on first launch.
pub async fn run_scanner(pool: Arc<DbPool>, ingestor: Ingestor, claude_home: PathBuf) {
    let mut interval = tokio::time::interval(SCAN_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        match scan_once(&pool, &ingestor, &claude_home, ScanClock::system()).await {
            Ok(s) if s.files_processed > 0 => tracing::info!(
                files = s.files_processed,
                tokens = s.tokens_written,
                cost = s.cost_written,
                "jsonl scan ingested transcripts"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = ?e, "jsonl scan failed; will retry"),
        }
    }
}

/// One scan pass: ingest every settled transcript that changed since it was
/// last ingested. A no-op while ingestion is paused.
#[tracing::instrument(skip(pool, ingestor))]
pub async fn scan_once(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    claude_home: &Path,
    clock: ScanClock,
) -> Result<IngestStats> {
    let mut stats = IngestStats::default();
    if ingestor.control.is_paused() {
        return Ok(stats);
    }

    let due = {
        let pool = Arc::clone(pool);
        let home = claude_home.to_path_buf();
        tokio::task::spawn_blocking(move || due_files(&pool, &home, clock)).await??
    };
    if due.is_empty() {
        return Ok(stats);
    }

    let started_at = now_ms();
    let run_id = insert_run(pool, "scan", started_at)?;
    for (path, stamp) in &due {
        match ingest_one_inner(pool, ingestor, path).await {
            Ok(s) => {
                stats.files_processed += 1;
                stats.records_processed += s.records_processed;
                stats.records_errored += s.records_errored;
                stats.sessions_added += s.sessions_added;
                stats.tokens_written += s.tokens_written;
                stats.cost_written += s.cost_written;
                if let Err(e) = record_stamp(pool, path, *stamp) {
                    tracing::warn!(?path, error = ?e, "jsonl file state not recorded");
                }
            }
            Err(e) => {
                tracing::error!(?path, error = ?e, "jsonl scan ingest failed");
                stats.records_errored += 1;
            }
        }
    }
    stats.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &stats)?;
    Ok(stats)
}

fn due_files(pool: &DbPool, claude_home: &Path, clock: ScanClock) -> Result<Vec<(PathBuf, FileStamp)>> {
    let known = load_stamps(pool)?;
    Ok(walker::enumerate(claude_home)
        .into_iter()
        .filter_map(|path| {
            let stamp = stat(&path)?;
            let last = known.get(&path_key(&path)).copied();
            is_due(stamp, last, clock, pricing::PRICING_VERSION).then_some((path, stamp))
        })
        .collect())
}

fn stat(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some(FileStamp {
        mtime_ms,
        size_bytes: meta.len() as i64,
    })
}

fn path_key(path: &Path) -> String {
    path.display().to_string()
}

fn load_stamps(pool: &DbPool) -> Result<HashMap<String, IngestedStamp>> {
    let conn = pool.get()?;
    let mut stmt =
        conn.prepare("SELECT path, mtime_ms, size_bytes, pricing_version FROM jsonl_file_state")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            IngestedStamp {
                stamp: FileStamp {
                    mtime_ms: r.get(1)?,
                    size_bytes: r.get(2)?,
                },
                pricing_version: r.get(3)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn record_stamp(pool: &DbPool, path: &Path, stamp: FileStamp) -> Result<()> {
    pool.get()?.execute(
        "INSERT INTO jsonl_file_state (path, mtime_ms, size_bytes, pricing_version, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET
             mtime_ms = ?2, size_bytes = ?3, pricing_version = ?4, ingested_at = ?5",
        params![
            path_key(path),
            stamp.mtime_ms,
            stamp.size_bytes,
            pricing::PRICING_VERSION,
            now_ms()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: FileStamp = FileStamp {
        mtime_ms: 1_000,
        size_bytes: 10,
    };

    fn clock(now_ms: i64) -> ScanClock {
        ScanClock {
            now_ms,
            settle_ms: 100,
        }
    }

    fn ingested(stamp: FileStamp, pricing_version: i64) -> Option<IngestedStamp> {
        Some(IngestedStamp {
            stamp,
            pricing_version,
        })
    }

    #[test]
    fn new_settled_file_is_due() {
        assert!(is_due(STAMP, None, clock(1_100), 1));
    }

    #[test]
    fn unsettled_file_is_not_due() {
        assert!(!is_due(STAMP, None, clock(1_099), 1));
    }

    #[test]
    fn unchanged_file_is_not_due() {
        assert!(!is_due(STAMP, ingested(STAMP, 1), clock(5_000), 1));
    }

    #[test]
    fn grown_file_is_due() {
        let grown = FileStamp {
            size_bytes: 20,
            ..STAMP
        };
        assert!(is_due(grown, ingested(STAMP, 1), clock(5_000), 1));
    }

    #[test]
    fn pricing_bump_makes_unchanged_file_due() {
        assert!(is_due(STAMP, ingested(STAMP, 1), clock(5_000), 2));
    }
}
