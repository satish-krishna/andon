use andon_lib::diagnostics::Diagnostics;

/// Diagnostics::default() creates a well-shaped snapshot
#[test]
fn default_diagnostics_produces_valid_snapshot() {
    let diag = Diagnostics::default();
    let snapshot = diag.snapshot();

    assert!(snapshot.start_ms > 0, "start_ms should be set");
    assert!(snapshot.uptime_seconds >= 0, "uptime_seconds should be >= 0");
    assert_eq!(snapshot.total_records, 0, "should start with zero records");
    assert_eq!(snapshot.total_payloads, 0, "should start with zero payloads");
    assert!(snapshot.event_counters.is_empty(), "event_counters should start empty");
    assert!(snapshot.transport_counters.is_empty(), "transport_counters should start empty");
    assert!(snapshot.last_event_ms.is_none(), "last_event_ms should be None");
    assert!(snapshot.last_session_id.is_none(), "last_session_id should be None");
    assert!(snapshot.last_payload.is_none(), "last_payload should be None");
    assert!(snapshot.last_error.is_none(), "last_error should be None");
}

/// Recording a payload via record_payload increments counters
#[test]
fn record_payload_increments_counters() {
    let diag = Diagnostics::default();

    diag.record_payload(
        "grpc",
        1,                                   // resource_logs
        2,                                   // records
        vec!["event_1".into(), "event_2".into()], // event_names
        Some("sess-1".into()),
    );

    let snapshot = diag.snapshot();
    assert_eq!(snapshot.total_records, 2, "total_records should be 2");
    assert_eq!(snapshot.total_payloads, 1, "total_payloads should be 1");
    assert!(snapshot.last_event_ms.is_some(), "last_event_ms should be set");
    assert_eq!(snapshot.last_session_id, Some("sess-1".into()));
    assert!(snapshot.last_payload.is_some(), "last_payload should be recorded");

    let payload = snapshot.last_payload.unwrap();
    assert_eq!(payload.transport, "grpc");
    assert_eq!(payload.records, 2);
    assert_eq!(payload.event_names, vec!["event_1", "event_2"]);
}

/// event_counters increments for each unique event name
#[test]
fn event_counters_track_event_frequency() {
    let diag = Diagnostics::default();

    // Record first payload with two events
    diag.record_payload(
        "grpc",
        1,
        1,
        vec!["api_request".into()],
        None,
    );

    // Record second payload with one of the same events and a new one
    diag.record_payload(
        "http",
        1,
        1,
        vec!["api_request".into(), "session_start".into()],
        None,
    );

    let snapshot = diag.snapshot();
    assert_eq!(snapshot.event_counters.get("api_request"), Some(&2));
    assert_eq!(snapshot.event_counters.get("session_start"), Some(&1));
}

/// transport_counters tracks which transports received payloads
#[test]
fn transport_counters_track_transport_frequency() {
    let diag = Diagnostics::default();

    diag.record_payload("grpc", 1, 1, vec!["event_1".into()], None);
    diag.record_payload("grpc", 1, 1, vec!["event_2".into()], None);
    diag.record_payload("http", 1, 1, vec!["event_3".into()], None);

    let snapshot = diag.snapshot();
    assert_eq!(snapshot.transport_counters.get("grpc"), Some(&2));
    assert_eq!(snapshot.transport_counters.get("http"), Some(&1));
}

/// record_bind updates the bind status for a given port
#[test]
fn record_bind_updates_port_status() {
    let diag = Diagnostics::default();

    diag.record_bind("grpc", true, None);
    diag.record_bind("http", true, None);
    diag.record_bind("api", false, Some("connection refused".into()));

    let snapshot = diag.snapshot();
    assert_eq!(snapshot.binds.grpc_4317, Some(true));
    assert_eq!(snapshot.binds.http_4318, Some(true));
    assert_eq!(snapshot.binds.api_8765, Some(false));
    assert!(snapshot.last_error.is_some(), "bind failure should set last_error");
}

/// Unknown port in record_bind is safely ignored
#[test]
fn record_bind_ignores_unknown_port() {
    let diag = Diagnostics::default();

    diag.record_bind("unknown_port", true, None);

    let snapshot = diag.snapshot();
    assert!(snapshot.binds.grpc_4317.is_none());
    assert!(snapshot.binds.http_4318.is_none());
    assert!(snapshot.binds.api_8765.is_none());
}

/// snapshot serializes to JSON without panic
#[test]
fn snapshot_serializes_to_json() {
    let diag = Diagnostics::default();
    diag.record_payload(
        "grpc",
        1,
        1,
        vec!["api_request".into()],
        Some("sess-1".into()),
    );
    diag.record_bind("grpc", true, None);

    let snapshot = diag.snapshot();
    let json = serde_json::to_value(&snapshot).expect("snapshot should serialize to JSON");

    assert!(json["start_ms"].is_number());
    assert!(json["uptime_seconds"].is_number());
    assert!(json["total_records"].is_number());
    assert!(json["total_payloads"].is_number());
    assert!(json["event_counters"].is_object());
    assert!(json["transport_counters"].is_object());
    assert!(json["binds"].is_object());
    assert!(json["health"].is_object());
    assert!(json["last_payload"].is_object());
}

/// Multiple payloads accumulate correctly
#[test]
fn multiple_payloads_accumulate() {
    let diag = Diagnostics::default();

    for i in 1..=3 {
        diag.record_payload(
            if i % 2 == 0 { "http" } else { "grpc" },
            i,
            i * 10,
            vec![format!("event_{i}")],
            Some(format!("sess-{i}")),
        );
    }

    let snapshot = diag.snapshot();
    assert_eq!(snapshot.total_payloads, 3);
    // total_records = 10 + 20 + 30 = 60
    assert_eq!(snapshot.total_records, 60);
    assert_eq!(snapshot.last_session_id, Some("sess-3".into()));
}

/// seconds_since_last_event is computed correctly
#[test]
fn seconds_since_last_event_is_computed() {
    let diag = Diagnostics::default();

    diag.record_payload("grpc", 1, 1, vec!["event".into()], None);
    let snapshot1 = diag.snapshot();

    // Just after recording, seconds_since_last_event should be small (0 or 1)
    assert!(
        snapshot1.seconds_since_last_event.unwrap_or(0) <= 1,
        "should be very recent"
    );
}

/// Health verdict respects bind failures
#[test]
fn health_verdict_detects_bind_failure() {
    let diag = Diagnostics::default();

    diag.record_bind("grpc", false, Some("address already in use".into()));

    let snapshot = diag.snapshot();
    let health_json = serde_json::to_value(&snapshot.health).unwrap();
    assert_eq!(health_json["state"], "bind_failed", "health should be BindFailed");
    assert!(health_json["ports"].is_array());
}

/// Health verdict shows Healthy when records exist
#[test]
fn health_verdict_healthy_when_records_exist() {
    let diag = Diagnostics::default();

    diag.record_bind("grpc", true, None);
    diag.record_bind("http", true, None);
    diag.record_bind("api", true, None);
    diag.record_payload("grpc", 1, 1, vec!["event".into()], None);

    let snapshot = diag.snapshot();
    let health_json = serde_json::to_value(&snapshot.health).unwrap();
    assert_eq!(health_json["state"], "healthy");
}

/// Health verdict shows Starting within grace period
#[test]
fn health_verdict_starting_within_grace_period() {
    let diag = Diagnostics::default();

    // Within 5 seconds of creation and no data: should be Starting
    let snapshot = diag.snapshot();
    let health_json = serde_json::to_value(&snapshot.health).unwrap();
    assert_eq!(health_json["state"], "starting");
}
