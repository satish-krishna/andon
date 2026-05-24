//! AIEC scoring formula:
//!   sevPenalty = {high: 12, medium: 7, low: 3}
//!   penalty    = Σ sevPenalty[r.severity] for r in triggered_in_practice
//!   maxPenalty = |enabled_detectors_in_practice| × 12
//!   score      = max(0, round(100 × (1 - penalty / maxPenalty)))

use std::sync::Arc;
use serde::Serialize;
use tracing::instrument;

use crate::coach::rules::{Window, DbPool, RULES};
use crate::coach::Result;

#[derive(Debug, Serialize)]
pub struct PracticeScore {
    pub practice: String,
    pub score: Option<i64>,
    pub status: String,
    pub triggered_count: i64,
}

#[instrument(skip(pool))]
pub fn practice_score(pool: &Arc<DbPool>, practice: &str, window: &Window) -> Result<PracticeScore> {
    let conn = pool.get()?;
    let mut enabled_stmt = conn.prepare(
        "SELECT id FROM coach_rules
         WHERE practice = ?1 AND kind = 'binary' AND enabled = 1",
    )?;
    let enabled_ids: Vec<String> = enabled_stmt
        .query_map(rusqlite::params![practice], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    if enabled_ids.is_empty() {
        return Ok(PracticeScore {
            practice: practice.into(),
            score: None,
            status: "n/a".into(),
            triggered_count: 0,
        });
    }

    let placeholders: String = enabled_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 3))
        .collect::<Vec<_>>().join(",");
    let q = format!(
        "SELECT DISTINCT cf.rule_id
         FROM coach_findings cf
         JOIN sessions s ON s.session_id = cf.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND cf.rule_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&q)?;
    let mut params_dyn: Vec<&dyn rusqlite::ToSql> = vec![&window.from_ms, &window.to_ms];
    for id in &enabled_ids { params_dyn.push(id); }
    let triggered_ids: Vec<String> = stmt.query_map(&*params_dyn, |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let penalty: i64 = triggered_ids.iter().map(|id| {
        RULES.iter()
            .find(|r| r.id == id)
            .and_then(|r| r.severity)
            .map(|s| s.penalty())
            .unwrap_or(5)
    }).sum();

    let max_penalty = enabled_ids.len() as i64 * 12;
    let raw = 100.0 * (1.0 - penalty as f64 / max_penalty as f64);
    let score = raw.round().max(0.0) as i64;
    let status = if score >= 70 { "good" }
        else if score >= 40 { "needs-improvement" }
        else { "critical" };

    Ok(PracticeScore {
        practice: practice.into(),
        score: Some(score),
        status: status.into(),
        triggered_count: triggered_ids.len() as i64,
    })
}

pub fn trends_wow(pool: &Arc<DbPool>, practice: &str, now_ms: i64) -> Result<i64> {
    let day = 86_400_000i64;
    let last  = count_findings(pool, practice, now_ms - 7*day, now_ms)?;
    let prev  = count_findings(pool, practice, now_ms - 14*day, now_ms - 7*day)?;
    Ok(if prev > 0 { (((last - prev) as f64 / prev as f64) * 100.0).round() as i64 } else { 0 })
}

pub fn trends_mom(pool: &Arc<DbPool>, practice: &str, now_ms: i64) -> Result<i64> {
    let day = 86_400_000i64;
    let week_sum = |from: i64, to: i64| -> Result<f64> {
        count_findings(pool, practice, from, to).map(|n| n as f64)
    };
    let recent: f64 = (0..4).map(|w|
        week_sum(now_ms - (w+1)*7*day, now_ms - w*7*day).unwrap_or(0.0)
    ).sum::<f64>() / 4.0;
    let prior: f64 = (4..8).map(|w|
        week_sum(now_ms - (w+1)*7*day, now_ms - w*7*day).unwrap_or(0.0)
    ).sum::<f64>() / 4.0;
    Ok(if prior > 0.0 { (((recent - prior) / prior) * 100.0).round() as i64 } else { 0 })
}

fn count_findings(pool: &Arc<DbPool>, practice: &str, from_ms: i64, to_ms: i64) -> Result<i64> {
    let conn = pool.get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM coach_findings cf
         JOIN coach_rules cr ON cr.id = cf.rule_id
         WHERE cr.practice = ?1 AND cf.detected_at >= ?2 AND cf.detected_at < ?3",
        rusqlite::params![practice, from_ms, to_ms],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(n)
}
