use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, KeyValue, any_value::Value as AnyV},
    metrics::v1::{
        Metric, ResourceMetrics, metric::Data, number_data_point::Value as NumberValue,
    },
};
use rusqlite::params;

use crate::db::DbPool;
use crate::diagnostics::Diagnostics;

use super::IngestionControl;

#[derive(Clone, Default)]
struct ResourceCtx {
    session_id: Option<String>,
    user_account_uuid: Option<String>,
    organization_id: Option<String>,
    service_version: Option<String>,
    host_arch: Option<String>,
    os_type: Option<String>,
    terminal_type: Option<String>,
}

impl ResourceCtx {
    fn from_attrs(attrs: &[KeyValue]) -> Self {
        Self {
            session_id: attr_string(attrs, "session.id"),
            user_account_uuid: attr_string(attrs, "user.account_uuid")
                .or_else(|| attr_string(attrs, "user.id")),
            organization_id: attr_string(attrs, "organization.id")
                .or_else(|| attr_string(attrs, "user.organization.id")),
            service_version: attr_string(attrs, "service.version"),
            host_arch: attr_string(attrs, "host.arch"),
            os_type: attr_string(attrs, "os.type"),
            terminal_type: attr_string(attrs, "terminal.type"),
        }
    }
}

pub struct Ingestor {
    pool: Arc<DbPool>,
    control: IngestionControl,
    diagnostics: Diagnostics,
}

impl Ingestor {
    pub fn new(pool: Arc<DbPool>, control: IngestionControl, diagnostics: Diagnostics) -> Self {
        Self {
            pool,
            control,
            diagnostics,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.control.is_paused()
    }

    pub fn ingest_metrics_v2(
        &self,
        request: Vec<ResourceMetrics>,
        transport: &str,
    ) -> Result<()> {
        // OTel metrics — Claude Code (as of 2.1.x) doesn't emit these; kept
        // for forward-compat with any future shift back to metric exporters.
        let count = request.iter().map(|r| r.scope_metrics.iter().map(|s| s.metrics.len()).sum::<usize>()).sum::<usize>();
        if count > 0 {
            self.diagnostics.record_payload(
                &format!("{transport}/metrics"),
                request.len(),
                count,
                vec!["<metrics>".into()],
                None,
            );
        }
        self.ingest_metrics(request)
    }

    pub fn ingest_logs_v2(
        &self,
        request: Vec<opentelemetry_proto::tonic::logs::v1::ResourceLogs>,
        transport: &str,
    ) -> Result<()> {
        if self.control.is_paused() {
            tracing::debug!("ingestion paused — dropping log payload");
            return Ok(());
        }

        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let mut event_names: Vec<String> = Vec::new();
        let mut record_count = 0usize;
        let mut last_sid: Option<String> = None;

        for rl in request.iter() {
            let resource_attrs = rl
                .resource
                .as_ref()
                .map(|r| r.attributes.as_slice())
                .unwrap_or(&[]);
            let ctx = ResourceCtx::from_attrs(resource_attrs);

            for sl in rl.scope_logs.iter() {
                for record in sl.log_records.iter() {
                    record_count += 1;
                    let mut record_ctx = ctx.clone();
                    if record_ctx.session_id.is_none() {
                        record_ctx.session_id = attr_string(&record.attributes, "session.id");
                    }
                    if record_ctx.user_account_uuid.is_none() {
                        record_ctx.user_account_uuid =
                            attr_string(&record.attributes, "user.account_uuid")
                                .or_else(|| attr_string(&record.attributes, "user.id"));
                    }
                    if record_ctx.organization_id.is_none() {
                        record_ctx.organization_id =
                            attr_string(&record.attributes, "organization.id");
                    }
                    if record_ctx.terminal_type.is_none() {
                        record_ctx.terminal_type =
                            attr_string(&record.attributes, "terminal.type");
                    }
                    if record_ctx.service_version.is_none() {
                        record_ctx.service_version =
                            attr_string(&record.attributes, "service.version")
                                .or_else(|| attr_string(&record.attributes, "claude_code.version"));
                    }

                    if let Some(sid) = record_ctx.session_id.clone() {
                        let _ = upsert_session(&tx, &sid, &record_ctx);
                        last_sid = Some(sid);
                    }

                    let event_name = attr_string(&record.attributes, "event.name")
                        .or_else(|| {
                            record
                                .body
                                .as_ref()
                                .and_then(|b| anyvalue_to_string(b.value.as_ref()))
                                .map(|s| s.trim_start_matches("claude_code.").to_string())
                        })
                        .unwrap_or_else(|| "<unnamed>".into());
                    event_names.push(event_name.clone());

                    let ts_ms = if record.time_unix_nano > 0 {
                        (record.time_unix_nano / 1_000_000) as i64
                    } else if record.observed_time_unix_nano > 0 {
                        (record.observed_time_unix_nano / 1_000_000) as i64
                    } else {
                        now_ms()
                    };

                    let body_str = record
                        .body
                        .as_ref()
                        .and_then(|b| anyvalue_to_string(b.value.as_ref()));

                    // 1. Always persist a verbatim copy.
                    let attrs_json = attrs_to_json(&record.attributes);
                    let _ = tx.execute(
                        "INSERT INTO log_events (session_id, timestamp, event_name, body, attributes_json, transport)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            record_ctx.session_id,
                            ts_ms,
                            event_name,
                            body_str,
                            attrs_json,
                            transport,
                        ],
                    );

                    // 2. Best-effort typed mapping.
                    if let Err(e) =
                        handle_event(&tx, &record_ctx, &event_name, ts_ms, &record.attributes)
                    {
                        tracing::warn!(event = %event_name, error = ?e, "typed mapping failed; raw row still saved");
                    }
                }
            }
        }
        tx.commit()?;

        self.diagnostics.record_payload(
            &format!("{transport}/logs"),
            request.len(),
            record_count,
            event_names,
            last_sid,
        );
        Ok(())
    }

    #[tracing::instrument(skip(self, request))]
    pub fn ingest_metrics(&self, request: Vec<ResourceMetrics>) -> Result<()> {
        if self.control.is_paused() {
            tracing::debug!("ingestion paused — dropping payload");
            return Ok(());
        }

        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;

        for rm in request {
            let resource_attrs = rm
                .resource
                .as_ref()
                .map(|r| r.attributes.as_slice())
                .unwrap_or(&[]);
            let ctx = ResourceCtx::from_attrs(resource_attrs);

            if let Some(sid) = &ctx.session_id {
                upsert_session(&tx, sid, &ctx)?;
            }

            for sm in rm.scope_metrics {
                for metric in sm.metrics {
                    if let Err(e) = handle_metric(&tx, &ctx, &metric) {
                        tracing::warn!(metric = %metric.name, error = ?e, "metric handler failed");
                    }
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

}

fn handle_event(
    tx: &rusqlite::Transaction,
    ctx: &ResourceCtx,
    event_name: &str,
    ts_ms: i64,
    attrs: &[KeyValue],
) -> Result<()> {
    let Some(sid) = ctx.session_id.as_deref() else {
        return Ok(());
    };

    match event_name {
        "api_request" => {
            let model =
                attr_string(attrs, "model").unwrap_or_else(|| "unknown".into());
            let input = attr_i64(attrs, "input_tokens").unwrap_or(0);
            let output = attr_i64(attrs, "output_tokens").unwrap_or(0);
            let cache_read = attr_i64(attrs, "cache_read_tokens").unwrap_or(0);
            let cache_create = attr_i64(attrs, "cache_creation_tokens").unwrap_or(0);
            let cost = attr_f64(attrs, "cost_usd")
                .or_else(|| attr_i64(attrs, "cost_usd_micros").map(|m| m as f64 / 1_000_000.0))
                .unwrap_or(0.0);
            let duration_ms = attr_i64(attrs, "duration_ms").unwrap_or(0);

            if cost > 0.0 {
                let _ = tx.execute(
                    "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![sid, ts_ms, model, cost],
                );
            }
            for (kind, n) in [
                ("input", input),
                ("output", output),
                ("cacheRead", cache_read),
                ("cacheCreation", cache_create),
            ] {
                if n > 0 {
                    let _ = tx.execute(
                        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![sid, ts_ms, model, kind, n],
                    );
                }
            }
            // duration as CLI active time
            if duration_ms > 0 {
                let _ = tx.execute(
                    "INSERT INTO active_time (session_id, timestamp, seconds, kind)
                     VALUES (?1, ?2, ?3, 'cli')",
                    params![sid, ts_ms, duration_ms as f64 / 1000.0],
                );
            }
            Ok(())
        }

        "user_prompt" => {
            // No native table for prompts — record as user active time + raw.
            let prompt_length = attr_i64(attrs, "prompt_length").unwrap_or(0);
            // Heuristic: estimate user "active" seconds as 1s per 5 chars typed.
            let secs = (prompt_length as f64 / 5.0).max(1.0);
            let _ = tx.execute(
                "INSERT INTO active_time (session_id, timestamp, seconds, kind)
                 VALUES (?1, ?2, ?3, 'user')",
                params![sid, ts_ms, secs],
            );
            Ok(())
        }

        "tool_decision" | "tool_result" => {
            let tool = attr_string(attrs, "tool_name")
                .or_else(|| attr_string(attrs, "tool"))
                .unwrap_or_else(|| "unknown".into());
            let decision = attr_string(attrs, "decision")
                .or_else(|| attr_string(attrs, "result"))
                .unwrap_or_else(|| "accept".into());
            let language = attr_string(attrs, "language");
            let file_path = attr_string(attrs, "file.path")
                .or_else(|| attr_string(attrs, "file_path"));
            let _ = tx.execute(
                "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision, language, file_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![sid, ts_ms, tool, decision, language, file_path],
            );
            Ok(())
        }

        "code_edit" | "file_edit" => {
            let added = attr_i64(attrs, "lines_added").unwrap_or(0);
            let removed = attr_i64(attrs, "lines_removed").unwrap_or(0);
            let file_path = attr_string(attrs, "file.path")
                .or_else(|| attr_string(attrs, "file_path"));
            let _ = tx.execute(
                "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sid, ts_ms, file_path, added, removed],
            );
            Ok(())
        }

        "commit" | "git_commit" => {
            let _ = tx.execute(
                "INSERT INTO git_activity (session_id, timestamp, activity, count)
                 VALUES (?1, ?2, 'commit', 1)",
                params![sid, ts_ms],
            );
            Ok(())
        }

        "pull_request" | "pr_created" => {
            let _ = tx.execute(
                "INSERT INTO git_activity (session_id, timestamp, activity, count)
                 VALUES (?1, ?2, 'pull_request', 1)",
                params![sid, ts_ms],
            );
            Ok(())
        }

        _ => {
            // Stash unknown events as raw for future mapping.
            let attrs_json = serde_json::to_string(
                &attrs
                    .iter()
                    .map(|kv| (kv.key.clone(), anyvalue_to_json(kv.value.as_ref())))
                    .collect::<HashMap<_, _>>(),
            )
            .unwrap_or_else(|_| "{}".into());
            let _ = tx.execute(
                "INSERT INTO metrics_raw (session_id, timestamp, metric_name, attributes_json, value_json)
                 VALUES (?1, ?2, ?3, ?4, '{}')",
                params![sid, ts_ms, format!("event:{event_name}"), attrs_json],
            );
            Ok(())
        }
    }
}

fn attr_i64(attrs: &[KeyValue], key: &str) -> Option<i64> {
    for kv in attrs {
        if kv.key == key {
            return kv.value.as_ref().and_then(|v| match v.value.as_ref()? {
                AnyV::IntValue(i) => Some(*i),
                AnyV::DoubleValue(d) => Some(*d as i64),
                AnyV::StringValue(s) => s.parse().ok(),
                _ => None,
            });
        }
    }
    None
}

fn attr_f64(attrs: &[KeyValue], key: &str) -> Option<f64> {
    for kv in attrs {
        if kv.key == key {
            return kv.value.as_ref().and_then(|v| match v.value.as_ref()? {
                AnyV::DoubleValue(d) => Some(*d),
                AnyV::IntValue(i) => Some(*i as f64),
                AnyV::StringValue(s) => s.parse().ok(),
                _ => None,
            });
        }
    }
    None
}

fn upsert_session(
    tx: &rusqlite::Transaction,
    session_id: &str,
    ctx: &ResourceCtx,
) -> Result<()> {
    let now_ms = now_ms();
    tx.execute(
        "INSERT INTO sessions (session_id, started_at, user_account_uuid, organization_id,
                               service_version, host_arch, os_type, terminal_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(session_id) DO UPDATE SET
            user_account_uuid = COALESCE(excluded.user_account_uuid, sessions.user_account_uuid),
            organization_id   = COALESCE(excluded.organization_id,   sessions.organization_id),
            service_version   = COALESCE(excluded.service_version,   sessions.service_version),
            host_arch         = COALESCE(excluded.host_arch,         sessions.host_arch),
            os_type           = COALESCE(excluded.os_type,           sessions.os_type),
            terminal_type     = COALESCE(excluded.terminal_type,     sessions.terminal_type),
            ended_at          = ?2",
        params![
            session_id,
            now_ms,
            ctx.user_account_uuid,
            ctx.organization_id,
            ctx.service_version,
            ctx.host_arch,
            ctx.os_type,
            ctx.terminal_type,
        ],
    )?;
    Ok(())
}

fn handle_metric(
    tx: &rusqlite::Transaction,
    ctx: &ResourceCtx,
    metric: &Metric,
) -> Result<()> {
    let name = metric.name.as_str();
    let sid = ctx.session_id.as_deref();

    match name {
        "claude_code.session.count" => Ok(()),

        "claude_code.token.usage" => {
            for_each_number(metric, |attrs, ts_ms, value| {
                let Some(sid) = sid else { return };
                let token_type = attr_string(attrs, "type")
                    .or_else(|| attr_string(attrs, "token.type"))
                    .unwrap_or_else(|| "unknown".into());
                let model = attr_string(attrs, "model").unwrap_or_else(|| "unknown".into());
                let _ = tx.execute(
                    "INSERT INTO token_usage (session_id, timestamp, model, token_type, count)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![sid, ts_ms, model, token_type, value.as_i64()],
                );
            });
            Ok(())
        }

        "claude_code.cost.usage" => {
            for_each_number(metric, |attrs, ts_ms, value| {
                let Some(sid) = sid else { return };
                let model = attr_string(attrs, "model").unwrap_or_else(|| "unknown".into());
                let _ = tx.execute(
                    "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![sid, ts_ms, model, value.as_f64()],
                );
            });
            Ok(())
        }

        "claude_code.code_edit_tool.decision" => {
            for_each_number(metric, |attrs, ts_ms, _v| {
                let Some(sid) = sid else { return };
                let tool = attr_string(attrs, "tool")
                    .or_else(|| attr_string(attrs, "tool.name"))
                    .unwrap_or_else(|| "unknown".into());
                let decision =
                    attr_string(attrs, "decision").unwrap_or_else(|| "unknown".into());
                let language = attr_string(attrs, "language");
                let file_path = attr_string(attrs, "file.path")
                    .or_else(|| attr_string(attrs, "file_path"));
                let _ = tx.execute(
                    "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision, language, file_path)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![sid, ts_ms, tool, decision, language, file_path],
                );
            });
            Ok(())
        }

        "claude_code.lines_of_code.count" => {
            for_each_number(metric, |attrs, ts_ms, value| {
                let Some(sid) = sid else { return };
                let kind = attr_string(attrs, "type").unwrap_or_default();
                let file_path = attr_string(attrs, "file.path")
                    .or_else(|| attr_string(attrs, "file_path"));
                let n = value.as_i64();
                let (added, removed) = match kind.as_str() {
                    "added" => (n, 0i64),
                    "removed" => (0i64, n),
                    _ => (n, 0i64),
                };
                let _ = tx.execute(
                    "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![sid, ts_ms, file_path, added, removed],
                );
            });
            Ok(())
        }

        "claude_code.commit.count" | "claude_code.pull_request.count" => {
            let activity = if name.ends_with("commit.count") {
                "commit"
            } else {
                "pull_request"
            };
            for_each_number(metric, |_attrs, ts_ms, value| {
                let Some(sid) = sid else { return };
                let count = value.as_i64().max(1);
                let _ = tx.execute(
                    "INSERT INTO git_activity (session_id, timestamp, activity, count)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![sid, ts_ms, activity, count],
                );
            });
            Ok(())
        }

        "claude_code.active_time.total" => {
            for_each_number(metric, |attrs, ts_ms, value| {
                let Some(sid) = sid else { return };
                let kind = attr_string(attrs, "type")
                    .or_else(|| attr_string(attrs, "active_time.type"))
                    .unwrap_or_else(|| "user".into());
                let _ = tx.execute(
                    "INSERT INTO active_time (session_id, timestamp, seconds, kind)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![sid, ts_ms, value.as_f64(), kind],
                );
            });
            Ok(())
        }

        _ => store_raw(tx, ctx, metric),
    }
}

fn store_raw(
    tx: &rusqlite::Transaction,
    ctx: &ResourceCtx,
    metric: &Metric,
) -> Result<()> {
    for_each_number(metric, |attrs, ts_ms, value| {
        let attrs_json = serde_json::to_string(
            &attrs
                .iter()
                .map(|kv| (kv.key.clone(), anyvalue_to_json(kv.value.as_ref())))
                .collect::<HashMap<_, _>>(),
        )
        .unwrap_or_else(|_| "{}".into());
        let value_json = match value {
            NumValue::Int(i) => serde_json::json!({"kind":"int","value":i}).to_string(),
            NumValue::Double(d) => serde_json::json!({"kind":"double","value":d}).to_string(),
        };
        let _ = tx.execute(
            "INSERT INTO metrics_raw (session_id, timestamp, metric_name, attributes_json, value_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ctx.session_id, ts_ms, metric.name, attrs_json, value_json],
        );
    });
    Ok(())
}

#[derive(Copy, Clone)]
enum NumValue {
    Int(i64),
    Double(f64),
}

impl NumValue {
    fn as_i64(self) -> i64 {
        match self {
            NumValue::Int(i) => i,
            NumValue::Double(d) => d as i64,
        }
    }
    fn as_f64(self) -> f64 {
        match self {
            NumValue::Int(i) => i as f64,
            NumValue::Double(d) => d,
        }
    }
}

fn for_each_number<F: FnMut(&[KeyValue], i64, NumValue)>(metric: &Metric, mut f: F) {
    let Some(data) = metric.data.as_ref() else {
        return;
    };
    match data {
        Data::Sum(s) => {
            for p in &s.data_points {
                if let Some(v) = number_value(p.value.as_ref()) {
                    f(&p.attributes, ts_ms(p.time_unix_nano), v);
                }
            }
        }
        Data::Gauge(g) => {
            for p in &g.data_points {
                if let Some(v) = number_value(p.value.as_ref()) {
                    f(&p.attributes, ts_ms(p.time_unix_nano), v);
                }
            }
        }
        Data::Histogram(h) => {
            for p in &h.data_points {
                f(
                    &p.attributes,
                    ts_ms(p.time_unix_nano),
                    NumValue::Double(p.sum.unwrap_or(0.0)),
                );
            }
        }
        _ => {}
    }
}

fn number_value(v: Option<&NumberValue>) -> Option<NumValue> {
    match v? {
        NumberValue::AsInt(i) => Some(NumValue::Int(*i)),
        NumberValue::AsDouble(d) => Some(NumValue::Double(*d)),
    }
}

fn ts_ms(nanos: u64) -> i64 {
    if nanos == 0 {
        now_ms()
    } else {
        (nanos / 1_000_000) as i64
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn attr_string(attrs: &[KeyValue], key: &str) -> Option<String> {
    for kv in attrs {
        if kv.key == key {
            return kv
                .value
                .as_ref()
                .and_then(|v| anyvalue_to_string(v.value.as_ref()));
        }
    }
    None
}

fn anyvalue_to_string(v: Option<&AnyV>) -> Option<String> {
    match v? {
        AnyV::StringValue(s) => Some(s.clone()),
        AnyV::IntValue(i) => Some(i.to_string()),
        AnyV::DoubleValue(d) => Some(d.to_string()),
        AnyV::BoolValue(b) => Some(b.to_string()),
        _ => None,
    }
}

fn attrs_to_json(attrs: &[KeyValue]) -> String {
    let map: HashMap<String, serde_json::Value> = attrs
        .iter()
        .map(|kv| (kv.key.clone(), anyvalue_to_json(kv.value.as_ref())))
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

fn anyvalue_to_json(v: Option<&AnyValue>) -> serde_json::Value {
    let Some(av) = v else {
        return serde_json::Value::Null;
    };
    match av.value.as_ref() {
        Some(AnyV::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(AnyV::IntValue(i)) => serde_json::Value::Number((*i).into()),
        Some(AnyV::DoubleValue(d)) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Some(AnyV::BoolValue(b)) => serde_json::Value::Bool(*b),
        _ => serde_json::Value::Null,
    }
}
