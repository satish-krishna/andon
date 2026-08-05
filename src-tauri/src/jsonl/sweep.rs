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
    /// the record for every path passed in.
    pub fn select_changed(&mut self, entries: Vec<(PathBuf, SystemTime)>) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for (path, mtime) in entries {
            let is_new = match self.last_seen.get(&path) {
                Some(prev) => mtime > *prev,
                None => true,
            };
            if is_new {
                changed.push(path.clone());
            }
            self.last_seen.insert(path, mtime);
        }
        changed
    }
}

impl Default for Sweeper {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

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
        // a's mtime advanced; c is brand new; a unchanged-b is gone.
        let c = PathBuf::from("c.jsonl");
        let out = s.select_changed(vec![(a.clone(), t(200)), (c.clone(), t(50))]);
        assert!(out.contains(&a));
        assert!(out.contains(&c));
        assert_eq!(out.len(), 2);
    }
}
