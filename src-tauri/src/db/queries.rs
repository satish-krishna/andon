// Query helpers — populated in later sections.

/// Total cost (USD) recorded in `cost_entries` with `timestamp` in
/// `[from_ms, to_ms)`, across all models. Used by the budget monitor for the
/// month-to-date sum.
pub fn month_to_date_cost(
    conn: &rusqlite::Connection,
    from_ms: i64,
    to_ms: i64,
) -> rusqlite::Result<f64> {
    conn.query_row(
        "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries \
         WHERE timestamp >= ? AND timestamp < ?",
        rusqlite::params![from_ms, to_ms],
        |r| r.get(0),
    )
}
