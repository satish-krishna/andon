use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{Local, TimeZone};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use super::{ApiState, dto::*};

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/overview/today", get(overview_today))
        .route("/api/overview/cost-by-day", get(cost_by_day))
        .route("/api/overview/tokens-by-day", get(tokens_by_day))
        .route("/api/overview/accept-by-language", get(accept_by_language))
        .route("/api/overview/active-time/today", get(active_time_today))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/:id", get(session_detail))
        .route("/api/files/heatmap", get(files_heatmap))
        .route("/api/control/pause", post(pause_ingestion))
        .route("/api/control/resume", post(resume_ingestion))
        .route("/api/control/status", get(control_status))
        .route("/api/stats", get(db_stats))
        .with_state(state)
}

async fn health(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "db": state.db_path.display().to_string(),
        "paused": state.control.is_paused(),
    }))
}

// ---------- overview ----------

async fn overview_today(State(state): State<ApiState>) -> Result<Json<OverviewToday>, ApiError> {
    let (from, to) = today_bounds();
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let cost_usd: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    let sessions: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id) FROM sessions WHERE started_at >= ?1 AND started_at < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let (accepts, rejects, aborts): (i64, i64, i64) = conn
        .query_row(
            "SELECT
                SUM(CASE WHEN decision = 'accept' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision = 'reject' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision = 'abort'  THEN 1 ELSE 0 END)
             FROM tool_decisions WHERE timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| Ok((r.get::<_, i64>(0).unwrap_or(0), r.get::<_, i64>(1).unwrap_or(0), r.get::<_, i64>(2).unwrap_or(0))),
        )
        .unwrap_or((0, 0, 0));

    let accept_rate = accept_rate(accepts, rejects, aborts);

    let tokens_input: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(count), 0) FROM token_usage WHERE token_type = 'input' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let tokens_output: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(count), 0) FROM token_usage WHERE token_type = 'output' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(Json(OverviewToday {
        cost_usd: round4(cost_usd),
        sessions,
        accept_rate,
        tokens_input,
        tokens_output,
    }))
}

#[derive(Deserialize)]
struct DaysQuery {
    #[serde(default = "default_days")]
    days: i64,
}
fn default_days() -> i64 {
    30
}

async fn cost_by_day(
    State(state): State<ApiState>,
    Query(q): Query<DaysQuery>,
) -> Result<Json<DailySeries>, ApiError> {
    let days = q.days.clamp(1, 365);
    let (from, _to) = last_n_days_bounds(days);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let mut stmt = conn.prepare(
        "SELECT timestamp, model, cost_usd FROM cost_entries WHERE timestamp >= ?1",
    )?;
    let rows = stmt.query_map(params![from], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, f64>(2)?,
        ))
    })?;

    let day_labels = day_labels(days);
    let mut by_model: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in rows.flatten() {
        let day_idx = day_index_for(r.0, days);
        if let Some(idx) = day_idx {
            let v = by_model.entry(r.1).or_insert_with(|| vec![0.0; days as usize]);
            v[idx] += r.2;
        }
    }
    Ok(Json(DailySeries {
        days: day_labels,
        series: by_model
            .into_iter()
            .map(|(k, v)| NamedSeries {
                name: k,
                values: v.into_iter().map(round4).collect(),
            })
            .collect(),
    }))
}

async fn tokens_by_day(
    State(state): State<ApiState>,
    Query(q): Query<DaysQuery>,
) -> Result<Json<DailySeries>, ApiError> {
    let days = q.days.clamp(1, 365);
    let (from, _to) = last_n_days_bounds(days);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let mut stmt = conn.prepare(
        "SELECT timestamp, token_type, count FROM token_usage WHERE timestamp >= ?1",
    )?;
    let rows = stmt.query_map(params![from], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    let day_labels = day_labels(days);
    let mut by_type: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in rows.flatten() {
        if let Some(idx) = day_index_for(r.0, days) {
            let v = by_type.entry(r.1).or_insert_with(|| vec![0.0; days as usize]);
            v[idx] += r.2 as f64;
        }
    }
    Ok(Json(DailySeries {
        days: day_labels,
        series: by_type
            .into_iter()
            .map(|(k, v)| NamedSeries { name: k, values: v })
            .collect(),
    }))
}

async fn accept_by_language(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AcceptByLanguage>>, ApiError> {
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(language, 'unknown') AS lang,
                SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS a,
                SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END) AS r,
                SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END) AS x,
                COUNT(*) AS total
         FROM tool_decisions
         GROUP BY lang
         ORDER BY total DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let out: Vec<AcceptByLanguage> = rows
        .flatten()
        .map(|(lang, a, r, x, total)| AcceptByLanguage {
            language: lang,
            accept_rate: accept_rate(a, r, x),
            total,
        })
        .collect();
    Ok(Json(out))
}

async fn active_time_today(
    State(state): State<ApiState>,
) -> Result<Json<ActiveTimeToday>, ApiError> {
    let (from, to) = today_bounds();
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let user_seconds: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE kind='user' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    let cli_seconds: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE kind='cli' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    Ok(Json(ActiveTimeToday {
        user_seconds,
        cli_seconds,
    }))
}

// ---------- sessions ----------

#[derive(Deserialize)]
struct SessionListQuery {
    from: Option<i64>,
    to: Option<i64>,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    100
}

async fn list_sessions(
    State(state): State<ApiState>,
    Query(q): Query<SessionListQuery>,
) -> Result<Json<Vec<SessionSummary>>, ApiError> {
    let from = q.from.unwrap_or(0);
    let to = q.to.unwrap_or(i64::MAX);
    let limit = q.limit.clamp(1, 1000);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.started_at, s.ended_at, s.service_version, s.host_arch, s.os_type,
                COALESCE((SELECT SUM(cost_usd) FROM cost_entries WHERE session_id = s.session_id), 0),
                COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='input'), 0),
                COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='output'), 0),
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='accept'), 0),
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject'), 0)
         FROM sessions s
         WHERE s.started_at >= ?1 AND s.started_at <= ?2
         ORDER BY s.started_at DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![from, to, limit], |r| {
        Ok(SessionSummary {
            session_id: r.get(0)?,
            started_at: r.get(1)?,
            ended_at: r.get(2).ok(),
            service_version: r.get(3).ok(),
            host_arch: r.get(4).ok(),
            os_type: r.get(5).ok(),
            cost_usd: round4(r.get::<_, f64>(6).unwrap_or(0.0)),
            tokens_input: r.get(7).unwrap_or(0),
            tokens_output: r.get(8).unwrap_or(0),
            accepts: r.get(9).unwrap_or(0),
            rejects: r.get(10).unwrap_or(0),
        })
    })?;
    Ok(Json(rows.flatten().collect()))
}

async fn session_detail(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<SessionDetail>, ApiError> {
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let session: SessionSummary = conn
        .query_row(
            "SELECT s.session_id, s.started_at, s.ended_at, s.service_version, s.host_arch, s.os_type,
                    COALESCE((SELECT SUM(cost_usd) FROM cost_entries WHERE session_id = s.session_id), 0),
                    COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='input'), 0),
                    COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='output'), 0),
                    COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='accept'), 0),
                    COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject'), 0)
             FROM sessions s WHERE s.session_id = ?1",
            params![id],
            |r| {
                Ok(SessionSummary {
                    session_id: r.get(0)?,
                    started_at: r.get(1)?,
                    ended_at: r.get(2).ok(),
                    service_version: r.get(3).ok(),
                    host_arch: r.get(4).ok(),
                    os_type: r.get(5).ok(),
                    cost_usd: round4(r.get::<_, f64>(6).unwrap_or(0.0)),
                    tokens_input: r.get(7).unwrap_or(0),
                    tokens_output: r.get(8).unwrap_or(0),
                    accepts: r.get(9).unwrap_or(0),
                    rejects: r.get(10).unwrap_or(0),
                })
            },
        )
        .map_err(|_| ApiError::not_found("session not found"))?;

    let cost_by_model = key_value_query(
        &conn,
        "SELECT model, SUM(cost_usd) FROM cost_entries WHERE session_id = ?1 GROUP BY model ORDER BY 2 DESC",
        &id,
    );
    let tokens_by_type = key_value_query(
        &conn,
        "SELECT token_type, SUM(count) FROM token_usage WHERE session_id = ?1 GROUP BY token_type",
        &id,
    );

    let mut stmt = conn.prepare(
        "SELECT timestamp, tool_name, decision, language, file_path
         FROM tool_decisions WHERE session_id = ?1 ORDER BY timestamp ASC",
    )?;
    let decisions: Vec<ToolDecisionRow> = stmt
        .query_map(params![id], |r| {
            Ok(ToolDecisionRow {
                timestamp: r.get(0)?,
                tool_name: r.get(1)?,
                decision: r.get(2)?,
                language: r.get(3).ok(),
                file_path: r.get(4).ok(),
            })
        })?
        .flatten()
        .collect();

    let mut stmt = conn.prepare(
        "SELECT COALESCE(file_path, '?'), SUM(lines_added), SUM(lines_removed)
         FROM file_changes WHERE session_id = ?1 GROUP BY file_path ORDER BY 2+3 DESC",
    )?;
    let files: Vec<FileRow> = stmt
        .query_map(params![id], |r| {
            Ok(FileRow {
                file_path: r.get(0)?,
                lines_added: r.get(1).unwrap_or(0),
                lines_removed: r.get(2).unwrap_or(0),
            })
        })?
        .flatten()
        .collect();

    let active_time_seconds: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE session_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    Ok(Json(SessionDetail {
        session,
        cost_by_model,
        tokens_by_type,
        tool_decisions: decisions,
        files,
        active_time_seconds,
    }))
}

// ---------- files ----------

async fn files_heatmap(
    State(state): State<ApiState>,
    Query(q): Query<DaysQuery>,
) -> Result<Json<Vec<FileHeatmapRow>>, ApiError> {
    let days = q.days.clamp(1, 365);
    let (from, _to) = last_n_days_bounds(days);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let mut stmt = conn.prepare(
        "WITH edits AS (
             SELECT COALESCE(file_path, '?') AS f, COUNT(*) AS c
             FROM file_changes WHERE timestamp >= ?1 GROUP BY f
         ),
         decs AS (
             SELECT COALESCE(file_path, '?') AS f,
                    SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS a,
                    SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END) AS r,
                    SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END) AS x
             FROM tool_decisions WHERE timestamp >= ?1 GROUP BY f
         )
         SELECT e.f, e.c,
                COALESCE(d.a, 0), COALESCE(d.r, 0), COALESCE(d.x, 0)
         FROM edits e LEFT JOIN decs d ON d.f = e.f
         ORDER BY e.c DESC
         LIMIT 200",
    )?;
    let rows = stmt.query_map(params![from], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    Ok(Json(
        rows.flatten()
            .map(|(f, c, a, r, x)| FileHeatmapRow {
                file_path: f,
                edit_count: c,
                accept_rate: accept_rate(a, r, x),
            })
            .collect(),
    ))
}

// ---------- control + stats ----------

async fn pause_ingestion(State(state): State<ApiState>) -> Json<serde_json::Value> {
    state.control.set_paused(true);
    Json(json!({"paused": true}))
}
async fn resume_ingestion(State(state): State<ApiState>) -> Json<serde_json::Value> {
    state.control.set_paused(false);
    Json(json!({"paused": false}))
}
async fn control_status(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(json!({"paused": state.control.is_paused()}))
}

async fn db_stats(State(state): State<ApiState>) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let tables = [
        "sessions",
        "token_usage",
        "cost_entries",
        "tool_decisions",
        "file_changes",
        "git_activity",
        "active_time",
        "metrics_raw",
    ];
    let mut out = serde_json::Map::new();
    for t in tables {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .unwrap_or(0);
        out.insert(t.to_string(), json!(n));
    }
    Ok(Json(json!({
        "db_path": state.db_path.display().to_string(),
        "tables": out,
    })))
}

// ---------- helpers ----------

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

fn accept_rate(accepts: i64, rejects: i64, aborts: i64) -> f64 {
    let denom = accepts + rejects + aborts;
    if denom == 0 {
        0.0
    } else {
        round4(accepts as f64 / denom as f64)
    }
}

fn today_bounds() -> (i64, i64) {
    let now = Local::now();
    let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let from = Local
        .from_local_datetime(&start)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0);
    let to = from + 24 * 60 * 60 * 1000;
    (from, to)
}

fn last_n_days_bounds(days: i64) -> (i64, i64) {
    let (today_from, today_to) = today_bounds();
    let from = today_from - (days - 1) * 24 * 60 * 60 * 1000;
    (from, today_to)
}

fn day_labels(days: i64) -> Vec<String> {
    let today = Local::now().date_naive();
    (0..days)
        .rev()
        .map(|i| {
            today
                .checked_sub_days(chrono::Days::new(i as u64))
                .unwrap_or(today)
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect()
}

fn day_index_for(ts_ms: i64, days: i64) -> Option<usize> {
    let dt = Local.timestamp_millis_opt(ts_ms).single()?;
    let date = dt.date_naive();
    let today = Local::now().date_naive();
    let diff = (today - date).num_days();
    if diff < 0 || diff >= days {
        return None;
    }
    Some((days - 1 - diff) as usize)
}

fn key_value_query(conn: &rusqlite::Connection, sql: &str, sid: &str) -> Vec<KeyValueNum> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![sid], |r| {
        Ok(KeyValueNum {
            key: r.get::<_, String>(0).unwrap_or_default(),
            value: r.get::<_, f64>(1).unwrap_or(0.0),
        })
    })
    .map(|it| it.flatten().collect())
    .unwrap_or_default()
}

// ---------- error ----------

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn pool(e: r2d2::Error) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: format!("db pool: {e}"),
        }
    }
    fn not_found(msg: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.to_string(),
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("sqlite: {e}"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}
