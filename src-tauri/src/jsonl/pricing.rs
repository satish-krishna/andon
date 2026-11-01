//! Per-model token pricing for retroactive cost computation.
//! Used only for JSONL-only sessions (no OTLP cost available).
//! USD per million tokens, Anthropic first-party list prices.
//!
//! Cache writes are priced at the 5-minute TTL rate (1.25x input); JSONL's
//! `cache_creation_input_tokens` does not distinguish the 1-hour tier.
//!
//! When this table changes, bump [`PRICING_VERSION`] so the background scanner
//! re-ingests every transcript and reprices the rows it already wrote.

/// Version of [`TABLE`]. The JSONL scanner records it per transcript and
/// re-ingests any file last priced under an older version.
pub const PRICING_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_create_per_mtok: f64,
}

/// Look up pricing for a model id as Claude Code writes it in the transcript.
///
/// Accepts bare ids (`claude-opus-4-7`), date-suffixed snapshots
/// (`claude-sonnet-4-5-20250929`), Vertex `@` snapshots, Bedrock-style
/// prefixes (`us.anthropic.claude-...-v1:0`) and context-window suffixes
/// (`claude-opus-4-6[1m]`). A model that is merely a *prefix-extension* of a
/// known family (e.g. a future `claude-opus-4-9` against `claude-opus-4`) is
/// deliberately NOT matched: an unknown model must surface as unpriced rather
/// than be silently charged at a neighbor's rate.
pub fn lookup(model: &str) -> Option<ModelPricing> {
    let id = normalize(model)?;
    TABLE
        .iter()
        .filter(|(family, _)| matches_family(id, family))
        .max_by_key(|(family, _)| family.len())
        .map(|(_, price)| *price)
}

pub fn cost_for(
    model: &str,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_create: i64,
) -> Option<f64> {
    let p = lookup(model)?;
    let n = |toks: i64, per_m: f64| (toks as f64) / 1_000_000.0 * per_m;
    Some(
        n(input, p.input_per_mtok)
            + n(output, p.output_per_mtok)
            + n(cache_read, p.cache_read_per_mtok)
            + n(cache_create, p.cache_create_per_mtok),
    )
}

/// Strip provider prefixes and `[...]` suffixes, leaving `claude-...`.
fn normalize(model: &str) -> Option<&str> {
    let start = model.find("claude-")?;
    let id = &model[start..];
    Some(match id.find('[') {
        Some(i) => &id[..i],
        None => id,
    })
}

/// `id` belongs to `family` when it is exactly the family, or the family
/// followed by a snapshot marker: `@...` (Vertex) or `-YYYYMMDD...`.
fn matches_family(id: &str, family: &str) -> bool {
    let Some(rest) = id.strip_prefix(family) else {
        return false;
    };
    if rest.is_empty() || rest.starts_with('@') {
        return true;
    }
    let Some(tail) = rest.strip_prefix('-') else {
        return false;
    };
    tail.len() >= 8 && tail.as_bytes()[..8].iter().all(u8::is_ascii_digit)
}

const fn price(input: f64, output: f64, cache_read: f64) -> ModelPricing {
    ModelPricing {
        input_per_mtok: input,
        output_per_mtok: output,
        cache_read_per_mtok: cache_read,
        cache_create_per_mtok: input * 1.25,
    }
}

const TABLE: &[(&str, ModelPricing)] = &[
    // Claude 5 family
    ("claude-fable-5-1", price(10.0, 50.0, 0.25)),
    ("claude-mythos-5-1", price(10.0, 50.0, 0.25)),
    ("claude-fable-5", price(10.0, 50.0, 1.00)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.20)),
    ("claude-opus-5", price(5.0, 25.0, 0.50)),
    ("claude-sonnet-5-5", price(2.0, 10.0, 0.20)),
    ("claude-sonnet-5", price(2.0, 10.0, 0.20)),
    // Base rate for prompts up to 100K tokens; the long-prompt tier is not
    // modeled because JSONL usage does not carry the request's prompt size
    // separately from cache reads.
    ("claude-haiku-5-5", price(0.10, 0.50, 0.01)),
    // Claude 4 family
    ("claude-opus-4-8", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-5", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-1", price(15.0, 75.0, 1.50)),
    ("claude-opus-4", price(15.0, 75.0, 1.50)),
    ("claude-sonnet-4-6", price(3.0, 15.0, 0.30)),
    ("claude-sonnet-4-5", price(3.0, 15.0, 0.30)),
    ("claude-sonnet-4", price(3.0, 15.0, 0.30)),
    ("claude-haiku-4-5", price(1.0, 5.0, 0.10)),
    // Claude 3.x, for old transcripts
    ("claude-3-7-sonnet", price(3.0, 15.0, 0.30)),
    ("claude-3-5-haiku", price(0.80, 4.0, 0.08)),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn out(model: &str) -> f64 {
        lookup(model).unwrap().output_per_mtok
    }

    #[test]
    fn prefix_match_works_for_date_suffixed_models() {
        assert_eq!(out("claude-opus-4-7-20260101"), 25.0);
        assert_eq!(out("claude-sonnet-4-5-20250929"), 15.0);
        assert_eq!(out("claude-opus-4-20250514"), 75.0);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(lookup("gpt-4").is_none());
        assert!(lookup("<synthetic>").is_none());
    }

    #[test]
    fn cost_for_sums_token_types() {
        assert!((cost_for("claude-opus-4-7", 1_000_000, 0, 0, 0).unwrap() - 5.0).abs() < 1e-9);
        assert!((cost_for("claude-opus-4-7", 0, 1_000_000, 0, 0).unwrap() - 25.0).abs() < 1e-9);
        assert!((cost_for("claude-opus-4-7", 0, 0, 1_000_000, 0).unwrap() - 0.5).abs() < 1e-9);
        assert!((cost_for("claude-opus-4-7", 0, 0, 0, 1_000_000).unwrap() - 6.25).abs() < 1e-9);
    }

    #[test]
    fn cost_for_unknown_model_none() {
        assert!(cost_for("mystery-model", 1000, 1000, 0, 0).is_none());
    }

    #[test]
    fn opus_4_5_and_later_use_the_reduced_opus_rate() {
        for m in ["claude-opus-4-5", "claude-opus-4-6", "claude-opus-4-7", "claude-opus-4-8"] {
            let p = lookup(m).unwrap();
            assert_eq!((p.input_per_mtok, p.output_per_mtok), (5.0, 25.0), "{m}");
        }
        assert_eq!(out("claude-opus-4-1"), 75.0);
    }

    #[test]
    fn claude_5_family_is_priced() {
        assert_eq!(out("claude-opus-5-5"), 20.0);
        assert_eq!(out("claude-opus-5"), 25.0);
        assert_eq!(out("claude-sonnet-5-5"), 10.0);
        assert_eq!(out("claude-sonnet-5"), 10.0);
        assert_eq!(out("claude-haiku-5-5"), 0.50);
        assert_eq!(out("claude-fable-5-1"), 50.0);
        assert_eq!(out("claude-fable-5"), 50.0);
    }

    #[test]
    fn point_releases_do_not_fall_back_to_the_base_family() {
        // `claude-opus-5` is a string prefix of `claude-opus-5-5`; the more
        // specific entry must win.
        assert_eq!(lookup("claude-opus-5-5").unwrap().input_per_mtok, 4.0);
        assert_eq!(lookup("claude-fable-5-1").unwrap().cache_read_per_mtok, 0.25);
    }

    #[test]
    fn unreleased_point_versions_are_unpriced_not_misattributed() {
        assert!(lookup("claude-opus-4-9").is_none());
        assert!(lookup("claude-sonnet-5-7").is_none());
    }

    #[test]
    fn provider_prefixes_and_context_suffixes_are_normalized() {
        assert_eq!(out("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), 15.0);
        assert_eq!(out("anthropic.claude-opus-5-5"), 20.0);
        assert_eq!(out("claude-opus-4-6[1m]"), 25.0);
        assert_eq!(out("claude-opus-4-1@20250805"), 75.0);
    }
}
