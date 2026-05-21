//! Integration tests for settings persistence, defaults, and corrupt-file
//! recovery.  `autostart` and `config` are private modules and are therefore
//! covered by `#[cfg(test)]` unit-test blocks added directly to those source
//! files; see `src/autostart.rs` and `src/config.rs`.

use andon_lib::settings::{AppSettings, BudgetSettings, ForwarderSettings, SettingsStore};
use std::collections::BTreeMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tmp() -> TempDir {
    tempfile::tempdir().expect("create tempdir")
}

fn settings_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("settings.json")
}

// ---------------------------------------------------------------------------
// 1. Round-trip: write values, reload, values match
// ---------------------------------------------------------------------------

#[test]
fn settings_round_trip() {
    let dir = tmp();
    let path = settings_path(&dir);

    let store = SettingsStore::load(path.clone()).expect("initial load");

    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_string(), "Bearer tok".to_string());
    headers.insert("x-tenant".to_string(), "acme".to_string());

    let new_fwd = ForwarderSettings {
        enabled: true,
        endpoint: "https://otel.example.com:4318".into(),
        timeout_ms: 3500,
        headers,
    };

    store
        .save_forwarder(new_fwd.clone())
        .expect("save_forwarder");

    // Reload from disk — a fresh SettingsStore must read back identical values.
    let reloaded = SettingsStore::load(path).expect("reload");
    let got = reloaded.snapshot();

    assert_eq!(got.forwarder.enabled, true, "enabled round-trip");
    assert_eq!(
        got.forwarder.endpoint, "https://otel.example.com:4318",
        "endpoint round-trip"
    );
    assert_eq!(got.forwarder.timeout_ms, 3500, "timeout_ms round-trip");
    assert_eq!(
        got.forwarder.headers.get("Authorization").map(String::as_str),
        Some("Bearer tok"),
        "Authorization header round-trip"
    );
    assert_eq!(
        got.forwarder.headers.get("x-tenant").map(String::as_str),
        Some("acme"),
        "x-tenant header round-trip"
    );
}

// ---------------------------------------------------------------------------
// 2. Missing file → returns defaults, file is created on disk
// ---------------------------------------------------------------------------

#[test]
fn settings_missing_file_returns_defaults() {
    let dir = tmp();
    let path = settings_path(&dir);

    // The file must NOT exist before the call.
    assert!(!path.exists(), "precondition: settings file must not exist yet");

    let store = SettingsStore::load(path.clone()).expect("load from missing path");
    let snapshot = store.snapshot();

    assert_eq!(snapshot, AppSettings::default(), "snapshot must equal defaults");
    assert!(
        path.exists(),
        "SettingsStore must create the file when it is missing"
    );
}

// ---------------------------------------------------------------------------
// 3a. Corrupt file (valid UTF-8, invalid JSON) → fallback + backup created
// ---------------------------------------------------------------------------

/// When the settings file contains valid UTF-8 that cannot be parsed as JSON,
/// the loader must fall back to defaults AND create a timestamped backup.
#[test]
fn settings_corrupt_json_falls_back_to_defaults_with_backup() {
    let dir = tmp();
    let path = settings_path(&dir);

    // Valid UTF-8, but not valid JSON.
    std::fs::write(&path, b"{ this is definitely not valid json !!! }").expect("write corrupt file");

    // Must not panic.
    let store =
        SettingsStore::load(path.clone()).expect("load must succeed even with corrupt file");

    assert_eq!(
        store.snapshot(),
        AppSettings::default(),
        "corrupt JSON must fall back to defaults"
    );

    // A backup file with "corrupt-" in the name should have been created.
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
        .collect();

    assert_eq!(backups.len(), 1, "exactly one backup file must be created");
}

// ---------------------------------------------------------------------------
// 3b. Unreadable file (non-UTF-8 bytes) → silent fallback to defaults
// ---------------------------------------------------------------------------

/// When `read_to_string` fails (e.g., non-UTF-8 bytes), the loader falls back
/// to defaults silently without creating a backup (the backup path requires a
/// successful read).  The call must not panic.
#[test]
fn settings_non_utf8_file_falls_back_to_defaults_silently() {
    let dir = tmp();
    let path = settings_path(&dir);

    // Write bytes that are not valid UTF-8.
    std::fs::write(&path, b"\xff\xfe invalid utf8 bytes").expect("write non-UTF-8 file");

    let store =
        SettingsStore::load(path.clone()).expect("load must succeed even with non-UTF-8 file");

    assert_eq!(
        store.snapshot(),
        AppSettings::default(),
        "non-UTF-8 file must fall back to defaults"
    );

    // No backup should be created in this path (the raw read itself failed).
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
        .collect();

    assert_eq!(
        backups.len(),
        0,
        "no backup should be created when the file cannot be read as UTF-8"
    );
}

// ---------------------------------------------------------------------------
// 4. save_forwarder snapshot is immediately visible without reload
// ---------------------------------------------------------------------------

#[test]
fn settings_in_memory_snapshot_reflects_save() {
    let dir = tmp();
    let path = settings_path(&dir);
    let store = SettingsStore::load(path).expect("load");

    let new_fwd = ForwarderSettings {
        enabled: true,
        endpoint: "http://localhost:4318".into(),
        timeout_ms: 1000,
        headers: BTreeMap::new(),
    };

    store.save_forwarder(new_fwd.clone()).expect("save");

    // snapshot() and forwarder() must reflect the update without a reload.
    let fwd = store.forwarder();
    assert_eq!(fwd.enabled, true);
    assert_eq!(fwd.endpoint, "http://localhost:4318");
    assert_eq!(fwd.timeout_ms, 1000);

    let snap = store.snapshot();
    assert_eq!(snap.forwarder, new_fwd);
}

// ---------------------------------------------------------------------------
// 5. AppSettings default values are as documented
// ---------------------------------------------------------------------------

#[test]
fn settings_default_values() {
    let defaults = AppSettings::default();
    assert_eq!(defaults.version, 1);
    assert!(!defaults.forwarder.enabled, "forwarder disabled by default");
    assert!(defaults.forwarder.endpoint.is_empty(), "endpoint empty by default");
    assert_eq!(defaults.forwarder.timeout_ms, 2000, "default timeout 2000 ms");
    assert!(defaults.forwarder.headers.is_empty(), "no headers by default");
}

// ---------------------------------------------------------------------------
// 6. Budget round-trip: save a budget, reload, value matches
// ---------------------------------------------------------------------------

#[test]
fn budget_round_trip() {
    let dir = tmp();
    let path = settings_path(&dir);
    let store = SettingsStore::load(path.clone()).expect("initial load");

    store
        .save_budget(BudgetSettings { monthly_usd: 250.0 })
        .expect("save_budget");

    let reloaded = SettingsStore::load(path).expect("reload");
    assert_eq!(reloaded.budget().monthly_usd, 250.0, "budget round-trip");
}

// ---------------------------------------------------------------------------
// 7. A legacy settings.json with no `budget` key loads cleanly (no backup)
// ---------------------------------------------------------------------------

/// CRITICAL REGRESSION GUARD. Existing installs have a settings.json written
/// before the `budget` field existed. Without `#[serde(default)]` on
/// `AppSettings.budget`, parsing fails, the loader treats the file as corrupt,
/// and overwrites it with defaults — wiping the user's forwarder config.
#[test]
fn settings_without_budget_key_loads_without_backup() {
    let dir = tmp();
    let path = settings_path(&dir);

    // An old settings.json: version + forwarder only, no `budget` key.
    std::fs::write(
        &path,
        r#"{
  "version": 1,
  "forwarder": {
    "enabled": true,
    "endpoint": "http://otel.example.com:4318",
    "timeout_ms": 3000,
    "headers": {}
  }
}"#,
    )
    .expect("write legacy settings file");

    let store = SettingsStore::load(path).expect("legacy file must load");

    // The missing `budget` key falls back to the default — not a corrupt wipe.
    assert_eq!(store.budget(), BudgetSettings::default(), "budget defaults");
    // The forwarder config survived — proof the file was parsed, not discarded.
    assert_eq!(
        store.forwarder().endpoint,
        "http://otel.example.com:4318",
        "forwarder config must survive a budget-less file",
    );
    // No `corrupt-*` backup must exist.
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
        .collect();
    assert_eq!(backups.len(), 0, "a budget-less file is valid, not corrupt");
}
