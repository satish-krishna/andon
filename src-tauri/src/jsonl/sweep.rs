//! Periodic transcript sweep: re-ingest changed JSONL files when live OTLP is
//! absent. Dedup is handled at the SQL layer by `ingest_one`; this module only
//! avoids re-parsing files whose mtime has not changed since the last tick.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Tracks the last-seen mtime of each transcript so unchanged files are skipped.
/// In-memory only: on restart the map is empty, so the first tick re-ingests
/// everything — safe because `ingest_one` is idempotent.
pub struct Sweeper {
    last_seen: HashMap<PathBuf, SystemTime>,
}

impl Sweeper {
    pub fn new() -> Self {
        Self { last_seen: HashMap::new() }
    }

    /// Return the paths whose mtime is new or newer than last recorded, updating
    /// the record for every path passed in. Paths that were tracked previously
    /// but are absent from `entries` (deleted, moved, or otherwise no longer
    /// enumerated) are evicted — otherwise a path that reappears later with a
    /// coincidentally identical mtime would be skipped as "unchanged", and the
    /// map would grow without bound as transcripts come and go.
    pub fn select_changed(&mut self, entries: Vec<(PathBuf, SystemTime)>) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        let mut seen_this_call: HashMap<PathBuf, SystemTime> = HashMap::with_capacity(entries.len());
        for (path, mtime) in entries {
            let is_new = match self.last_seen.get(&path) {
                Some(prev) => mtime > *prev,
                None => true,
            };
            if is_new {
                changed.push(path.clone());
            }
            seen_this_call.insert(path, mtime);
        }
        self.last_seen = seen_this_call;
        changed
    }
}

impl Default for Sweeper {
    fn default() -> Self { Self::new() }
}

use std::sync::Arc;
use std::time::Duration;

use crate::db::DbPool;
use crate::diagnostics::Diagnostics;
use crate::otlp::ingestor::Ingestor;
use crate::otlp::IngestionControl;
use crate::settings::{SettingsStore, SweepSettings};

/// One sweep pass: enumerate, gate by mtime, ingest changed files. Per-file
/// failures are logged and skipped, never fatal.
#[tracing::instrument(skip(pool, ingestor, sweeper))]
pub async fn run_once(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    claude_home: &std::path::Path,
    sweeper: &mut Sweeper,
) -> anyhow::Result<usize> {
    let paths = crate::jsonl::walker::enumerate(claude_home);
    let entries: Vec<(PathBuf, SystemTime)> = paths
        .into_iter()
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, mtime))
        })
        .collect();
    let changed = sweeper.select_changed(entries);
    let mut ingested = 0usize;
    for path in &changed {
        match crate::jsonl::ingest_one(pool, ingestor, path).await {
            Ok(_) => ingested += 1,
            Err(e) => tracing::warn!(error = ?e, path = %path.display(), "sweep ingest_one failed"),
        }
    }
    Ok(ingested)
}

/// Whether a sweep tick should do work this pass: the feature must be enabled
/// in settings and ingestion must not be paused. Extracted as a pure function
/// so the loop's gating logic is unit-testable without spinning up the loop.
fn should_sweep(cfg: &SweepSettings, control: &IngestionControl) -> bool {
    cfg.enabled && !control.is_paused()
}

/// Delay before the next tick. `enabled=false` polls every 60s so a settings
/// toggle-on takes effect within a minute; `interval_minutes` is clamped to >=1
/// to prevent a busy loop.
pub fn next_delay(cfg: &SweepSettings) -> Duration {
    if !cfg.enabled {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(cfg.interval_minutes.max(1) as u64 * 60)
    }
}

/// Background loop. Reads settings fresh each tick so interval/toggle changes
/// take effect without a restart; skips work while ingestion is paused. The
/// first tick fires immediately (startup catch-up).
pub async fn run_sweep(
    pool: Arc<DbPool>,
    settings: Arc<SettingsStore>,
    control: IngestionControl,
    diagnostics: Diagnostics,
) {
    let claude_home = match dirs::home_dir() {
        Some(h) => h.join(".claude"),
        None => {
            tracing::warn!("no home directory; transcript sweep disabled");
            return;
        }
    };
    let ingestor = Ingestor::new(pool.clone(), control.clone(), diagnostics);
    let mut sweeper = Sweeper::new();
    loop {
        let cfg = settings.sweep();
        if should_sweep(&cfg, &control) {
            match run_once(&pool, &ingestor, &claude_home, &mut sweeper).await {
                Ok(n) if n > 0 => tracing::info!(files = n, "transcript sweep ingested changed files"),
                Ok(_) => tracing::debug!("transcript sweep: nothing changed"),
                Err(e) => tracing::warn!(error = ?e, "transcript sweep tick failed; will retry"),
            }
        }
        tokio::time::sleep(next_delay(&cfg)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    fn t(secs: u64) -> std::time::SystemTime { UNIX_EPOCH + Duration::from_secs(secs) }

    #[test]
    fn first_call_returns_all_then_unchanged_returns_none() {
        let mut s = Sweeper::new();
        let a = PathBuf::from("a.jsonl");
        let b = PathBuf::from("b.jsonl");
        let entries = vec![(a.clone(), t(100)), (b.clone(), t(100))];
        let first = s.select_changed(entries.clone());
        assert_eq!(first.len(), 2);
        let second = s.select_changed(entries);
        assert!(second.is_empty());
    }

    #[test]
    fn changed_mtime_and_new_path_are_selected() {
        let mut s = Sweeper::new();
        let a = PathBuf::from("a.jsonl");
        s.select_changed(vec![(a.clone(), t(100))]);
        // a's mtime advanced; c is brand new.
        let c = PathBuf::from("c.jsonl");
        let out = s.select_changed(vec![(a.clone(), t(200)), (c.clone(), t(50))]);
        assert!(out.contains(&a));
        assert!(out.contains(&c));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn vanished_path_is_evicted_and_treated_as_new_on_reappearance() {
        let mut s = Sweeper::new();
        let a = PathBuf::from("a.jsonl");
        let b = PathBuf::from("b.jsonl");
        // Call 1: both present.
        s.select_changed(vec![(a.clone(), t(100)), (b.clone(), t(100))]);
        // Call 2: b is gone (deleted); a unchanged.
        let out2 = s.select_changed(vec![(a.clone(), t(100))]);
        assert!(out2.is_empty());
        // Call 3: b reappears with the SAME mtime it had in call 1. If it were
        // still tracked, this would look "unchanged" and be skipped; since it
        // was evicted in call 2, it must be treated as new.
        let out3 = s.select_changed(vec![(a.clone(), t(100)), (b.clone(), t(100))]);
        assert_eq!(out3, vec![b]);
    }

    #[test]
    fn next_delay_honours_enabled_and_interval() {
        use crate::settings::SweepSettings;
        let off = SweepSettings { interval_minutes: 5, enabled: false };
        assert_eq!(next_delay(&off), std::time::Duration::from_secs(60));
        let on = SweepSettings { interval_minutes: 5, enabled: true };
        assert_eq!(next_delay(&on), std::time::Duration::from_secs(300));
        // interval 0 must not busy-loop: clamp to 1 minute.
        let zero = SweepSettings { interval_minutes: 0, enabled: true };
        assert_eq!(next_delay(&zero), std::time::Duration::from_secs(60));
    }

    #[test]
    fn should_sweep_covers_enabled_and_paused_combinations() {
        use crate::otlp::IngestionControl;

        let enabled_cfg = SweepSettings { interval_minutes: 5, enabled: true };
        let disabled_cfg = SweepSettings { interval_minutes: 5, enabled: false };

        let running = IngestionControl::new();
        assert!(!running.is_paused());
        let paused = IngestionControl::new();
        paused.set_paused(true);

        assert!(should_sweep(&enabled_cfg, &running));
        assert!(!should_sweep(&enabled_cfg, &paused));
        assert!(!should_sweep(&disabled_cfg, &running));
        assert!(!should_sweep(&disabled_cfg, &paused));
    }

    #[tokio::test]
    async fn run_once_on_empty_home_ingests_nothing() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        // No projects/ dir at all -> enumerate returns empty.
        let pool = Arc::new(crate::db::init(&tmp.path().join("t.db")).unwrap());
        let control = crate::otlp::IngestionControl::new();
        let diag = crate::diagnostics::Diagnostics::new();
        let ing = crate::otlp::ingestor::Ingestor::new(pool.clone(), control, diag);
        let mut sweeper = Sweeper::new();
        let n = run_once(&pool, &ing, tmp.path(), &mut sweeper).await.unwrap();
        assert_eq!(n, 0);
    }
}
