//! Coach module — anti-pattern rules, scorecard, and Skill Finder.
//!
//! See `docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md`
//! for the architecture and rule catalogue. The module reads existing
//! Andon tables plus `prompt_turns` and writes findings to
//! `coach_findings`.

pub mod queries;
pub mod rules;
pub mod engine;
pub mod score;
pub mod skill;
pub mod eval;

use std::sync::Arc;

/// The five practice areas. Keep ordered — the UI renders left-to-right.
pub const PRACTICES: &[&str] = &["prompt", "hygiene", "review", "tool", "context"];

#[derive(Debug, thiserror::Error)]
pub enum CoachError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoachError>;

/// Insert every entry in [`rules::RULES`] into `coach_rules` using
/// `INSERT OR IGNORE`, so the user's enable/disable state survives upgrades.
#[tracing::instrument(skip(pool))]
pub fn seed_rules(pool: &Arc<rules::DbPool>) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    for r in rules::RULES {
        let sev = r.severity.map(|s| s.as_str()).unwrap_or("none");
        let kind = match r.kind {
            rules::RuleKind::Binary => "binary",
            rules::RuleKind::Continuous => "continuous",
        };
        tx.execute(
            "INSERT OR IGNORE INTO coach_rules
               (id, practice, severity, kind, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            rusqlite::params![r.id, r.practice, sev, kind, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}
