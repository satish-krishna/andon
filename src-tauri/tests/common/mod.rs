#![allow(dead_code)] // helpers are shared across many test files

use std::sync::Arc;

use andon_lib::db::DbPool;
use rusqlite::params;
use tempfile::TempDir;

/// Build an isolated SQLite pool backed by a temp file (WAL requires a real file).
/// Returns the pool plus the TempDir guard — drop the guard to delete the DB.
pub fn fixture_pool() -> (Arc<DbPool>, TempDir) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("test.db");
    let pool = andon_lib::db::init(&db_path).expect("open pool and run migrations");
    (Arc::new(pool), dir)
}

// ---------------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FileChange {
    pub path: &'static str,
    pub added: i64,
    pub removed: i64,
}

#[derive(Default, Clone)]
pub struct SeedOpts {
    pub session_id: String,
    /// Overrides the current timestamp (unix ms). Defaults to `now`.
    pub started_at_ms: Option<i64>,
    /// None = session still open.
    pub ended_at_ms: Option<i64>,
    /// Full model id, e.g. `claude-opus-4-5-20251001`.
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    /// Pairs of (decision, language). decision ∈ accept | reject | abort.
    pub decisions: Vec<(&'static str, &'static str)>,
    pub files: Vec<FileChange>,
}

/// Insert a session row plus all related rows described by `opts`.
pub fn seed_session(pool: &Arc<DbPool>, opts: &SeedOpts) {
    let now = chrono::Utc::now().timestamp_millis();
    let started = opts.started_at_ms.unwrap_or(now);
    let conn = pool.get().expect("checkout connection");

    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES (?, ?, ?)",
        params![opts.session_id, started, opts.ended_at_ms],
    )
    .expect("insert session");

    if opts.input_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES (?, ?, ?, 'input', ?)",
            params![opts.session_id, started, opts.model, opts.input_tokens],
        )
        .expect("insert input token_usage");
    }
    if opts.output_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES (?, ?, ?, 'output', ?)",
            params![opts.session_id, started, opts.model, opts.output_tokens],
        )
        .expect("insert output token_usage");
    }
    if opts.cost_usd != 0.0 {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES (?, ?, ?, ?)",
            params![opts.session_id, started, opts.model, opts.cost_usd],
        )
        .expect("insert cost_entries");
    }
    for (decision, lang) in &opts.decisions {
        conn.execute(
            "INSERT INTO tool_decisions \
             (session_id, timestamp, tool_name, decision, language) \
             VALUES (?, ?, 'Edit', ?, ?)",
            params![opts.session_id, started, decision, lang],
        )
        .expect("insert tool_decisions");
    }
    for f in &opts.files {
        conn.execute(
            "INSERT INTO file_changes \
             (session_id, timestamp, file_path, lines_added, lines_removed) \
             VALUES (?, ?, ?, ?, ?)",
            params![opts.session_id, started, f.path, f.added, f.removed],
        )
        .expect("insert file_changes");
    }
}

// ---------------------------------------------------------------------------
// Test-router and test-ingestor helpers
// ---------------------------------------------------------------------------

use std::sync::Mutex;

use andon_lib::{
    api::{routes, ApiState},
    diagnostics::Diagnostics,
    integration::IntegrationStatus,
    otlp::{
        IngestionControl,
        forwarder::Forwarder,
        ingestor::Ingestor,
    },
    settings::SettingsStore,
};
use axum::Router;

/// Build a test `Ingestor` wired to the given pool.
/// Returns the ingestor; the caller keeps the pool guard alive via the `TempDir`.
pub fn test_ingestor(pool: &Arc<DbPool>) -> Ingestor {
    let control = IngestionControl::default();
    let diagnostics = Diagnostics::default();
    Ingestor::new(Arc::clone(pool), control, diagnostics)
}

/// Build a test axum `Router` wired to the given pool.
/// Returns `(Router, TempDir)` — drop the `TempDir` only after the router is
/// no longer needed (it backs the settings file and reports directory).
pub fn test_router(pool: &Arc<DbPool>) -> (Router, TempDir) {
    let dir = tempfile::tempdir().expect("create router tempdir");

    let settings_path = dir.path().join("settings.json");
    let settings = Arc::new(
        SettingsStore::load(settings_path.clone()).expect("load settings store"),
    );
    let forwarder = Arc::new(Forwarder::new(Arc::clone(&settings)));
    let reports_dir = dir.path().join("reports");
    std::fs::create_dir_all(&reports_dir).expect("create reports dir");

    let state = ApiState {
        pool: Arc::clone(pool),
        db_path: dir.path().join("test.db"),
        control: IngestionControl::default(),
        integration: Arc::new(Mutex::new(IntegrationStatus::AlreadyConfigured {
            settings_path: settings_path.display().to_string(),
        })),
        diagnostics: Diagnostics::default(),
        settings,
        forwarder,
        reports_dir,
    };

    let router = routes::router(state);
    (router, dir)
}

// ---------------------------------------------------------------------------
// OTLP sample-payload builders
// ---------------------------------------------------------------------------

use opentelemetry_proto::tonic::{
    common::v1::{any_value::Value as AnyV, AnyValue, KeyValue},
    metrics::v1::{
        metric::Data, number_data_point::Value as NumberValue, Metric, NumberDataPoint,
        ResourceMetrics, ScopeMetrics, Sum,
    },
    resource::v1::Resource,
};

/// Convenience constructor for a string-valued `KeyValue`.
pub fn kv(key: &str, val: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(AnyV::StringValue(val.into())),
        }),
    }
}

/// Build a `Vec<ResourceMetrics>` containing one `Sum`-typed metric with a
/// single `NumberDataPoint`. Useful for exercising the ingestor in Phase 2+.
pub fn sample_sum_metric(
    resource_attrs: Vec<KeyValue>,
    metric_name: &str,
    point_attrs: Vec<KeyValue>,
    value: f64,
) -> Vec<ResourceMetrics> {
    vec![ResourceMetrics {
        resource: Some(Resource {
            attributes: resource_attrs,
            dropped_attributes_count: 0,
        }),
        scope_metrics: vec![ScopeMetrics {
            scope: None,
            metrics: vec![Metric {
                name: metric_name.into(),
                description: String::new(),
                unit: String::new(),
                metadata: vec![],
                data: Some(Data::Sum(Sum {
                    data_points: vec![NumberDataPoint {
                        attributes: point_attrs,
                        start_time_unix_nano: 0,
                        time_unix_nano: 1_700_000_000_000_000_000,
                        exemplars: vec![],
                        flags: 0,
                        value: Some(NumberValue::AsDouble(value)),
                    }],
                    aggregation_temporality: 2, // Cumulative
                    is_monotonic: true,
                })),
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
}
