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

    // Idempotent: every required var already present with the expected value.
    let all_set = REQUIRED_ENV.iter().all(|(k, v)| {
        env_obj
            .get(*k)
            .and_then(|x| x.as_str())
            .map(|s| s == *v)
            .unwrap_or(false)
    });
    if all_set && file_existed {
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

    let serialized = serde_json::to_string_pretty(&Value::Object(merged))?;
    std::fs::write(&path, serialized).with_context(|| format!("write {}", path.display()))?;

    tracing::info!(path = %path.display(), "patched claude code settings.json with OTel env vars");
    Ok(IntegrationStatus::Patched {
        settings_path: display_path,
        backup_path,
    })
}

fn settings_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("resolve home directory")?;
    Ok(home.join(".claude").join("settings.json"))
}
