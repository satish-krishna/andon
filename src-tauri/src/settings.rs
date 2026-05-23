use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub version: u32,
    pub forwarder: ForwarderSettings,
    /// `#[serde(default)]` lets settings.json files written before this field
    /// existed still parse — without it, every existing install is treated as
    /// corrupt and overwritten. See the regression test in settings_roundtrip.rs.
    #[serde(default)]
    pub budget: BudgetSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwarderSettings {
    pub enabled: bool,
    pub endpoint: String,
    pub timeout_ms: u64,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetSettings {
    /// Monthly cost budget in USD. `0.0` (the default) disables alerts.
    pub monthly_usd: f64,
}

impl Default for BudgetSettings {
    fn default() -> Self {
        Self { monthly_usd: 0.0 }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            forwarder: ForwarderSettings {
                enabled: false,
                endpoint: String::new(),
                timeout_ms: 2000,
                headers: Default::default(),
            },
            budget: BudgetSettings::default(),
        }
    }
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
    inner: Arc<RwLock<AppSettings>>,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let settings = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(raw) => match serde_json::from_str::<AppSettings>(&raw) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = ?e, path = %path.display(),
                            "settings.json unparseable — backing up + writing defaults");
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let bak = path.with_extension(format!("json.corrupt-{ts}"));
                        let _ = std::fs::copy(&path, &bak);
                        let defaults = AppSettings::default();
                        write_atomic(&path, &serde_json::to_string_pretty(&defaults)?)?;
                        defaults
                    }
                },
                Err(e) => {
                    tracing::warn!(error = ?e, "settings.json unreadable; using defaults");
                    AppSettings::default()
                }
            }
        } else {
            let defaults = AppSettings::default();
            write_atomic(&path, &serde_json::to_string_pretty(&defaults)?)?;
            defaults
        };

        Ok(Self {
            path,
            inner: Arc::new(RwLock::new(settings)),
        })
    }

    pub fn snapshot(&self) -> AppSettings {
        self.inner.read().expect("settings lock").clone()
    }

    pub fn forwarder(&self) -> ForwarderSettings {
        self.inner.read().expect("settings lock").forwarder.clone()
    }

    pub fn save_forwarder(&self, new: ForwarderSettings) -> Result<ForwarderSettings> {
        let mut w = self.inner.write().expect("settings lock");
        w.forwarder = new.clone();
        let serialized = serde_json::to_string_pretty(&*w)?;
        write_atomic(&self.path, &serialized)?;
        Ok(new)
    }

    pub fn budget(&self) -> BudgetSettings {
        self.inner.read().expect("settings lock").budget.clone()
    }

    pub fn save_budget(&self, new: BudgetSettings) -> Result<BudgetSettings> {
        let mut w = self.inner.write().expect("settings lock");
        w.budget = new.clone();
        let serialized = serde_json::to_string_pretty(&*w)?;
        write_atomic(&self.path, &serialized)?;
        Ok(new)
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_defaults_when_missing() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p.clone()).unwrap();
        assert_eq!(store.snapshot(), AppSettings::default());
        assert!(p.exists());
    }

    #[test]
    fn save_forwarder_persists() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p.clone()).unwrap();

        let new_fwd = ForwarderSettings {
            enabled: true,
            endpoint: "https://otel.example.com".into(),
            timeout_ms: 1500,
            headers: [("Authorization".to_string(), "Bearer x".to_string())]
                .into_iter()
                .collect(),
        };
        store.save_forwarder(new_fwd.clone()).unwrap();

        let reloaded = SettingsStore::load(p).unwrap();
        assert_eq!(reloaded.forwarder(), new_fwd);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ this is not json").unwrap();
        let store = SettingsStore::load(p.clone()).unwrap();
        assert_eq!(store.snapshot(), AppSettings::default());
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
    }
}
