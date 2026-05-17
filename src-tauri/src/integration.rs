use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value, json};

const REQUIRED_ENV: &[(&str, &str)] = &[
    ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
    ("OTEL_METRICS_EXPORTER", "otlp"),
    ("OTEL_LOGS_EXPORTER", "otlp"),
    ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
    ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4317"),
];

const ENDPOINT_KEY: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OUR_ENDPOINT: &str = "http://localhost:4317";

const HOOK_MATCHER: &str = "Write|Edit|MultiEdit";
const HOOK_COMMAND: &str =
    "curl -s -X POST http://127.0.0.1:8765/api/hooks/tool-use -H \"Content-Type: application/json\" --data-binary @-";
const HOOK_MARKER: &str = "/api/hooks/tool-use"; // used to detect our hook in existing config

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IntegrationStatus {
    AlreadyConfigured { settings_path: String },
    Patched { settings_path: String, backup_path: String },
    Conflict { settings_path: String, existing_endpoint: String },
    Error { message: String },
}

pub fn ensure_claude_settings() -> IntegrationStatus {
    match try_ensure() {
        Ok(s) => s,
        Err(e) => IntegrationStatus::Error {
            message: format!("{e:#}"),
        },
    }
}

fn try_ensure() -> Result<IntegrationStatus> {
    let path = settings_path()?;
    let display_path = path.display().to_string();

    let (existing, file_existed) = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let parsed: Value = if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
        };
        (parsed, true)
    } else {
        (Value::Object(Map::new()), false)
    };

    let env_obj = existing
        .get("env")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    // Conflict: a different endpoint is already configured.
    if let Some(Value::String(existing_endpoint)) = env_obj.get(ENDPOINT_KEY) {
        if existing_endpoint != OUR_ENDPOINT {
            tracing::warn!(
                existing = %existing_endpoint,
                "claude settings.json already targets a different OTLP endpoint — leaving it alone"
            );
            return Ok(IntegrationStatus::Conflict {
                settings_path: display_path,
                existing_endpoint: existing_endpoint.clone(),
            });
        }
    }

    // Idempotent: env + hook already present.
    let env_set = REQUIRED_ENV.iter().all(|(k, v)| {
        env_obj
            .get(*k)
            .and_then(|x| x.as_str())
            .map(|s| s == *v)
            .unwrap_or(false)
    });
    let hook_installed = has_our_hook(&existing);
    if env_set && hook_installed && file_existed {
        return Ok(IntegrationStatus::AlreadyConfigured {
            settings_path: display_path,
        });
    }

    // Patch: backup, then merge.
    let backup_path = if file_existed {
        let bp = path.with_extension("json.andon-backup");
        std::fs::copy(&path, &bp)
            .with_context(|| format!("backup {} -> {}", path.display(), bp.display()))?;
        bp.display().to_string()
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        String::from("(no backup — file did not previously exist)")
    };

    let mut merged = match existing {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let mut env_merged = merged
        .get("env")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (k, v) in REQUIRED_ENV {
        env_merged.insert((*k).to_string(), json!(*v));
    }
    merged.insert("env".to_string(), Value::Object(env_merged));

    // Install PostToolUse hook (idempotent — won't duplicate if already present).
    let mut merged_val = Value::Object(merged);
    install_hook(&mut merged_val);

    let serialized = serde_json::to_string_pretty(&merged_val)?;
    std::fs::write(&path, serialized).with_context(|| format!("write {}", path.display()))?;

    tracing::info!(path = %path.display(), "patched claude code settings.json with OTel env vars + hook");
    Ok(IntegrationStatus::Patched {
        settings_path: display_path,
        backup_path,
    })
}

fn settings_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("resolve home directory")?;
    Ok(home.join(".claude").join("settings.json"))
}

/// True if the settings.json already has a PostToolUse hook whose command
/// references our /api/hooks/tool-use endpoint.
fn has_our_hook(value: &Value) -> bool {
    let Some(post_tool) = value
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array())
    else {
        return false;
    };
    for entry in post_tool {
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            for hook in hooks {
                if let Some(cmd) = hook.get("command").and_then(|c| c.as_str()) {
                    if cmd.contains(HOOK_MARKER) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Insert our PostToolUse hook into `value` (mutates in place).
/// Won't duplicate if already present. Won't touch other hook entries.
fn install_hook(value: &mut Value) {
    if has_our_hook(value) {
        return;
    }
    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    let hooks = obj.entry("hooks".to_string()).or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let post_tool = hooks_obj
        .entry("PostToolUse".to_string())
        .or_insert_with(|| json!([]));
    if !post_tool.is_array() {
        *post_tool = json!([]);
    }
    let arr = post_tool.as_array_mut().unwrap();
    arr.push(json!({
        "matcher": HOOK_MATCHER,
        "hooks": [{
            "type": "command",
            "command": HOOK_COMMAND
        }]
    }));
}

/// Remove our hook from settings.json (returns true if removed anything).
fn remove_hook(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else { return false };
    let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) else { return false };
    let Some(post_tool) = hooks.get_mut("PostToolUse").and_then(|p| p.as_array_mut()) else { return false };

    let before = post_tool.len();
    post_tool.retain(|entry| {
        let has_ours = entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter().any(|hook| {
                    hook.get("command")
                        .and_then(|c| c.as_str())
                        .map(|s| s.contains(HOOK_MARKER))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        !has_ours
    });
    let removed = post_tool.len() < before;

    // Clean up empty containers
    if post_tool.is_empty() {
        hooks.remove("PostToolUse");
    }
    if hooks.is_empty() {
        obj.remove("hooks");
    }
    removed
}

/// Remove andon's OTel env vars from claude settings.json, leaving everything
/// else intact. Backup the current file first.
pub fn unpatch_claude_settings() -> Result<String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok("nothing to unpatch — settings.json does not exist".into());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut value: Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;

    let mut removed_any = false;
    if let Some(Value::Object(env)) = value.get_mut("env") {
        for (k, _) in REQUIRED_ENV {
            if env.remove(*k).is_some() {
                removed_any = true;
            }
        }
        if env.is_empty() {
            if let Value::Object(top) = &mut value {
                top.remove("env");
            }
        }
    }
    if remove_hook(&mut value) {
        removed_any = true;
    }

    if !removed_any {
        return Ok("nothing to unpatch — andon vars not present".into());
    }

    let bp = path.with_extension("json.andon-unpatch-backup");
    std::fs::copy(&path, &bp)?;
    let serialized = serde_json::to_string_pretty(&value)?;
    std::fs::write(&path, serialized)?;
    tracing::info!(path = %path.display(), "unpatched claude settings.json");
    Ok(format!(
        "unpatched · backup at {}",
        bp.display()
    ))
}

/// Restore the andon-backup created by the original patch.
pub fn restore_backup() -> Result<String> {
    let path = settings_path()?;
    let bp = path.with_extension("json.andon-backup");
    if !bp.exists() {
        return Err(anyhow::anyhow!(
            "no backup found at {}",
            bp.display()
        ));
    }
    std::fs::copy(&bp, &path)?;
    tracing::info!(restored_from = %bp.display(), "restored claude settings.json from backup");
    Ok(format!("restored from {}", bp.display()))
}
