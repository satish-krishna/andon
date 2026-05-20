use anyhow::Result;
use rusqlite::{Connection, params};

const MIGRATION_V1: &str = r#"
CREATE TABLE sessions (
    session_id        TEXT PRIMARY KEY,
    started_at        INTEGER NOT NULL,
    ended_at          INTEGER,
    user_account_uuid TEXT,
    organization_id   TEXT,
    service_version   TEXT,
    host_arch         TEXT,
    os_type           TEXT,
    terminal_type     TEXT
);

CREATE TABLE token_usage (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    model           TEXT NOT NULL,
    token_type      TEXT NOT NULL,
    count           INTEGER NOT NULL
);

CREATE TABLE cost_entries (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    model           TEXT NOT NULL,
    cost_usd        REAL NOT NULL
);

CREATE TABLE tool_decisions (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    tool_name       TEXT NOT NULL,
    decision        TEXT NOT NULL,
    language        TEXT,
    file_path       TEXT
);

CREATE TABLE file_changes (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    file_path       TEXT,
    lines_added     INTEGER NOT NULL DEFAULT 0,
    lines_removed   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE git_activity (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    activity        TEXT NOT NULL,
    count           INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE active_time (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT NOT NULL,
    timestamp       INTEGER NOT NULL,
    seconds         REAL NOT NULL,
    kind            TEXT NOT NULL
);

CREATE TABLE metrics_raw (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT,
    timestamp       INTEGER NOT NULL,
    metric_name     TEXT NOT NULL,
    attributes_json TEXT NOT NULL,
    value_json      TEXT NOT NULL
);

CREATE INDEX idx_token_session     ON token_usage(session_id, timestamp);
CREATE INDEX idx_cost_session      ON cost_entries(session_id, timestamp);
CREATE INDEX idx_decisions_session ON tool_decisions(session_id, timestamp);
CREATE INDEX idx_files_session     ON file_changes(session_id, timestamp);
"#;

const MIGRATION_V2: &str = r#"
CREATE TABLE log_events (
    id              INTEGER PRIMARY KEY,
    session_id      TEXT,
    timestamp       INTEGER NOT NULL,
    event_name      TEXT NOT NULL,
    body            TEXT,
    attributes_json TEXT NOT NULL,
    transport       TEXT NOT NULL  -- 'grpc' | 'http'
);
CREATE INDEX idx_log_events_session ON log_events(session_id, timestamp);
CREATE INDEX idx_log_events_name    ON log_events(event_name, timestamp);
CREATE INDEX idx_log_events_ts      ON log_events(timestamp DESC);
"#;

const MIGRATION_V3: &str = r#"
ALTER TABLE sessions ADD COLUMN cwd TEXT;
ALTER TABLE sessions ADD COLUMN repo_root TEXT;
ALTER TABLE sessions ADD COLUMN repo_remote TEXT;
ALTER TABLE sessions ADD COLUMN repo_branch TEXT;
ALTER TABLE sessions ADD COLUMN repo_name TEXT;
CREATE INDEX idx_sessions_repo_remote ON sessions(repo_remote);
CREATE INDEX idx_sessions_repo_root   ON sessions(repo_root);
"#;

const MIGRATION_V4: &str = r#"
-- Distinguishes OTLP-derived from JSONL-derived sessions.
ALTER TABLE sessions ADD COLUMN data_source TEXT;
UPDATE sessions SET data_source = 'otlp' WHERE data_source IS NULL;

-- Distinguishes OTLP-emitted decisions from JSONL-derived tool calls.
ALTER TABLE tool_decisions ADD COLUMN source TEXT NOT NULL DEFAULT 'otlp';
ALTER TABLE tool_decisions ADD COLUMN model TEXT;

-- New tables.
CREATE TABLE slash_commands (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT NOT NULL,
    timestamp     INTEGER NOT NULL,
    command_name  TEXT NOT NULL,
    arg_count     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_slash_session ON slash_commands(session_id);
CREATE INDEX idx_slash_name    ON slash_commands(command_name);

CREATE TABLE subagent_calls (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_session_id  TEXT NOT NULL,
    child_session_id   TEXT,
    subagent_type      TEXT,
    started_at         INTEGER NOT NULL,
    ended_at           INTEGER,
    tool_call_count    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_subagent_parent ON subagent_calls(parent_session_id);

CREATE TABLE jsonl_errors (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    jsonl_path     TEXT NOT NULL,
    line_no        INTEGER NOT NULL,
    error_kind     TEXT NOT NULL,
    error_msg      TEXT NOT NULL,
    cc_version     TEXT,
    ingested_at    INTEGER NOT NULL
);
CREATE INDEX idx_jsonl_errors_ts ON jsonl_errors(ingested_at);

CREATE TABLE jsonl_ingest_runs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    kind                TEXT NOT NULL,
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER,
    files_processed     INTEGER NOT NULL DEFAULT 0,
    records_processed   INTEGER NOT NULL DEFAULT 0,
    records_errored     INTEGER NOT NULL DEFAULT 0
);
"#;

const MIGRATION_V5: &str = r#"
-- Per-API-call identity for JSONL-derived rows. NULL on OTLP-derived rows.
ALTER TABLE cost_entries ADD COLUMN request_id TEXT;
ALTER TABLE token_usage  ADD COLUMN request_id TEXT;

-- Uniqueness enforced ONLY on JSONL rows; OTLP rows (request_id IS NULL) are unconstrained.
CREATE UNIQUE INDEX idx_cost_request
    ON cost_entries(request_id)            WHERE request_id IS NOT NULL;
CREATE UNIQUE INDEX idx_token_request
    ON token_usage(request_id, token_type) WHERE request_id IS NOT NULL;

-- Per-session transcript API-call count; powers partial-OTLP detection.
CREATE TABLE session_jsonl_calls (
    session_id  TEXT PRIMARY KEY,
    api_calls   INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
"#;

const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
    (5, MIGRATION_V5),
];

pub fn apply(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )?;

    let current: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        tracing::info!(version, "applying migration");
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            params![version, now_ms],
        )?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn v3_adds_repo_columns_and_indexes() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(|r| r.unwrap()).collect();
        for expected in ["cwd", "repo_root", "repo_remote", "repo_branch", "repo_name"] {
            assert!(cols.contains(&expected.to_string()), "missing column {expected}");
        }

        let idxs: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='sessions'").unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap()
            .map(|r| r.unwrap()).collect();
        assert!(idxs.contains(&"idx_sessions_repo_remote".to_string()));
        assert!(idxs.contains(&"idx_sessions_repo_root".to_string()));

        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(v, 5);
    }

    #[test]
    fn migrations_are_idempotent_across_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        apply(&mut conn).unwrap(); // second call must be a no-op
        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(v, 5);
    }

    #[test]
    fn v4_creates_jsonl_tables_and_extends_decisions() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        for tbl in ["slash_commands", "subagent_calls", "jsonl_errors", "jsonl_ingest_runs"] {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [tbl], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1, "missing table {tbl}");
        }

        let cols: Vec<String> = conn.prepare("PRAGMA table_info(tool_decisions)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(|r| r.unwrap()).collect();
        for c in ["source", "model"] {
            assert!(cols.contains(&c.to_string()), "missing tool_decisions column {c}");
        }

        let cols: Vec<String> = conn.prepare("PRAGMA table_info(sessions)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(|r| r.unwrap()).collect();
        assert!(cols.contains(&"data_source".to_string()));

        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(v, 5);
    }

    #[test]
    fn v5_adds_request_id_and_coverage_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        for tbl in ["cost_entries", "token_usage"] {
            let cols: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({tbl})")).unwrap()
                .query_map([], |r| r.get::<_, String>(1)).unwrap()
                .map(|r| r.unwrap()).collect();
            assert!(cols.contains(&"request_id".to_string()), "{tbl} missing request_id");
        }

        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_jsonl_calls'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "session_jsonl_calls table missing");

        for idx in ["idx_cost_request", "idx_token_request"] {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [idx], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1, "missing index {idx}");
        }

        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(v, 5);
    }
}
