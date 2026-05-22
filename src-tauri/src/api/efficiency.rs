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

use crate::jsonl::pricing;

/// Share of *prompt* tokens served from cache. Output is excluded — it is not
/// part of the prompt. Returns `0.0` when there are no prompt tokens.
pub fn hit_ratio(input: i64, cache_create: i64, cache_read: i64) -> f64 {
    let denom = input + cache_create + cache_read;
    if denom <= 0 {
        0.0
    } else {
        cache_read as f64 / denom as f64
    }
}

/// Prompt-cache savings, in USD.
#[derive(Debug, Clone, Copy)]
pub struct Savings {
    /// Discount won on cache reads vs. paying the input rate for them.
    pub gross: f64,
    /// Premium paid to write the cache vs. the input rate.
    pub creation_overhead: f64,
    /// `gross - creation_overhead` — the true saving.
    pub net: f64,
    /// cache-read + cache-create tokens on models absent from the price table.
    pub unpriced_cache_tokens: i64,
}

/// Compute cache savings from per-model `(model, cache_read, cache_create)`
/// token counts. Models not in the pricing table contribute nothing to the
/// dollar figures; their tokens are tallied into `unpriced_cache_tokens`.
pub fn cache_savings<'a>(rows: impl Iterator<Item = (&'a str, i64, i64)>) -> Savings {
    let mut gross = 0.0;
    let mut creation_overhead = 0.0;
    let mut unpriced = 0i64;
    for (model, cache_read, cache_create) in rows {
        match pricing::lookup(model) {
            Some(p) => {
                gross += cache_read as f64 / 1e6 * (p.input_per_mtok - p.cache_read_per_mtok);
                creation_overhead +=
                    cache_create as f64 / 1e6 * (p.cache_create_per_mtok - p.input_per_mtok);
            }
            None => unpriced += cache_read + cache_create,
        }
    }
    Savings {
        gross,
        creation_overhead,
        net: gross - creation_overhead,
        unpriced_cache_tokens: unpriced,
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

    #[test]
    fn hit_ratio_is_cache_read_over_prompt_tokens() {
        // 300 cache-read of (100 input + 100 create + 300 read) = 0.6
        assert!((hit_ratio(100, 100, 300) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn hit_ratio_zero_when_no_tokens() {
        assert_eq!(hit_ratio(0, 0, 0), 0.0);
    }

    #[test]
    fn cache_savings_opus_nets_gross_minus_overhead() {
        // opus: input 15, cache_read 1.50, cache_create 18.75 ($/Mtok).
        // 1M read  -> gross    = 1M * (15 - 1.50)  / 1e6 = 13.50
        // 1M create-> overhead = 1M * (18.75 - 15) / 1e6 = 3.75
        let s = cache_savings(
            [("claude-opus-4-7", 1_000_000i64, 1_000_000i64)].into_iter(),
        );
        assert!((s.gross - 13.50).abs() < 1e-9, "gross {}", s.gross);
        assert!(
            (s.creation_overhead - 3.75).abs() < 1e-9,
            "overhead {}",
            s.creation_overhead
        );
        assert!((s.net - 9.75).abs() < 1e-9, "net {}", s.net);
        assert_eq!(s.unpriced_cache_tokens, 0);
    }

    #[test]
    fn cache_savings_counts_unpriced_models() {
        let s = cache_savings([("mystery-model", 500i64, 500i64)].into_iter());
        assert_eq!(s.gross, 0.0);
        assert_eq!(s.net, 0.0);
        assert_eq!(s.unpriced_cache_tokens, 1000);
    }
}
