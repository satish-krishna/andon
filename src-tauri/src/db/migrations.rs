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

const MIGRATIONS: &[(i32, &str)] = &[(1, MIGRATION_V1)];

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
