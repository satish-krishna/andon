mod common;

// ---------------------------------------------------------------------------
// log_events: every record produces a raw row
// ---------------------------------------------------------------------------

/// Every ingested log record — regardless of event name — must land in
/// `log_events` with the correct session_id, event_name, and transport.
#[test]
fn log_record_always_persisted_in_log_events() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-log-basic")],
        "some_event",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "http").expect("ingest");

    let conn = pool.get().unwrap();
    let (event_name, transport): (String, String) = conn
        .query_row(
            "SELECT event_name, transport FROM log_events \
             WHERE session_id = 'sess-log-basic'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(event_name, "some_event");
    assert_eq!(transport, "http");
}

// ---------------------------------------------------------------------------
// api_request: inserts cost_entries, token_usage, and active_time rows
// ---------------------------------------------------------------------------

#[test]
fn api_request_event_inserts_cost_entry() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-api-cost")],
        "api_request",
        vec![
            common::kv("model", "claude-sonnet-4-6"),
            common::kv_f64("cost_usd", 0.042),
            common::kv_int("input_tokens", 100),
            common::kv_int("output_tokens", 50),
            common::kv_int("duration_ms", 1200),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (model, cost): (String, f64) = conn
        .query_row(
            "SELECT model, cost_usd FROM cost_entries WHERE session_id = 'sess-api-cost'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(model, "claude-sonnet-4-6");
    assert!((cost - 0.042).abs() < 1e-9, "cost_usd mismatch: {cost}");
}

#[test]
fn api_request_event_inserts_token_usage_rows() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-api-tok")],
        "api_request",
        vec![
            common::kv("model", "claude-haiku"),
            common::kv_int("input_tokens", 200),
            common::kv_int("output_tokens", 80),
            common::kv_int("cache_read_tokens", 30),
            common::kv_int("cache_creation_tokens", 10),
            common::kv_f64("cost_usd", 0.001),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    // Collect all token_usage rows for this session.
    let mut stmt = conn
        .prepare(
            "SELECT token_type, count FROM token_usage \
             WHERE session_id = 'sess-api-tok' ORDER BY token_type",
        )
        .unwrap();
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    // Expect four rows: cacheCreation, cacheRead, input, output (sorted).
    assert_eq!(rows.len(), 4, "expected 4 token_usage rows, got: {rows:?}");
    let map: std::collections::HashMap<_, _> = rows.into_iter().collect();
    assert_eq!(map["input"], 200);
    assert_eq!(map["output"], 80);
    assert_eq!(map["cacheRead"], 30);
    assert_eq!(map["cacheCreation"], 10);
}

#[test]
fn api_request_event_inserts_active_time_cli_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-api-at")],
        "api_request",
        vec![
            common::kv("model", "claude-opus"),
            common::kv_int("duration_ms", 3000),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (seconds, kind): (f64, String) = conn
        .query_row(
            "SELECT seconds, kind FROM active_time WHERE session_id = 'sess-api-at'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((seconds - 3.0).abs() < 1e-9, "seconds mismatch: {seconds}");
    assert_eq!(kind, "cli");
}

// ---------------------------------------------------------------------------
// user_prompt: inserts active_time row, does NOT persist prompt text
// ---------------------------------------------------------------------------

#[test]
fn user_prompt_event_inserts_active_time_user_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // prompt_length = 50 chars → secs = max(50/5, 1) = 10.0
    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-up-at")],
        "user_prompt",
        vec![common::kv_int("prompt_length", 50)],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (seconds, kind): (f64, String) = conn
        .query_row(
            "SELECT seconds, kind FROM active_time WHERE session_id = 'sess-up-at'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((seconds - 10.0).abs() < 1e-9, "seconds mismatch: {seconds}");
    assert_eq!(kind, "user");
}

/// Privacy check: a `prompt` attribute on a `user_prompt` event must NOT be
/// stored anywhere in the database. Raw user prompts are never persisted.
/// The "prompt" key must still appear in attributes_json (with a redacted
/// placeholder) so that key-existence and count analytics remain possible.
#[test]
fn user_prompt_raw_text_is_never_persisted() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let secret = "super secret user input that must not land in DB";
    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-privacy")],
        "user_prompt",
        vec![
            common::kv("prompt", secret),
            common::kv_int("prompt_length", secret.len() as i64),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();

    // Check log_events body and attributes_json columns.
    let (body, attrs_json): (Option<String>, String) = conn
        .query_row(
            "SELECT body, attributes_json FROM log_events WHERE session_id = 'sess-privacy'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    // Body must be NULL — raw prompt content is never written to log_events.body.
    assert!(
        body.is_none(),
        "log_events.body must be NULL for user_prompt events, got: {body:?}"
    );

    // The secret must not appear anywhere in attributes_json.
    assert!(
        !attrs_json.contains(secret),
        "raw prompt text found in log_events.attributes_json: {attrs_json}"
    );

    // The "prompt" key must still be present (with "[redacted]" value) so that
    // key-existence and length-based analytics still work.
    assert!(
        attrs_json.contains("\"prompt\""),
        "attributes_json must still contain the 'prompt' key: {attrs_json}"
    );
    assert!(
        attrs_json.contains("[redacted]"),
        "attributes_json 'prompt' value must be '[redacted]': {attrs_json}"
    );

    // One active_time row must still be written (prompt_length-based heuristic).
    let active_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM active_time WHERE kind = 'user' AND session_id = 'sess-privacy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 1, "expected exactly one active_time row");

    // No cost entry should exist for a user_prompt event.
    let cost_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_count, 0, "no cost entry should exist for a user_prompt event");

    // Confirm no dedicated prompts / user_prompts table exists.
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name IN ('prompts', 'user_prompts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(table_exists, 0, "no dedicated prompt-storage table should exist");
}

/// Privacy check: when the secret appears in the log record *body* (not just
/// attributes), it must also be wiped for user_prompt events.
#[test]
fn user_prompt_body_text_is_never_persisted() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let secret = "another secret that must not land in DB via body";
    // Build a payload where the body carries the secret text.
    let payload = common::sample_export_logs_with_body(
        vec![common::kv("session.id", "sess-privacy-body")],
        "user_prompt",
        secret,
        vec![common::kv_int("prompt_length", secret.len() as i64)],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();

    let (body, attrs_json): (Option<String>, String) = conn
        .query_row(
            "SELECT body, attributes_json FROM log_events WHERE session_id = 'sess-privacy-body'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    // Body must be NULL — raw prompt content is never written to log_events.body.
    assert!(
        body.is_none(),
        "log_events.body must be NULL for user_prompt events even when set on the record, got: {body:?}"
    );

    // Secret must not appear in attributes_json either.
    assert!(
        !attrs_json.contains(secret),
        "raw prompt body text found in log_events.attributes_json: {attrs_json}"
    );
}

// ---------------------------------------------------------------------------
// tool_decision / tool_result: inserts tool_decisions rows
// ---------------------------------------------------------------------------

#[test]
fn tool_decision_event_inserts_tool_decisions_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-td")],
        "tool_decision",
        vec![
            common::kv("tool_name", "Bash"),
            common::kv("decision", "accept"),
            common::kv("language", "shell"),
            common::kv("file.path", "scripts/build.sh"),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (tool, decision, language, file_path): (String, String, String, String) = conn
        .query_row(
            "SELECT tool_name, decision, language, file_path \
             FROM tool_decisions WHERE session_id = 'sess-td'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(tool, "Bash");
    assert_eq!(decision, "accept");
    assert_eq!(language, "shell");
    assert_eq!(file_path, "scripts/build.sh");
}

#[test]
fn tool_result_event_inserts_tool_decisions_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-tr")],
        "tool_result",
        vec![
            common::kv("tool_name", "Edit"),
            common::kv("decision", "reject"),
        ],
    );
    ingestor.ingest_logs_v2(payload, "http").expect("ingest");

    let conn = pool.get().unwrap();
    let (tool, decision): (String, String) = conn
        .query_row(
            "SELECT tool_name, decision FROM tool_decisions WHERE session_id = 'sess-tr'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tool, "Edit");
    assert_eq!(decision, "reject");
}

// ---------------------------------------------------------------------------
// code_edit / file_edit: inserts file_changes rows
// ---------------------------------------------------------------------------

#[test]
fn code_edit_event_inserts_file_changes_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-ce")],
        "code_edit",
        vec![
            common::kv("file.path", "src/main.rs"),
            common::kv_int("lines_added", 15),
            common::kv_int("lines_removed", 3),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (file_path, added, removed): (String, i64, i64) = conn
        .query_row(
            "SELECT file_path, lines_added, lines_removed \
             FROM file_changes WHERE session_id = 'sess-ce'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(file_path, "src/main.rs");
    assert_eq!(added, 15);
    assert_eq!(removed, 3);
}

#[test]
fn file_edit_event_inserts_file_changes_row() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-fe")],
        "file_edit",
        vec![
            common::kv("file_path", "web/app.ts"),
            common::kv_int("lines_added", 5),
            common::kv_int("lines_removed", 0),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (file_path, added): (String, i64) = conn
        .query_row(
            "SELECT file_path, lines_added \
             FROM file_changes WHERE session_id = 'sess-fe'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(file_path, "web/app.ts");
    assert_eq!(added, 5);
}

// ---------------------------------------------------------------------------
// commit / git_commit: inserts git_activity rows
// ---------------------------------------------------------------------------

#[test]
fn commit_event_inserts_git_activity_commit() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-commit-log")],
        "commit",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let activity: String = conn
        .query_row(
            "SELECT activity FROM git_activity WHERE session_id = 'sess-commit-log'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activity, "commit");
}

#[test]
fn git_commit_event_inserts_git_activity_commit() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-git-commit")],
        "git_commit",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let activity: String = conn
        .query_row(
            "SELECT activity FROM git_activity WHERE session_id = 'sess-git-commit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activity, "commit");
}

// ---------------------------------------------------------------------------
// pull_request / pr_created: inserts git_activity rows
// ---------------------------------------------------------------------------

#[test]
fn pull_request_event_inserts_git_activity_pull_request() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-pr-log")],
        "pull_request",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let activity: String = conn
        .query_row(
            "SELECT activity FROM git_activity WHERE session_id = 'sess-pr-log'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activity, "pull_request");
}

#[test]
fn pr_created_event_inserts_git_activity_pull_request() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-pr-created")],
        "pr_created",
        vec![],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let activity: String = conn
        .query_row(
            "SELECT activity FROM git_activity WHERE session_id = 'sess-pr-created'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activity, "pull_request");
}

// ---------------------------------------------------------------------------
// Unknown event: lands in both log_events and metrics_raw
// ---------------------------------------------------------------------------

/// An event name the handler doesn't recognize must still produce a row in
/// `log_events` (the always-persist path) and a row in `metrics_raw` (the
/// default branch fallback) without returning an error.
#[test]
fn unknown_event_name_lands_in_log_events_and_metrics_raw() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![common::kv("session.id", "sess-unknown")],
        "some_future_event_not_yet_handled",
        vec![common::kv("custom_key", "custom_value")],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest should not error for unknown events");

    let conn = pool.get().unwrap();

    // Must appear in log_events.
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM log_events \
             WHERE session_id = 'sess-unknown' \
             AND event_name = 'some_future_event_not_yet_handled'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 1, "unknown event should produce one log_events row");

    // Must also appear in metrics_raw via the default branch.
    let raw_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM metrics_raw \
             WHERE session_id = 'sess-unknown' \
             AND metric_name = 'event:some_future_event_not_yet_handled'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw_count, 1, "unknown event should produce one metrics_raw row");
}

// ---------------------------------------------------------------------------
// Paused control: zero rows written when ingestion is paused
// ---------------------------------------------------------------------------

#[test]
fn paused_control_drops_log_payload_and_writes_zero_rows() {
    let (pool, _g) = common::fixture_pool();
    let (ingestor, control) = common::test_ingestor_with_control(&pool);

    control.set_paused(true);

    // Send several different event types — none should land.
    let mut payloads: Vec<opentelemetry_proto::tonic::logs::v1::ResourceLogs> = Vec::new();
    payloads.extend(common::sample_export_logs(
        vec![common::kv("session.id", "sess-paused")],
        "api_request",
        vec![
            common::kv("model", "claude-sonnet"),
            common::kv_f64("cost_usd", 1.0),
        ],
    ));
    payloads.extend(common::sample_export_logs(
        vec![common::kv("session.id", "sess-paused")],
        "user_prompt",
        vec![common::kv_int("prompt_length", 100)],
    ));
    payloads.extend(common::sample_export_logs(
        vec![common::kv("session.id", "sess-paused")],
        "commit",
        vec![],
    ));

    ingestor.ingest_logs_v2(payloads, "grpc").expect("ingest should return Ok even when paused");

    let conn = pool.get().unwrap();

    let log_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM log_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(log_count, 0, "log_events must be empty when ingestion is paused");

    let cost_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_count, 0, "cost_entries must be empty when ingestion is paused");

    let active_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM active_time", [], |r| r.get(0))
        .unwrap();
    assert_eq!(active_count, 0, "active_time must be empty when ingestion is paused");

    let git_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_activity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(git_count, 0, "git_activity must be empty when ingestion is paused");
}

// ---------------------------------------------------------------------------
// Session upsert: resource-level session.id creates a sessions row
// ---------------------------------------------------------------------------

#[test]
fn log_record_upserts_sessions_row_from_resource_attrs() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_export_logs(
        vec![
            common::kv("session.id", "sess-upsert"),
            common::kv("service.version", "2.1.0"),
            common::kv("host.arch", "x86_64"),
        ],
        "user_prompt",
        vec![common::kv_int("prompt_length", 10)],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let (version, arch): (String, String) = conn
        .query_row(
            "SELECT service_version, host_arch \
             FROM sessions WHERE session_id = 'sess-upsert'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(version, "2.1.0");
    assert_eq!(arch, "x86_64");
}

// ---------------------------------------------------------------------------
// Session fallback: record-level session.id is used when resource has none
// ---------------------------------------------------------------------------

#[test]
fn log_record_falls_back_to_record_level_session_id() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // No session.id on the resource — only on the record attributes.
    let payload = common::sample_export_logs(
        vec![],                                         // no resource attrs
        "commit",
        vec![common::kv("session.id", "sess-record-fallback")],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest");

    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id = 'sess-record-fallback'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "session row should be upserted from record-level session.id");
}

// ---------------------------------------------------------------------------
// No session.id: typed handlers are skipped but log_events row is still written
// ---------------------------------------------------------------------------

#[test]
fn missing_session_id_skips_typed_handler_but_writes_log_event() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // A costly api_request with no session.id at all.
    let payload = common::sample_export_logs(
        vec![],
        "api_request",
        vec![
            common::kv("model", "claude-opus"),
            common::kv_f64("cost_usd", 5.0),
        ],
    );
    ingestor.ingest_logs_v2(payload, "grpc").expect("ingest should succeed with no session.id");

    let conn = pool.get().unwrap();

    // The raw log_events row has NULL session_id — count it without filtering.
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM log_events WHERE event_name = 'api_request'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 1, "log_events row should be written even without session.id");

    // No cost entry should exist (typed handler bails out without session_id).
    let cost_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_count, 0, "no cost row should be written without a session.id");
}
