use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const STARTUP_GRACE_MS: i64 = 5_000;

#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    start_ms: i64,
    counters: HashMap<String, u64>,
    transport_counters: HashMap<String, u64>,
    total_records: u64,
    total_payloads: u64,
    last_event_ms: Option<i64>,
    last_session_id: Option<String>,
    last_payload_summary: Option<PayloadSummary>,
    grpc_bound: Option<bool>,
    http_bound: Option<bool>,
    api_bound: Option<bool>,
    last_error: Option<String>,
    persisted_records: u64,        // seeded from log_events at startup
    persisted_last_ms: Option<i64>,
}

#[derive(Clone, Serialize)]
pub struct PayloadSummary {
    pub transport: String,
    pub received_at: i64,
    pub resource_logs: usize,
    pub records: usize,
    pub event_names: Vec<String>,
}

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub start_ms: i64,
    pub uptime_seconds: i64,
    pub total_records: u64,
    pub total_payloads: u64,
    pub last_event_ms: Option<i64>,
    pub seconds_since_last_event: Option<i64>,
    pub last_session_id: Option<String>,
    pub event_counters: HashMap<String, u64>,
    pub transport_counters: HashMap<String, u64>,
    pub last_payload: Option<PayloadSummary>,
    pub binds: BindStatuses,
    pub last_error: Option<String>,
    pub health: HealthVerdict,
}

#[derive(Serialize)]
pub struct BindStatuses {
    pub grpc_4317: Option<bool>,
    pub http_4318: Option<bool>,
    pub api_8765: Option<bool>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HealthVerdict {
    Healthy,
    NoData { reason: String },
    BindFailed { ports: Vec<String> },
    Starting,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                start_ms: now_ms(),
                counters: HashMap::new(),
                transport_counters: HashMap::new(),
                total_records: 0,
                total_payloads: 0,
                last_event_ms: None,
                last_session_id: None,
                last_payload_summary: None,
                grpc_bound: None,
                http_bound: None,
                api_bound: None,
                last_error: None,
                persisted_records: 0,
                persisted_last_ms: None,
            })),
        }
    }

    /// Seed the persisted history from the log_events table so that "health"
    /// reflects whether ANY telemetry has ever arrived (not just since this
    /// process started).
    pub fn seed_from_db(&self, pool: &crate::db::DbPool) {
        let Ok(conn) = pool.get() else { return };
        let records: u64 = conn
            .query_row("SELECT COUNT(*) FROM log_events", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
            .max(0) as u64;
        let last_ms: Option<i64> = conn
            .query_row("SELECT MAX(timestamp) FROM log_events", [], |r| r.get::<_, Option<i64>>(0))
            .unwrap_or(None);
        let mut i = self.inner.lock().unwrap();
        i.persisted_records = records;
        i.persisted_last_ms = last_ms;
        tracing::info!(
            persisted_records = records,
            persisted_last_ms = ?last_ms,
            "diagnostics seeded from log_events"
        );
    }

    pub fn record_bind(&self, port: &str, ok: bool, error: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        match port {
            "grpc" => i.grpc_bound = Some(ok),
            "http" => i.http_bound = Some(ok),
            "api" => i.api_bound = Some(ok),
            _ => {}
        }
        if !ok {
            if let Some(e) = error {
                tracing::error!(port, error = %e, "diag: bind failed");
                i.last_error = Some(format!("bind {port}: {e}"));
            }
        } else {
            tracing::info!(port, "diag: bind ok");
        }
    }

    pub fn record_payload(
        &self,
        transport: &str,
        resource_logs: usize,
        records: usize,
        event_names: Vec<String>,
        session_id: Option<String>,
    ) {
        let now = now_ms();
        let mut i = self.inner.lock().unwrap();
        i.total_payloads += 1;
        i.total_records += records as u64;
        i.last_event_ms = Some(now);
        if let Some(s) = session_id {
            i.last_session_id = Some(s);
        }
        *i.transport_counters.entry(transport.to_string()).or_insert(0) += 1;
        for name in &event_names {
            *i.counters.entry(name.clone()).or_insert(0) += 1;
        }
        i.last_payload_summary = Some(PayloadSummary {
            transport: transport.to_string(),
            received_at: now,
            resource_logs,
            records,
            event_names,
        });
        tracing::info!(
            transport,
            resource_logs,
            records,
            total = i.total_records,
            "diag: payload"
        );
    }

    pub fn snapshot(&self) -> DiagnosticsSnapshot {
        let i = self.inner.lock().unwrap();
        let now = now_ms();
        let uptime = (now - i.start_ms) / 1000;
        let seconds_since_last = i.last_event_ms.map(|t| (now - t) / 1000);

        let bind_failed: Vec<String> = [
            ("4317 (gRPC)", i.grpc_bound),
            ("4318 (HTTP)", i.http_bound),
            ("8765 (API)", i.api_bound),
        ]
        .into_iter()
        .filter(|(_, b)| matches!(b, Some(false)))
        .map(|(n, _)| n.to_string())
        .collect();

        let has_persisted_recent = i
            .persisted_last_ms
            .map(|t| now - t < 24 * 60 * 60 * 1000)  // saw something in the last 24h
            .unwrap_or(false);
        let has_persisted_ever = i.persisted_records > 0;

        let health = if !bind_failed.is_empty() {
            HealthVerdict::BindFailed {
                ports: bind_failed,
            }
        } else if i.total_records > 0 || has_persisted_recent {
            HealthVerdict::Healthy
        } else if now - i.start_ms < STARTUP_GRACE_MS {
            HealthVerdict::Starting
        } else if has_persisted_ever {
            HealthVerdict::NoData {
                reason: format!(
                    "No telemetry since andon started. \
                     Last event recorded {} hours ago. \
                     Start a new Claude Code session so it picks up the OTel env vars.",
                    i.persisted_last_ms
                        .map(|t| ((now - t) / (60 * 60 * 1000)).max(1))
                        .unwrap_or(0)
                ),
            }
        } else {
            HealthVerdict::NoData {
                reason: "No OTLP traffic received yet. \
                 Make sure ~/.claude/settings.json has the OTel env vars \
                 AND restart any running Claude Code sessions so they pick them up."
                    .to_string(),
            }
        };

        // expose "last event" using whichever is most recent: this-session or persisted
        let display_last = i.last_event_ms.or(i.persisted_last_ms);
        let display_since = display_last.map(|t| (now - t) / 1000);

        DiagnosticsSnapshot {
            start_ms: i.start_ms,
            uptime_seconds: uptime,
            total_records: i.total_records + i.persisted_records,
            total_payloads: i.total_payloads,
            last_event_ms: display_last,
            seconds_since_last_event: display_since,
            last_session_id: i.last_session_id.clone(),
            event_counters: i.counters.clone(),
            transport_counters: i.transport_counters.clone(),
            last_payload: i.last_payload_summary.clone(),
            binds: BindStatuses {
                grpc_4317: i.grpc_bound,
                http_4318: i.http_bound,
                api_8765: i.api_bound,
            },
            last_error: i.last_error.clone(),
            health,
        }
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
