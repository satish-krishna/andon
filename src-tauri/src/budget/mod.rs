//! Budget alerting: projects month-end cost and decides when to signal the
//! user. This module is pure — no clock, no database, no filesystem. The I/O
//! shell that drives it lives in `monitor.rs`.

use chrono::{Datelike, NaiveDate};

/// Alerts are suppressed for the first two days of the month — the linear
/// projection is too volatile that early. Day 3 onward evaluates normally.
const WARMUP_DAYS: u32 = 3;
/// Projected cost at or above this fraction of the budget → Amber.
const AMBER_FRACTION: f64 = 0.80;

/// The budget signal level. `Red` means projected spend will meet or exceed
/// the budget; `Amber` means it will reach `AMBER_FRACTION` of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Neutral,
    Amber,
    Red,
}

impl BudgetStatus {
    /// Lowercase wire name — the JSON value the API and frontend agree on.
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetStatus::Neutral => "neutral",
            BudgetStatus::Amber => "amber",
            BudgetStatus::Red => "red",
        }
    }
}

/// Number of days in the month containing `date`.
pub fn days_in_month(date: NaiveDate) -> u32 {
    match date.month() {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if date.leap_year() => 29,
        2 => 28,
        _ => 30, // unreachable: month() is always 1..=12
    }
}

/// Linear extrapolation of month-end cost from spend so far.
pub fn project_eom(mtd_cost: f64, day_of_month: u32, days_in_month: u32) -> f64 {
    if day_of_month == 0 {
        return mtd_cost;
    }
    (mtd_cost / day_of_month as f64) * days_in_month as f64
}

/// Decide the budget status for a projected month-end cost.
///
/// Returns `Neutral` when the feature is off (`monthly_usd <= 0`) or during
/// the warm-up window (the first two days of the month).
pub fn evaluate(projected_eom: f64, monthly_usd: f64, day_of_month: u32) -> BudgetStatus {
    if monthly_usd <= 0.0 {
        return BudgetStatus::Neutral;
    }
    if day_of_month < WARMUP_DAYS {
        return BudgetStatus::Neutral;
    }
    if projected_eom >= monthly_usd {
        BudgetStatus::Red
    } else if projected_eom >= monthly_usd * AMBER_FRACTION {
        BudgetStatus::Amber
    } else {
        BudgetStatus::Neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid test date")
    }

    #[test]
    fn days_in_month_known_values() {
        assert_eq!(days_in_month(date(2026, 1, 15)), 31);
        assert_eq!(days_in_month(date(2026, 4, 1)), 30);
        assert_eq!(days_in_month(date(2026, 2, 10)), 28);
        assert_eq!(days_in_month(date(2024, 2, 10)), 29); // leap year
    }

    #[test]
    fn project_eom_extrapolates_linearly() {
        // $40 spent by day 10 of a 30-day month → $120 projected.
        assert_eq!(project_eom(40.0, 10, 30), 120.0);
    }

    #[test]
    fn project_eom_day_zero_guard_returns_mtd() {
        assert_eq!(project_eom(40.0, 0, 30), 40.0);
    }

    #[test]
    fn evaluate_off_when_budget_zero() {
        // Projected far over, but no budget set → Neutral.
        assert_eq!(evaluate(9999.0, 0.0, 15), BudgetStatus::Neutral);
    }

    #[test]
    fn evaluate_warmup_suppresses_first_two_days() {
        // Projected at 300% of budget, but day 1 / day 2 → Neutral.
        assert_eq!(evaluate(300.0, 100.0, 1), BudgetStatus::Neutral);
        assert_eq!(evaluate(300.0, 100.0, 2), BudgetStatus::Neutral);
    }

    #[test]
    fn evaluate_thresholds_from_day_three() {
        assert_eq!(evaluate(79.0, 100.0, 3), BudgetStatus::Neutral);
        assert_eq!(evaluate(80.0, 100.0, 3), BudgetStatus::Amber);
        assert_eq!(evaluate(99.99, 100.0, 3), BudgetStatus::Amber);
        assert_eq!(evaluate(100.0, 100.0, 3), BudgetStatus::Red);
        assert_eq!(evaluate(150.0, 100.0, 15), BudgetStatus::Red);
    }
}
