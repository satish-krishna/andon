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
#[instrument(skip(pool, coach_settings))]
pub fn evaluate_window(
    pool: &Arc<DbPool>,
    window: &Window,
    coach_settings: &crate::settings::CoachSettings,
) -> Result<()> {
    let enabled_ids = enabled_rule_ids(pool)?;
    for rule in RULES.iter().filter(|r| !r.reserved && enabled_ids.contains(&r.id.to_string())) {
        match rule.kind {
            RuleKind::Binary => {
                match run_detector(pool, rule, window, coach_settings) {
                    Ok(findings) => write_findings(pool, &findings)?,
                    Err(e) => tracing::warn!(rule = rule.id, error = ?e, "detector failed"),
                }
            }
            RuleKind::Continuous => { /* continuous scores are read at scorecard time */ }
        }
    }
    Ok(())
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
fn run_detector(
    pool: &Arc<DbPool>,
    rule: &Rule,
    window: &Window,
    coach_settings: &crate::settings::CoachSettings,
) -> Result<Vec<Finding>> {
    match rule.id {
        "repeated-prompts" => crate::coach::rules::detect_repeated_prompts(pool, window),
        "lazy-prompting" => crate::coach::rules::detect_lazy_prompting(pool, window),
        "low-constraint-usage" => crate::coach::rules::detect_low_constraint_usage(pool, window),
        "long-session-no-commit" => crate::coach::rules::detect_long_session_no_commit(pool, window),
        "late-night-coding" => crate::coach::rules::detect_late_night_coding(pool, window),
        "abandon-sessions" => crate::coach::rules::detect_abandon_sessions(pool, window),
        "speed-accept" => crate::coach::rules::detect_speed_accept(pool, window),
        "no-slash-commands" => crate::coach::rules::detect_no_slash_commands(pool, window),
        "cache-hit-starvation" => crate::coach::rules::detect_cache_hit_starvation(pool, window),
        "low-spec-rate" => crate::coach::rules::detect_low_spec_rate(pool, window, coach_settings),
        _ => Ok(vec![]),
    }
}
