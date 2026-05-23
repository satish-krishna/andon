//! Per-model token pricing for retroactive cost computation.
//! Used only for JSONL-only sessions (no OTLP cost available).
//! USD per million tokens.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_create_per_mtok: f64,
}

pub fn lookup(model: &str) -> Option<ModelPricing> {
    for (prefix, price) in TABLE {
        if model.starts_with(prefix) {
            return Some(*price);
        }
    }
    None
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

const TABLE: &[(&str, ModelPricing)] = &[
    (
        "claude-opus-4-7",
        ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.50,
            cache_create_per_mtok: 18.75,
        },
    ),
    (
        "claude-opus-4-6",
        ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.50,
            cache_create_per_mtok: 18.75,
        },
    ),
    (
        "claude-sonnet-4-6",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
            cache_create_per_mtok: 3.75,
        },
    ),
    (
        "claude-sonnet-4-5",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
            cache_create_per_mtok: 3.75,
        },
    ),
    (
        "claude-haiku-4-5",
        ModelPricing {
            input_per_mtok: 1.0,
            output_per_mtok: 5.0,
            cache_read_per_mtok: 0.10,
            cache_create_per_mtok: 1.25,
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_works_for_date_suffixed_models() {
        assert_eq!(
            lookup("claude-opus-4-7-20260101").unwrap().output_per_mtok,
            75.0
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(lookup("gpt-4").is_none());
    }

    #[test]
    fn cost_for_sums_token_types() {
        assert!((cost_for("claude-opus-4-7", 1_000_000, 0, 0, 0).unwrap() - 15.0).abs() < 1e-9);
        assert!((cost_for("claude-opus-4-7", 0, 1_000_000, 0, 0).unwrap() - 75.0).abs() < 1e-9);
    }

    #[test]
    fn cost_for_unknown_model_none() {
        assert!(cost_for("mystery-model", 1000, 1000, 0, 0).is_none());
    }
}
