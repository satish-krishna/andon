//! Cost-efficiency math for the Efficiency page (cache savings + per-model
//! cost-efficiency). Pure and DB-free so it can be unit-tested in isolation.

/// Classify a full model id (e.g. `claude-opus-4-7-20260101`) into a coarse
/// family. Case-insensitive substring match; anything unrecognized is `other`.
pub fn model_family(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m.contains("opus") {
        "opus"
    } else if m.contains("sonnet") {
        "sonnet"
    } else if m.contains("haiku") {
        "haiku"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_family_classifies_known_families() {
        assert_eq!(model_family("claude-opus-4-7"), "opus");
        assert_eq!(model_family("claude-opus-4-7-20260101"), "opus");
        assert_eq!(model_family("claude-sonnet-4-6"), "sonnet");
        assert_eq!(model_family("claude-haiku-4-5"), "haiku");
    }

    #[test]
    fn model_family_is_case_insensitive() {
        assert_eq!(model_family("Claude-OPUS-X"), "opus");
    }

    #[test]
    fn model_family_unknown_is_other() {
        assert_eq!(model_family("gpt-4"), "other");
    }
}
