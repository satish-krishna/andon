//! Per-session OTLP-vs-JSONL routing. OTLP-covered iff any token_usage row exists.

use anyhow::Result;
use rusqlite::params;
use std::sync::Arc;

use crate::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    Otlp,
    JsonlOnly,
}

pub fn coverage_for(pool: &Arc<DbPool>, session_id: &str) -> Result<Coverage> {
    let conn = pool.get()?;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id = ?1 LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(if n > 0 {
        Coverage::Otlp
    } else {
        Coverage::JsonlOnly
    })
}

/// Returns true if `token_usage` already has a row for this
/// (session_id, model, token_type) with a timestamp within ±window_ms of `ts_ms`.
/// Used to dedup JSONL-derived rows against any OTLP-emitted rows for the same turn.
pub fn token_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    token_type: &str,
    window_ms: i64,
) -> bool {
    let Ok(conn) = pool.get() else {
        return true; // conservative: skip the write on pool failure
    };
    let lo = ts_ms - window_ms;
    let hi = ts_ms + window_ms;
    conn.query_row(
        "SELECT 1 FROM token_usage
         WHERE session_id = ?1 AND model = ?2 AND token_type = ?3
           AND timestamp BETWEEN ?4 AND ?5
         LIMIT 1",
        params![session_id, model, token_type, lo, hi],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

/// Returns true if `cost_entries` already has a row for this
/// (session_id, model) with a timestamp within ±window_ms of `ts_ms`.
pub fn cost_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    window_ms: i64,
) -> bool {
    let Ok(conn) = pool.get() else {
        return true;
    };
    let lo = ts_ms - window_ms;
    let hi = ts_ms + window_ms;
    conn.query_row(
        "SELECT 1 FROM cost_entries
         WHERE session_id = ?1 AND model = ?2
           AND timestamp BETWEEN ?3 AND ?4
         LIMIT 1",
        params![session_id, model, lo, hi],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> Arc<DbPool> {
        let dir = tempfile::tempdir().unwrap();
        let p = crate::db::init(&dir.path().join("t.db")).unwrap();
        Box::leak(Box::new(dir));
        Arc::new(p)
    }

    #[test]
    fn no_token_usage_is_jsonl_only() {
        assert_eq!(coverage_for(&pool(), "sX").unwrap(), Coverage::JsonlOnly);
    }

    #[test]
    fn token_usage_present_is_otlp() {
        let p = pool();
        let c = p.get().unwrap();
        c.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('sY', 0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES ('sY', 0, 'm', 'input', 1)",
            [],
        )
        .unwrap();
        assert_eq!(coverage_for(&p, "sY").unwrap(), Coverage::Otlp);
    }

    #[test]
    fn token_row_already_covered_within_5s_window() {
        let p = pool();
        let c = p.get().unwrap();
        c.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES ('s1', 10000, 'claude-opus-4-7', 'input', 500)",
            [],
        )
        .unwrap();
        drop(c);

        let pool_ref = p.as_ref();
        // Exact-timestamp hit.
        assert!(token_row_already_covered(
            pool_ref, "s1", 10_000, "claude-opus-4-7", "input", 5_000,
        ));
        // Within window.
        assert!(token_row_already_covered(
            pool_ref, "s1", 11_000, "claude-opus-4-7", "input", 5_000,
        ));
        // Outside window.
        assert!(!token_row_already_covered(
            pool_ref, "s1", 16_000, "claude-opus-4-7", "input", 5_000,
        ));
        // Different model.
        assert!(!token_row_already_covered(
            pool_ref, "s1", 10_000, "claude-sonnet-4-6", "input", 5_000,
        ));
        // Different token_type.
        assert!(!token_row_already_covered(
            pool_ref, "s1", 10_000, "claude-opus-4-7", "output", 5_000,
        ));
        // Different session.
        assert!(!token_row_already_covered(
            pool_ref, "s2", 10_000, "claude-opus-4-7", "input", 5_000,
        ));
    }

    #[test]
    fn cost_row_already_covered_within_5s_window() {
        let p = pool();
        let c = p.get().unwrap();
        c.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES ('s1', 10000, 'claude-opus-4-7', 0.05)",
            [],
        )
        .unwrap();
        drop(c);

        let pool_ref = p.as_ref();
        assert!(cost_row_already_covered(
            pool_ref, "s1", 11_000, "claude-opus-4-7", 5_000,
        ));
        assert!(!cost_row_already_covered(
            pool_ref, "s1", 16_000, "claude-opus-4-7", 5_000,
        ));
        assert!(!cost_row_already_covered(
            pool_ref, "s1", 10_000, "claude-sonnet-4-6", 5_000,
        ));
    }
}
