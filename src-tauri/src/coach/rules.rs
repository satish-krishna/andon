//! Static rule catalogue. Each `Rule` is pure data — detector logic
//! lives next to its literal as `pub fn detect_<id>(…)` (added in Section E).

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub type DbPool = Pool<SqliteConnectionManager>;

#[derive(Debug, Clone, Copy)]
pub enum RuleKind {
    Binary,
    Continuous,
}

#[derive(Debug, Clone, Copy)]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn penalty(&self) -> i64 {
        match self {
            Self::High => 12,
            Self::Medium => 7,
            Self::Low => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: &'static str,
    pub practice: &'static str,
    pub severity: Option<Severity>,
    pub kind: RuleKind,
    pub aiec_origin: Option<&'static str>,
    pub description: &'static str,
    pub suggestion: &'static str,
    pub respects_model_filter: bool,
    /// `true` means the rule is shown in the UI as a reserved slot but
    /// has no detector. Used for `high-cancellation`.
    pub reserved: bool,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub rule_id: String,
    pub session_id: String,
    pub detected_at: i64,
    pub payload_json: String,
}

#[derive(Debug)]
pub struct Window {
    pub from_ms: i64,
    pub to_ms: i64,
    pub models: Option<Vec<String>>,
}

pub static RULES: &[Rule] = &[
    Rule {
        id: "repeated-prompts",
        practice: "prompt",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("repeated-prompts.md"),
        description: "Same prompt repeated 3+ times in one session.",
        suggestion: "If you find yourself asking the same thing, turn it into a slash command or skill.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "lazy-prompting",
        practice: "prompt",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("lazy-prompting.md"),
        description: "Many very short prompts — likely missing context.",
        suggestion: "Spend a sentence describing intent, constraints, and expected output.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "low-constraint-usage",
        practice: "prompt",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("low-constraint-usage.md"),
        description: "Prompts rarely state constraints (must / should / limit / …).",
        suggestion: "Tell the model the rules of the game — what it must / must not do.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "long-session-no-commit",
        practice: "hygiene",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: None,
        description: "Session ran over 90 minutes with no commits.",
        suggestion: "Commit checkpoints; restart sessions after major milestones.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "late-night-coding",
        practice: "hygiene",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("late-night-coding.md"),
        description: "5+ sessions started between 23:00 and 05:00.",
        suggestion: "Late-night sessions correlate with rework. Sleep is undefeated.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "abandon-sessions",
        practice: "hygiene",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("abandon-sessions.md"),
        description: "3+ sessions had tool decisions but zero accepts.",
        suggestion: "Mid-session abandonment is a sign the prompt or plan was off — pause and re-spec.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "speed-accept",
        practice: "review",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: Some("speed-accept.md"),
        description: "Accepting 20+ lines of AI code within 15 seconds, repeatedly.",
        suggestion: "Speed-accepting large diffs masks bugs. Read before you accept.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "high-cancellation",
        practice: "review",
        severity: None,
        kind: RuleKind::Binary,
        aiec_origin: Some("high-cancellation.md"),
        description: "Reserved — upstream signal not yet captured in Andon's OTLP.",
        suggestion: "Re-add when request-level cancellation is ingested.",
        respects_model_filter: false,
        reserved: true,
    },
    Rule {
        id: "no-slash-commands",
        practice: "tool",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("no-slash-commands.md"),
        description: "Session over 30 minutes with zero slash commands.",
        suggestion: "Slash commands codify your recurring workflows. Use or build them.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "model-diversity",
        practice: "tool",
        severity: None,
        kind: RuleKind::Continuous,
        aiec_origin: Some("PatternsAnalyzer::Model Diversity"),
        description: "Distinct models used in the window.",
        suggestion: "Pick the right model for the task — cheap models for simple work.",
        respects_model_filter: true,
        reserved: false,
    },
    Rule {
        id: "cache-hit-starvation",
        practice: "context",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: Some("cache-hit-starvation.md"),
        description: "Cache hit rate below 10% on large-prompt sessions.",
        suggestion: "Keep CLAUDE.md and project context stable; long sessions over short ones.",
        respects_model_filter: true,
        reserved: false,
    },
    Rule {
        id: "low-spec-rate",
        practice: "context",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("no-spec-driven-development.md"),
        description: "Less than 20% of agent-mode sessions start spec-driven.",
        suggestion: "Open sessions with a spec — file ref, bullet list, or planning command.",
        respects_model_filter: true,
        reserved: false,
    },
];

/// Resolve a rule by id. O(N) but N=12.
pub fn by_id(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

// ---------------------------------------------------------------------------
// Detector implementations (Section E — one function per rule)
// ---------------------------------------------------------------------------

/// Fires when <20% of prompts in a session state a constraint, with >=5 turns total.
pub fn detect_low_constraint_usage(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, COUNT(*) AS total,
                SUM(has_constraint) AS with_constraint,
                MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id
         HAVING total >= 5 AND CAST(with_constraint AS REAL) / total < 0.2",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, total, with_c, ts)| Finding {
        rule_id: "low-constraint-usage".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "total": total, "with_constraint": with_c }).to_string(),
    }).collect())
}

/// Fires when >30% of prompts in a session are under 30 chars, with >10 turns total.
pub fn detect_lazy_prompting(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id,
                COUNT(*) AS total,
                SUM(CASE WHEN length < 30 THEN 1 ELSE 0 END) AS short_count,
                MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id
         HAVING total > 10 AND CAST(short_count AS REAL) / total > 0.3",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, total, short, ts)| Finding {
        rule_id: "lazy-prompting".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "total": total, "short_count": short }).to_string(),
    }).collect())
}

/// Fires once per session when the same `norm_hash` appears 3 or more times.
pub fn detect_repeated_prompts(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, norm_hash, COUNT(*) AS n, MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id, norm_hash
         HAVING n >= 3",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        let sid: String = r.get(0)?;
        let hash: String = r.get(1)?;
        let n: i64 = r.get(2)?;
        let last_ts: i64 = r.get(3)?;
        Ok((sid, hash, n, last_ts))
    })?;
    let mut out = vec![];
    let mut sessions_seen = std::collections::HashSet::new();
    for row in rows.filter_map(|r| r.ok()) {
        if !sessions_seen.insert(row.0.clone()) {
            continue;
        }
        out.push(Finding {
            rule_id: "repeated-prompts".into(),
            session_id: row.0,
            detected_at: row.3,
            payload_json: serde_json::json!({ "norm_hash": row.1, "count": row.2 }).to_string(),
        });
    }
    Ok(out)
}

/// Fires for sessions longer than 90 minutes that have no git activity.
pub fn detect_long_session_no_commit(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let ninety_min_ms: i64 = 90 * 60 * 1000;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.ended_at - s.started_at AS dur, s.ended_at
         FROM sessions s
         LEFT JOIN (SELECT session_id, COUNT(*) AS n FROM git_activity GROUP BY session_id) g
           ON g.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND s.ended_at IS NOT NULL
           AND (s.ended_at - s.started_at) > ?3
           AND COALESCE(g.n, 0) = 0",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms, ninety_min_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, dur, ended)| Finding {
        rule_id: "long-session-no-commit".into(),
        session_id: sid,
        detected_at: ended,
        payload_json: serde_json::json!({ "duration_ms": dur, "commits": 0 }).to_string(),
    }).collect())
}

/// Fires when >=5 sessions in the window started between 23:00 and 05:00 local time.
pub fn detect_late_night_coding(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, started_at
         FROM sessions
         WHERE started_at >= ?1 AND started_at < ?2
           AND (CAST(strftime('%H', started_at/1000, 'unixepoch', 'localtime') AS INTEGER) >= 23
            OR CAST(strftime('%H', started_at/1000, 'unixepoch', 'localtime') AS INTEGER) < 5)",
    )?;
    let late: Vec<(String, i64)> = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?.filter_map(|r| r.ok()).collect();
    if late.len() < 5 { return Ok(vec![]); }
    let latest = late.iter().max_by_key(|(_, ts)| *ts).unwrap();
    Ok(vec![Finding {
        rule_id: "late-night-coding".into(),
        session_id: latest.0.clone(),
        detected_at: latest.1,
        payload_json: serde_json::json!({ "count": late.len() }).to_string(),
    }])
}

/// Fires when a session has 5+ accept events followed within 15s by a user turn,
/// where the accepted change had >=20 lines added.
pub fn detect_speed_accept(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "WITH accepts AS (
           SELECT td.session_id, td.timestamp AS acc_ts
           FROM tool_decisions td
           JOIN sessions s USING (session_id)
           WHERE td.decision = 'accept'
             AND s.started_at >= ?1 AND s.started_at < ?2
         ),
         qualifying AS (
           SELECT a.session_id, a.acc_ts
           FROM accepts a
           WHERE EXISTS (
             SELECT 1 FROM file_changes fc
              WHERE fc.session_id = a.session_id
                AND fc.timestamp <= a.acc_ts
                AND fc.timestamp >= a.acc_ts - 60000
                AND fc.lines_added >= 20
           )
           AND EXISTS (
             SELECT 1 FROM prompt_turns pt
              WHERE pt.session_id = a.session_id
                AND pt.ts > a.acc_ts
                AND pt.ts <= a.acc_ts + 15000
           )
         )
         SELECT session_id, COUNT(*) AS n, MAX(acc_ts) AS last_ts
         FROM qualifying
         GROUP BY session_id
         HAVING n >= 5",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, n, ts)| Finding {
        rule_id: "speed-accept".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "occurrences": n }).to_string(),
    }).collect())
}

/// Fires for sessions longer than 30 minutes with zero slash commands.
pub fn detect_no_slash_commands(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let thirty_min: i64 = 30 * 60 * 1000;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.ended_at
         FROM sessions s
         LEFT JOIN (SELECT session_id, COUNT(*) AS n FROM slash_commands GROUP BY session_id) sc
           ON sc.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND s.ended_at IS NOT NULL
           AND (s.ended_at - s.started_at) > ?3
           AND COALESCE(sc.n, 0) = 0",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms, thirty_min], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, ts)| Finding {
        rule_id: "no-slash-commands".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: "{}".into(),
    }).collect())
}

// ---------------------------------------------------------------------------
// E9: model-diversity (continuous) — returns a 0-100 score, not findings
// ---------------------------------------------------------------------------

/// Returns a 0–100 diversity score based on distinct models used in the window.
/// 4+ models → 100, 3 → 80, 2 → 50, else → 20.
pub fn score_model_diversity(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<i64> {
    let conn = pool.get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT model)
         FROM cost_entries
         WHERE timestamp >= ?1 AND timestamp < ?2",
        rusqlite::params![window.from_ms, window.to_ms],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(match n {
        x if x >= 4 => 100,
        3 => 80,
        2 => 50,
        _ => 20,
    })
}

/// Fires when >=3 sessions in the window have tool decisions but zero accepts.
pub fn detect_abandon_sessions(pool: &std::sync::Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.started_at
         FROM sessions s
         JOIN (
           SELECT session_id, COUNT(*) AS total,
                  SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS accepts
           FROM tool_decisions GROUP BY session_id
         ) td ON td.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND td.total > 0 AND td.accepts = 0",
    )?;
    let abandoned: Vec<(String, i64)> = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?.filter_map(|r| r.ok()).collect();
    if abandoned.len() < 3 { return Ok(vec![]); }
    let latest = abandoned.iter().max_by_key(|(_, ts)| *ts).unwrap();
    Ok(vec![Finding {
        rule_id: "abandon-sessions".into(),
        session_id: latest.0.clone(),
        detected_at: latest.1,
        payload_json: serde_json::json!({ "count": abandoned.len() }).to_string(),
    }])
}
