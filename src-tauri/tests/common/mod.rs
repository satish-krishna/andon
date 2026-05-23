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
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
    /// When true, every cost/token row seeded by this call is tagged
    /// `is_subagent = 1` — simulates a JSONL-ingested subagent session.
    pub is_subagent: bool,
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
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count, is_subagent) \
             VALUES (?, ?, ?, 'input', ?, ?)",
            params![opts.session_id, started, opts.model, opts.input_tokens, opts.is_subagent as i64],
        )
        .expect("insert input token_usage");
    }
    if opts.output_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count, is_subagent) \
             VALUES (?, ?, ?, 'output', ?, ?)",
            params![opts.session_id, started, opts.model, opts.output_tokens, opts.is_subagent as i64],
        )
        .expect("insert output token_usage");
    }
    if opts.cache_read_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count, is_subagent) \
             VALUES (?, ?, ?, 'cacheRead', ?, ?)",
            params![opts.session_id, started, opts.model, opts.cache_read_tokens, opts.is_subagent as i64],
        )
        .expect("insert cacheRead token_usage");
    }
    if opts.cache_create_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count, is_subagent) \
             VALUES (?, ?, ?, 'cacheCreation', ?, ?)",
            params![opts.session_id, started, opts.model, opts.cache_create_tokens, opts.is_subagent as i64],
        )
        .expect("insert cacheCreation token_usage");
    }
    if opts.cost_usd != 0.0 {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd, is_subagent) \
             VALUES (?, ?, ?, ?, ?)",
            params![opts.session_id, started, opts.model, opts.cost_usd, opts.is_subagent as i64],
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

/// Like `test_ingestor` but also returns the `IngestionControl` so tests can
/// call `control.set_paused(true)` before ingesting.
pub fn test_ingestor_with_control(pool: &Arc<DbPool>) -> (Ingestor, IngestionControl) {
    let control = IngestionControl::default();
    let diagnostics = Diagnostics::default();
    let ingestor = Ingestor::new(Arc::clone(pool), control.clone(), diagnostics);
    (ingestor, control)
}

/// Build a test `Ingestor` sharing the provided `Diagnostics` and
/// `IngestionControl` instances with the caller (and typically the router).
/// Use together with `test_router_with` so the router and ingestor see the
/// same diagnostics state.
pub fn test_ingestor_with(
    pool: &Arc<DbPool>,
    diagnostics: Diagnostics,
    control: IngestionControl,
) -> Ingestor {
    Ingestor::new(Arc::clone(pool), control, diagnostics)
}

/// Like `test_router` but accepts externally-created `Diagnostics` and
/// `IngestionControl` so the router shares state with an ingestor built via
/// `test_ingestor_with`.
pub fn test_router_with(
    pool: &Arc<DbPool>,
    diagnostics: Diagnostics,
    control: IngestionControl,
) -> (Router, TempDir) {
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
        control,
        integration: Arc::new(Mutex::new(IntegrationStatus::AlreadyConfigured {
            settings_path: settings_path.display().to_string(),
        })),
        diagnostics,
        settings,
        forwarder,
        reports_dir,
        budget_changed: Arc::new(tokio::sync::Notify::new()),
    };

    let router = routes::router(state);
    (router, dir)
}

/// Build a test axum `Router` wired to the given pool.
/// Returns `(Router, TempDir)` — drop the `TempDir` only after the router is
/// no longer needed (it backs the settings file and reports directory).
pub fn test_router(pool: &Arc<DbPool>) -> (Router, TempDir) {
    let (router, _budget_changed, dir) = test_router_with_signal(pool);
    (router, dir)
}

/// Like `test_router` but also returns the `Notify` the API pings when the
/// budget changes — lets tests assert that `PUT /api/settings/budget` wakes
/// the budget monitor.
pub fn test_router_with_signal(
    pool: &Arc<DbPool>,
) -> (Router, Arc<tokio::sync::Notify>, TempDir) {
    let dir = tempfile::tempdir().expect("create router tempdir");

    let settings_path = dir.path().join("settings.json");
    let settings = Arc::new(
        SettingsStore::load(settings_path.clone()).expect("load settings store"),
    );
    let forwarder = Arc::new(Forwarder::new(Arc::clone(&settings)));
    let reports_dir = dir.path().join("reports");
    std::fs::create_dir_all(&reports_dir).expect("create reports dir");

    let budget_changed = Arc::new(tokio::sync::Notify::new());
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
        budget_changed: Arc::clone(&budget_changed),
    };

    let router = routes::router(state);
    (router, budget_changed, dir)
}

// ---------------------------------------------------------------------------
// Git repo helpers (reused by api_git and future test files)
// ---------------------------------------------------------------------------

/// Create a real git repository in a temporary directory with the given commit
/// messages. Returns the `TempDir` guard — drop it to delete the repo.
///
/// The helper sets `GIT_AUTHOR_*` / `GIT_COMMITTER_*` env vars and disables
/// GPG signing so tests work even without a git identity configured.
pub fn init_temp_repo(commits: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    };
    run(&["init", "-q"]);
    run(&["config", "commit.gpgsign", "false"]);
    for (i, msg) in commits.iter().enumerate() {
        std::fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", msg]);
    }
    dir
}

// ---------------------------------------------------------------------------
// OTLP sample-payload builders
// ---------------------------------------------------------------------------

use opentelemetry_proto::tonic::{
    common::v1::{any_value::Value as AnyV, AnyValue, KeyValue},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
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

/// Convenience constructor for an integer-valued `KeyValue`.
pub fn kv_int(key: &str, val: i64) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(AnyV::IntValue(val)),
        }),
    }
}

/// Convenience constructor for a double-valued `KeyValue`.
pub fn kv_f64(key: &str, val: f64) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(AnyV::DoubleValue(val)),
        }),
    }
}

/// Build a single `LogRecord` with the given event name and record-level
/// attributes. The `time_unix_nano` is set to a fixed non-zero value so tests
/// get a deterministic timestamp.
pub fn sample_log_record(event_name: &str, record_attrs: Vec<KeyValue>) -> LogRecord {
    let mut attrs = vec![kv("event.name", event_name)];
    attrs.extend(record_attrs);
    LogRecord {
        time_unix_nano: 1_700_000_000_000_000_000,
        observed_time_unix_nano: 0,
        severity_number: 0,
        severity_text: String::new(),
        body: None,
        attributes: attrs,
        dropped_attributes_count: 0,
        flags: 0,
        trace_id: vec![],
        span_id: vec![],
    }
}

/// Build a `Vec<ResourceLogs>` containing one resource with the given
/// `resource_attrs` and a single log record built with `sample_log_record`.
///
/// Mirrors the shape of `sample_sum_metric` for ergonomic test writing.
pub fn sample_export_logs(
    resource_attrs: Vec<KeyValue>,
    event_name: &str,
    record_attrs: Vec<KeyValue>,
) -> Vec<ResourceLogs> {
    vec![ResourceLogs {
        resource: Some(Resource {
            attributes: resource_attrs,
            dropped_attributes_count: 0,
        }),
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![sample_log_record(event_name, record_attrs)],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
}

/// Like `sample_export_logs` but also sets `body` on the log record to the
/// given string value. Used to test that body content is scrubbed correctly.
pub fn sample_export_logs_with_body(
    resource_attrs: Vec<KeyValue>,
    event_name: &str,
    body: &str,
    record_attrs: Vec<KeyValue>,
) -> Vec<ResourceLogs> {
    let mut record = sample_log_record(event_name, record_attrs);
    record.body = Some(AnyValue {
        value: Some(AnyV::StringValue(body.into())),
    });
    vec![ResourceLogs {
        resource: Some(Resource {
            attributes: resource_attrs,
            dropped_attributes_count: 0,
        }),
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![record],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
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
