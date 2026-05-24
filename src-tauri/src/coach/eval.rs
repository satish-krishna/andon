//! Re-evaluator entry points (SessionEnd hook + JSONL backfill batch end).

use std::sync::Arc;
use tracing::instrument;

use crate::coach::rules::{DbPool, Window};
use crate::coach::Result;
use crate::settings::CoachSettings;

const DEFAULT_WINDOW_DAYS: i64 = 30;

/// Evaluate the last 30-day window. Called after a SessionEnd event.
/// The `session_id` is accepted for tracing context but the window is always
/// the trailing 30 days (rules are window-scoped, not session-scoped).
#[instrument(skip(pool, settings))]
pub fn evaluate_session(pool: &Arc<DbPool>, _session_id: &str, settings: &CoachSettings) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window {
        from_ms: now - DEFAULT_WINDOW_DAYS * 86_400_000,
        to_ms: now + 1,
        models: None,
    };
    crate::coach::engine::evaluate_window(pool, &win, settings)
}

/// Evaluate an explicit time window. Called after a JSONL backfill batch.
#[instrument(skip(pool, settings))]
pub fn evaluate_window(
    pool: &Arc<DbPool>,
    from_ms: i64,
    to_ms: i64,
    settings: &CoachSettings,
) -> Result<()> {
    let win = Window { from_ms, to_ms, models: None };
    crate::coach::engine::evaluate_window(pool, &win, settings)
}
