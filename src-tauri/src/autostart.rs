//! Windows logon autostart via HKCU\Software\Microsoft\Windows\CurrentVersion\Run.
//! Per-user, no admin required.

use anyhow::{Context, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "andon";

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::*;

    fn current_exe() -> Result<String> {
        let exe = std::env::current_exe().context("resolving current exe")?;
        // Quote the path so spaces (e.g. "Program Files") aren't a problem.
        Ok(format!("\"{}\"", exe.display()))
    }

    pub fn is_enabled() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey(RUN_KEY) {
            Ok(key) => key.get_value::<String, _>(VALUE_NAME).is_ok(),
            Err(_) => false,
        }
    }

    pub fn registered_command() -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(RUN_KEY)
            .ok()?
            .get_value::<String, _>(VALUE_NAME)
            .ok()
    }

    pub fn enable() -> Result<String> {
        let exe = current_exe()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(RUN_KEY)
            .context("open HKCU Run key")?;
        key.set_value(VALUE_NAME, &exe)
            .context("set autostart value")?;
        tracing::info!(cmd = %exe, "autostart enabled");
        Ok(exe)
    }

    pub fn disable() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            let _ = key.delete_value(VALUE_NAME);
        }
        tracing::info!("autostart disabled");
        Ok(())
    }

    /// Make sure autostart is enabled AND points at the current exe.
    /// If user moved/reinstalled the app, this updates the registered path.
    pub fn ensure_current() -> Result<EnsureOutcome> {
        let want = current_exe()?;
        match registered_command() {
            Some(existing) if existing == want => Ok(EnsureOutcome::AlreadyCorrect),
            Some(_) => {
                enable()?;
                Ok(EnsureOutcome::Updated)
            }
            None => {
                enable()?;
                Ok(EnsureOutcome::Enabled)
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn is_enabled() -> bool { false }
    pub fn registered_command() -> Option<String> { None }
    pub fn enable() -> Result<String> { Ok("(no-op: not windows)".into()) }
    pub fn disable() -> Result<()> { Ok(()) }
    pub fn ensure_current() -> Result<EnsureOutcome> { Ok(EnsureOutcome::Unsupported) }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EnsureOutcome {
    Enabled,
    Updated,
    AlreadyCorrect,
    Unsupported,
}

pub use imp::*;
