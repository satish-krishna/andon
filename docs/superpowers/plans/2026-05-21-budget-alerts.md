# Budget Alerts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user set a monthly cost budget; a background monitor signals when projected end-of-month spend crosses 80%/100% of it, via the tray icon colour and desktop notifications.

**Architecture:** A backend Rust task (`budget` module) periodically projects month-end cost, repaints the tray icon, and fires one notification per threshold per month. Pure decision logic (`budget/mod.rs`) is split from the I/O shell (`budget/monitor.rs`). The Angular dashboard mirrors the same status on the Overview cost tile and gains a budget input in Settings.

**Tech Stack:** Rust (Tauri 2, axum, rusqlite, chrono, `tauri-plugin-notification`), Angular 21 (standalone components, signals), Vitest.

**Spec:** `docs/superpowers/specs/2026-05-21-budget-alerts-design.md`

**Branch:** `feature/budget-alerts` (already created and checked out).

**Commits:** every commit message ends with a blank line then the trailer
`Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.

---

## File structure

**Created:**

- `src-tauri/src/budget/mod.rs` — pure budget logic: `BudgetStatus`, `project_eom`, `days_in_month`, `evaluate`, `AlertState`, `evaluate_once`. No clock/DB/filesystem. Unit-tested in-module.
- `src-tauri/src/budget/monitor.rs` — the I/O shell: the periodic task, DB read, tray repaint, notification dispatch, `budget-alerts.json` load/save.
- `src-tauri/tests/api_budget.rs` — API integration tests for the budget endpoint and the `v2_kpis` budget block.
- `src-tauri/tests/budget_query.rs` — integration test for the `month_to_date_cost` query.
- `src-tauri/icons/tray-neutral.png`, `tray-amber.png`, `tray-red.png` — tray status icons.
- `web/src/app/features/settings/budget-card.component.ts` — the budget Settings card.
- `web/src/app/features/settings/budget-card.component.spec.ts` — its test.
- `web/src/app/features/overview/budget-indicator.ts` — pure status→Tailwind-class helpers.
- `web/src/app/features/overview/budget-indicator.spec.ts` — its test.

**Modified:**

- `src-tauri/Cargo.toml` — add `tauri-plugin-notification`.
- `src-tauri/capabilities/default.json` — add `notification:default`.
- `src-tauri/src/settings.rs` — `BudgetSettings`, `AppSettings.budget`, `budget()`, `save_budget()`.
- `src-tauri/src/db/queries.rs` — `month_to_date_cost`.
- `src-tauri/src/lib.rs` — declare `budget` module, register notification plugin, `TRAY_ID` const, spawn the monitor.
- `src-tauri/src/api/routes.rs` — `PUT /api/settings/budget`; `v2_kpis` budget block.
- `src-tauri/tests/settings_roundtrip.rs` — budget persistence + the legacy-file regression test.
- `web/src/app/core/api.service.ts` — budget types + `saveBudget()`.
- `web/src/app/features/settings/settings.component.ts` + `.html` — mount the budget card.
- `web/src/app/features/overview/overview.component.ts` + `.html` — the budget indicator.
- `docs/features.md`, `docs/architecture.md` — document the feature.

---

## Task 1: Budget settings persistence

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Test: `src-tauri/tests/settings_roundtrip.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/tests/settings_roundtrip.rs`, change the import line at the top:

```rust
use andon_lib::settings::{AppSettings, BudgetSettings, ForwarderSettings, SettingsStore};
```

Then append these two tests to the end of the file:

```rust
// ---------------------------------------------------------------------------
// 6. Budget round-trip: save a budget, reload, value matches
// ---------------------------------------------------------------------------

#[test]
fn budget_round_trip() {
    let dir = tmp();
    let path = settings_path(&dir);
    let store = SettingsStore::load(path.clone()).expect("initial load");

    store
        .save_budget(BudgetSettings { monthly_usd: 250.0 })
        .expect("save_budget");

    let reloaded = SettingsStore::load(path).expect("reload");
    assert_eq!(reloaded.budget().monthly_usd, 250.0, "budget round-trip");
}

// ---------------------------------------------------------------------------
// 7. A legacy settings.json with no `budget` key loads cleanly (no backup)
// ---------------------------------------------------------------------------

/// CRITICAL REGRESSION GUARD. Existing installs have a settings.json written
/// before the `budget` field existed. Without `#[serde(default)]` on
/// `AppSettings.budget`, parsing fails, the loader treats the file as corrupt,
/// and overwrites it with defaults — wiping the user's forwarder config.
#[test]
fn settings_without_budget_key_loads_without_backup() {
    let dir = tmp();
    let path = settings_path(&dir);

    // An old settings.json: version + forwarder only, no `budget` key.
    std::fs::write(
        &path,
        r#"{
  "version": 1,
  "forwarder": {
    "enabled": true,
    "endpoint": "http://otel.example.com:4318",
    "timeout_ms": 3000,
    "headers": {}
  }
}"#,
    )
    .expect("write legacy settings file");

    let store = SettingsStore::load(path).expect("legacy file must load");

    // The missing `budget` key falls back to the default — not a corrupt wipe.
    assert_eq!(store.budget(), BudgetSettings::default(), "budget defaults");
    // The forwarder config survived — proof the file was parsed, not discarded.
    assert_eq!(
        store.forwarder().endpoint,
        "http://otel.example.com:4318",
        "forwarder config must survive a budget-less file",
    );
    // No `corrupt-*` backup must exist.
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
        .collect();
    assert_eq!(backups.len(), 0, "a budget-less file is valid, not corrupt");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri; cargo test --features test-support --test settings_roundtrip`
Expected: compile error — `BudgetSettings` not found, `save_budget`/`budget` methods missing.

- [ ] **Step 3: Implement `BudgetSettings` and the store methods**

In `src-tauri/src/settings.rs`, add the `BudgetSettings` struct after the `ForwarderSettings` struct (after line 20):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetSettings {
    /// Monthly cost budget in USD. `0.0` (the default) disables alerts.
    pub monthly_usd: f64,
}

impl Default for BudgetSettings {
    fn default() -> Self {
        Self { monthly_usd: 0.0 }
    }
}
```

Add the `budget` field to `AppSettings` — note `#[serde(default)]` is mandatory:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub version: u32,
    pub forwarder: ForwarderSettings,
    /// `#[serde(default)]` lets settings.json files written before this field
    /// existed still parse — without it, every existing install is treated as
    /// corrupt and overwritten. See the regression test in settings_roundtrip.rs.
    #[serde(default)]
    pub budget: BudgetSettings,
}
```

Add `budget` to the `Default for AppSettings` impl:

```rust
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            forwarder: ForwarderSettings {
                enabled: false,
                endpoint: String::new(),
                timeout_ms: 2000,
                headers: Default::default(),
            },
            budget: BudgetSettings::default(),
        }
    }
}
```

Add the two accessor methods to `impl SettingsStore`, after `save_forwarder` (after line 93):

```rust
    pub fn budget(&self) -> BudgetSettings {
        self.inner.read().expect("settings lock").budget.clone()
    }

    pub fn save_budget(&self, new: BudgetSettings) -> Result<BudgetSettings> {
        let mut w = self.inner.write().expect("settings lock");
        w.budget = new.clone();
        let serialized = serde_json::to_string_pretty(&*w)?;
        write_atomic(&self.path, &serialized)?;
        Ok(new)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support --test settings_roundtrip`
Expected: PASS — all 7 tests, including `budget_round_trip` and `settings_without_budget_key_loads_without_backup`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings.rs src-tauri/tests/settings_roundtrip.rs
git commit -m "feat(settings): add monthly budget to AppSettings" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Budget status evaluation and projection

**Files:**
- Create: `src-tauri/src/budget/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Declare the module**

In `src-tauri/src/lib.rs`, add this line immediately after `pub mod jsonl;` (line 5):

```rust
mod budget;
```

- [ ] **Step 2: Write `budget/mod.rs` with the failing tests**

Create `src-tauri/src/budget/mod.rs`:

```rust
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
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support --lib budget`
Expected: PASS — 6 tests in `budget::tests`.

(They are written and pass together because the module is new — there is no separate "fails first" state for a brand-new file. The discipline here is that every function has a test before the next task builds on it.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/budget/mod.rs src-tauri/src/lib.rs
git commit -m "feat(budget): add budget status evaluation and projection" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Per-month alert de-dup state

**Files:**
- Modify: `src-tauri/src/budget/mod.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/budget/mod.rs`, change the top `use chrono` line:

```rust
use chrono::{DateTime, Datelike, Local, NaiveDate};
use serde::{Deserialize, Serialize};
```

In the `#[cfg(test)] mod tests` block, change its `use chrono` line:

```rust
    use chrono::{Local, NaiveDate, TimeZone};
```

Append these tests inside the `tests` module, after `evaluate_thresholds_from_day_three`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri; cargo test --features test-support --lib budget`
Expected: compile error — `AlertState`, `EvalOutcome`, `evaluate_once` not found.

- [ ] **Step 3: Implement `AlertState`, `EvalOutcome`, and `evaluate_once`**

In `src-tauri/src/budget/mod.rs`, add after the `evaluate` function (before the `#[cfg(test)]` block):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support --lib budget`
Expected: PASS — 11 tests in `budget::tests`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/budget/mod.rs
git commit -m "feat(budget): add per-month alert de-dup state" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Month-to-date cost query

**Files:**
- Modify: `src-tauri/src/db/queries.rs`
- Test: `src-tauri/tests/budget_query.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/budget_query.rs`:

```rust
mod common;

use andon_lib::db::queries::month_to_date_cost;

#[test]
fn month_to_date_cost_sums_cost_entries_in_window() {
    let (pool, _dir) = common::fixture_pool();

    // Two sessions with cost inside the window, one before it.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "in-1".into(),
            started_at_ms: Some(1_700_000_000_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 12.50,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "in-2".into(),
            started_at_ms: Some(1_700_000_500_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 7.50,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "before-window".into(),
            started_at_ms: Some(1_699_000_000_000),
            model: "claude-opus-4-7".into(),
            cost_usd: 99.0,
            ..Default::default()
        },
    );

    let conn = pool.get().expect("checkout connection");
    let total = month_to_date_cost(&conn, 1_700_000_000_000, 1_700_001_000_000)
        .expect("query month_to_date_cost");

    assert!((total - 20.0).abs() < 1e-9, "expected 20.0, got {total}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri; cargo test --features test-support --test budget_query`
Expected: compile error — `month_to_date_cost` not found in `andon_lib::db::queries`.

- [ ] **Step 3: Implement the query**

Append to `src-tauri/src/db/queries.rs`:

```rust
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri; cargo test --features test-support --test budget_query`
Expected: PASS — `month_to_date_cost_sums_cost_entries_in_window`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/queries.rs src-tauri/tests/budget_query.rs
git commit -m "feat(db): add month_to_date_cost query" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Expose budget status in `v2_kpis`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Test: `src-tauri/tests/api_budget.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/api_budget.rs`:

```rust
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    };
    (status, v)
}

#[tokio::test]
async fn v2_kpis_includes_budget_block() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) = get_json(router, "/api/v2/kpis").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let budget = &body["cost"]["budget"];
    assert!(budget.is_object(), "cost.budget must be present: {body}");
    assert_eq!(budget["monthly_usd"], json!(0.0), "default budget is 0");
    assert_eq!(
        budget["status"], json!("neutral"),
        "status is neutral with no budget set"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri; cargo test --features test-support --test api_budget`
Expected: FAIL — `cost.budget` is absent, `budget.is_object()` assertion fails.

- [ ] **Step 3: Implement the budget block in `v2_kpis`**

In `src-tauri/src/api/routes.rs`, inside `v2_kpis`, replace the inline projection (the `let projected_eom = if day_of_month > 0 { ... };` block):

```rust
    let projected_eom =
        crate::budget::project_eom(cost, day_of_month as u32, days_in_month as u32);
```

Then, immediately before the `Ok(Json(json!({` line, insert:

```rust
    let budget = state.settings.budget();
    let budget_status =
        crate::budget::evaluate(projected_eom, budget.monthly_usd, day_of_month as u32);

```

Finally, in the `"cost"` object of the returned JSON, add the `budget` field after `"days_in_month": days_in_month,`:

```rust
        "cost": {
            "current": round4(cost),
            "previous": round4(prev_cost),
            "delta_pct": delta_pct(cost, prev_cost),
            "projected_eom": round4(projected_eom),
            "day_of_month": day_of_month,
            "days_in_month": days_in_month,
            "budget": {
                "monthly_usd": budget.monthly_usd,
                "status": budget_status.as_str(),
            },
        },
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri; cargo test --features test-support --test api_budget`
Expected: PASS — `v2_kpis_includes_budget_block`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/tests/api_budget.rs
git commit -m "feat(api): expose budget status in v2 kpis" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `PUT /api/settings/budget` endpoint

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Test: `src-tauri/tests/api_budget.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/tests/api_budget.rs`, add a `put_json` helper after `get_json`:

```rust
async fn put_json(router: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    use axum::http::Method;
    let req = Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    };
    (status, v)
}
```

Append these tests to the file:

```rust
#[tokio::test]
async fn put_budget_round_trip() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (put_status, put_body) =
        put_json(router.clone(), "/api/settings/budget", json!({ "monthly_usd": 150.0 })).await;
    assert_eq!(put_status, StatusCode::OK, "PUT body: {put_body}");
    assert_eq!(put_body["monthly_usd"], json!(150.0));

    let (get_status, get_body) = get_json(router, "/api/settings").await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(
        get_body["budget"]["monthly_usd"], json!(150.0),
        "budget must persist into /api/settings"
    );
}

#[tokio::test]
async fn put_budget_rejects_negative() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _router_dir) = common::test_router(&pool);

    let (status, body) =
        put_json(router, "/api/settings/budget", json!({ "monthly_usd": -5.0 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri; cargo test --features test-support --test api_budget`
Expected: FAIL — `PUT /api/settings/budget` returns 404/405 (route not registered).

- [ ] **Step 3: Register the route**

In `src-tauri/src/api/routes.rs`, in the `router` function, add a line after `.route("/api/settings/forwarder/test", post(test_forwarder))`:

```rust
        .route("/api/settings/budget", axum::routing::put(put_budget))
```

- [ ] **Step 4: Implement the handler**

In `src-tauri/src/api/routes.rs`, append to the settings section (after the `test_forwarder` function):

```rust
#[derive(Deserialize)]
struct BudgetPayload {
    monthly_usd: f64,
}

fn validate_budget(p: &BudgetPayload) -> Result<(), String> {
    if !p.monthly_usd.is_finite() || p.monthly_usd < 0.0 {
        return Err("monthly_usd must be zero or a positive number".into());
    }
    if p.monthly_usd > 1_000_000.0 {
        return Err("monthly_usd must not exceed 1000000".into());
    }
    Ok(())
}

async fn put_budget(
    State(state): State<ApiState>,
    Json(p): Json<BudgetPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Err(msg) = validate_budget(&p) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: msg,
        });
    }
    let new = crate::settings::BudgetSettings {
        monthly_usd: p.monthly_usd,
    };
    let saved = state.settings.save_budget(new).map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("{e:#}"),
    })?;
    Ok(Json(serde_json::to_value(saved).unwrap_or_else(|_| json!({}))))
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support --test api_budget`
Expected: PASS — all 3 tests in `api_budget`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/tests/api_budget.rs
git commit -m "feat(api): add PUT /api/settings/budget" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Add the notification plugin

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/capabilities/default.json`, `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml`, add this line in the `[dependencies]` section after `tauri-plugin-single-instance = "2"`:

```toml
tauri-plugin-notification = "2"
```

- [ ] **Step 2: Grant the capability**

In `src-tauri/capabilities/default.json`, add `"notification:default"` to the `permissions` array:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability set for the main window.",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open",
    "notification:default"
  ]
}
```

- [ ] **Step 3: Register the plugin**

In `src-tauri/src/lib.rs`, in the `tauri::Builder` chain, add a line after `.plugin(tauri_plugin_opener::init())`:

```rust
        .plugin(tauri_plugin_notification::init())
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src-tauri; cargo build`
Expected: build succeeds (downloads `tauri-plugin-notification`). No behaviour change yet.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/capabilities/default.json src-tauri/src/lib.rs
git commit -m "chore(deps): add tauri-plugin-notification" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Tray icon variants

**Files:**
- Create: `src-tauri/icons/tray-neutral.png`, `tray-amber.png`, `tray-red.png`

This task produces three 64×64 PNGs by recolouring the existing brand icon
(`src-tauri/icons/icon.png` — a yellow lightning bolt on a dark square). It
uses ImageMagick v7 (`magick`). If absent on Windows: `winget install
ImageMagick.ImageMagick`, then open a fresh shell.

- [ ] **Step 1: Generate the three tray icons**

From the repo root, run:

```bash
magick src-tauri/icons/icon.png -resize 64x64 src-tauri/icons/tray-neutral.png
magick src-tauri/icons/icon.png -modulate 100,115,90 -resize 64x64 src-tauri/icons/tray-amber.png
magick src-tauri/icons/icon.png -modulate 100,130,72 -resize 64x64 src-tauri/icons/tray-red.png
```

`-modulate brightness,saturation,hue` rotates the yellow bolt's hue toward
amber (hue 90) and red (hue 72) while leaving the near-black background — which
has no saturation — unchanged.

- [ ] **Step 2: Verify the files exist and differ**

Run: `ls -l src-tauri/icons/tray-*.png`
Expected: three non-empty PNG files. Open them — `tray-amber.png` should read
clearly amber/orange and `tray-red.png` clearly red, both distinct from
`tray-neutral.png` at small size. If the hue is off, re-run Step 1 adjusting
the third `-modulate` number (lower = more red).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/icons/tray-neutral.png src-tauri/icons/tray-amber.png src-tauri/icons/tray-red.png
git commit -m "feat(tray): add amber/red budget tray icons" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: The budget-alert monitor

**Files:**
- Create: `src-tauri/src/budget/monitor.rs`
- Modify: `src-tauri/src/budget/mod.rs`, `src-tauri/src/lib.rs`

This task wires the I/O shell. Its logic is already covered by the
`evaluate_once` unit tests (Task 3); the monitor itself is verified by
compilation here and the manual smoke test in Task 15.

- [ ] **Step 1: Declare the submodule**

In `src-tauri/src/budget/mod.rs`, add this line directly below the closing line of the `//!` doc-comment block and above the `use chrono` line:

```rust
pub mod monitor;
```

- [ ] **Step 2: Add the `TRAY_ID` constant and use it**

In `src-tauri/src/lib.rs`, add after `const MAIN_WINDOW: &str = "main";`:

```rust
const TRAY_ID: &str = "andon-tray";
```

In the same file, change the tray builder line `TrayIconBuilder::with_id("andon-tray")` to:

```rust
            let _tray = TrayIconBuilder::with_id(TRAY_ID)
```

- [ ] **Step 3: Write `budget/monitor.rs`**

Create `src-tauri/src/budget/monitor.rs`:

```rust
//! The budget-alert monitor: a background task that periodically projects
//! month-end cost, repaints the tray icon, and fires desktop notifications.
//! All decision logic lives in the pure `evaluate_once`; this file is the
//! I/O shell — clock, database, tray, notifications, state file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone};
use tauri::image::Image;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::{evaluate_once, AlertState, BudgetStatus};
use crate::db::{queries, DbPool};
use crate::settings::SettingsStore;

/// How often the monitor re-evaluates the budget.
const MONITOR_INTERVAL: Duration = Duration::from_secs(30 * 60);

// Tray icon variants, baked into the binary.
const ICON_NEUTRAL: &[u8] = include_bytes!("../../icons/tray-neutral.png");
const ICON_AMBER: &[u8] = include_bytes!("../../icons/tray-amber.png");
const ICON_RED: &[u8] = include_bytes!("../../icons/tray-red.png");

/// Run the budget monitor forever. Evaluates once immediately (the first
/// `interval.tick()` resolves at once), then every `MONITOR_INTERVAL`.
pub async fn run_monitor(
    app: AppHandle,
    settings: Arc<SettingsStore>,
    pool: Arc<DbPool>,
    data_dir: PathBuf,
) {
    let state_path = data_dir.join("budget-alerts.json");
    let mut last_status: Option<BudgetStatus> = None;
    let mut interval = tokio::time::interval(MONITOR_INTERVAL);

    loop {
        interval.tick().await;
        if let Err(e) = tick(&app, &settings, &pool, &state_path, &mut last_status) {
            tracing::warn!(error = ?e, "budget monitor tick failed; will retry");
        }
    }
}

/// One evaluation cycle. Returns `Err` only for failures worth a log line;
/// side-effect failures (tray, notification, state write) are logged inside
/// their helpers and never abort the loop.
fn tick(
    app: &AppHandle,
    settings: &SettingsStore,
    pool: &DbPool,
    state_path: &Path,
    last_status: &mut Option<BudgetStatus>,
) -> anyhow::Result<()> {
    let budget = settings.budget();
    let now = Local::now();

    let conn = pool.get()?;
    let mtd_cost = queries::month_to_date_cost(&conn, month_start_ms(now), now.timestamp_millis())?;
    drop(conn); // never hold a DB connection past this point

    let prior = load_state(state_path);
    let outcome = evaluate_once(mtd_cost, budget.monthly_usd, now, prior);

    // The tray is a live gauge — repaint only when the status actually changed.
    if *last_status != Some(outcome.status) {
        apply_tray(app, outcome.status, outcome.projected_eom, budget.monthly_usd);
        *last_status = Some(outcome.status);
    }

    if let Some(level) = outcome.notify {
        fire_notification(app, level, outcome.projected_eom, budget.monthly_usd);
    }

    save_state(state_path, &outcome.next_state);
    Ok(())
}

/// Unix-ms of 00:00 local time on the first of `now`'s month.
fn month_start_ms(now: DateTime<Local>) -> i64 {
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

/// Read the persisted `AlertState`; a missing or unparseable file yields the
/// default (empty) state — never an error, mirroring `SettingsStore::load`.
fn load_state(path: &Path) -> AlertState {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(error = ?e, "budget-alerts.json unparseable; treating as empty");
            AlertState::default()
        }),
        Err(_) => AlertState::default(),
    }
}

/// Persist the `AlertState`. Failures are logged, never propagated — a missed
/// write risks at most one duplicate notification, not a crash.
fn save_state(path: &Path, state: &AlertState) {
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                tracing::warn!(error = ?e, path = %path.display(),
                    "failed to write budget-alerts.json");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "failed to serialise AlertState"),
    }
}

/// Repaint the tray icon + tooltip for `status`. All failures are logged.
fn apply_tray(app: &AppHandle, status: BudgetStatus, projected: f64, monthly_usd: f64) {
    let Some(tray) = app.tray_by_id(crate::TRAY_ID) else {
        tracing::warn!("budget monitor: tray not found; skipping repaint");
        return;
    };
    let bytes = match status {
        BudgetStatus::Neutral => ICON_NEUTRAL,
        BudgetStatus::Amber => ICON_AMBER,
        BudgetStatus::Red => ICON_RED,
    };
    match Image::from_bytes(bytes) {
        Ok(icon) => {
            if let Err(e) = tray.set_icon(Some(icon)) {
                tracing::warn!(error = ?e, "failed to set tray icon");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "failed to decode tray icon"),
    }
    if let Err(e) = tray.set_tooltip(Some(tooltip_for(status, projected, monthly_usd))) {
        tracing::warn!(error = ?e, "failed to set tray tooltip");
    }
}

fn tooltip_for(status: BudgetStatus, projected: f64, monthly_usd: f64) -> String {
    match status {
        BudgetStatus::Neutral => "andon — Claude Code dashboard".to_string(),
        BudgetStatus::Amber | BudgetStatus::Red => {
            let pct = pct_of_budget(projected, monthly_usd);
            format!("andon — {pct:.0}% of monthly budget (projected)")
        }
    }
}

/// Show a desktop notification. Failures are logged, never propagated.
fn fire_notification(app: &AppHandle, level: BudgetStatus, projected: f64, monthly_usd: f64) {
    let pct = pct_of_budget(projected, monthly_usd);
    let (title, body) = match level {
        BudgetStatus::Amber => (
            "Andon — budget warning",
            format!(
                "Projected spend ${projected:.2} is {pct:.0}% of your \
                 ${monthly_usd:.2} monthly budget."
            ),
        ),
        BudgetStatus::Red => (
            "Andon — budget exceeded",
            format!(
                "Projected spend ${projected:.2} will exceed your \
                 ${monthly_usd:.2} monthly budget."
            ),
        ),
        BudgetStatus::Neutral => return,
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = ?e, "failed to show budget notification");
    }
}

fn pct_of_budget(projected: f64, monthly_usd: f64) -> f64 {
    if monthly_usd > 0.0 {
        projected / monthly_usd * 100.0
    } else {
        0.0
    }
}
```

- [ ] **Step 4: Spawn the monitor in `lib.rs`**

In `src-tauri/src/lib.rs`, immediately before the `tauri::Builder::default()` line, add:

```rust
    let monitor_pool = pool.clone();
    let monitor_settings = settings_store.clone();
    let monitor_data_dir = paths.data_dir.clone();

```

Inside the `.setup(move |app| { ... })` closure, after the tray block's `.build(app)?;` and before the `// Start hidden` comment, add:

```rust
            // Budget-alert monitor
            let monitor_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                budget::monitor::run_monitor(
                    monitor_app,
                    monitor_settings,
                    monitor_pool,
                    monitor_data_dir,
                )
                .await;
            });

```

- [ ] **Step 5: Verify it compiles and existing tests still pass**

Run: `cd src-tauri; cargo build`
Expected: build succeeds. `include_bytes!` finds the three PNGs from Task 8.

Run: `cd src-tauri; cargo test --features test-support`
Expected: PASS — the whole Rust suite, no regressions.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/budget/monitor.rs src-tauri/src/budget/mod.rs src-tauri/src/lib.rs
git commit -m "feat(budget): add the background budget-alert monitor" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Budget types in the API client

**Files:**
- Modify: `web/src/app/core/api.service.ts`

- [ ] **Step 1: Add the types**

In `web/src/app/core/api.service.ts`, add a `BudgetStatus` type and a
`BudgetSettings` interface just before the `ForwarderSettings` interface:

```ts
export type BudgetStatus = 'neutral' | 'amber' | 'red';

export interface BudgetSettings {
  monthly_usd: number;
}
```

In the `V2Kpis` interface, add a `budget` field to the `cost` object:

```ts
  cost: {
    current: number;
    previous: number;
    delta_pct: number | null;
    projected_eom: number;
    day_of_month: number;
    days_in_month: number;
    budget: { monthly_usd: number; status: BudgetStatus };
  };
```

In the `AppSettings` interface, add the `budget` field:

```ts
export interface AppSettings {
  version: number;
  forwarder: ForwarderSettings;
  budget: BudgetSettings;
}
```

- [ ] **Step 2: Add the `saveBudget` method**

In the `ApiService` class, add this method after `saveForwarder`:

```ts
  saveBudget(b: BudgetSettings): Observable<BudgetSettings> {
    return this.http.put<BudgetSettings>(`${BASE}/api/settings/budget`, b);
  }
```

- [ ] **Step 3: Verify the frontend type-checks**

Run: `cd web; npm run build`
Expected: the Angular build succeeds with no type errors.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/core/api.service.ts
git commit -m "feat(web): add budget types to the API client" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: The budget Settings card

**Files:**
- Create: `web/src/app/features/settings/budget-card.component.ts`
- Test: `web/src/app/features/settings/budget-card.component.spec.ts`

- [ ] **Step 1: Write the failing test**

Create `web/src/app/features/settings/budget-card.component.spec.ts`:

```ts
// BudgetCardComponent tests using bare TestBed with a stubbed ApiService.
import { importProvidersFrom } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { Gauge, LucideAngularModule } from 'lucide-angular';
import { BudgetCardComponent } from './budget-card.component';
import { ApiService } from '../../core/api.service';

function setup(saveSpy: (b: { monthly_usd: number }) => unknown = (b) => of(b)) {
  const fakeApi = {
    getSettings: () =>
      of({
        version: 1,
        forwarder: { enabled: false, endpoint: '', timeout_ms: 2000, headers: {} },
        budget: { monthly_usd: 120 },
      }),
    saveBudget: saveSpy,
  };
  TestBed.configureTestingModule({
    imports: [BudgetCardComponent],
    providers: [
      { provide: ApiService, useValue: fakeApi },
      importProvidersFrom(LucideAngularModule.pick({ Gauge })),
    ],
  });
  const fixture = TestBed.createComponent(BudgetCardComponent);
  fixture.detectChanges();
  return { fixture };
}

describe('BudgetCardComponent', () => {
  it('loads the existing budget into the input', () => {
    const { fixture } = setup();
    const input: HTMLInputElement =
      fixture.nativeElement.querySelector('input[type="number"]');
    expect(input.value).toBe('120');
  });

  it('save() sends the entered budget to the API', () => {
    const sent: number[] = [];
    const { fixture } = setup((b) => {
      sent.push(b.monthly_usd);
      return of(b);
    });
    const cmp = fixture.componentInstance;
    cmp.monthly.set(250);
    cmp.dirty.set(true);
    cmp.save();
    expect(sent).toEqual([250]);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd web; npm test -- budget-card.component`
Expected: FAIL — cannot resolve `./budget-card.component`.

- [ ] **Step 3: Implement the component**

Create `web/src/app/features/settings/budget-card.component.ts`:

```ts
import { CommonModule } from '@angular/common';
import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-budget-card',
  standalone: true,
  imports: [CommonModule, FormsModule, LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
  <section class="panel" id="budget">
    <div class="panel-title">
      <span class="flex items-center gap-1.5">
        <lucide-icon name="gauge" class="w-3.5 h-3.5"></lucide-icon>Monthly budget
      </span>
    </div>
    <div class="panel-body">
      <p class="text-[12px] text-muted mb-3">
        Set a monthly cost budget. Andon shifts the tray icon to amber at 80% and
        red at 100% of the projected end-of-month spend, and sends one desktop
        notification per threshold. Set to 0 to disable.
      </p>
      <div class="flex items-end gap-3">
        <label class="text-[11px] font-mono">
          <span class="block text-muted mb-1">monthly budget (USD)</span>
          <input class="w-40 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono"
                 type="number" min="0" max="1000000" step="1"
                 [(ngModel)]="monthlyModel" (ngModelChange)="dirty.set(true)" />
        </label>
        <button class="filter-chip" [disabled]="!dirty()" (click)="save()"
                [attr.data-active]="dirty() ? 'true' : null">save</button>
        @if (msg()) {
          <span class="text-[11px] font-mono pb-1"
                [class.text-accent]="ok()" [class.text-err]="!ok()">{{ msg() }}</span>
        }
      </div>
    </div>
  </section>
  `,
})
export class BudgetCardComponent implements OnInit {
  private api = inject(ApiService);

  monthly = signal(0);
  dirty = signal(false);
  msg = signal('');
  ok = signal(false);

  get monthlyModel(): number {
    return this.monthly();
  }
  set monthlyModel(v: number) {
    this.monthly.set(v);
  }

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.monthly.set(s.budget.monthly_usd);
      this.dirty.set(false);
    });
  }

  save() {
    this.api.saveBudget({ monthly_usd: Number(this.monthly()) }).subscribe({
      next: () => {
        this.flash('saved', true);
        this.dirty.set(false);
      },
      error: (e) => this.flash(`error: ${e?.error?.error ?? e.message ?? 'failed'}`, false),
    });
  }

  private flash(text: string, ok: boolean) {
    this.msg.set(text);
    this.ok.set(ok);
    setTimeout(() => {
      this.msg.set('');
      this.ok.set(false);
    }, 4000);
  }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web; npm test -- budget-card.component`
Expected: PASS — both tests in `BudgetCardComponent`.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/settings/budget-card.component.ts web/src/app/features/settings/budget-card.component.spec.ts
git commit -m "feat(web): add the budget settings card" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Mount the budget card in Settings

**Files:**
- Modify: `web/src/app/features/settings/settings.component.ts`, `settings.component.html`

- [ ] **Step 1: Import the component**

In `web/src/app/features/settings/settings.component.ts`, add the import after the `ForwarderCardComponent` import:

```ts
import { BudgetCardComponent } from './budget-card.component';
```

Add `BudgetCardComponent` to the `imports` array of the `@Component` decorator:

```ts
  imports: [CommonModule, DecimalPipe, RouterLink, LucideAngularModule, ForwarderCardComponent, BudgetCardComponent],
```

- [ ] **Step 2: Place the card and a TOC entry**

In `web/src/app/features/settings/settings.component.html`, add a TOC link
after the Forwarder link (the `<a href="#forwarder">…</a>` block):

```html
    <a href="#budget" class="block px-3 py-1.5 font-mono text-[11px] text-muted hover:text-text border-l border-border flex items-center gap-1.5">
      <lucide-icon name="gauge" class="w-3 h-3"></lucide-icon>Budget
    </a>
```

Add the card itself immediately after `<app-forwarder-card></app-forwarder-card>`:

```html
    <!-- BUDGET -->
    <app-budget-card></app-budget-card>
```

- [ ] **Step 3: Verify the build**

Run: `cd web; npm run build`
Expected: the Angular build succeeds.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/features/settings/settings.component.ts web/src/app/features/settings/settings.component.html
git commit -m "feat(web): wire the budget card into Settings" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: Budget indicator on the Overview cost tile

**Files:**
- Create: `web/src/app/features/overview/budget-indicator.ts`
- Test: `web/src/app/features/overview/budget-indicator.spec.ts`
- Modify: `web/src/app/features/overview/overview.component.ts`, `overview.component.html`

- [ ] **Step 1: Write the failing test**

Create `web/src/app/features/overview/budget-indicator.spec.ts`:

```ts
import { budgetTextClass, budgetBarClass } from './budget-indicator';

describe('budget-indicator', () => {
  it('maps status to a text-colour class', () => {
    expect(budgetTextClass('neutral')).toBe('text-muted');
    expect(budgetTextClass('amber')).toBe('text-warn');
    expect(budgetTextClass('red')).toBe('text-err');
  });

  it('maps status to a bar background class', () => {
    expect(budgetBarClass('neutral')).toBe('bg-muted');
    expect(budgetBarClass('amber')).toBe('bg-warn');
    expect(budgetBarClass('red')).toBe('bg-err');
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd web; npm test -- budget-indicator`
Expected: FAIL — cannot resolve `./budget-indicator`.

- [ ] **Step 3: Implement the helper module**

Create `web/src/app/features/overview/budget-indicator.ts`:

```ts
import type { BudgetStatus } from '../../core/api.service';

/** Tailwind text-colour class for a budget status. */
export function budgetTextClass(status: BudgetStatus): string {
  switch (status) {
    case 'red':
      return 'text-err';
    case 'amber':
      return 'text-warn';
    default:
      return 'text-muted';
  }
}

/** Tailwind background-colour class for the budget progress bar. */
export function budgetBarClass(status: BudgetStatus): string {
  switch (status) {
    case 'red':
      return 'bg-err';
    case 'amber':
      return 'bg-warn';
    default:
      return 'bg-muted';
  }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web; npm test -- budget-indicator`
Expected: PASS — both tests in `budget-indicator`.

- [ ] **Step 5: Expose the helpers on the component**

In `web/src/app/features/overview/overview.component.ts`, add the import after
the `TopReposTileComponent` import:

```ts
import { budgetTextClass, budgetBarClass } from './budget-indicator';
```

Add these two fields to the `OverviewComponent` class, next to the existing
`Math = Math;` line:

```ts
  budgetTextClass = budgetTextClass; // template access
  budgetBarClass = budgetBarClass; // template access
```

- [ ] **Step 6: Render the indicator**

In `web/src/app/features/overview/overview.component.html`, replace the
"Projected EOM" block:

```html
          <div class="mt-2 text-[11px] text-muted">
            Projected EOM · <span class="font-mono text-text">${{ k.cost.projected_eom | number : '1.2-2' }}</span> at current pace
          </div>
```

with the same line plus the budget indicator:

```html
          <div class="mt-2 text-[11px] text-muted">
            Projected EOM · <span class="font-mono text-text">${{ k.cost.projected_eom | number : '1.2-2' }}</span> at current pace
          </div>
          @if (k.cost.budget.monthly_usd > 0) {
            <div class="mt-2">
              <div class="flex items-center justify-between text-[11px]">
                <span class="text-muted">Budget</span>
                <span class="font-mono" [class]="budgetTextClass(k.cost.budget.status)">
                  ${{ k.cost.projected_eom | number : '1.2-2' }} / ${{ k.cost.budget.monthly_usd | number : '1.2-2' }}
                  · {{ k.cost.projected_eom / k.cost.budget.monthly_usd | percent : '1.0-0' }}
                </span>
              </div>
              <div class="mt-1 h-1 bg-border rounded-sm overflow-hidden">
                <div class="h-full" [class]="budgetBarClass(k.cost.budget.status)"
                     [style.width.%]="Math.min(100, k.cost.projected_eom / k.cost.budget.monthly_usd * 100)"></div>
              </div>
            </div>
          }
```

(`PercentPipe` and `DecimalPipe` are already imported in `overview.component.ts`;
`Math` is already exposed on the component.)

- [ ] **Step 7: Verify the build**

Run: `cd web; npm run build`
Expected: the Angular build succeeds.

- [ ] **Step 8: Commit**

```bash
git add web/src/app/features/overview/budget-indicator.ts web/src/app/features/overview/budget-indicator.spec.ts web/src/app/features/overview/overview.component.ts web/src/app/features/overview/overview.component.html
git commit -m "feat(web): show budget status on the Overview cost tile" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: Documentation

**Files:**
- Modify: `docs/features.md`, `docs/architecture.md`

- [ ] **Step 1: Document the Overview indicator**

In `docs/features.md`, under the `## Overview` section, add a sentence about
the budget indicator on the cost tile:

```markdown
When a monthly budget is set (Settings → Monthly budget), the cost tile also
shows projected spend as a percentage of that budget, with a progress bar that
turns amber at 80% and red at 100%.
```

- [ ] **Step 2: Document the Settings card**

In `docs/features.md`, under the `## Settings` section, add:

```markdown
**Monthly budget** — a monthly cost budget in USD. When the projected
end-of-month spend crosses 80% / 100% of it, Andon shifts the tray icon to
amber / red and fires one desktop notification per threshold per month. Set to
0 to disable. Alerts are suppressed for the first two days of each month, when
the projection is too volatile to trust.
```

- [ ] **Step 3: Document the monitor and data file**

In `docs/architecture.md`, under the `## Process model` section, add:

```markdown
A **budget monitor** task wakes every 30 minutes (and once at startup). It
projects month-end cost from `cost_entries`, compares it to the user's monthly
budget, repaints the tray icon, and fires desktop notifications. Notification
de-dup state is persisted to `budget-alerts.json` in the data directory; the
budget amount itself lives in `settings.json` under the `budget` key.
```

- [ ] **Step 4: Verify the prose**

Read both edited sections to confirm the additions fit the surrounding text
and use US English. Adjust wording to match the document's voice if needed.

- [ ] **Step 5: Commit**

```bash
git add docs/features.md docs/architecture.md
git commit -m "docs: document budget alerts" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: Full verification and manual smoke test

**Files:** none — verification only.

- [ ] **Step 1: Run the full Rust suite**

Run: `cd src-tauri; cargo test --features test-support`
Expected: PASS — entire suite, no regressions.

- [ ] **Step 2: Run the full Angular suite**

Run: `cd web; npm test`
Expected: PASS — entire suite, including `budget-card.component` and `budget-indicator`.

- [ ] **Step 3: Manual smoke test**

Run `cargo tauri dev` from the repo root. Then:

1. Open Settings → **Monthly budget**. Enter a budget low enough that the
   current month's projection is already over 100% of it (check the Overview
   "Projected EOM" figure first). Click **save**.
2. Within 30 minutes — or restart the app for an immediate re-evaluation — the
   tray icon should turn **red** and a "budget exceeded" desktop notification
   should appear. (On Windows, native notifications are most reliable from an
   installed build; `cargo tauri dev` may behave inconsistently — note any
   difference but do not block on it.)
3. Open the Overview page: the cost tile shows the **Budget** line and progress
   bar, tinted red.
4. Restart the app — the tray should go red again with **no** repeat
   notification (de-dup via `budget-alerts.json`).
5. Set the budget back to **0** and save; after the next tick the tray returns
   to neutral.

- [ ] **Step 4: Confirm the Definition of Done**

Re-read `CONTRIBUTING.md` → Definition of Done and confirm each item holds for
this branch (tests pass, US English, Conventional Commits, no `unwrap`/`expect`
outside `main.rs` setup, privacy guarantees intact).

- [ ] **Step 5: Finish the branch**

The implementation is complete. Use the `superpowers:finishing-a-development-branch`
skill to decide how to integrate `feature/budget-alerts` (squash-merge via PR
into `dev`, per `CLAUDE.md`).

---

## Self-review notes

- **Spec coverage:** every spec section maps to a task — settings (T1), pure
  logic incl. `evaluate`/`project_eom`/`evaluate_once` (T2–T3), `month_to_date_cost`
  (T4), `v2_kpis` block (T5), budget endpoint (T6), notification plugin (T7),
  tray assets (T8), monitor + lib wiring (T9), frontend types (T10), Settings
  card (T11–T12), Overview indicator (T13), docs (T14), verification (T15). All
  9 spec tests are present: T2 (tests 1–2 of the spec — projection/evaluate),
  T3 (test 3 — `evaluate_once`), T1 (tests 4–5 — settings + the `#[serde(default)]`
  regression), T5/T6 (tests 6–7 — API), T11/T13 (tests 8–9 — Angular). The
  `month_to_date_cost` query gains its own test (T4) for TDD completeness.
- **Build order:** matches the spec's build-order table; `month_to_date_cost`
  (T4) precedes the monitor (T9) that calls it.
- **Type consistency:** `BudgetStatus` (Rust enum, `as_str()` → `"neutral"`/
  `"amber"`/`"red"`) lines up with the TS `BudgetStatus` union; `BudgetSettings`
  / `AppSettings.budget` / `V2Kpis.cost.budget` match between `settings.rs`,
  `routes.rs`, and `api.service.ts`; `evaluate_once` returns `EvalOutcome`
  consistently in T3 and consumed by `monitor::tick` in T9.
