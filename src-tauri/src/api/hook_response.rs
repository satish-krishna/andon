//! Claude Code hook output envelope.
//!
//! Claude Code's hook runtime reads the hook command's stdout and validates it
//! against its hook output schema. Returning ad-hoc shapes (e.g. `{"ok": true}`)
//! triggers "Hook JSON output validation failed". `HookOutput` models the
//! envelope so handlers always return something CC accepts.

use serde::Serialize;

/// Models Claude Code's hook output JSON envelope.
///
/// Every field is optional; `HookOutput::ok()` serializes to `{}` which is a
/// valid no-op hook output. Populating `system_message` surfaces a message in
/// the CC transcript; `r#continue: Some(false)` blocks the tool call (only
/// meaningful for PreToolUse — included for future use).
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#continue: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

impl HookOutput {
    /// Empty, valid hook output. Serializes to `{}`.
    pub fn ok() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::HookOutput;

    #[test]
    fn ok_serializes_to_empty_object() {
        let s = serde_json::to_string(&HookOutput::ok()).unwrap();
        assert_eq!(s, "{}");
    }

    #[test]
    fn none_fields_are_omitted() {
        let out = HookOutput {
            system_message: Some("hi".into()),
            ..Default::default()
        };
        let s = serde_json::to_string(&out).unwrap();
        assert_eq!(s, r#"{"systemMessage":"hi"}"#);
    }

    #[test]
    fn field_names_are_camel_case() {
        let out = HookOutput {
            r#continue: Some(false),
            suppress_output: Some(true),
            system_message: Some("paused".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&out).unwrap();
        assert!(v.get("continue").is_some(), "continue field missing");
        assert!(v.get("suppressOutput").is_some(), "suppressOutput field missing");
        assert!(v.get("systemMessage").is_some(), "systemMessage field missing");
        assert!(v.get("suppress_output").is_none(), "snake_case leaked");
        assert!(v.get("system_message").is_none(), "snake_case leaked");
    }
}
