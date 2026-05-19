# JSONL gap-filling for OTLP-partial sessions — Design (Plan C+)

> Status: draft 2026-05-19 · author: SatishKrishna Pilla
> Builds on: `docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md`
> Depends on: PR #9 (`feature/jsonl-ingest`) — the JSONL ingest pipeline this spec extends.

## Motivation

When an enterprise process rewrites `~/.claude/settings.json` and removes Andon's OTel env vars partway through a Claude Code session, OTLP stops flowing. The session continues; the JSONL transcript on disk still contains every turn. Today, the reconciler treats *any* OTLP `token_usage` row as proof of full OTLP coverage and refuses to write JSONL-derived tokens or cost for that session. The result: the dashboard underreports tokens and cost for any session that lost OTLP mid-flight.

This spec replaces the binary reconciler with per-turn deduplication so JSONL fills exactly the rows OTLP missed, without ever overwriting an OTLP row.

## Non-goals

- No new tables, no new migration. Schema is stable at v4.
- No change to `tool_decisions` ingest behaviour (mixing the 'invoke' sentinel with 'accept/reject/abort' values complicates accept-rate dashboards). JSONL still only writes `tool_decisions` rows for fully JSONL-only sessions.
- No automatic detection / alerting for OTLP-partial sessions. Gap-filling happens during user-triggered backfill (Settings → "Ingest JSONL history") and during the `SessionEnd` hook — same surfaces as today.
- No destructive operations: this design never deletes OTLP rows.
- No retroactive backfill of `slash_commands` / `subagent_calls` for OTLP-covered sessions where those rows might already have been recorded by some other mechanism. Idempotency of those two tables is addressed as a small bonus (existing latent Plan C bug) but the design point is gap-filling token/cost.

## Architecture

### The change in one sentence

Replace the binary `Coverage::{Otlp, JsonlOnly}` reconciler with a per-row dedup rule that lets JSONL write any `token_usage` / `cost_entries` row a session is missing.

### The dedup rule

For every JSONL-derived `TokenUsage` and `CostEntry` event, write iff no existing row matches:

| Table | Match key |
|---|---|
| `token_usage` | `session_id` AND `model` AND `token_type` AND `ABS(timestamp - ts) <= window_ms` |
| `cost_entries` | `session_id` AND `model` AND `ABS(timestamp - ts) <= window_ms` |

`window_ms` is hardcoded at **5_000 ms**. Justification: OTLP `api_request` events and JSONL `assistant` records are both emitted by the same Claude Code process immediately after the API response returns. Real-world drift is single-digit milliseconds; 5 s is generous slack for write batching and clock granularity without false-positive matches across distinct turns (the shortest plausible inter-turn interval is many seconds).

### Reconciler API

`src-tauri/src/jsonl/reconciler.rs`:

```rust
// Existing — retained for `sessions.data_source` flip logic on lifecycle insert.
pub fn coverage_for(pool: &Arc<DbPool>, session_id: &str) -> Result<Coverage>;
pub enum Coverage { Otlp, JsonlOnly }

// New.
pub fn token_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    token_type: &str,
    window_ms: i64,
) -> bool;

pub fn cost_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    window_ms: i64,
) -> bool;
```

The two new helpers do a single `SELECT 1 FROM ... LIMIT 1` with a `BETWEEN ts - window AND ts + window` clause. The existing `idx_token_session(session_id, timestamp)` index serves the query directly.

### Ingestor changes

In `Ingestor::ingest_derived` (`src-tauri/src/otlp/ingestor.rs`):

- The `TokenUsage` arm drops the `if matches!(coverage, Coverage::JsonlOnly)` guard. Instead, for each non-zero `(token_type, n)` pair it calls `token_row_already_covered(...)` and inserts only if `false`.
- The `CostEntry` arm drops the same guard and calls `cost_row_already_covered(...)` before insert.
- The `Coverage` parameter is retained on the function signature — it's still consulted by the `SessionLifecycle` arm to decide whether to flip `data_source` to `'mixed'`.
- The `SlashCommand` and `SubAgentCall` arms get a `WHERE NOT EXISTS` guard against `(session_id, timestamp, command_name)` / `(parent_session_id, started_at, COALESCE(subagent_type, ''))` respectively, so repeated backfill runs are truly idempotent across all tables. `subagent_type` is the dedup key (not `child_session_id`) because the JSONL `Task` tool input reliably carries `subagent_type` but does not always carry `session_id`. (Fixes a latent Plan C bug.)

### Stats and observability

`IngestStats` (in `src-tauri/src/jsonl/mod.rs`) gains two fields:

```rust
pub struct IngestStats {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub duration_ms: i64,
    pub tokens_filled: i64,   // NEW: token_usage rows written
    pub cost_filled: i64,     // NEW: cost_entries rows written
}
```

`Ingestor::ingest_derived` returns a `(tokens_filled, cost_filled)` tuple that the caller in `ingest_one_inner` accumulates into `IngestStats`.

When `tokens_filled + cost_filled > 0` for a session, log one structured `tracing::info!` line: `session_id`, model breakdown, totals filled. Logged to the existing daily rolling file via the configured tracing layer — useful for grepping affected sessions in the enterprise-strips-hooks scenario.

### Data flow (end-to-end)

```
User clicks "Ingest JSONL history" in Settings
     │
     ▼
POST /api/jsonl/backfill
     │
     ▼
jsonl::backfill — walker enumerates ~/.claude/projects/<slug>/*.jsonl
     │
     ▼
for each transcript file → ingest_one_inner (in tokio blocking task)
     │
     ▼
parser::for_each_record → reducer::reduce → DerivedEvent stream
     │
     ▼
reconciler::coverage_for(session_id)         ┐
     ├─── used for sessions.data_source flip │ unchanged
ingestor::ingest_derived(events, coverage)   ┘
     │
     ├─── TokenUsage:  token_row_already_covered(...)? skip : INSERT
     ├─── CostEntry:   cost_row_already_covered(...)?  skip : INSERT
     ├─── ToolCall:    if coverage == JsonlOnly: INSERT     (unchanged)
     ├─── SlashCommand: WHERE NOT EXISTS (...) INSERT       (idempotency fix)
     ├─── SubAgentCall: WHERE NOT EXISTS (...) INSERT       (idempotency fix)
     └─── SessionLifecycle: INSERT OR IGNORE, flip data_source if mixed
     │
     ▼
return IngestStats with tokens_filled / cost_filled populated
     │
     ▼
JsonlBackfillResponse → Angular toast: "Ingested N records; filled K gap rows."
```

## Schema and migrations

**No changes.** Migration v4 is stable.

## API surface

No new endpoints. Existing `POST /api/jsonl/backfill` response gains two integer fields:

```diff
 {
   "files_processed": 15,
   "records_processed": 9502,
   "records_errored": 156,
   "sessions_added": 11,
+  "tokens_filled": 247,
+  "cost_filled": 42,
   "duration_ms": 792
 }
```

`GET /api/jsonl/ingest-runs` rows do **not** persist the new fields. Rationale: the run-level history table is for audit, not metrics. If those fields ever become user-facing on the Diagnostics page, a migration can add them later.

## UI changes

One change: the toast in `web/src/app/features/settings/settings.component.ts` becomes:

```ts
const summary = `Ingested ${s.records_processed} records from ${s.files_processed} files`;
const tail = s.tokens_filled + s.cost_filled > 0
  ? ` · filled ${s.tokens_filled + s.cost_filled} gap rows from JSONL`
  : '';
this.jsonlToast.set(summary + tail + ` (${s.records_errored} errors).`);
```

`JsonlBackfillResponse` in `web/src/app/core/models.ts` gains the two fields.

No new components, no new routes, no nav changes.

## Error handling

- Per-row dedup happens inside the same transaction as the inserts. If the transaction commits, all dedup decisions are consistent with the row state at commit time.
- A `pool.get()` failure during dedup-check defaults to "covered" (false), causing the row to be skipped. This is conservative — we'd rather miss a gap than risk double-counting.
- The existing `catch_unwind` wrapper in `ingest_one_inner` still catches reducer panics; the new dedup helpers are pure SQL and not panic-prone, but they're inside the same boundary.
- No new error variants on the public API. Failures still surface as the existing 500 with `{"error": "..."}`.

## Testing

Three new tests, all in existing files.

1. **`token_row_already_covered_within_5s_window`** — unit, in `src-tauri/src/jsonl/reconciler.rs`. Seed one row at `ts=10_000`, model=`'m'`, token_type=`'input'`. Assert:
   - `covered(ts=11_000, model='m', token_type='input')` → true
   - `covered(ts=16_000, model='m', token_type='input')` → false
   - `covered(ts=10_000, model='other', token_type='input')` → false
   - `covered(ts=10_000, model='m', token_type='output')` → false

2. **`gap_fills_when_otlp_partial`** — integration, in `src-tauri/tests/jsonl_ingest_writes.rs`. Seed session `s1` with one OTLP `token_usage` row at `t=100ms`. Build JSONL events with two turns: one at `t=100ms` (overlap) and one at `t=10_000ms` (gap). Call `ingest_derived`. Assert:
   - Original OTLP row preserved (unchanged).
   - The `t=10_000ms` JSONL turn's token rows wrote (one per non-zero token_type).
   - The `t=100ms` JSONL turn wrote nothing.
   - `sessions.data_source` for `s1` is `'mixed'`.

3. **Strengthened `backfill_is_idempotent`** — in `src-tauri/tests/jsonl_pipeline.rs`. Write a transcript with one assistant turn (tokens + cost), call `backfill` twice, assert `token_usage` and `cost_entries` row counts are identical after the second run.

**Existing test repurposed:** `skips_token_usage_when_otlp_covered` in `tests/jsonl_ingest_writes.rs` becomes `dedups_token_usage_against_otlp_within_window`. Semantic shift: instead of asserting "no JSONL tokens at all for OTLP-covered sessions", it asserts "no JSONL tokens at the matching timestamp". The test scenario stays nearly identical — same OTLP setup, same JSONL event — only the assertion text and the expected row count change (still 0 when timestamps overlap exactly).

**Privacy proptest untouched.** The reducer doesn't change; the trust boundary is structurally preserved.

**No new web tests.** The toast string change is mechanical and the existing `app.component.spec.ts` icon-registry smoke test continues to pass.

## Files touched (build order)

| # | File | Change |
|---|---|---|
| 1 | `src-tauri/src/jsonl/reconciler.rs` | Add `token_row_already_covered` + `cost_row_already_covered`. Add unit test. |
| 2 | `src-tauri/src/jsonl/mod.rs` | Add `tokens_filled` and `cost_filled` to `IngestStats`. Accumulate the counts from `ingest_derived` into `IngestStats` in `ingest_one_inner`. Emit `tracing::info!` line when filled > 0. |
| 3 | `src-tauri/src/otlp/ingestor.rs` | Update `ingest_derived` to return `(tokens_filled, cost_filled)`, use per-row dedup in `TokenUsage` and `CostEntry` arms, add `WHERE NOT EXISTS` guards for `SlashCommand` and `SubAgentCall`. |
| 4 | `src-tauri/src/api/dto.rs` | Add `tokens_filled` and `cost_filled` to `JsonlBackfillResponse`. `From<IngestStats>` propagates them. |
| 5 | `src-tauri/tests/jsonl_ingest_writes.rs` | Rename existing test, add `gap_fills_when_otlp_partial`. |
| 6 | `src-tauri/tests/jsonl_pipeline.rs` | Strengthen `backfill_is_idempotent`. |
| 7 | `web/src/app/core/models.ts` | Add the two fields to `JsonlBackfillResponse`. |
| 8 | `web/src/app/features/settings/settings.component.ts` | Update toast string. |
| 9 | `docs/architecture.md` | If the PR #9 work adds an OTLP-vs-JSONL routing paragraph, update it to describe the per-row dedup behaviour. Otherwise no change. |

**Not changing:** the v4 migration, the proptest privacy test, any new web routes or components, `README.md` (the "Retroactive: Yes" claim still holds — gets stronger), `docs/pitch.md`, the `SessionEnd` hook handler.

## Risks

- **Window too tight (5 s).** If real-world drift between OTLP emit and JSONL write ever exceeds 5 s, we'd double-write. Mitigation: the per-row log line surfaces every fill, so a sudden spike in `tokens_filled` for previously-quiet sessions is grep-detectable. If observed, widen the constant.
- **Window too wide (5 s).** If a session had two turns within 5 s of each other for the same model+token_type and OTLP captured both, JSONL would think the second was already covered. Mitigation: turns within 5 s are vanishingly rare in practice (round-trip API latency alone is usually >1 s, often >3 s), and the worst outcome is one missed fill — never a double-count.
- **Idempotency guard cost.** Adding `WHERE NOT EXISTS` to `slash_commands` and `subagent_calls` inserts adds a small per-row cost. Given the volumes (hundreds of slash commands across thousands of sessions), the cost is negligible against existing indexes.

## Out of scope (deferred)

- Per-session "this session was repaired" badge in the UI.
- Programmatic detection of OTLP-partial sessions (e.g., "JSONL totals > OTLP totals by > 10%") with a dedicated diagnostic surface.
- Backfilling `file_changes` from JSONL `tool_use` Edit/Write inputs.
- Backfilling `active_time` from JSONL turn intervals.

These can land in future iterations without disturbing this design — none of them require schema changes either.
