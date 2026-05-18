use andon_lib::integration::IntegrationStatus;

/// IntegrationStatus::AlreadyConfigured serializes with correct serde shape
#[test]
fn already_configured_serializes_with_serde_tag() {
    let status = IntegrationStatus::AlreadyConfigured {
        settings_path: "/home/user/.claude/settings.json".into(),
    };
    let json = serde_json::to_value(&status).expect("serialize");
    assert_eq!(json["state"], "already_configured");
    assert_eq!(json["settings_path"], "/home/user/.claude/settings.json");
}

/// IntegrationStatus::Patched serializes with serde(tag, rename_all) shape
#[test]
fn patched_serializes_with_backup_path() {
    let status = IntegrationStatus::Patched {
        settings_path: "/home/user/.claude/settings.json".into(),
        backup_path: "/home/user/.claude/settings.json.andon-backup".into(),
    };
    let json = serde_json::to_value(&status).expect("serialize");
    assert_eq!(json["state"], "patched");
    assert_eq!(json["settings_path"], "/home/user/.claude/settings.json");
    assert_eq!(json["backup_path"], "/home/user/.claude/settings.json.andon-backup");
}

/// IntegrationStatus::Conflict serializes with existing_endpoint
#[test]
fn conflict_serializes_with_existing_endpoint() {
    let status = IntegrationStatus::Conflict {
        settings_path: "/home/user/.claude/settings.json".into(),
        existing_endpoint: "http://cloud.example.com:4317".into(),
    };
    let json = serde_json::to_value(&status).expect("serialize");
    assert_eq!(json["state"], "conflict");
    assert_eq!(json["settings_path"], "/home/user/.claude/settings.json");
    assert_eq!(json["existing_endpoint"], "http://cloud.example.com:4317");
}

/// IntegrationStatus::Error serializes with message
#[test]
fn error_serializes_with_message() {
    let status = IntegrationStatus::Error {
        message: "permission denied".into(),
    };
    let json = serde_json::to_value(&status).expect("serialize");
    assert_eq!(json["state"], "error");
    assert_eq!(json["message"], "permission denied");
}

/// Serialization produces correct JSON schema for complex payload
#[test]
fn serialized_json_has_correct_structure() {
    let status = IntegrationStatus::Patched {
        settings_path: "/home/user/.claude/settings.json".into(),
        backup_path: "/home/user/.claude/settings.json.andon-backup".into(),
    };
    let serialized = serde_json::to_string(&status).expect("serialize");
    let parsed: serde_json::Value =
        serde_json::from_str(&serialized).expect("valid JSON");

    // Verify the JSON has the expected structure
    assert!(parsed.is_object());
    assert_eq!(parsed["state"], "patched");
    assert!(parsed["settings_path"].is_string());
    assert!(parsed["backup_path"].is_string());
}

/// All variants serialize to distinct state field values
#[test]
fn all_variants_serialize_to_distinct_states() {
    let already_configured = serde_json::to_value(&IntegrationStatus::AlreadyConfigured {
        settings_path: "p1".into(),
    })
    .unwrap();
    let patched = serde_json::to_value(&IntegrationStatus::Patched {
        settings_path: "p2".into(),
        backup_path: "p3".into(),
    })
    .unwrap();
    let conflict = serde_json::to_value(&IntegrationStatus::Conflict {
        settings_path: "p4".into(),
        existing_endpoint: "e1".into(),
    })
    .unwrap();
    let error = serde_json::to_value(&IntegrationStatus::Error {
        message: "m1".into(),
    })
    .unwrap();

    // Collect the state values
    let state1 = already_configured["state"].as_str().unwrap();
    let state2 = patched["state"].as_str().unwrap();
    let state3 = conflict["state"].as_str().unwrap();
    let state4 = error["state"].as_str().unwrap();

    // All should be distinct
    assert_ne!(state1, state2);
    assert_ne!(state1, state3);
    assert_ne!(state1, state4);
    assert_ne!(state2, state3);
    assert_ne!(state2, state4);
    assert_ne!(state3, state4);

    // And match expected names from serde(rename_all = "snake_case")
    assert_eq!(state1, "already_configured");
    assert_eq!(state2, "patched");
    assert_eq!(state3, "conflict");
    assert_eq!(state4, "error");
}
