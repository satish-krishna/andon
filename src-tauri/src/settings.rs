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
    #[serde(default)]
    pub coach: CoachSettings,
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

/// Coach feature settings — vocabulary lists and Skill Finder thresholds.
///
/// `planning_keywords` and `constraint_keywords` deliberately share words
/// like `must` / `should` / `ensure`. The two lists feed *different* rules
/// operating on *different* signals: `planning_keywords` is matched against
/// a session's first user turn only (powers `low-spec-rate`), while
/// `constraint_keywords` is matched against every turn at ingest time to
/// set the `prompt_turns.has_constraint` flag (powers `low-constraint-usage`).
/// The overlap is intentional — a turn like "this must be idempotent" is
/// legitimately *both* a constrained turn and a spec-driven opener.
/// Upstream AIEC's `no-spec-driven-development` rule uses the same
/// modal-verb vocabulary for the same reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoachSettings {
    pub skill_min_occurrences: u32,
    pub skill_min_sessions: u32,
    pub planning_commands: Vec<String>,
    pub planning_keywords: Vec<String>,
    pub constraint_keywords: Vec<String>,
}

impl Default for CoachSettings {
    fn default() -> Self {
        Self {
            skill_min_occurrences: 3,
            skill_min_sessions: 2,
            planning_commands: vec![
                "plan".into(), "brainstorm".into(), "design".into(),
                "spec".into(), "specify".into(), "rfc".into(),
            ],
            planning_keywords: vec![
                "spec".into(), "specs".into(), "requirement".into(),
                "requirements".into(), "acceptance criteria".into(),
                "design doc".into(), "PRD".into(), "RFC".into(),
                "plan file".into(), "constraint".into(), "must".into(),
                "should".into(), "ensure".into(),
            ],
            constraint_keywords: vec![
                "must".into(), "should".into(), "limit".into(), "ensure".into(),
                "require".into(), "only".into(), "without".into(),
                "never".into(), "always".into(),
            ],
        }
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
            coach: CoachSettings::default(),
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

    pub fn coach(&self) -> CoachSettings {
        self.inner.read().expect("settings lock").coach.clone()
    }

    pub fn save_coach(&self, new: CoachSettings) -> Result<CoachSettings> {
        let mut w = self.inner.write().expect("settings lock");
        w.coach = new.clone();
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

    #[test]
    fn coach_defaults_are_seeded() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p).unwrap();
        let coach = store.coach();
        assert_eq!(coach.skill_min_occurrences, 3);
        assert_eq!(coach.skill_min_sessions, 2);
        assert!(coach.planning_commands.contains(&"plan".to_string()));
        assert!(coach.planning_commands.contains(&"brainstorm".to_string()));
        assert!(coach.constraint_keywords.contains(&"must".to_string()));
    }

    #[test]
    fn save_coach_persists() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p.clone()).unwrap();
        let mut new_coach = store.coach();
        new_coach.skill_min_occurrences = 5;
        new_coach.planning_commands.push("rfc".into());
        store.save_coach(new_coach.clone()).unwrap();
        let reloaded = SettingsStore::load(p).unwrap();
        assert_eq!(reloaded.coach(), new_coach);
    }

    #[test]
    fn settings_file_without_coach_key_still_parses() {
        // Pre-existing installs have no `coach` field — must not break.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, r#"{"version":1,"forwarder":{"enabled":false,"endpoint":"","timeout_ms":2000,"headers":{}},"budget":{"monthly_usd":0.0}}"#).unwrap();
        let store = SettingsStore::load(p).unwrap();
        assert_eq!(store.coach(), CoachSettings::default());
    }
}
