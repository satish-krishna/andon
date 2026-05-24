//! Rule engine: runs every enabled detector against a window and writes
//! findings via INSERT OR IGNORE so re-runs are idempotent.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::params;
use tracing::instrument;

use crate::coach::rules::{DbPool, Finding, Rule, RuleKind, Window, RULES};
use crate::coach::Result;

/// Run every enabled, non-reserved detector against `window`.
/// Findings persist via INSERT OR IGNORE on `coach_findings`.
#[instrument(skip(pool))]
pub fn evaluate_window(pool: &Arc<DbPool>, window: &Window) -> Result<()> {
    let enabled_ids = enabled_rule_ids(pool)?;
    for rule in RULES.iter().filter(|r| !r.reserved && enabled_ids.contains(r.id)) {
        match rule.kind {
            RuleKind::Binary => {
                match run_detector(pool, rule, window) {
                    Ok(findings) => write_findings(pool, &findings)?,
                    Err(e) => tracing::warn!(rule = rule.id, error = ?e, "detector failed"),
                }
            }
            RuleKind::Continuous => { /* continuous scores are read at scorecard time */ }
        }
    }
    Ok(())
}

/// Convenience wrapper: run `evaluate_window` over the last 30 days.
#[instrument(skip(pool))]
pub fn evaluate_session(pool: &Arc<DbPool>, _session_id: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: now - 30 * 86_400_000, to_ms: now, models: None };
    evaluate_window(pool, &win)
}

fn enabled_rule_ids(pool: &Arc<DbPool>) -> Result<HashSet<String>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT id FROM coach_rules WHERE enabled = 1")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn write_findings(pool: &Arc<DbPool>, findings: &[Finding]) -> Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    for f in findings {
        let _ = tx.execute(
            "INSERT OR IGNORE INTO coach_findings
               (rule_id, session_id, detected_at, payload)
             VALUES (?1, ?2, ?3, ?4)",
            params![f.rule_id, f.session_id, f.detected_at, f.payload_json],
        );
    }
    tx.commit()?;
    Ok(())
}

/// Dispatch table — populated as detectors land in Section E.
fn run_detector(pool: &Arc<DbPool>, rule: &Rule, window: &Window) -> Result<Vec<Finding>> {
    match rule.id {
        "repeated-prompts" => crate::coach::rules::detect_repeated_prompts(pool, window),
        "lazy-prompting" => crate::coach::rules::detect_lazy_prompting(pool, window),
        "low-constraint-usage" => crate::coach::rules::detect_low_constraint_usage(pool, window),
        "long-session-no-commit" => crate::coach::rules::detect_long_session_no_commit(pool, window),
        "late-night-coding" => crate::coach::rules::detect_late_night_coding(pool, window),
        "abandon-sessions" => crate::coach::rules::detect_abandon_sessions(pool, window),
        _ => Ok(vec![]),
    }
}
