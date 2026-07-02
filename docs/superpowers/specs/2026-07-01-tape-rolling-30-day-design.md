# Tape: rolling-30-day ribbon with window highlighting

**Date:** 2026-07-01
**Status:** Approved design, pending spec review
**Scope:** Overview page "tape" visualization + its backend read path

## Problem

The Overview "tape" is a two-row calendar strip: current month on top, previous
month below, each bar a day-of-month cost. It ignores the on-screen date filter
entirely (fetched as `api.tape(undefined, models)` — the `month` param is always
empty) and only the current-month row carries a cost-on-hover tooltip; the
previous-month row is bare bars.

Two gaps:

1. The previous-month row has no cost tooltip — hovering it shows nothing, an
   asymmetry with the current-month row.
2. The tape does not react to the date filter beyond highlighting a single
   selected day.

## Decision

Replace the two-row calendar layout with **one rolling 30-day ribbon** ending
today, and highlight the days inside the active filter window.

Decisions made during brainstorming, in order:

- **Highlight, not re-anchor.** The tape reflects the filter by lighting bars,
  not by changing which period it shows. (Later superseded by the rolling-strip
  decision, but the principle — filter drives *highlighting* — holds.)
- **Single 30-day strip, no comparison row.** The month-over-month comparison is
  dropped. One recency ribbon.
- **Exact local dates.** Each bar is a real local calendar date. Backend
  bucketing is already local-timezone (`Local.timestamp_millis_opt(...).day()`),
  so "exact dates" needs no UTC conversion and carries no asterisk.
- **Lane B — backend rolling window.** The endpoint returns an explicit 30-point
  dated series ending today, rather than the frontend stitching `previous[]` +
  `current[]` client-side. Chosen so the ribbon is always exactly 30 correct
  days, including the short-previous-month boundary (e.g., early March behind
  February) where client stitching would underflow the two-month payload.

## Behavior

- One horizontal strip of **30 bars**, oldest → today. Today is always the last
  bar.
- Each bar is one exact local date.
- A bar is **lit** when its date falls inside the active filter window
  (`filter.window()` → `{ fromMs, toMs }`), **dimmed** otherwise. Overlap test:
  `barDayStartMs < toMs && barDayEndMs > fromMs`.
- Hover any bar → tooltip showing the exact date (`2026-07-01`) and `$cost`.
- Click a bar → selects that single day via the existing `filter.selectDay(date)`
  path, which sets a custom single-day range (and thus lights just that bar).
- Empty data (no cost in the last 30 days) → all bars zero-height; scaling
  guarded by `Math.max(1, ...)` as today.

## Backend

File: `src-tauri/src/api/routes.rs` (handler `v2_tape`, query `tape_for_month`).

- **Reshape `/api/v2/tape`.** Drop the `month` param (dead — the only caller
  passes it empty). Keep the `models` LIKE filter unchanged. Accept an optional
  `days` param defaulting to 30, clamped 1–365, matching the `cost-by-day`
  sibling convention.
- **Reuse existing rolling-window helpers** (`routes.rs:669–698`):
  `last_n_days_bounds(days)`, `day_labels(days)`, `day_index_for(ts_ms, days)`.
  These already bucket by local day and emit `YYYY-MM-DD` labels;
  `/api/overview/cost-by-day` exercises them. The new query = that windowing plus
  the tape's existing `model_clause("model")` WHERE clause. One SQL pass over
  `cost_entries`, same cost as today.
- **Typed serde DTO** replaces the hand-rolled `json!` blob (aligns with the
  repo's "serde for every JSON payload" rule):

  ```rust
  #[derive(Serialize)]
  struct TapePoint { date: String, cost: f64 } // date = "YYYY-MM-DD", local

  #[derive(Serialize)]
  struct TapeResponse { days: Vec<TapePoint> }  // exactly `days` points, oldest → today
  ```

  Today is `days.last()` by construction; no separate `today_day` field.
- Keep `#[tracing::instrument]` on the public async handler, `anyhow::Result` at
  the boundary, no `unwrap()`/`expect()` (repo Rust rules).

### The one non-reused piece

`cost-by-day` returns per-model breakdowns and has **no** model filter, so it
cannot be reused wholesale. The new tape query keeps the tape's own
`LIKE`-substring `model_clause`. Only the day-windowing helpers are shared.

## Frontend

Angular SPA under `web/src/app/features/overview`.

- `core/api.service.ts` — replace the `V2Tape` interface with
  `{ days: { date: string; cost: number }[] }`; `tape()` drops the `month`
  argument, keeps `models`.
- `overview.component.ts` — retype the `tape` signal; collapse `tapeMax` and
  `prevMax` into a single `tapeMax` over the 30 costs; add a per-bar
  `inWindow(date)` predicate driven by `filter.window()`.
- `overview.component.html` — one `@for` row of 30 bars; move the existing
  current-row tooltip block onto it; bind a lit/dim class to `inWindow`.

### Deletions

- `previous[]` and its entire row.
- `prevMax` computed.
- `tape-selection.ts` `selectedTapeDay()` / `tapeDayDate()` day-of-month math,
  replaced by real-date comparison.
- `month` param plumbing end to end.

Net result: less code than the starting point.

## Testing (TDD — failing test first)

- **Rust** (`cargo test --features test-support`): unit test on the new query.
  Seed `cost_entries` across a month boundary and specifically across a
  February → March boundary (the case that motivated Lane B). Assert exactly 30
  dated points ending today, correct local-day sums, and correct model
  filtering.
- **Angular** (Vitest): test `inWindow` for a window that covers a sub-slice of
  the 30 days; assert the 30-bar render and the tooltip content.

## Privacy & scope

Read-only aggregate over `cost_entries`. No new listener, no outbound call, no
prompt data, still bound to `127.0.0.1`. None of the privacy rules in
`docs/architecture.md` are touched.

## Out of scope

- Configurable window length in the UI (the `days` param exists on the endpoint
  but the UI hardcodes 30).
- Month-over-month comparison (explicitly dropped).
- Any change to the OTLP receivers or `~/.claude/settings.json` patching.
