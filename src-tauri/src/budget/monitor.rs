//! The budget-alert monitor: a background task that periodically projects
//! month-end cost, repaints the tray icon, and fires desktop notifications.
//! All decision logic lives in the pure `evaluate_once`; this file is the
//! I/O shell — clock, database, tray, notifications, state file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use tauri::image::Image;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use super::{evaluate_once, month_start_ms, AlertState, BudgetStatus};
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
    // After a system sleep, skip missed ticks rather than firing a catch-up
    // burst of redundant evaluations.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

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

    // A single small SUM query on a 30-minute cadence — run inline rather
    // than via spawn_blocking; the runtime stall is sub-millisecond and the
    // connection is dropped before this fn returns (no await is ever crossed).
    let conn = pool.get()?;
    let mtd_cost = queries::month_to_date_cost(&conn, month_start_ms(now), now.timestamp_millis())?;
    drop(conn);

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

/// Read the persisted `AlertState`; a missing or unparseable file yields the
/// default (empty) state — never an error, mirroring `SettingsStore::load`.
fn load_state(path: &Path) -> AlertState {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(error = ?e, "budget-alerts.json unparseable; treating as empty");
            AlertState::default()
        }),
        // A missing file on first run is expected; any other read error
        // (permissions, disk) is worth a log line before falling back.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AlertState::default(),
        Err(e) => {
            tracing::warn!(error = ?e, "failed to read budget-alerts.json; treating as empty");
            AlertState::default()
        }
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
