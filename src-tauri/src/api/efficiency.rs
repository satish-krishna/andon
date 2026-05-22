//! Cost-efficiency math for the Efficiency page (cache savings + per-model
//! cost-efficiency). Pure and DB-free so it can be unit-tested in isolation.

use std::collections::HashMap;

use crate::api::dto::ModelEfficiencyRow;
use crate::jsonl::pricing;

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

/// Round a USD figure to 4 decimal places — matches the `round4` used by the
/// v2 API handlers. Kept local so this module stays free of `routes.rs`.
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// The family that spent the most in a session. Ties are broken toward the
/// fixed order opus > sonnet > haiku > other (strict `>` keeps the first).
///
/// Expects `costs` to be non-empty (every `aggregate_model_efficiency` session
/// has at least one cost row); returns `"other"` as a safe sentinel if not.
fn dominant_family(costs: &HashMap<&'static str, f64>) -> &'static str {
    const ORDER: [&str; 4] = ["opus", "sonnet", "haiku", "other"];
    let mut best: &'static str = "other";
    let mut best_cost = f64::NEG_INFINITY;
    for fam in ORDER {
        if let Some(&c) = costs.get(fam) {
            if c > best_cost {
                best_cost = c;
                best = fam;
            }
        }
    }
    best
}

/// Aggregate per-session cost/output into per-family rows, attributing each
/// session wholly to its dominant family. `cost_rows` are
/// `(session_id, model, cost_usd)`; `output_rows` are `(session_id, output)`.
/// Rows are sorted by total cost descending.
pub fn aggregate_model_efficiency(
    cost_rows: &[(String, String, f64)],
    output_rows: &[(String, i64)],
) -> Vec<ModelEfficiencyRow> {
    // session_id -> family -> cost
    let mut per_session: HashMap<&str, HashMap<&'static str, f64>> = HashMap::new();
    for (sid, model, cost) in cost_rows {
        *per_session
            .entry(sid.as_str())
            .or_default()
            .entry(model_family(model))
            .or_insert(0.0) += *cost;
    }
    // session_id -> output tokens
    let mut output: HashMap<&str, i64> = HashMap::new();
    for (sid, toks) in output_rows {
        *output.entry(sid.as_str()).or_insert(0) += *toks;
    }
    // family -> (sessions, total_cost, output_tokens)
    let mut buckets: HashMap<&'static str, (i64, f64, i64)> = HashMap::new();
    for (sid, fam_costs) in &per_session {
        let fam = dominant_family(fam_costs);
        let total_cost: f64 = fam_costs.values().sum();
        let out = output.get(sid).copied().unwrap_or(0);
        let entry = buckets.entry(fam).or_insert((0, 0.0, 0));
        entry.0 += 1;
        entry.1 += total_cost;
        entry.2 += out;
    }
    let mut rows: Vec<ModelEfficiencyRow> = buckets
        .into_iter()
        .map(|(family, (sessions, total_cost, output_tokens))| ModelEfficiencyRow {
            family: family.to_string(),
            sessions,
            total_cost_usd: round4(total_cost),
            cost_per_session: round4(total_cost / sessions as f64),
            output_tokens,
            cost_per_1k_output: if output_tokens > 0 {
                round4(total_cost / output_tokens as f64 * 1000.0)
            } else {
                0.0
            },
        })
        .collect();
    rows.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
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

    #[test]
    fn aggregate_buckets_session_by_dominant_family() {
        // s1: opus 5.0 + haiku 1.0 -> dominant opus, whole session (6.0) -> opus
        // s2: haiku 2.0           -> dominant haiku
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(), 5.0),
            ("s1".to_string(), "claude-haiku-4-5".to_string(), 1.0),
            ("s2".to_string(), "claude-haiku-4-5".to_string(), 2.0),
        ];
        let output_rows = vec![("s1".to_string(), 1000i64), ("s2".to_string(), 500i64)];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);

        assert_eq!(rows.len(), 2);
        // sorted by total cost desc -> opus first
        assert_eq!(rows[0].family, "opus");
        assert_eq!(rows[0].sessions, 1);
        assert!((rows[0].total_cost_usd - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_session - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_1k_output - 6.0).abs() < 1e-9); // 6.0/1000*1000
        assert_eq!(rows[1].family, "haiku");
        assert!((rows[1].cost_per_1k_output - 4.0).abs() < 1e-9); // 2.0/500*1000
    }

    #[test]
    fn aggregate_breaks_ties_toward_opus() {
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(), 2.0),
            ("s1".to_string(), "claude-sonnet-4-6".to_string(), 2.0),
        ];
        let rows = aggregate_model_efficiency(&cost_rows, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].family, "opus");
    }

    #[test]
    fn aggregate_handles_zero_output() {
        let cost_rows = vec![("s1".to_string(), "claude-opus-4-7".to_string(), 3.0)];
        let rows = aggregate_model_efficiency(&cost_rows, &[]);
        assert_eq!(rows[0].cost_per_1k_output, 0.0);
    }
}
