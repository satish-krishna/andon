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
}
