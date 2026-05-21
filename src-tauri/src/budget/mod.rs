//! Budget alerting: projects month-end cost and decides when to signal the
//! user. This module is pure — no clock, no database, no filesystem. The I/O
//! shell that drives it lives in `monitor.rs`.

pub mod monitor;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

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

/// Unix-ms of 00:00 local time on the first of `now`'s month — the start of
/// the month-to-date window. Shared by the API and the monitor so both
/// project against an identical window.
pub fn month_start_ms(now: DateTime<Local>) -> i64 {
    let first = now.date_naive().with_day(1).unwrap_or(now.date_naive());
    match first.and_hms_opt(0, 0, 0) {
        Some(naive) => Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|| now.timestamp_millis()),
        None => now.timestamp_millis(),
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

/// Per-month notification de-dup state, persisted to `budget-alerts.json`.
/// The `fired_*` flags latch on; they reset when the month or the budget
/// amount changes (see `evaluate_once`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AlertState {
    /// `"YYYY-MM"` of the month these flags belong to. Empty on first run.
    #[serde(default)]
    pub month: String,
    /// Budget amount the flags were evaluated against.
    #[serde(default)]
    pub monthly_usd: f64,
    #[serde(default)]
    pub fired_amber: bool,
    #[serde(default)]
    pub fired_red: bool,
}

/// The result of one budget evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalOutcome {
    /// The live status — drives the tray colour. Moves freely up and down.
    pub status: BudgetStatus,
    /// Projected end-of-month cost the status was derived from.
    pub projected_eom: f64,
    /// The `AlertState` to persist back to disk.
    pub next_state: AlertState,
    /// `Some(level)` when a first-crossing notification should fire now.
    pub notify: Option<BudgetStatus>,
}

/// Pure budget evaluation for one monitor tick.
///
/// `now` is injected so this is fully testable. `prior` is the persisted
/// `AlertState`; the returned `next_state` must be written back. Notifications
/// fire at most once per threshold per month — `fired_*` flags latch on and
/// reset only when the month rolls over or the budget amount changes.
pub fn evaluate_once(
    mtd_cost: f64,
    monthly_usd: f64,
    now: DateTime<Local>,
    prior: AlertState,
) -> EvalOutcome {
    let today = now.date_naive();
    let day = today.day();
    let projected = project_eom(mtd_cost, day, days_in_month(today));
    let status = evaluate(projected, monthly_usd, day);

    let month = format!("{:04}-{:02}", today.year(), today.month());
    let budget_changed = (prior.monthly_usd - monthly_usd).abs() > 0.001;

    let mut state = if prior.month != month || budget_changed {
        AlertState {
            month,
            monthly_usd,
            fired_amber: false,
            fired_red: false,
        }
    } else {
        prior
    };

    // Notifications fire once per threshold, on the first upward crossing.
    // Crossing red subsumes amber — no late amber notification afterward.
    let notify = match status {
        BudgetStatus::Red if !state.fired_red => {
            state.fired_red = true;
            state.fired_amber = true;
            Some(BudgetStatus::Red)
        }
        BudgetStatus::Amber if !state.fired_amber => {
            state.fired_amber = true;
            Some(BudgetStatus::Amber)
        }
        _ => None,
    };

    EvalOutcome {
        status,
        projected_eom: projected,
        next_state: state,
        notify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, NaiveDate, TimeZone};

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
    fn month_start_ms_is_first_of_month_local_midnight() {
        let now = Local
            .with_ymd_and_hms(2026, 5, 21, 14, 30, 0)
            .single()
            .expect("valid now");
        let expected = Local
            .with_ymd_and_hms(2026, 5, 1, 0, 0, 0)
            .single()
            .expect("valid month start");
        assert_eq!(month_start_ms(now), expected.timestamp_millis());
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

    fn may_15() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 5, 15, 12, 0, 0)
            .single()
            .expect("valid local datetime")
    }

    #[test]
    fn evaluate_once_first_amber_crossing_fires_once() {
        // $45 by day 15 of 31 → projected $93 → Amber.
        let first = evaluate_once(45.0, 100.0, may_15(), AlertState::default());
        assert_eq!(first.status, BudgetStatus::Amber);
        assert_eq!(first.projected_eom, 93.0);
        assert_eq!(first.notify, Some(BudgetStatus::Amber));
        assert!(first.next_state.fired_amber);

        // Same inputs, state carried forward → no second notification.
        let second = evaluate_once(45.0, 100.0, may_15(), first.next_state);
        assert_eq!(second.status, BudgetStatus::Amber);
        assert_eq!(second.notify, None);
    }

    #[test]
    fn evaluate_once_red_jump_marks_amber_fired_too() {
        // $60 by day 15 of 31 → projected $124 → Red, from a fresh state.
        let out = evaluate_once(60.0, 100.0, may_15(), AlertState::default());
        assert_eq!(out.status, BudgetStatus::Red);
        assert_eq!(out.notify, Some(BudgetStatus::Red));
        assert!(out.next_state.fired_red);
        assert!(out.next_state.fired_amber, "crossing red latches amber too");

        // A later amber dip must NOT produce a late amber notification.
        let later = evaluate_once(45.0, 100.0, may_15(), out.next_state);
        assert_eq!(later.status, BudgetStatus::Amber);
        assert_eq!(later.notify, None);
    }

    #[test]
    fn evaluate_once_resets_latch_on_new_month() {
        // Prior state belongs to April with amber already fired.
        let april = AlertState {
            month: "2026-04".into(),
            monthly_usd: 100.0,
            fired_amber: true,
            fired_red: false,
        };
        let out = evaluate_once(45.0, 100.0, may_15(), april);
        assert_eq!(out.status, BudgetStatus::Amber);
        assert_eq!(out.notify, Some(BudgetStatus::Amber), "new month re-arms amber");
        assert_eq!(out.next_state.month, "2026-05");
    }

    #[test]
    fn evaluate_once_resets_latch_when_budget_changes() {
        // Amber already fired this month against a $200 budget.
        let prior = AlertState {
            month: "2026-05".into(),
            monthly_usd: 200.0,
            fired_amber: true,
            fired_red: false,
        };
        // Budget lowered to $100; $45 by day 15 → projected $93 → Amber again.
        let out = evaluate_once(45.0, 100.0, may_15(), prior);
        assert_eq!(out.notify, Some(BudgetStatus::Amber), "budget change re-arms");
        assert_eq!(out.next_state.monthly_usd, 100.0);
    }

    #[test]
    fn evaluate_once_status_falls_without_notifying() {
        // Red fired earlier this month.
        let prior = AlertState {
            month: "2026-05".into(),
            monthly_usd: 100.0,
            fired_amber: true,
            fired_red: true,
        };
        // Pace slowed: $5 by day 15 → projected ~$10 → Neutral.
        let out = evaluate_once(5.0, 100.0, may_15(), prior);
        assert_eq!(out.status, BudgetStatus::Neutral);
        assert_eq!(out.notify, None);
    }
}
