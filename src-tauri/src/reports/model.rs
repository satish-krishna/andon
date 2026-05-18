use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

use crate::db::DbPool;

#[derive(Serialize)]
pub struct ReportData {
    pub session_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: f64,
    pub service_version: Option<String>,
    pub host_arch: Option<String>,
    pub os_type: Option<String>,
    pub terminal_type: Option<String>,

    pub cost_usd: f64,
    pub tokens: Vec<KV>,
    pub accept_rate: f64,
    pub active_user_seconds: f64,
    pub active_cli_seconds: f64,

    pub cost_by_model: Vec<KVFloat>,
    pub tokens_by_type: Vec<KVFloat>,

    pub files: Vec<FileRow>,
    pub decisions: Vec<DecisionRow>,
}

#[derive(Serialize)]
pub struct KV { pub key: String, pub value: i64 }
#[derive(Serialize)]
pub struct KVFloat { pub key: String, pub value: f64 }

#[derive(Serialize)]
pub struct FileRow {
    pub file_path: String,
    pub added: i64,
    pub removed: i64,
    pub accept_rate: f64,
}

#[derive(Serialize)]
pub struct DecisionRow {
    pub timestamp: i64,
    pub tool_name: String,
    pub decision: String,
    pub language: Option<String>,
    pub file_path: Option<String>,
}

fn rate(a: i64, r: i64, x: i64) -> f64 {
    let d = a + r + x;
    if d == 0 { 0.0 } else { ((a as f64 / d as f64) * 10000.0).round() / 10000.0 }
}

impl ReportData {
    pub fn load(pool: &DbPool, sid: &str) -> Result<Self> {
        let conn = pool.get()?;

        let (started_at, ended_at, sv, ha, ot, tt) = conn
            .query_row(
                "SELECT started_at, ended_at, service_version, host_arch, os_type, terminal_type
                 FROM sessions WHERE session_id = ?1",
                params![sid],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                )),
            )
            .unwrap_or((0, None, None, None, None, None));

        let cost_usd: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id = ?1",
            params![sid], |r| r.get(0)).unwrap_or(0.0);

        let mut tokens = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT token_type, COALESCE(SUM(count), 0) FROM token_usage
             WHERE session_id = ?1 GROUP BY token_type")?;
        for row in stmt.query_map(params![sid], |r| Ok(KV { key: r.get(0)?, value: r.get(1)? }))?.flatten() {
            tokens.push(row);
        }

        let (a, r, x): (i64, i64, i64) = conn.query_row(
            "SELECT
                SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END)
             FROM tool_decisions WHERE session_id = ?1",
            params![sid],
            |r| Ok((r.get::<_,i64>(0).unwrap_or(0), r.get::<_,i64>(1).unwrap_or(0), r.get::<_,i64>(2).unwrap_or(0))),
        ).unwrap_or((0,0,0));
        let accept_rate = rate(a, r, x);

        let active_user: f64 = conn.query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE session_id = ?1 AND kind='user'",
            params![sid], |r| r.get(0)).unwrap_or(0.0);
        let active_cli: f64 = conn.query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE session_id = ?1 AND kind='cli'",
            params![sid], |r| r.get(0)).unwrap_or(0.0);

        let mut cost_by_model = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT model, SUM(cost_usd) FROM cost_entries
             WHERE session_id = ?1 GROUP BY model ORDER BY 2 DESC")?;
        for row in stmt.query_map(params![sid], |r| Ok(KVFloat { key: r.get(0)?, value: r.get(1).unwrap_or(0.0) }))?.flatten() {
            cost_by_model.push(row);
        }

        let tokens_by_type: Vec<KVFloat> = tokens.iter()
            .map(|kv| KVFloat { key: kv.key.clone(), value: kv.value as f64 })
            .collect();

        let mut files = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(file_path, '?'), SUM(lines_added), SUM(lines_removed),
                    COALESCE((
                        SELECT
                            CAST(SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS REAL)
                            / NULLIF(COUNT(*), 0)
                        FROM tool_decisions td
                        WHERE td.session_id = fc.session_id AND td.file_path = fc.file_path
                    ), 0)
             FROM file_changes fc WHERE session_id = ?1 GROUP BY file_path ORDER BY 2+3 DESC")?;
        for row in stmt.query_map(params![sid], |r| Ok(FileRow {
            file_path: r.get(0)?,
            added: r.get(1).unwrap_or(0),
            removed: r.get(2).unwrap_or(0),
            accept_rate: ((r.get::<_, f64>(3).unwrap_or(0.0) * 10000.0).round()) / 10000.0,
        }))?.flatten() {
            files.push(row);
        }

        let mut decisions = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT timestamp, tool_name, decision, language, file_path
             FROM tool_decisions WHERE session_id = ?1 ORDER BY timestamp ASC")?;
        for row in stmt.query_map(params![sid], |r| Ok(DecisionRow {
            timestamp: r.get(0)?,
            tool_name: r.get(1)?,
            decision: r.get(2)?,
            language: r.get(3).ok(),
            file_path: r.get(4).ok(),
        }))?.flatten() {
            decisions.push(row);
        }

        let duration_seconds = match ended_at {
            Some(e) if e > started_at => ((e - started_at) as f64) / 1000.0,
            _ => active_user + active_cli,
        };

        Ok(ReportData {
            session_id: sid.to_string(),
            started_at, ended_at,
            duration_seconds,
            service_version: sv, host_arch: ha, os_type: ot, terminal_type: tt,
            cost_usd: ((cost_usd * 10000.0).round()) / 10000.0,
            tokens, accept_rate,
            active_user_seconds: active_user,
            active_cli_seconds: active_cli,
            cost_by_model, tokens_by_type,
            files, decisions,
        })
    }
}
