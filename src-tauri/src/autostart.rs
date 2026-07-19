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

    /// Keep autostart pointing at the current exe *only if the user already
    /// enabled it*. A fresh install (no Run-key value) is left untouched —
    /// autostart is opt-in via the Settings toggle. If the user opted in and
    /// later moved or reinstalled the app, self-heal the registered path.
    pub fn ensure_current() -> Result<EnsureOutcome> {
        let want = current_exe()?;
        let outcome = decide(registered_command().as_deref(), &want);
        if matches!(outcome, EnsureOutcome::Updated) {
            enable()?;
        }
        Ok(outcome)
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

/// Pure autostart policy — no registry access, so it is unit-testable on
/// every platform. `registered` is the current Run-key value (already
/// quoted) or `None` when the key is absent; `want` is the quoted command
/// we would register for the current exe. Absent means the user never opted
/// in, so we leave it alone; a stale value is self-healed.
pub fn decide(registered: Option<&str>, want: &str) -> EnsureOutcome {
    match registered {
        None => EnsureOutcome::NotEnabled,
        Some(existing) if existing == want => EnsureOutcome::AlreadyCorrect,
        Some(_) => EnsureOutcome::Updated,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EnsureOutcome {
    Updated,
    AlreadyCorrect,
    NotEnabled,
    Unsupported,
}

pub use imp::*;

// ---------------------------------------------------------------------------
// Unit tests — pure Rust only, no real OS state touched
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // Verify the registry key path and value name constants have not been
    // accidentally changed.  These strings are load-bearing on Windows.
    #[test]
    fn registry_constants_are_correct() {
        assert_eq!(
            RUN_KEY,
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            "RUN_KEY must match the standard Windows logon autostart location"
        );
        assert_eq!(VALUE_NAME, "andon", "VALUE_NAME must match the app identity");
    }

    // EnsureOutcome must be serializable with the expected JSON tags so the
    // Tauri front-end can pattern-match on `outcome`.
    #[test]
    fn ensure_outcome_serializes_to_snake_case_tag() {
        let cases: &[(EnsureOutcome, &str)] = &[
            (EnsureOutcome::Updated, r#"{"outcome":"updated"}"#),
            (EnsureOutcome::AlreadyCorrect, r#"{"outcome":"already_correct"}"#),
            (EnsureOutcome::NotEnabled, r#"{"outcome":"not_enabled"}"#),
            (EnsureOutcome::Unsupported, r#"{"outcome":"unsupported"}"#),
        ];
        for (outcome, expected) in cases {
            let got = serde_json::to_string(outcome).expect("serialize EnsureOutcome");
            assert_eq!(&got, expected, "EnsureOutcome serialization mismatch");
        }
    }

    // Pure policy: an absent Run-key value means the user has NOT opted in,
    // so ensure_current must do nothing.
    #[test]
    fn decide_absent_key_is_not_enabled() {
        let want = r#""C:\Apps\andon.exe""#;
        assert!(matches!(decide(None, want), EnsureOutcome::NotEnabled));
    }

    // Value already points at the current exe: nothing to do.
    #[test]
    fn decide_matching_key_is_already_correct() {
        let want = r#""C:\Apps\andon.exe""#;
        assert!(matches!(decide(Some(want), want), EnsureOutcome::AlreadyCorrect));
    }

    // Value present but stale (app moved/reinstalled): self-heal the path.
    // This is the grandfather path for existing opted-in users.
    #[test]
    fn decide_stale_key_is_updated() {
        let want = r#""C:\Apps\andon.exe""#;
        let stale = r#""C:\Old\andon.exe""#;
        assert!(matches!(decide(Some(stale), want), EnsureOutcome::Updated));
    }

    // On non-Windows the no-op stubs must return the Unsupported outcome and
    // must not error.  On Windows the imp functions interact with the real
    // registry so we do NOT call enable()/disable() here.
    #[cfg(not(windows))]
    #[test]
    fn non_windows_stubs_are_safe() {
        assert!(!is_enabled(), "is_enabled() stub must return false");
        assert!(registered_command().is_none(), "registered_command() stub must return None");

        let r = enable();
        assert!(r.is_ok(), "enable() stub must not error");

        let r = disable();
        assert!(r.is_ok(), "disable() stub must not error");

        let r = ensure_current();
        assert!(r.is_ok(), "ensure_current() stub must not error");
        assert!(
            matches!(r.unwrap(), EnsureOutcome::Unsupported),
            "ensure_current() stub must return Unsupported"
        );
    }
}
