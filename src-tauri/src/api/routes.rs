use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{Datelike, Local, Months, NaiveDate, TimeZone};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use super::{ApiState, dto::*, efficiency::round4, filter::{FilterQuery, today_bounds}, hook_response::HookOutput};

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        // legacy v1 endpoints (kept for compat)
        .route("/api/overview/today", get(overview_today))
        .route("/api/overview/cost-by-day", get(cost_by_day))
        .route("/api/overview/tokens-by-day", get(tokens_by_day))
        .route("/api/overview/accept-by-language", get(accept_by_language))
        .route("/api/overview/active-time/today", get(active_time_today))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/:id", get(session_detail))
        .route("/api/files/heatmap", get(files_heatmap))
        // v2 — filterable
        .route("/api/v2/kpis", get(v2_kpis))
        .route("/api/v2/tape", get(v2_tape))
        .route("/api/v2/cost-by-model", get(v2_cost_by_model))
        .route("/api/v2/cache-efficiency", get(v2_cache_efficiency))
        .route("/api/v2/model-efficiency", get(v2_model_efficiency))
        .route("/api/v2/accept-by-language", get(v2_accept_by_language))
        .route("/api/v2/active-time", get(v2_active_time))
        .route("/api/v2/sessions", get(v2_sessions))
        .route("/api/v2/files", get(v2_files))
        // control + system
        .route("/api/control/pause", post(pause_ingestion))
        .route("/api/control/resume", post(resume_ingestion))
        .route("/api/control/status", get(control_status))
        .route("/api/stats", get(db_stats))
        .route("/api/open-data-folder", post(open_data_folder))
        .route("/api/integration/status", get(integration_status))
        .route("/api/integration/reapply", post(integration_reapply))
        .route("/api/integration/unpatch", post(integration_unpatch))
        .route("/api/integration/restore-backup", post(integration_restore))
        .route("/api/hooks/tool-use", post(hook_tool_use))
        .route("/api/hooks/session-end", post(hook_session_end))
        .route("/api/session/context", post(hook_session_context))
        .route("/api/sessions/:id/report", get(get_report).post(generate_report_handler))
        .route("/api/sessions/:id/report/open", post(open_report))
        .route("/api/sessions/reports/index", get(reports_index))
        .route("/api/autostart/status", get(autostart_status))
        .route("/api/autostart/enable", post(autostart_enable))
        .route("/api/autostart/disable", post(autostart_disable))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/diagnostics/events", get(recent_events))
        .route("/api/diagnostics/export", get(export_diag))
        .route("/api/settings", get(get_settings))
        .route("/api/settings/forwarder", axum::routing::put(put_forwarder))
        .route("/api/settings/forwarder/test", post(test_forwarder))
        .route("/api/settings/budget", axum::routing::put(put_budget))
        .route("/api/repo/backfill", post(repo_backfill))
        .route("/api/repos", get(list_repos))
        .route("/api/overview/top-repos", get(overview_top_repos))
        // JSONL ingest + diagnostics
        .route("/api/jsonl/backfill", post(jsonl_backfill))
        .route("/api/jsonl/errors", get(jsonl_errors))
        .route("/api/jsonl/ingest-runs", get(jsonl_ingest_runs))
        .route("/api/jsonl/coverage-gaps", get(jsonl_coverage_gaps))
        // Behaviour views (Plan C)
        .route("/api/behaviour/model-mix", get(behaviour_model_mix))
        .route("/api/behaviour/slash-commands", get(behaviour_slash_commands))
        .route("/api/behaviour/subagents", get(behaviour_subagents))
        .with_state(state)
}

async fn health(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "db": state.db_path.display().to_string(),
        "paused": state.control.is_paused(),
        "version": env!("CARGO_PKG_VERSION"),
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
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject'), 0),
                s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name,
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source
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
            cwd: r.get(11)?,
            repo_root: r.get(12)?,
            repo_remote: r.get(13)?,
            repo_branch: r.get(14)?,
            repo_name: r.get(15)?,
            cost_source: r.get::<_, Option<String>>(16)?,
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
                    COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject'), 0),
                    s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name,
                    (CASE
                       WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                       WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                       ELSE NULL
                     END) AS cost_source
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
                    cwd: r.get(11)?,
                    repo_root: r.get(12)?,
                    repo_remote: r.get(13)?,
                    repo_branch: r.get(14)?,
                    repo_name: r.get(15)?,
                    cost_source: r.get::<_, Option<String>>(16)?,
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

async fn diagnostics(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let snap = state.diagnostics.snapshot();
    Json(serde_json::to_value(&snap).unwrap_or(json!({})))
}

#[derive(Deserialize)]
struct EventQuery {
    #[serde(default = "default_event_limit")]
    limit: i64,
    event: Option<String>,
}
fn default_event_limit() -> i64 {
    100
}

async fn recent_events(
    State(state): State<ApiState>,
    Query(q): Query<EventQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = q.limit.clamp(1, 1000);
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let sql = match q.event {
        Some(_) => "SELECT id, session_id, timestamp, event_name, body, attributes_json, transport
                    FROM log_events WHERE event_name = ?1 ORDER BY timestamp DESC LIMIT ?2",
        None => "SELECT id, session_id, timestamp, event_name, body, attributes_json, transport
                 FROM log_events ORDER BY timestamp DESC LIMIT ?1",
    };
    let rows: Vec<serde_json::Value> = if let Some(evt) = &q.event {
        let mut stmt = conn.prepare(sql)?;
        let v: Vec<_> = stmt
            .query_map(params![evt, limit], row_to_event)?
            .flatten()
            .collect();
        v
    } else {
        let mut stmt = conn.prepare(sql)?;
        let v: Vec<_> = stmt
            .query_map(params![limit], row_to_event)?
            .flatten()
            .collect();
        v
    };
    Ok(Json(json!({ "events": rows })))
}

fn row_to_event(r: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    let attrs_str: String = r.get(5)?;
    let attrs: serde_json::Value =
        serde_json::from_str(&attrs_str).unwrap_or(serde_json::Value::Null);
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "session_id": r.get::<_, Option<String>>(1)?,
        "timestamp": r.get::<_, i64>(2)?,
        "event_name": r.get::<_, String>(3)?,
        "body": r.get::<_, Option<String>>(4)?,
        "attributes": attrs,
        "transport": r.get::<_, String>(6)?,
    }))
}

async fn export_diag(State(state): State<ApiState>) -> Result<Json<serde_json::Value>, ApiError> {
    // Bundle everything needed for a support report.
    let snap = state.diagnostics.snapshot();
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let mut stmt = conn.prepare(
        "SELECT id, session_id, timestamp, event_name, body, attributes_json, transport
         FROM log_events ORDER BY timestamp DESC LIMIT 200",
    )?;
    let events: Vec<serde_json::Value> =
        stmt.query_map([], row_to_event)?.flatten().collect();
    let integration = state.integration.lock().unwrap().clone();
    Ok(Json(json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "diagnostics": snap,
        "integration": integration,
        "db_path": state.db_path.display().to_string(),
        "recent_events": events,
    })))
}

async fn integration_status(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let s = state.integration.lock().unwrap().clone();
    Json(serde_json::to_value(&s).unwrap_or_else(|_| json!({})))
}

async fn integration_reapply(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let new_status = crate::integration::ensure_claude_settings();
    *state.integration.lock().unwrap() = new_status.clone();
    Json(serde_json::to_value(&new_status).unwrap_or_else(|_| json!({})))
}

async fn open_data_folder(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let dir = state
        .db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state.db_path.clone());
    let result = std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn();
    match result {
        Ok(_) => Json(json!({"opened": true, "path": dir.display().to_string()})),
        Err(e) => Json(json!({"opened": false, "error": e.to_string()})),
    }
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

fn accept_rate(accepts: i64, rejects: i64, aborts: i64) -> f64 {
    let denom = accepts + rejects + aborts;
    if denom == 0 {
        0.0
    } else {
        round4(accepts as f64 / denom as f64)
    }
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

// ---------- settings (forwarder) ----------

async fn get_settings(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.settings.snapshot()).unwrap_or_else(|_| json!({})))
}

#[derive(Deserialize)]
struct ForwarderPayload {
    enabled: bool,
    endpoint: String,
    timeout_ms: u64,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
}

fn validate_forwarder(p: &ForwarderPayload) -> Result<(), String> {
    if p.timeout_ms < 100 || p.timeout_ms > 30_000 {
        return Err("timeout_ms must be between 100 and 30000".into());
    }
    if p.enabled {
        if p.endpoint.is_empty() {
            return Err("endpoint is required when enabled=true".into());
        }
        if !(p.endpoint.starts_with("http://") || p.endpoint.starts_with("https://")) {
            return Err("endpoint must start with http:// or https://".into());
        }
    }
    for k in p.headers.keys() {
        if axum::http::HeaderName::from_bytes(k.as_bytes()).is_err() {
            return Err(format!("invalid header name: {k}"));
        }
    }
    Ok(())
}

async fn put_forwarder(
    State(state): State<ApiState>,
    Json(p): Json<ForwarderPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Err(msg) = validate_forwarder(&p) {
        return Err(ApiError { status: StatusCode::BAD_REQUEST, message: msg });
    }
    let new = crate::settings::ForwarderSettings {
        enabled: p.enabled,
        endpoint: p.endpoint,
        timeout_ms: p.timeout_ms,
        headers: p.headers,
    };
    let saved = state
        .settings
        .save_forwarder(new)
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("{e:#}"),
        })?;
    Ok(Json(serde_json::to_value(saved).unwrap_or_else(|_| json!({}))))
}

async fn test_forwarder(
    State(_state): State<ApiState>,
    Json(p): Json<ForwarderPayload>,
) -> Json<serde_json::Value> {
    if let Err(msg) = validate_forwarder(&p) {
        return Json(json!({ "ok": false, "error": msg }));
    }
    let client = crate::otlp::forwarder::build_client(p.timeout_ms);
    let url = crate::otlp::forwarder::join_url(&p.endpoint, "/v1/metrics");
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use prost::Message;
    let body = ExportMetricsServiceRequest::default().encode_to_vec();

    let mut req_headers = reqwest::header::HeaderMap::new();
    req_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/x-protobuf"),
    );
    for (k, v) in &p.headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::try_from(k.as_str()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            req_headers.insert(name, val);
        }
    }
    match client.post(&url).headers(req_headers).body(body).send().await {
        Ok(resp) => Json(json!({ "ok": resp.status().is_success(), "status": resp.status().as_u16() })),
        Err(e) => Json(json!({ "ok": false, "error": format!("{e}") })),
    }
}

#[derive(Deserialize)]
struct BudgetPayload {
    monthly_usd: f64,
}

fn validate_budget(p: &BudgetPayload) -> Result<(), String> {
    if !p.monthly_usd.is_finite() || p.monthly_usd < 0.0 {
        return Err("monthly_usd must be zero or a positive number".into());
    }
    if p.monthly_usd > 1_000_000.0 {
        return Err("monthly_usd must not exceed 1000000".into());
    }
    Ok(())
}

async fn put_budget(
    State(state): State<ApiState>,
    Json(p): Json<BudgetPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Err(msg) = validate_budget(&p) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: msg,
        });
    }
    let new = crate::settings::BudgetSettings {
        monthly_usd: p.monthly_usd,
    };
    let saved = state.settings.save_budget(new).map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("{e:#}"),
    })?;
    // Wake the budget monitor so the tray and notifications reflect the new
    // budget immediately, rather than at the next 30-minute tick.
    state.budget_changed.notify_one();
    Ok(Json(serde_json::to_value(saved).unwrap_or_else(|_| json!({}))))
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

// ---------- session-end hook + reports ----------

#[derive(Deserialize)]
struct SessionEndPayload {
    session_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
}

/// Resolve a hook-supplied `transcript_path`, but only if it is a `.jsonl` file
/// genuinely under `~/.claude/projects/`. The web API is unauthenticated and
/// localhost-only; validating the path keeps the session-end hook from doubling
/// as a generic local file reader if the endpoint is ever reached from outside.
/// Canonicalizing both sides resolves `..` and symlinks before the prefix check.
fn validate_transcript_path(raw: &str) -> Option<std::path::PathBuf> {
    let projects = dirs::home_dir()?
        .join(".claude")
        .join("projects")
        .canonicalize()
        .ok()?;
    let path = std::path::PathBuf::from(raw).canonicalize().ok()?;
    let is_jsonl = path.extension().and_then(|e| e.to_str()) == Some("jsonl");
    (path.starts_with(&projects) && is_jsonl).then_some(path)
}

async fn hook_session_end(
    State(state): State<ApiState>,
    payload: Result<Json<SessionEndPayload>, JsonRejection>,
) -> Json<HookOutput> {
    let Json(p) = match payload {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "hook_session_end: invalid payload; skipping persist");
            return Json(HookOutput::ok());
        }
    };
    let sid = p.session_id.unwrap_or_default();
    if sid.is_empty() {
        tracing::warn!("hook_session_end: missing session_id; skipping persist");
        return Json(HookOutput::ok());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let pool_for_db = state.pool.clone();
    let sid_for_db = sid.clone();
    tokio::task::spawn_blocking(move || {
        let Ok(conn) = pool_for_db.get() else { return };
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE session_id = ?1",
                params![sid_for_db],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            let _ = conn.execute(
                "INSERT INTO sessions (session_id, started_at, ended_at) VALUES (?1, ?2, ?2)",
                params![sid_for_db, now],
            );
        } else {
            let _ = conn.execute(
                "UPDATE sessions SET ended_at = ?2 WHERE session_id = ?1 AND ended_at IS NULL",
                params![sid_for_db, now],
            );
        }
    })
    .await
    .ok();

    let pool = state.pool.clone();
    let reports_dir = state.reports_dir.clone();
    let sid_for_task = sid.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            crate::reports::generate_report(pool, &reports_dir, &sid_for_task)
        })
        .await;
        match result {
            Ok(Ok(path)) => tracing::info!(path = %path.display(), "report generated"),
            Ok(Err(e))   => tracing::error!(error = ?e, "report render failed"),
            Err(e)       => tracing::error!(error = ?e, "report task panicked"),
        }
    });

    if let Some(tp) = p.transcript_path.clone() {
        match validate_transcript_path(&tp) {
            Some(path) => {
                let pool_j = state.pool.clone();
                let control_j = state.control.clone();
                let diag_j = state.diagnostics.clone();
                let coach_j = state.settings.coach();
                tokio::spawn(async move {
                    let ing =
                        crate::otlp::ingestor::Ingestor::new(pool_j.clone(), control_j, diag_j, coach_j);
                    if let Err(e) = crate::jsonl::ingest_one(&pool_j, &ing, &path).await {
                        tracing::error!(error = ?e, "session-end JSONL ingest failed");
                    }
                });
            }
            None => {
                tracing::warn!(
                    transcript_path = %tp,
                    "ignoring transcript_path outside ~/.claude/projects/"
                );
            }
        }
    }

    // Best-effort repo inference for sessions the hook didn't cover.
    let pool_for_inf = state.pool.clone();
    let sid_for_inf = sid.clone();
    tokio::spawn(async move {
        // Skip if repo_root is already populated.
        let needs = {
            let pool = pool_for_inf.clone();
            let sid = sid_for_inf.clone();
            tokio::task::spawn_blocking(move || -> bool {
                let Ok(conn) = pool.get() else { return false };
                conn.query_row(
                    "SELECT repo_root IS NULL FROM sessions WHERE session_id = ?1",
                    params![sid], |r| r.get::<_, i64>(0)
                ).map(|n| n != 0).unwrap_or(false)
            }).await.unwrap_or(false)
        };
        if !needs { return; }

        let Ok(Some(info)) = crate::repo_inference::infer_repo_for_session(
            pool_for_inf.clone(), sid_for_inf.clone()
        ).await else { return };

        let pool = pool_for_inf.clone();
        let sid  = sid_for_inf.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let Ok(conn) = pool.get() else { return };
            let root = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
            let _ = conn.execute(
                "UPDATE sessions
                   SET repo_root   = COALESCE(repo_root,   ?2),
                       repo_remote = COALESCE(repo_remote, ?3),
                       repo_branch = COALESCE(repo_branch, ?4),
                       repo_name   = COALESCE(repo_name,   ?5)
                 WHERE session_id = ?1",
                params![sid, root, info.repo_remote, info.repo_branch, info.repo_name],
            );
        }).await;
    });

    // Coach re-evaluation: tokio::spawn after all writes, never inline.
    let pool_for_coach = std::sync::Arc::clone(&state.pool);
    let settings_for_coach = state.settings.coach();
    let session_id_owned = sid.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::coach::eval::evaluate_session(
            &pool_for_coach,
            &session_id_owned,
            &settings_for_coach,
        ) {
            tracing::warn!(
                error = ?e,
                session_id = session_id_owned,
                "coach evaluate_session failed"
            );
        }
    });

    let _ = p.reason; // diagnostic field; intentionally dropped from wire
    Json(HookOutput::ok())
}

async fn hook_session_context(
    State(state): State<ApiState>,
    payload: Result<Json<crate::api::dto::SessionContextPayload>, JsonRejection>,
) -> Json<HookOutput> {
    let Json(p) = match payload {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "hook_session_context: invalid payload; skipping persist");
            return Json(HookOutput::ok());
        }
    };
    let sid = p.session_id.trim().to_string();
    if sid.is_empty() {
        tracing::warn!("hook_session_context: missing session_id; skipping persist");
        return Json(HookOutput::ok());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cwd_str = p.cwd.clone().unwrap_or_default();

    // 1) Persist cwd synchronously. Never overwrite an existing non-NULL value.
    {
        let pool = state.pool.clone();
        let sid = sid.clone();
        let cwd_str = cwd_str.clone();
        tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE session_id = ?1",
                    params![sid],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists == 0 {
                conn.execute(
                    "INSERT INTO sessions (session_id, started_at, cwd) VALUES (?1, ?2, ?3)",
                    params![sid, now, &cwd_str],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions SET cwd = COALESCE(cwd, ?2) WHERE session_id = ?1",
                    params![sid, &cwd_str],
                )?;
            }
            Ok(())
        })
        .await
        .ok();
    }

    // 2) Enrich asynchronously — git queries don't block the hook.
    if !cwd_str.is_empty() {
        let pool = state.pool.clone();
        let sid_for_task = sid.clone();
        let cwd_path = std::path::PathBuf::from(&cwd_str);
        tokio::spawn(async move {
            let info = crate::git_query::query_repo(&cwd_path).await;
            let pool_inner = pool.clone();
            let sid_inner = sid_for_task.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let Ok(conn) = pool_inner.get() else { return };
                let root = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
                let rem = info.repo_remote.clone();
                let brn = info.repo_branch.clone();
                let name = info.repo_name.clone();
                let _ = conn.execute(
                    "UPDATE sessions
                       SET repo_root   = COALESCE(repo_root,   ?2),
                           repo_remote = COALESCE(repo_remote, ?3),
                           repo_branch = COALESCE(repo_branch, ?4),
                           repo_name   = COALESCE(repo_name,   ?5)
                     WHERE session_id = ?1",
                    params![sid_inner, root, rem, brn, name],
                );
            })
            .await;
        });
    }

    Json(HookOutput::ok())
}

#[derive(serde::Deserialize)]
struct BackfillQuery {
    #[serde(default)] limit: Option<usize>,
}

async fn repo_backfill(
    State(state): State<ApiState>,
    Query(q): Query<BackfillQuery>,
) -> Json<crate::api::dto::BackfillResult> {
    let limit = q.limit.unwrap_or(50);
    // Collect candidate session ids (repo_root still NULL), capped at limit.
    let pool = state.pool.clone();
    let ids: Vec<String> = match tokio::task::spawn_blocking({
        let pool = pool.clone();
        move || -> rusqlite::Result<Vec<String>> {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let mut stmt = conn.prepare(
                "SELECT session_id FROM sessions WHERE repo_root IS NULL ORDER BY started_at DESC LIMIT ?1"
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        }
    }).await {
        Ok(Ok(v)) => v,
        _ => return Json(crate::api::dto::BackfillResult { scanned: 0, updated: 0 }),
    };

    let scanned = ids.len();
    let concurrency = 4;
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();
    for sid in ids {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let pool_inner = pool.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let Ok(Some(info)) = crate::repo_inference::infer_repo_for_session(
                pool_inner.clone(), sid.clone()
            ).await else { return 0usize };

            let pool2 = pool_inner.clone();
            let sid2  = sid.clone();
            let written = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
                let conn = pool2.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let root = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
                conn.execute(
                    "UPDATE sessions
                       SET repo_root   = COALESCE(repo_root,   ?2),
                           repo_remote = COALESCE(repo_remote, ?3),
                           repo_branch = COALESCE(repo_branch, ?4),
                           repo_name   = COALESCE(repo_name,   ?5)
                     WHERE session_id = ?1",
                    params![sid2, root, info.repo_remote, info.repo_branch, info.repo_name],
                )
            }).await.unwrap_or(Ok(0)).unwrap_or(0);
            if written > 0 { 1 } else { 0 }
        }));
    }
    let mut updated = 0usize;
    for h in handles {
        updated += h.await.unwrap_or(0);
    }
    Json(crate::api::dto::BackfillResult { scanned, updated })
}

async fn get_report(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let path = crate::reports::report_path(&state.reports_dir, &id);
    let exists = path.exists();
    let generated_at = if exists {
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
    } else { None };
    Json(json!({
        "exists": exists,
        "path": path.display().to_string(),
        "generated_at": generated_at,
    }))
}

async fn generate_report_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state.pool.clone();
    let dir = state.reports_dir.clone();
    let sid = id.clone();
    let path = tokio::task::spawn_blocking(move || {
        crate::reports::generate_report(pool, &dir, &sid)
    })
    .await
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("join: {e}"),
    })?
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("{e:#}"),
    })?;
    let generated_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(Json(json!({
        "exists": true,
        "path": path.display().to_string(),
        "generated_at": generated_at,
    })))
}

async fn open_report(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let path = crate::reports::report_path(&state.reports_dir, &id);
    if !path.exists() {
        return Json(json!({ "ok": false, "error": "report not found" }));
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.display().to_string()])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&path).spawn();

    match result {
        Ok(_) => Json(json!({ "ok": true, "path": path.display().to_string() })),
        Err(e) => Json(json!({ "ok": false, "error": format!("{e}") })),
    }
}

async fn reports_index(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let mut ids: Vec<String> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(&state.reports_dir) {
        for entry in dir.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stem) = name.strip_suffix(".html") {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    Json(json!({ "session_ids": ids }))
}

// ============================================================================
// v2 — filterable endpoints
// ============================================================================

/// The selected `(from, to)` window shifted back by exactly one calendar
/// month — the comparison window for the KPI strip's "vs last month" deltas.
///
/// Shifting the *actual* filter window (rather than a fixed month-to-date
/// span) keeps the comparison honest: narrow the filter and the baseline
/// narrows with it. `checked_sub_months` clamps day-of-month overflow (e.g.
/// May 31 → Apr 30), so the result is always a valid instant.
fn prev_period_window(from: i64, to: i64) -> (i64, i64) {
    let shift = |ts: i64| -> i64 {
        Local
            .timestamp_millis_opt(ts)
            .single()
            .and_then(|dt| dt.checked_sub_months(Months::new(1)))
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(ts)
    };
    (shift(from), shift(to))
}

fn delta_pct(current: f64, previous: f64) -> Option<f64> {
    if previous == 0.0 {
        None
    } else {
        Some(round4((current - previous) / previous))
    }
}

fn sum_cost(conn: &rusqlite::Connection, from: i64, to: i64, models: &FilterQuery) -> f64 {
    let (m_sql, m_vals) = models.model_clause("model");
    let sql = format!(
        "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(from), Box::new(to)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    conn.query_row(&sql, refs.as_slice(), |r| r.get::<_, f64>(0))
        .unwrap_or(0.0)
}

fn count_sessions(conn: &rusqlite::Connection, from: i64, to: i64, models: &FilterQuery) -> i64 {
    let (model_inner_sql, model_inner_vals) = models.model_clause("c.model");
    let model_filter_sql = if model_inner_sql.is_empty() {
        String::new()
    } else {
        let inner = model_inner_sql.trim_start_matches(" AND ");
        format!(
            " AND EXISTS (SELECT 1 FROM cost_entries c
                          WHERE c.session_id = s.session_id AND {inner})"
        )
    };
    let sql = format!(
        "SELECT COUNT(DISTINCT s.session_id) FROM sessions s
         WHERE s.started_at >= ? AND s.started_at < ?{model_filter_sql}"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in model_inner_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    conn.query_row(&sql, refs.as_slice(), |r| r.get(0)).unwrap_or(0)
}

fn sum_tokens(
    conn: &rusqlite::Connection,
    from: i64,
    to: i64,
    token_type: &str,
    models: &FilterQuery,
) -> i64 {
    let (m_sql, m_vals) = models.model_clause("model");
    let sql = format!(
        "SELECT COALESCE(SUM(count), 0) FROM token_usage
         WHERE token_type = ? AND timestamp >= ? AND timestamp < ?{m_sql}"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(token_type.to_string()),
        Box::new(from),
        Box::new(to),
    ];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    conn.query_row(&sql, refs.as_slice(), |r| r.get(0)).unwrap_or(0)
}

async fn v2_kpis(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (from, to) = q.window();
    let (prev_from, prev_to) = prev_period_window(from, to);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let cost = sum_cost(&conn, from, to, &q);
    let sessions = count_sessions(&conn, from, to, &q);
    let tok_in = sum_tokens(&conn, from, to, "input", &q);
    let tok_out = sum_tokens(&conn, from, to, "output", &q);
    let tok_cache_r = sum_tokens(&conn, from, to, "cacheRead", &q);
    let tok_cache_c = sum_tokens(&conn, from, to, "cacheCreation", &q);

    let prev_cost = sum_cost(&conn, prev_from, prev_to, &q);
    let prev_sessions = count_sessions(&conn, prev_from, prev_to, &q);
    let prev_tok_in = sum_tokens(&conn, prev_from, prev_to, "input", &q);
    let prev_tok_out = sum_tokens(&conn, prev_from, prev_to, "output", &q);

    // Pace and projection are month-end figures — always computed over the
    // true current-month window, never the period filter `q`. Using `q`'s
    // window would make a non-month filter yield a nonsensical projection,
    // budget status, and session pace.
    let now = Local::now();
    let day_of_month = now.day() as i64;
    let days_in_month = days_in_current_month();
    let month_start = crate::budget::month_start_ms(now);
    let now_ms = now.timestamp_millis();
    let mtd_cost =
        crate::db::queries::month_to_date_cost(&conn, month_start, now_ms).unwrap_or(0.0);
    let mtd_sessions = count_sessions(&conn, month_start, now_ms, &q);
    let projected_eom =
        crate::budget::project_eom(mtd_cost, day_of_month as u32, days_in_month as u32);
    let session_pace = if day_of_month > 0 {
        (mtd_sessions as f64 / day_of_month as f64) * days_in_month as f64
    } else {
        mtd_sessions as f64
    };

    let budget = state.settings.budget();
    let budget_status =
        crate::budget::evaluate(projected_eom, budget.monthly_usd, day_of_month as u32);

    Ok(Json(json!({
        "window": { "from": from, "to": to, "label": format!("{:04}-{:02}", now.year(), now.month()) },
        "cost": {
            "current": round4(cost),
            "previous": round4(prev_cost),
            "delta_pct": delta_pct(cost, prev_cost),
            "projected_eom": round4(projected_eom),
            "day_of_month": day_of_month,
            "days_in_month": days_in_month,
            "budget": {
                "monthly_usd": budget.monthly_usd,
                "status": budget_status.as_str(),
            },
        },
        "sessions": {
            "current": sessions,
            "previous": prev_sessions,
            "delta_pct": delta_pct(sessions as f64, prev_sessions as f64),
            "pace": session_pace.round() as i64,
        },
        "tokens": {
            "input":         { "current": tok_in,       "previous": prev_tok_in,  "delta_pct": delta_pct(tok_in as f64, prev_tok_in as f64) },
            "output":        { "current": tok_out,      "previous": prev_tok_out, "delta_pct": delta_pct(tok_out as f64, prev_tok_out as f64) },
            "cache_read":    { "current": tok_cache_r },
            "cache_create":  { "current": tok_cache_c },
        },
    })))
}

fn days_in_current_month() -> i64 {
    crate::budget::days_in_month(Local::now().date_naive()) as i64
}

#[derive(Deserialize)]
struct TapeQuery {
    month: Option<String>,    // YYYY-MM; defaults to current
    models: Option<String>,
}

async fn v2_tape(
    State(state): State<ApiState>,
    Query(q): Query<TapeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = Local::now();
    let (year, month) = match q.month.as_deref() {
        Some(s) if s.len() == 7 => {
            let y: i32 = s[..4].parse().unwrap_or(now.year());
            let m: u32 = s[5..].parse().unwrap_or(now.month());
            (y, m)
        }
        _ => (now.year(), now.month()),
    };
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let models = FilterQuery {
        from: None,
        to: None,
        models: q.models.clone(),
    };

    let current = tape_for_month(&conn, year, month, &models);
    let (py, pm) = if month == 1 {
        (year - 1, 12u32)
    } else {
        (year, month - 1)
    };
    let previous = tape_for_month(&conn, py, pm, &models);

    let today_day = if year == now.year() && month == now.month() {
        Some(now.day() as i64)
    } else {
        None
    };

    Ok(Json(json!({
        "month": format!("{year:04}-{month:02}"),
        "days_in_month": current.len(),
        "today_day": today_day,
        "current": current,
        "previous": previous,
    })))
}

fn tape_for_month(
    conn: &rusqlite::Connection,
    year: i32,
    month: u32,
    models: &FilterQuery,
) -> Vec<f64> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
    };
    let days = (next - first).num_days() as usize;
    let mut bins = vec![0f64; days];
    let from = Local
        .from_local_datetime(&first.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    let to = Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(i64::MAX);
    let (m_sql, m_vals) = models.model_clause("model");
    let sql = format!(
        "SELECT timestamp, cost_usd FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(refs.as_slice(), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        }) {
            for (ts_ms, cost) in rows.flatten() {
                if let Some(dt) = Local.timestamp_millis_opt(ts_ms).single() {
                    let d = dt.day() as usize;
                    if d >= 1 && d <= days {
                        bins[d - 1] += cost;
                    }
                }
            }
        }
    }
    bins.iter().map(|v| round4(*v)).collect()
}

async fn v2_cost_by_model(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let (from, to) = q.window();
    let (m_sql, m_vals) = q.model_clause("model");
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let sql = format!(
        "SELECT model, SUM(cost_usd) FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY model ORDER BY 2 DESC"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1).unwrap_or(0.0)))
    })?;
    let out: Vec<serde_json::Value> = rows
        .flatten()
        .map(|(m, c)| json!({ "model": m, "cost_usd": round4(c) }))
        .collect();
    Ok(Json(out))
}

/// Per-model cache savings over `[from, to)` with the model filter applied.
/// One grouped query; the per-model dollar math lives in `efficiency`.
fn cache_savings_for_window(
    conn: &rusqlite::Connection,
    from: i64,
    to: i64,
    q: &FilterQuery,
) -> crate::api::efficiency::Savings {
    let (m_sql, m_vals) = q.model_clause("model");
    let sql = format!(
        "SELECT model,
                COALESCE(SUM(CASE WHEN token_type='cacheRead'     THEN count END), 0),
                COALESCE(SUM(CASE WHEN token_type='cacheCreation' THEN count END), 0)
         FROM token_usage
         WHERE token_type IN ('cacheRead', 'cacheCreation')
           AND timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY model"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    let mut rows: Vec<(String, i64, i64)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(mapped) = stmt.query_map(refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        }) {
            rows = mapped.flatten().collect();
        }
    }
    crate::api::efficiency::cache_savings(rows.iter().map(|(m, cr, cc)| (m.as_str(), *cr, *cc)))
}

async fn v2_cache_efficiency(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<CacheEfficiency>, ApiError> {
    let (from, to) = q.window();
    let (prev_from, prev_to) = prev_period_window(from, to);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let input = sum_tokens(&conn, from, to, "input", &q);
    let output = sum_tokens(&conn, from, to, "output", &q);
    let cache_read = sum_tokens(&conn, from, to, "cacheRead", &q);
    let cache_create = sum_tokens(&conn, from, to, "cacheCreation", &q);

    let hit_ratio = crate::api::efficiency::hit_ratio(input, cache_create, cache_read);
    let hit_ratio_prev = {
        let p_in = sum_tokens(&conn, prev_from, prev_to, "input", &q);
        let p_cc = sum_tokens(&conn, prev_from, prev_to, "cacheCreation", &q);
        let p_cr = sum_tokens(&conn, prev_from, prev_to, "cacheRead", &q);
        crate::api::efficiency::hit_ratio(p_in, p_cc, p_cr)
    };

    let savings = cache_savings_for_window(&conn, from, to, &q);
    let net_prev = cache_savings_for_window(&conn, prev_from, prev_to, &q).net;

    Ok(Json(CacheEfficiency {
        hit_ratio: round4(hit_ratio),
        hit_ratio_prev: round4(hit_ratio_prev),
        tokens: CacheTokenTotals {
            input,
            output,
            cache_read,
            cache_create,
        },
        savings: CacheSavings {
            net: round4(savings.net),
            gross: round4(savings.gross),
            creation_overhead: round4(savings.creation_overhead),
        },
        net_prev: round4(net_prev),
        unpriced_cache_tokens: savings.unpriced_cache_tokens,
    }))
}

async fn v2_model_efficiency(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<Vec<ModelEfficiencyRow>>, ApiError> {
    let (from, to) = q.window();
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let (m_sql, m_vals) = q.model_clause("model");

    // (session_id, model, cost, is_subagent) — folded into per-family-per-session costs.
    let cost_sql = format!(
        "SELECT session_id, model, SUM(cost_usd), is_subagent
         FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id, model, is_subagent"
    );
    let mut cp: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in &m_vals {
        cp.push(Box::new(v.clone()));
    }
    let crefs: Vec<&dyn rusqlite::ToSql> = cp.iter().map(|b| &**b).collect();
    let mut cost_rows: Vec<(String, String, f64, bool)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&cost_sql) {
        if let Ok(mapped) = stmt.query_map(crefs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2).unwrap_or(0.0),
                r.get::<_, i64>(3).unwrap_or(0) != 0,
            ))
        }) {
            cost_rows = mapped.flatten().collect();
        }
    }

    // (session_id, model, output tokens, is_subagent)
    let out_sql = format!(
        "SELECT session_id, model, SUM(count), is_subagent
         FROM token_usage
         WHERE token_type = 'output' AND timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id, model, is_subagent"
    );
    let mut op: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in &m_vals {
        op.push(Box::new(v.clone()));
    }
    let orefs: Vec<&dyn rusqlite::ToSql> = op.iter().map(|b| &**b).collect();
    let mut output_rows: Vec<(String, String, i64, bool)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&out_sql) {
        if let Ok(mapped) = stmt.query_map(orefs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2).unwrap_or(0),
                r.get::<_, i64>(3).unwrap_or(0) != 0,
            ))
        }) {
            output_rows = mapped.flatten().collect();
        }
    }

    Ok(Json(crate::api::efficiency::aggregate_model_efficiency(
        &cost_rows,
        &output_rows,
    )))
}

async fn v2_accept_by_language(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let (from, to) = q.window();
    let conn = state.pool.get().map_err(ApiError::pool)?;

    // tool_decisions has no model column, so filter via EXISTS over cost_entries
    // joined on session_id — same pattern as count_sessions / v2_sessions.
    let (model_inner_sql, model_inner_vals) = q.model_clause("c.model");
    let model_filter_sql = if model_inner_sql.is_empty() {
        String::new()
    } else {
        let inner = model_inner_sql.trim_start_matches(" AND ");
        format!(
            " AND EXISTS (SELECT 1 FROM cost_entries c
                          WHERE c.session_id = tool_decisions.session_id AND {inner})"
        )
    };

    let sql = format!(
        "SELECT COALESCE(language, 'unknown') AS lang,
                SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS a,
                SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END) AS r,
                SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END) AS x,
                COUNT(*) AS total
         FROM tool_decisions
         WHERE timestamp >= ? AND timestamp < ?{model_filter_sql}
         GROUP BY lang ORDER BY total DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in model_inner_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let out: Vec<serde_json::Value> = rows
        .flatten()
        .map(|(lang, a, r, x, total)| {
            json!({
                "language": lang,
                "accept_rate": accept_rate(a, r, x),
                "total": total,
            })
        })
        .collect();
    Ok(Json(out))
}

async fn v2_active_time(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (from, to) = q.window();
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let user: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time
             WHERE kind='user' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    let cli: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time
             WHERE kind='cli' AND timestamp >= ?1 AND timestamp < ?2",
            params![from, to],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    Ok(Json(json!({
        "user_seconds": user,
        "cli_seconds": cli,
        "total_seconds": user + cli,
    })))
}

#[derive(Deserialize)]
struct V2SessionsQuery {
    from: Option<i64>,
    to: Option<i64>,
    models: Option<String>,
    repo: Option<String>, // comma-separated repo keys
    search: Option<String>,
    sort: Option<String>, // time | cost | duration | decisions
    #[serde(default = "default_limit")]
    limit: i64,
}

async fn v2_sessions(
    State(state): State<ApiState>,
    Query(q): Query<V2SessionsQuery>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let filt = FilterQuery {
        from: q.from,
        to: q.to,
        models: q.models.clone(),
    };
    let (from, to) = filt.window();
    // Capped at 999: the totals-split query below lists one bound parameter
    // per returned session id, and 999 is SQLite's most conservative
    // SQLITE_MAX_VARIABLE_NUMBER, so the `IN (...)` list can never overflow it.
    let limit = q.limit.clamp(1, 999);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let repo_list: Vec<String> = q
        .repo
        .as_deref()
        .map(|s| s.split(',').filter(|p| !p.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();

    let (model_inner_sql, model_inner_vals) = filt.model_clause("c.model");
    let model_filter_sql = if model_inner_sql.is_empty() {
        String::new()
    } else {
        // model_inner_sql starts with " AND (...)". Strip the leading " AND " for embedding.
        let inner = model_inner_sql.trim_start_matches(" AND ");
        format!(
            " AND EXISTS (SELECT 1 FROM cost_entries c
                          WHERE c.session_id = s.session_id AND {inner})"
        )
    };
    let repo_filter_sql = if repo_list.is_empty() {
        String::new()
    } else {
        let placeholders = repo_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        format!(" AND COALESCE(s.repo_remote, s.repo_root, s.cwd, '\u{2014}') IN ({placeholders})")
    };
    let search_sql = if let Some(_) = q.search.as_deref() {
        " AND (s.session_id LIKE ? OR EXISTS (SELECT 1 FROM file_changes f
            WHERE f.session_id = s.session_id AND f.file_path LIKE ?))"
    } else {
        ""
    };
    let order = match q.sort.as_deref().unwrap_or("time") {
        "cost"      => "ORDER BY cost DESC",
        "duration"  => "ORDER BY duration_seconds DESC",
        "decisions" => "ORDER BY decisions DESC",
        _           => "ORDER BY s.started_at DESC",
    };

    let sql = format!(
        "SELECT s.session_id, s.started_at, s.ended_at, s.service_version, s.host_arch, s.os_type,
                COALESCE((SELECT SUM(cost_usd) FROM cost_entries WHERE session_id = s.session_id), 0) AS cost,
                COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='input'), 0) AS tok_in,
                COALESCE((SELECT SUM(count) FROM token_usage WHERE session_id = s.session_id AND token_type='output'), 0) AS tok_out,
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='accept'), 0) AS accepts,
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject'), 0) AS rejects,
                COALESCE((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='abort'), 0) AS aborts,
                COALESCE((SELECT SUM(seconds) FROM active_time WHERE session_id = s.session_id), 0) AS duration_seconds,
                (SELECT model FROM cost_entries WHERE session_id = s.session_id ORDER BY cost_usd DESC LIMIT 1) AS top_model,
                COALESCE((SELECT COUNT(*) FROM cost_entries WHERE session_id = s.session_id), 0) AS api_calls,
                ((SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='accept')
                 + (SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='reject')
                 + (SELECT COUNT(*) FROM tool_decisions WHERE session_id = s.session_id AND decision='abort')) AS decisions,
                s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name,
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source,
                COALESCE((SELECT SUM(lines_added)   FROM file_changes WHERE session_id = s.session_id), 0) AS lines_added,
                COALESCE((SELECT SUM(lines_removed) FROM file_changes WHERE session_id = s.session_id), 0) AS lines_removed
         FROM sessions s
         WHERE s.started_at >= ? AND s.started_at <= ?{model_filter_sql}{repo_filter_sql}{search_sql}
         {order}
         LIMIT ?"
    );

    let search_like = q.search.as_deref().map(|s| format!("%{s}%"));
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in &model_inner_vals {
        p.push(Box::new(v.clone()));
    }
    for r in &repo_list {
        p.push(Box::new(r.clone()));
    }
    if let Some(s) = &search_like {
        p.push(Box::new(s.clone()));
        p.push(Box::new(s.clone()));
    }
    p.push(Box::new(limit));
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok(json!({
            "session_id":      r.get::<_, String>(0)?,
            "started_at":      r.get::<_, i64>(1)?,
            "ended_at":        r.get::<_, Option<i64>>(2)?,
            "service_version": r.get::<_, Option<String>>(3)?,
            "host_arch":       r.get::<_, Option<String>>(4)?,
            "os_type":         r.get::<_, Option<String>>(5)?,
            "cost_usd":        round4(r.get::<_, f64>(6).unwrap_or(0.0)),
            "tokens_input":    r.get::<_, i64>(7)?,
            "tokens_output":   r.get::<_, i64>(8)?,
            "accepts":         r.get::<_, i64>(9)?,
            "rejects":         r.get::<_, i64>(10)?,
            "aborts":          r.get::<_, i64>(11)?,
            "duration_seconds":r.get::<_, f64>(12).unwrap_or(0.0),
            "top_model":       r.get::<_, Option<String>>(13)?,
            "api_calls":       r.get::<_, i64>(14)?,
            "decisions":       r.get::<_, i64>(15)?,
            "cwd":             r.get::<_, Option<String>>(16)?,
            "repo_root":       r.get::<_, Option<String>>(17)?,
            "repo_remote":     r.get::<_, Option<String>>(18)?,
            "repo_branch":     r.get::<_, Option<String>>(19)?,
            "repo_name":       r.get::<_, Option<String>>(20)?,
            "cost_source":     r.get::<_, Option<String>>(21)?,
            "lines_added":     r.get::<_, i64>(22)?,
            "lines_removed":   r.get::<_, i64>(23)?,
        }))
    })?;
    let sessions: Vec<serde_json::Value> = rows.flatten().collect();

    // Coverage query: unfiltered by repo/model so the banner is meaningful even with chips active.
    let (total, with_repo): (i64, i64) = {
        let mut cov_stmt = conn.prepare(
            "SELECT COUNT(*),
                    SUM(CASE WHEN repo_root IS NOT NULL OR repo_remote IS NOT NULL THEN 1 ELSE 0 END)
             FROM sessions WHERE started_at BETWEEN ?1 AND ?2",
        )?;
        cov_stmt.query_row(params![from, to], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        })?
    };

    // ---- Totals over exactly the rows just returned ----
    let mut t_cost = 0.0_f64;
    let mut t_tok_in = 0_i64;
    let mut t_tok_out = 0_i64;
    let mut t_accepts = 0_i64;
    let mut t_rejects = 0_i64;
    let mut t_aborts = 0_i64;
    let mut t_decisions = 0_i64;
    let mut t_duration = 0.0_f64;
    // cost_usd is summed from the already round4'd per-row values, so the
    // total matches what the table shows.
    for s in &sessions {
        t_cost += s["cost_usd"].as_f64().unwrap_or(0.0);
        t_tok_in += s["tokens_input"].as_i64().unwrap_or(0);
        t_tok_out += s["tokens_output"].as_i64().unwrap_or(0);
        t_accepts += s["accepts"].as_i64().unwrap_or(0);
        t_rejects += s["rejects"].as_i64().unwrap_or(0);
        t_aborts += s["aborts"].as_i64().unwrap_or(0);
        t_decisions += s["decisions"].as_i64().unwrap_or(0);
        t_duration += s["duration_seconds"].as_f64().unwrap_or(0.0);
    }

    // Code/Docs/Other split: one query over exactly the returned session ids.
    // The id list is bounded by `limit` (<= 999, see the clamp above), so the
    // `IN (?, ...)` placeholder list stays within SQLite's parameter limit.
    let mut split = LineSplit::default();
    let ids: Vec<String> = sessions
        .iter()
        .filter_map(|s| s["session_id"].as_str().map(str::to_string))
        .collect();
    if !ids.is_empty() {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let fc_sql = format!(
            "SELECT file_path, lines_added, lines_removed
             FROM file_changes WHERE session_id IN ({placeholders})"
        );
        let mut fc_stmt = conn.prepare(&fc_sql)?;
        let id_refs: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let fc_rows = fc_stmt.query_map(id_refs.as_slice(), |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, i64>(1).unwrap_or(0),
                r.get::<_, i64>(2).unwrap_or(0),
            ))
        })?;
        for (path, added, removed) in fc_rows.flatten() {
            let kind = match path.as_deref() {
                Some(p) => change_kind(p),
                None => ChangeKind::Other,
            };
            let pair = match kind {
                ChangeKind::Code => &mut split.code,
                ChangeKind::Docs => &mut split.docs,
                ChangeKind::Other => &mut split.other,
            };
            pair.added += added;
            pair.removed += removed;
        }
    }

    let totals = SessionTotals {
        sessions: sessions.len() as i64,
        cost_usd: round4(t_cost),
        tokens_input: t_tok_in,
        tokens_output: t_tok_out,
        accepts: t_accepts,
        rejects: t_rejects,
        aborts: t_aborts,
        decisions: t_decisions,
        duration_seconds: t_duration,
        lines_added: split.code.added + split.docs.added + split.other.added,
        lines_removed: split.code.removed + split.docs.removed + split.other.removed,
        lines: split,
    };

    Ok(Json(SessionListResponse {
        sessions,
        coverage: CoverageHint { total, with_repo },
        totals,
    }))
}

#[derive(Deserialize)]
struct V2FilesQuery {
    from: Option<i64>,
    to: Option<i64>,
    langs: Option<String>,
    repo: Option<String>, // comma-separated repo keys
    search: Option<String>,
    sort: Option<String>, // edits | accept | recent | churn
}

async fn v2_files(
    State(state): State<ApiState>,
    Query(q): Query<V2FilesQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let filt = FilterQuery { from: q.from, to: q.to, models: None };
    let (from, to) = filt.window();
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let repo_list: Vec<String> = q
        .repo
        .as_deref()
        .map(|s| s.split(',').filter(|p| !p.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();

    // Build repo subquery filters for file_changes and tool_decisions
    let (repo_fc_sql, repo_td_sql) = if repo_list.is_empty() {
        (String::new(), String::new())
    } else {
        let placeholders = repo_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let fc = format!(
            " AND session_id IN (SELECT session_id FROM sessions WHERE COALESCE(repo_remote, repo_root, cwd, '\u{2014}') IN ({placeholders}))"
        );
        let td = format!(
            " AND session_id IN (SELECT session_id FROM sessions WHERE COALESCE(repo_remote, repo_root, cwd, '\u{2014}') IN ({placeholders}))"
        );
        (fc, td)
    };

    // file list: pull edits + accept + churn + last_ts from file_changes & tool_decisions
    let sql = format!(
        "WITH edits AS (
             SELECT COALESCE(file_path, '?') AS f,
                    COUNT(*) AS edits,
                    SUM(lines_added) AS added,
                    SUM(lines_removed) AS removed,
                    MAX(timestamp) AS last_ts
             FROM file_changes WHERE timestamp >= ? AND timestamp < ?{repo_fc_sql}
             GROUP BY f
         ),
         decs AS (
             SELECT COALESCE(file_path, '?') AS f,
                    SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS a,
                    SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END) AS r,
                    SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END) AS x,
                    MAX(language) AS lang
             FROM tool_decisions WHERE timestamp >= ? AND timestamp < ?{repo_td_sql}
             GROUP BY f
         )
         SELECT e.f, e.edits, e.added, e.removed, e.last_ts,
                COALESCE(d.a, 0), COALESCE(d.r, 0), COALESCE(d.x, 0),
                COALESCE(d.lang, '?')
         FROM edits e LEFT JOIN decs d ON d.f = e.f"
    );

    let langs: Vec<String> = q
        .langs
        .as_deref()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let search_like = q.search.as_deref().filter(|s| !s.is_empty()).map(|s| s.to_lowercase());

    // Build bind params: from, to, [repo...], from, to, [repo...]
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(from), Box::new(to),
    ];
    for r in &repo_list {
        p.push(Box::new(r.clone()));
    }
    p.push(Box::new(from));
    p.push(Box::new(to));
    for r in &repo_list {
        p.push(Box::new(r.clone()));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();

    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<serde_json::Value> = stmt
        .query_map(refs.as_slice(), |r| {
            let path: String = r.get(0)?;
            let edits: i64 = r.get(1)?;
            let added: i64 = r.get::<_, i64>(2).unwrap_or(0);
            let removed: i64 = r.get::<_, i64>(3).unwrap_or(0);
            let last_ts: i64 = r.get(4)?;
            let a: i64 = r.get(5)?;
            let rej: i64 = r.get(6)?;
            let abr: i64 = r.get(7)?;
            let lang: String = r.get(8)?;
            let lang = if lang == "?" { lang_from_path(&path).to_string() } else { lang };
            Ok(json!({
                "file_path": path,
                "edits": edits,
                "added": added,
                "removed": removed,
                "last_ts": last_ts,
                "accept_rate": accept_rate(a, rej, abr),
                "decision_count": a + rej + abr,
                "lang": lang,
            }))
        })?
        .flatten()
        .collect();

    if !langs.is_empty() {
        rows.retain(|v| {
            v.get("lang")
                .and_then(|l| l.as_str())
                .map(|l| langs.iter().any(|x| x == l))
                .unwrap_or(false)
        });
    }
    if let Some(s) = &search_like {
        rows.retain(|v| {
            v.get("file_path")
                .and_then(|p| p.as_str())
                .map(|p| p.to_lowercase().contains(s))
                .unwrap_or(false)
        });
    }

    match q.sort.as_deref().unwrap_or("edits") {
        "edits"  => rows.sort_by(|a, b| b["edits"].as_i64().unwrap_or(0).cmp(&a["edits"].as_i64().unwrap_or(0))),
        "accept" => rows.sort_by(|a, b| b["accept_rate"].as_f64().unwrap_or(0.0).partial_cmp(&a["accept_rate"].as_f64().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)),
        "recent" => rows.sort_by(|a, b| b["last_ts"].as_i64().unwrap_or(0).cmp(&a["last_ts"].as_i64().unwrap_or(0))),
        "churn"  => rows.sort_by(|a, b| {
            let av = a["added"].as_i64().unwrap_or(0) + a["removed"].as_i64().unwrap_or(0);
            let bv = b["added"].as_i64().unwrap_or(0) + b["removed"].as_i64().unwrap_or(0);
            bv.cmp(&av)
        }),
        _ => {}
    }

    // lang breakdown
    let mut by_lang: BTreeMap<String, i64> = BTreeMap::new();
    for r in &rows {
        let l = r["lang"].as_str().unwrap_or("?").to_string();
        let e = r["edits"].as_i64().unwrap_or(0);
        *by_lang.entry(l).or_insert(0) += e;
    }
    let lang_breakdown: Vec<serde_json::Value> = by_lang
        .into_iter()
        .map(|(l, n)| json!({ "lang": l, "edits": n }))
        .collect();

    let total_edits: i64 = rows.iter().map(|r| r["edits"].as_i64().unwrap_or(0)).sum();
    let total_added: i64 = rows.iter().map(|r| r["added"].as_i64().unwrap_or(0)).sum();
    let total_removed: i64 = rows.iter().map(|r| r["removed"].as_i64().unwrap_or(0)).sum();

    Ok(Json(json!({
        "files": rows,
        "lang_breakdown": lang_breakdown,
        "totals": {
            "files": rows.iter().filter(|_| true).count(),
            "edits": total_edits,
            "added": total_added,
            "removed": total_removed,
        },
    })))
}

fn lang_from_path(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "rs"                          => "rust",
        "ts" | "tsx"                  => "typescript",
        "js" | "jsx" | "mjs" | "cjs"  => "javascript",
        "py"                          => "python",
        "go"                          => "go",
        "java" | "kt"                 => "jvm",
        "c" | "cc" | "cpp" | "h" | "hpp" => "c++",
        "cs"                          => "csharp",
        "rb"                          => "ruby",
        "html" | "htm"                => "html",
        "css" | "scss" | "sass"       => "css",
        "json"                        => "json",
        "toml"                        => "toml",
        "yaml" | "yml"                => "yaml",
        "md" | "markdown"             => "md",
        "sh" | "bash" | "zsh"         => "shell",
        _                             => "other",
    }
}

/// Coarse category for a changed file, derived from its path extension.
/// Used by `v2_sessions` to split aggregate line changes three ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Code,
    Docs,
    Other,
}

/// Bucket a file path into Code / Docs / Other.
///
/// Docs = prose file extensions. Other = anything `lang_from_path` cannot
/// name. Everything else — including config files like `.toml` / `.json` —
/// is Code. Classification is case-insensitive.
fn change_kind(path: &str) -> ChangeKind {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        // `md` / `markdown` also resolve to a named language in lang_from_path;
        // this Docs arm runs first and intentionally takes priority.
        "md" | "markdown" | "mdx" | "txt" | "rst" | "adoc" | "asciidoc" => ChangeKind::Docs,
        _ if lang_from_path(path) == "other" => ChangeKind::Other,
        _ => ChangeKind::Code,
    }
}

// ---------- integration: unpatch + restore ----------

// ---------- autostart ----------

async fn autostart_status(State(_state): State<ApiState>) -> Json<serde_json::Value> {
    Json(json!({
        "enabled": crate::autostart::is_enabled(),
        "registered_command": crate::autostart::registered_command(),
    }))
}

async fn autostart_enable(State(_state): State<ApiState>) -> Json<serde_json::Value> {
    match crate::autostart::enable() {
        Ok(cmd) => Json(json!({"ok": true, "registered_command": cmd})),
        Err(e) => Json(json!({"ok": false, "error": format!("{e:#}")})),
    }
}

async fn autostart_disable(State(_state): State<ApiState>) -> Json<serde_json::Value> {
    match crate::autostart::disable() {
        Ok(()) => Json(json!({"ok": true})),
        Err(e) => Json(json!({"ok": false, "error": format!("{e:#}")})),
    }
}

// ---------- claude code hook receiver ----------

async fn hook_tool_use(
    State(state): State<ApiState>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Json<HookOutput> {
    let Json(payload) = match body {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "hook_tool_use: invalid payload; skipping persist");
            return Json(HookOutput::ok());
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let sid = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let tool = payload
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input = payload.get("tool_input");
    let file_path = input
        .and_then(|v| v.get("file_path"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let (added, removed) = match tool.as_str() {
        "Write" => {
            let content = input
                .and_then(|v| v.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (count_lines(content), 0i64)
        }
        "Edit" => {
            let old = input
                .and_then(|v| v.get("old_string"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new = input
                .and_then(|v| v.get("new_string"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (count_lines(new), count_lines(old))
        }
        "MultiEdit" => {
            let edits = input
                .and_then(|v| v.get("edits"))
                .and_then(|v| v.as_array());
            let mut a = 0i64;
            let mut r = 0i64;
            if let Some(arr) = edits {
                for e in arr {
                    a += e
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .map(count_lines)
                        .unwrap_or(0);
                    r += e
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .map(count_lines)
                        .unwrap_or(0);
                }
            }
            (a, r)
        }
        _ => (0i64, 0i64),
    };

    let is_error = payload
        .get("tool_response")
        .and_then(|v| v.get("is_error"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let decision = if is_error { "reject" } else { "accept" };
    let language = file_path.as_deref().map(lang_from_path).map(String::from);

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!("hook_tool_use: db pool unavailable");
            return Json(HookOutput::ok());
        }
    };

    if let Some(sid_str) = &sid {
        if file_path.is_some() && (added > 0 || removed > 0) {
            if let Err(e) = conn.execute(
                "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sid_str, now, file_path, added, removed],
            ) {
                tracing::warn!(error = %e, "hook_tool_use: failed to write file_changes");
            }
        }
        if let Err(e) = conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision, language, file_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![sid_str, now, tool, decision, language, file_path],
        ) {
            tracing::warn!(error = %e, "hook_tool_use: failed to write tool_decisions");
        }
    }

    tracing::info!(
        ?sid, tool = %tool, file = ?file_path, added, removed, decision,
        "tool-use hook ingested"
    );

    Json(HookOutput::ok())
}

fn count_lines(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let lines = s.lines().count();
    // count trailing newline as one more line
    if s.ends_with('\n') && lines > 0 {
        lines as i64
    } else {
        lines as i64
    }
}

async fn integration_unpatch(State(_state): State<ApiState>) -> Json<serde_json::Value> {
    match crate::integration::unpatch_claude_settings() {
        Ok(msg) => Json(json!({"ok": true, "message": msg})),
        Err(e) => Json(json!({"ok": false, "error": format!("{e:#}")})),
    }
}

async fn integration_restore(State(_state): State<ApiState>) -> Json<serde_json::Value> {
    match crate::integration::restore_backup() {
        Ok(msg) => Json(json!({"ok": true, "message": msg})),
        Err(e) => Json(json!({"ok": false, "error": format!("{e:#}")})),
    }
}

// ---------- repos ----------

#[derive(Deserialize)]
struct ListReposQuery {
    #[serde(default)] from: Option<i64>,
    #[serde(default)] to:   Option<i64>,
    #[serde(default)] limit: Option<usize>,
}

async fn list_repos(
    State(state): State<ApiState>,
    Query(q): Query<ListReposQuery>,
) -> Json<Vec<crate::api::dto::RepoSummary>> {
    let limit = q.limit.unwrap_or(50);
    let from = q.from.unwrap_or(0);
    let to   = q.to.unwrap_or(i64::MAX);
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<crate::api::dto::RepoSummary>> {
        let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(repo_remote, repo_root, cwd, '—') AS k,
                COALESCE(repo_name,
                         CASE WHEN repo_remote IS NOT NULL THEN repo_remote ELSE NULL END,
                         repo_root, cwd, '—') AS label,
                COUNT(*) AS n,
                MAX(repo_root) AS rr
             FROM sessions
             WHERE started_at BETWEEN ?1 AND ?2
             GROUP BY k
             ORDER BY n DESC
             LIMIT ?3"
        )?;
        let rows = stmt.query_map(params![from, to, limit as i64], |r| {
            Ok(crate::api::dto::RepoSummary {
                key: r.get::<_, String>(0)?,
                label: r.get::<_, String>(1)?,
                session_count: r.get::<_, i64>(2)?,
                repo_root: r.get::<_, Option<String>>(3)?,
            })
        })?;
        rows.collect()
    }).await.unwrap_or(Ok(vec![])).unwrap_or_default();
    Json(rows)
}

#[derive(Deserialize)]
struct TopReposQuery {
    from: i64,
    to: i64,
    #[serde(default)] limit: Option<usize>,
}

async fn overview_top_repos(
    State(state): State<ApiState>,
    Query(q): Query<TopReposQuery>,
) -> Json<Vec<crate::api::dto::TopRepoEntry>> {
    let limit = q.limit.unwrap_or(5);
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<crate::api::dto::TopRepoEntry>> {
        let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;

        // 1) Top-N repos by cost.
        // Filter by c.timestamp (cost timestamp) so totals align with the
        // per-day sparkline query which also filters on c.timestamp.
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') AS k,
                COALESCE(s.repo_name, s.repo_remote, s.repo_root, s.cwd, '—') AS label,
                SUM(c.cost_usd) AS cost,
                COUNT(DISTINCT s.session_id) AS n
             FROM sessions s
             JOIN cost_entries c ON c.session_id = s.session_id
             WHERE c.timestamp BETWEEN ?1 AND ?2
             GROUP BY k
             ORDER BY cost DESC
             LIMIT ?3"
        )?;
        let summary: Vec<(String, String, f64, i64)> =
            stmt.query_map(params![q.from, q.to, limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?.collect::<rusqlite::Result<_>>()?;

        // 2) Per-day cost per repo for sparklines.
        let day_ms = 86_400_000i64;
        let days = ((q.to - q.from) / day_ms).max(1) as usize;

        let mut out = Vec::with_capacity(summary.len());
        for (k, label, cost, n) in summary {
            let mut spark = vec![0.0; days];
            let mut sp = conn.prepare(
                "SELECT (c.timestamp - ?2) / ?3 AS day_idx, SUM(c.cost_usd)
                 FROM cost_entries c
                 JOIN sessions s ON s.session_id = c.session_id
                 WHERE c.timestamp BETWEEN ?2 AND ?4
                   AND COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') = ?1
                 GROUP BY day_idx"
            )?;
            let rows = sp.query_map(params![k, q.from, day_ms, q.to], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (idx, v) = row?;
                if (0..spark.len() as i64).contains(&idx) {
                    spark[idx as usize] = v;
                }
            }
            out.push(crate::api::dto::TopRepoEntry { key: k, label, cost_usd: cost, session_count: n, spark });
        }
        Ok(out)
    }).await.unwrap_or(Ok(vec![])).unwrap_or_default();
    Json(rows)
}

// ---------- JSONL ingest endpoints ----------

#[tracing::instrument(skip(state))]
async fn jsonl_backfill(State(state): State<ApiState>) -> axum::response::Response {
    let Some(home) = dirs::home_dir() else {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"no home dir"})),
        )
            .into_response();
    };
    let claude_home = home.join(".claude");
    let pool = std::sync::Arc::clone(&state.pool);
    let coach_settings = state.settings.coach();
    let ingestor = crate::otlp::ingestor::Ingestor::new(
        std::sync::Arc::clone(&state.pool),
        state.control.clone(),
        state.diagnostics.clone(),
        coach_settings.clone(),
    );
    match crate::jsonl::backfill(&pool, &ingestor, &claude_home, &coach_settings).await {
        Ok(stats) => Json(crate::api::dto::JsonlBackfillResponse::from(stats)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "jsonl backfill failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(state))]
async fn jsonl_errors(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::JsonlErrorEntry>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT jsonl_path, line_no, error_kind, error_msg, cc_version, ingested_at
             FROM jsonl_errors ORDER BY ingested_at DESC LIMIT 100",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::JsonlErrorEntry {
                jsonl_path: r.get(0)?,
                line_no: r.get(1)?,
                error_kind: r.get(2)?,
                error_msg: r.get(3)?,
                cc_version: r.get(4)?,
                ingested_at: r.get(5)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(rows)
}

#[tracing::instrument(skip(state))]
async fn jsonl_ingest_runs(
    State(state): State<ApiState>,
) -> Json<Vec<crate::api::dto::JsonlIngestRunEntry>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, kind, started_at, ended_at, files_processed, records_processed, records_errored
             FROM jsonl_ingest_runs ORDER BY started_at DESC LIMIT 20",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::JsonlIngestRunEntry {
                id: r.get(0)?,
                kind: r.get(1)?,
                started_at: r.get(2)?,
                ended_at: r.get(3)?,
                files_processed: r.get(4)?,
                records_processed: r.get(5)?,
                records_errored: r.get(6)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(rows)
}

#[tracing::instrument(skip(state))]
async fn jsonl_coverage_gaps(
    State(state): State<ApiState>,
) -> Json<Vec<crate::api::dto::CoverageGap>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        // OTLP-covered sessions (otlp_calls > 0) whose transcript shows more API
        // calls than OTLP recorded — a likely mid-session loss of OTel coverage.
        // Note: otlp_calls counts distinct timestamps of OTLP-only token_usage rows,
        // which can undercount if two API calls share the same millisecond (acceptable for diagnostics).
        let Ok(mut stmt) = conn.prepare(
            "SELECT session_id, jsonl_calls, otlp_calls FROM (
                 SELECT sjc.session_id AS session_id,
                        sjc.api_calls  AS jsonl_calls,
                        (SELECT COUNT(DISTINCT timestamp) FROM token_usage tu
                         WHERE tu.session_id = sjc.session_id
                           AND tu.request_id IS NULL) AS otlp_calls
                 FROM session_jsonl_calls sjc
             )
             WHERE otlp_calls > 0 AND jsonl_calls > otlp_calls
             ORDER BY (jsonl_calls - otlp_calls) DESC",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::CoverageGap {
                session_id: r.get(0)?,
                jsonl_calls: r.get(1)?,
                otlp_calls: r.get(2)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(rows)
}

// ---------- Behaviour endpoints (Plan C) ----------

async fn behaviour_model_mix(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Json<crate::api::dto::ModelMixResponse> {
    let pool = state.pool.clone();
    // Model-mix is filterable: the Overview passes the dashboard window so its
    // "Invocations by model" tile tracks the filter. With no `from`/`to` — the
    // unfiltered Behaviour page — it stays all-time.
    let (m_sql, m_vals) = q.model_clause("model");
    // Apply each bound independently — a lone `from` or `to` still filters
    // (no bounds at all = the all-time Behaviour-page case above).
    let mut time_sql = String::new();
    let mut time_vals: Vec<i64> = Vec::new();
    if let Some(from) = q.from {
        time_sql.push_str(" AND timestamp >= ?");
        time_vals.push(from);
    }
    if let Some(to) = q.to {
        time_sql.push_str(" AND timestamp < ?");
        time_vals.push(to);
    }
    let out = tokio::task::spawn_blocking(move || {
        let conn = pool.get().ok()?;

        // The same WHERE fragment and bind parameters drive both queries.
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        for v in &time_vals {
            params.push(Box::new(*v));
        }
        for v in &m_vals {
            params.push(Box::new(v.clone()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| &**b).collect();

        let by_model: Vec<crate::api::dto::ModelMixEntry> = conn
            .prepare(&format!(
                // One API call writes several token_usage rows (input/output/
                // cacheRead/cacheCreation), so COUNT(*) would inflate
                // invocations ~2-4x. Count distinct calls instead: request_id
                // for JSONL rows, timestamp for OTLP rows (request_id IS NULL).
                "SELECT model,
                        COUNT(DISTINCT COALESCE(request_id, timestamp)),
                        COUNT(DISTINCT session_id)
                 FROM token_usage WHERE 1=1{time_sql}{m_sql}
                 GROUP BY model ORDER BY 2 DESC"
            ))
            .ok()?
            .query_map(refs.as_slice(), |r| {
                Ok(crate::api::dto::ModelMixEntry {
                    model: r.get(0)?,
                    invocations: r.get(1)?,
                    sessions: r.get(2)?,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        let by_model_tool: Vec<crate::api::dto::ModelToolCell> = conn
            .prepare(&format!(
                "SELECT model, tool_name, COUNT(*)
                 FROM tool_decisions WHERE model IS NOT NULL{time_sql}{m_sql}
                 GROUP BY model, tool_name ORDER BY 3 DESC"
            ))
            .ok()?
            .query_map(refs.as_slice(), |r| {
                Ok(crate::api::dto::ModelToolCell {
                    model: r.get(0)?,
                    tool: r.get(1)?,
                    count: r.get(2)?,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        Some(crate::api::dto::ModelMixResponse {
            by_model,
            by_model_tool,
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(crate::api::dto::ModelMixResponse {
        by_model: vec![],
        by_model_tool: vec![],
    });
    Json(out)
}

async fn behaviour_slash_commands(
    State(state): State<ApiState>,
) -> Json<Vec<crate::api::dto::SlashCommandEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT command_name, COUNT(*) FROM slash_commands
             GROUP BY command_name ORDER BY 2 DESC LIMIT 30",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::SlashCommandEntry {
                name: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(out)
}

async fn behaviour_subagents(
    State(state): State<ApiState>,
) -> Json<Vec<crate::api::dto::SubAgentEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT subagent_type, COUNT(*) FROM subagent_calls
             WHERE subagent_type IS NOT NULL
             GROUP BY subagent_type ORDER BY 2 DESC",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::SubAgentEntry {
                subagent_type: r.get(0)?,
                invocations: r.get(1)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("valid local datetime")
            .timestamp_millis()
    }

    // The KPI strip compares the selected window against the same window one
    // calendar month earlier — BOTH endpoints must shift, not just the start.
    #[test]
    fn prev_period_window_shifts_both_ends_back_one_month() {
        let (pf, pt) = prev_period_window(ms(2026, 5, 1, 0, 0), ms(2026, 5, 20, 17, 30));
        assert_eq!(pf, ms(2026, 4, 1, 0, 0));
        assert_eq!(pt, ms(2026, 4, 20, 17, 30));
    }

    // Regression: prev_period_window's predecessor discarded its argument, so
    // the "vs last month" deltas were frozen regardless of the filter window.
    // Moving the filter's start must move the comparison window with it.
    #[test]
    fn prev_period_window_tracks_the_selected_window() {
        let to = ms(2026, 5, 20, 0, 0);
        let wide = prev_period_window(ms(2026, 5, 1, 0, 0), to);
        let narrow = prev_period_window(ms(2026, 5, 18, 0, 0), to);
        assert_ne!(
            wide.0, narrow.0,
            "previous-window start must follow the filter, not ignore it"
        );
    }

    #[test]
    fn change_kind_buckets_paths_three_ways() {
        // Code: named languages, including config files.
        assert_eq!(change_kind("src/main.rs"), ChangeKind::Code);
        assert_eq!(change_kind("Cargo.toml"), ChangeKind::Code);
        assert_eq!(change_kind("web/package.json"), ChangeKind::Code);
        assert_eq!(change_kind("a.b.rs"), ChangeKind::Code);
        // Docs: prose extensions, case-insensitive.
        assert_eq!(change_kind("README.md"), ChangeKind::Docs);
        assert_eq!(change_kind("notes.txt"), ChangeKind::Docs);
        assert_eq!(change_kind("docs/guide.rst"), ChangeKind::Docs);
        assert_eq!(change_kind("guide.markdown"), ChangeKind::Docs);
        assert_eq!(change_kind("CHANGELOG.MD"), ChangeKind::Docs);
        // Other: nothing lang_from_path can name.
        assert_eq!(change_kind("Makefile"), ChangeKind::Other);
        assert_eq!(change_kind("data.xyz"), ChangeKind::Other);
    }
}
