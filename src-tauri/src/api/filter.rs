use chrono::{Datelike, Local, TimeZone};
use serde::Deserialize;

// ============================================================================
// FilterQuery — shared query-parameter DTO for v2 filterable endpoints.
// Extracted to its own module so integration tests can construct and inspect
// instances without pulling in the full routes module.
// ============================================================================

#[derive(Deserialize)]
pub struct FilterQuery {
    pub from: Option<i64>,      // unix ms
    pub to: Option<i64>,        // unix ms
    pub models: Option<String>, // comma-separated
}

impl FilterQuery {
    pub fn window(&self) -> (i64, i64) {
        let (default_from, default_to) = current_month_bounds();
        (self.from.unwrap_or(default_from), self.to.unwrap_or(default_to))
    }

    pub fn model_list(&self) -> Vec<String> {
        self.models
            .as_deref()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn model_clause(&self, col: &str) -> (String, Vec<String>) {
        let models = self.model_list();
        if models.is_empty() {
            return (String::new(), vec![]);
        }
        let likes: Vec<String> = models.iter().map(|m| format!("%{}%", m.to_lowercase())).collect();
        let ored = likes
            .iter()
            .map(|_| format!("LOWER({col}) LIKE ?"))
            .collect::<Vec<_>>()
            .join(" OR ");
        (format!(" AND ({ored})"), likes)
    }
}

/// Returns `(start_of_today_ms, end_of_today_ms)` in the local timezone.
pub fn today_bounds() -> (i64, i64) {
    let now = Local::now();
    let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let from = Local
        .from_local_datetime(&start)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0);
    let to = from + 24 * 60 * 60 * 1000;
    (from, to)
}

/// Returns `(start_of_current_month_ms, end_of_today_ms)` in the local timezone.
pub fn current_month_bounds() -> (i64, i64) {
    let now = Local::now();
    let start = now
        .date_naive()
        .with_day(1)
        .unwrap_or(now.date_naive())
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let from = Local
        .from_local_datetime(&start)
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    let (_, today_to) = today_bounds();
    (from, today_to)
}
