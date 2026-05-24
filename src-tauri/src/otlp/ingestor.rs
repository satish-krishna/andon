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
use crate::settings::CoachSettings;

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
    pub(crate) pool: Arc<DbPool>,
    pub(crate) control: IngestionControl,
    pub(crate) diagnostics: Diagnostics,
    pub(crate) coach_settings: CoachSettings,
}

impl Ingestor {
    pub fn new(
        pool: Arc<DbPool>,
        control: IngestionControl,
        diagnostics: Diagnostics,
        coach_settings: CoachSettings,
    ) -> Self {
        Self {
            pool,
            control,
            diagnostics,
            coach_settings,
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

                    let raw_body_str = record
                        .body
                        .as_ref()
                        .and_then(|b| anyvalue_to_string(b.value.as_ref()));

                    // Privacy amendment (see docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md
                    // §Privacy contract amendment): prompts are now allowed at rest. The
                    // forwarder strips them on egress (src/otlp/forwarder.rs::redact_user_prompt).
                    let (body_str, attrs_json) = (raw_body_str, attrs_to_json(&record.attributes));

                    // 1. Always persist a (possibly redacted) copy.
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

                    // 3. Phase-1 coach: persist a prompt_turns row for user_prompt events.
                    if event_name == "user_prompt" {
                        if let (Some(sid), Some(body)) = (record_ctx.session_id.as_deref(), body_str.as_ref()) {
                            let text = body.clone();
                            let length = text.chars().count() as i64;
                            let has_file_ref = text.contains('@');
                            let has_code = text.contains("```");
                            let lc = text.to_lowercase();
                            let has_constraint = self.coach_settings
                                .constraint_keywords.iter()
                                .any(|kw| lc.contains(&kw.to_lowercase()));
                            let command = text.strip_prefix('/').and_then(|rest| {
                                rest.split_whitespace().next().map(|s| s.to_string())
                            });
                            let norm_hash = crate::coach::skill::norm_hash(&text);
                            let turn_index: i64 = tx.query_row(
                                "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM prompt_turns WHERE session_id = ?1",
                                params![sid], |r| r.get(0),
                            ).unwrap_or(0);
                            let _ = tx.execute(
                                "INSERT OR IGNORE INTO prompt_turns
                                   (session_id, request_id, turn_index, ts, source, text,
                                    norm_hash, command, length, has_file_ref, has_code, has_constraint)
                                 VALUES (?1, NULL, ?2, ?3, 'otlp', ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                                params![
                                    sid, turn_index, ts_ms, text, norm_hash, command,
                                    length, has_file_ref as i64, has_code as i64, has_constraint as i64,
                                ],
                            );
                        }
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

    /// Returns `(tokens_written, cost_written, sessions_inserted)`, where
    /// `sessions_inserted` is the number of `sessions` rows the SessionLifecycle
    /// `INSERT OR IGNORE` actually created (0 when the session already existed).
    pub fn ingest_derived(
        &self,
        events: &[crate::jsonl::reducer::DerivedEvent],
        coverage: crate::jsonl::reconciler::Coverage,
    ) -> Result<(i64, i64, i64)> {
        use crate::jsonl::reconciler::Coverage;
        use crate::jsonl::reducer::DerivedEvent as E;

        if self.control.is_paused() {
            return Ok((0, 0, 0));
        }
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let mut tokens_written: i64 = 0;
        let mut cost_written: i64 = 0;
        let mut sessions_inserted: i64 = 0;

        for ev in events {
            match ev {
                E::SessionLifecycle {
                    session_id,
                    started_at,
                    ended_at,
                    cc_version,
                    cwd,
                    git_branch,
                } => {
                    // Binary routing: a JSONL-only session is 'jsonl'; an
                    // OTLP-covered session keeps 'otlp'. 'mixed' is no longer used.
                    match tx.execute(
                        "INSERT OR IGNORE INTO sessions
                           (session_id, started_at, ended_at, service_version, cwd, repo_branch, data_source)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'jsonl')",
                        params![session_id, started_at, ended_at, cc_version, cwd, git_branch],
                    ) {
                        Ok(rows) => sessions_inserted += rows as i64,
                        Err(e) => {
                            tracing::warn!(error = ?e, session_id, "JSONL session insert failed");
                        }
                    }
                }
                E::TokenUsage {
                    session_id,
                    request_id,
                    ts,
                    model,
                    input,
                    output,
                    cache_create,
                    cache_read,
                    is_subagent,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) {
                        for (kind, n) in [
                            ("input", *input),
                            ("output", *output),
                            ("cacheRead", *cache_read),
                            ("cacheCreation", *cache_create),
                        ] {
                            if n > 0 {
                                let affected = match tx.execute(
                                    "INSERT INTO token_usage
                                       (session_id, request_id, timestamp, model, token_type, count, is_subagent)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                                     ON CONFLICT(request_id, token_type)
                                       WHERE request_id IS NOT NULL DO UPDATE
                                       SET is_subagent = 1
                                       WHERE excluded.is_subagent = 1 AND token_usage.is_subagent = 0",
                                    params![session_id, request_id, ts, model, kind, n, *is_subagent as i64],
                                ) {
                                    Ok(rows) => rows,
                                    Err(e) => {
                                        // ON CONFLICT DO UPDATE returns Ok(1) only when this insert
                                        // actually flipped a row from 0 to 1 (the guarded WHERE
                                        // eliminates both re-flips and non-sidechain conflicts).
                                        // `tokens_written` therefore counts new rows + true 0->1
                                        // flips, never redundant updates.
                                        // Any Err is a genuine insert failure — log, never surface.
                                        tracing::warn!(error = ?e, session_id, "JSONL token_usage insert failed");
                                        0
                                    }
                                };
                                tokens_written += affected as i64;
                            }
                        }
                    }
                }
                E::CostEntry {
                    session_id,
                    request_id,
                    ts,
                    model,
                    cost_usd,
                    is_subagent,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) && *cost_usd > 0.0 {
                        let affected = match tx.execute(
                            "INSERT INTO cost_entries
                               (session_id, request_id, timestamp, model, cost_usd, is_subagent)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                             ON CONFLICT(request_id)
                               WHERE request_id IS NOT NULL DO UPDATE
                               SET is_subagent = 1
                               WHERE excluded.is_subagent = 1 AND cost_entries.is_subagent = 0",
                            params![session_id, request_id, ts, model, cost_usd, *is_subagent as i64],
                        ) {
                            Ok(rows) => rows,
                            Err(e) => {
                                // ON CONFLICT DO UPDATE returns Ok(1) only when this insert
                                // actually flipped a row from 0 to 1 (the guarded WHERE
                                // eliminates both re-flips and non-sidechain conflicts).
                                // `cost_written` therefore counts new rows + true 0->1 flips,
                                // never redundant updates.
                                // Any Err is a genuine insert failure — log, never surface.
                                tracing::warn!(error = ?e, session_id, "JSONL cost_entries insert failed");
                                0
                            }
                        };
                        cost_written += affected as i64;
                    }
                }
                E::ToolCall {
                    session_id,
                    ts,
                    tool_name,
                    file_path,
                    model,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) {
                        let _ = tx.execute(
                            "INSERT INTO tool_decisions
                               (session_id, timestamp, tool_name, decision, language, file_path, source, model)
                             VALUES (?1, ?2, ?3, 'invoke', NULL, ?4, 'jsonl', ?5)",
                            params![session_id, ts, tool_name, file_path, model],
                        );
                    }
                }
                E::SlashCommand {
                    session_id,
                    ts,
                    name,
                    arg_count,
                } => {
                    let _ = tx.execute(
                        "INSERT INTO slash_commands (session_id, timestamp, command_name, arg_count)
                         SELECT ?1, ?2, ?3, ?4
                         WHERE NOT EXISTS (
                             SELECT 1 FROM slash_commands
                             WHERE session_id = ?1 AND timestamp = ?2 AND command_name = ?3
                         )",
                        params![session_id, ts, name, arg_count],
                    );
                }
                E::SubAgentCall {
                    parent_id,
                    child_id,
                    subagent_type,
                    started_at,
                } => {
                    let _ = tx.execute(
                        "INSERT INTO subagent_calls
                           (parent_session_id, child_session_id, subagent_type, started_at)
                         SELECT ?1, ?2, ?3, ?4
                         WHERE NOT EXISTS (
                             SELECT 1 FROM subagent_calls
                             WHERE parent_session_id = ?1
                               AND started_at = ?4
                               AND COALESCE(subagent_type, '') = COALESCE(?3, '')
                         )",
                        params![parent_id, child_id, subagent_type, started_at],
                    );
                }
                E::PromptTurn {
                    session_id,
                    request_id,
                    turn_index,
                    ts_ms,
                    text,
                    norm_hash,
                    command,
                    length,
                    has_file_ref,
                    has_code,
                    has_constraint,
                } => {
                    let _ = tx.execute(
                        "INSERT OR IGNORE INTO prompt_turns
                           (session_id, request_id, turn_index, ts, source, text,
                            norm_hash, command, length, has_file_ref, has_code, has_constraint)
                         VALUES (?1, ?2, ?3, ?4, 'jsonl', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            session_id,
                            request_id,
                            turn_index,
                            ts_ms,
                            text,
                            norm_hash,
                            command,
                            length,
                            *has_file_ref as i64,
                            *has_code as i64,
                            *has_constraint as i64,
                        ],
                    );
                }
            }
        }
        tx.commit()?;
        Ok((tokens_written, cost_written, sessions_inserted))
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

/// Like `attrs_to_json` but replaces the values of any key in `redact_keys`
/// with the string `"[redacted]"`. The key is preserved so that key-existence
/// and count-based analytics still work.
fn attrs_to_json_redacted(attrs: &[KeyValue], redact_keys: &[&str]) -> String {
    let map: HashMap<String, serde_json::Value> = attrs
        .iter()
        .map(|kv| {
            let value = if redact_keys.contains(&kv.key.as_str()) {
                serde_json::Value::String("[redacted]".into())
            } else {
                anyvalue_to_json(kv.value.as_ref())
            };
            (kv.key.clone(), value)
        })
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
