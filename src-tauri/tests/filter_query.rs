mod common;

use andon_lib::api::filter::FilterQuery;

fn fq(from: Option<i64>, to: Option<i64>, models: Option<&str>) -> FilterQuery {
    FilterQuery {
        from,
        to,
        models: models.map(|s| s.to_string()),
    }
}

#[test]
fn window_defaults_to_current_month_when_unset() {
    let q = fq(None, None, None);
    let (from, to) = q.window();
    assert!(from < to);
    // `current_month_bounds()` returns (start_of_month, end_of_today). On the
    // 1st of a month the span is exactly one calendar day (≈ 86_399_999 ms,
    // the millisecond-rounded end-of-day), which a strict `>` against
    // 24 * 3600 * 1000 would flake on. Use `>=` against the one-day span,
    // and upper-bound it at 32 days to confirm the helper isn't returning
    // a multi-month range.
    let one_day_ms = 24 * 3600 * 1000;
    let thirty_two_days_ms = 32 * one_day_ms;
    let span = to - from;
    assert!(span >= one_day_ms - 1, "span should be at least one day (got {span} ms)");
    assert!(span <= thirty_two_days_ms, "span should fit in 32 days (got {span} ms)");
}

#[test]
fn window_uses_explicit_bounds_when_set() {
    let q = fq(Some(1000), Some(2000), None);
    assert_eq!(q.window(), (1000, 2000));
}

#[test]
fn model_list_handles_empty_whitespace_and_trailing_comma() {
    assert!(fq(None, None, None).model_list().is_empty());
    assert!(fq(None, None, Some("")).model_list().is_empty());
    assert!(fq(None, None, Some("  ,  ")).model_list().is_empty());
    assert_eq!(
        fq(None, None, Some("opus, sonnet,")).model_list(),
        vec!["opus".to_string(), "sonnet".to_string()],
    );
}

#[test]
fn model_clause_is_empty_when_no_models_set() {
    let q = fq(None, None, None);
    let (sql, params) = q.model_clause("model");
    assert!(sql.is_empty());
    assert!(params.is_empty());
}

#[test]
fn model_clause_builds_substring_or_chain_and_lowercases_params() {
    let q = fq(None, None, Some("Opus,Sonnet"));
    let (sql, params) = q.model_clause("tu.model");
    assert_eq!(
        sql,
        " AND (LOWER(tu.model) LIKE ? OR LOWER(tu.model) LIKE ?)"
    );
    assert_eq!(params, vec!["%opus%".to_string(), "%sonnet%".to_string()]);
}
