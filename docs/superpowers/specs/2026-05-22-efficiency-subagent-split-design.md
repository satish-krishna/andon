# Efficiency page — subagent split — Design

> Status: draft 2026-05-22 · author: SatishKrishna Pilla
> Branch: `feature/efficiency-page` (extends the pending PR #28; same feature, same merge).
> **Extends** [`2026-05-22-cost-efficiency-page-design.md`](2026-05-22-cost-efficiency-page-design.md) — the just-built Efficiency page hid subagent activity behind dominant-family-per-session attribution. This spec restores it.

## Motivation

The Efficiency page's model cost-efficiency table buckets every session by its
*dominant* model family. A session run on Opus as the main agent with a Haiku
subagent for grep work shows up as one row: `opus`. The Haiku spend is in the
data — it is bucketed into the Opus row.

This is the behaviour issue #20 explicitly asked for ("an Opus job vs a Haiku
job"). But the consequence, seen on real data, is that *subagent efficiency is
invisible*. A user who relies on cheap-model subagents to keep cost down cannot
see whether the strategy is working.

The needed view: a single table whose rows are `(family × role)`, where role is
`main` or `subagent`. A Haiku subagent's spend lands in an `haiku / subagent`
row, not in the Opus row of its parent session.

## What the data already supports

Claude Code stamps subagent JSONL records with `isSidechain: true` and nests
them under `<session>/subagents/agent-*.jsonl`. Andon's walker already recurses
into those directories, so the records get ingested — but:

- `record.rs` does not parse `isSidechain`.
- `cost_entries` / `token_usage` have no column to carry it.
- The reducer attributes the rows to the parent session's `sessionId` (which
  is what the subagent records themselves contain), so they are merged into
  the parent session indistinguishably.

The fix is therefore not a query change — it is one parse, one schema column,
one ingest tag.

## Goal

A single model cost-efficiency table whose rows are keyed by `(family, role)`,
where role ∈ `main` | `subagent`. A session that ran Opus as main and Haiku as
a subagent contributes to two rows: `opus / main` (the main-half cost) and
`haiku / subagent` (the subagent's cost).

## Non-goals

- Per-`subagent_type` breakdown (Explore vs. code-reviewer vs. …). Out of scope.
- An `agent_id` column on `cost_entries` / `token_usage`. Boolean is enough for
  this view.
- Live OTLP attribution. OTLP has no subagent boundary.
- New page, new endpoint, new component. This is a small extension of the
  existing endpoint, table, and DTO.

## Decisions (resolved during brainstorming)

| # | Decision | Choice |
|---|---|---|
| 1 | Placement | Same Efficiency page, same model-efficiency table, with a new **Role** column. |
| 2 | Tagging granularity | Boolean `is_subagent` per row in `cost_entries` and `token_usage`. |
| 3 | Attribution — main rows | **Dominant family per session** over `is_subagent = 0` rows (preserves issue #20's "Opus job vs Haiku job"). |
| 4 | Attribution — subagent rows | **Pure per-family** aggregation over `is_subagent = 1` rows; sessions counted = distinct session_ids with any subagent activity in that family. |
| 5 | Backfill | None at the migration step. The next JSONL ingest over the same transcripts heals the data via an upsert. |
| 6 | Branch | `feature/efficiency-page` — same branch as PR #28, single shipping unit. |

## Schema

```sql
ALTER TABLE cost_entries ADD COLUMN is_subagent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE token_usage  ADD COLUMN is_subagent INTEGER NOT NULL DEFAULT 0;
```

`INTEGER 0/1` is SQLite's boolean convention (consistent with the existing
codebase). Existing rows default to `0` (main). No index is added — the
existing `(session_id, timestamp)` indexes are sufficient; aggregations scan
the date-window anyway and the role split is a cheap WHERE filter on a tiny
column.

## JSONL parsing & ingest

### `record.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct JsonlRecord {
    // … existing fields …
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
}
```

Absent → `false` (the default). The flag is only ever read; it is never set
by Andon.

### `reducer.rs`

`DerivedEvent::TokenUsage` and `DerivedEvent::CostEntry` each gain a
`is_subagent: bool` field. The reducer copies `rec.is_sidechain` into both
when emitting them.

### `ingestor.rs` — upsert on conflict

Today the `cost_entries` and `token_usage` writes are `INSERT OR IGNORE` (the
unique index on `request_id` enforces dedup). With this change, the writes
become:

```sql
INSERT INTO cost_entries (..., is_subagent) VALUES (..., ?)
ON CONFLICT(request_id) DO UPDATE
   SET is_subagent = 1
   WHERE excluded.is_subagent = 1;

INSERT INTO token_usage (..., is_subagent) VALUES (..., ?)
ON CONFLICT(request_id, token_type) DO UPDATE
   SET is_subagent = 1
   WHERE excluded.is_subagent = 1;
```

The semantics are deliberate:

- **Insert**: new rows carry the flag from the JSONL record.
- **One-way flip false → true**: if the same `request_id` is already in the
  table with `is_subagent = 0` (e.g. live OTLP wrote it first, or an earlier
  JSONL ingest predated this code), a later JSONL ingest whose record carries
  `isSidechain: true` upgrades the row to `is_subagent = 1`.
- **Never demoted true → false.** The `WHERE excluded.is_subagent = 1` guard
  prevents a subsequent non-sidechain record (impossible in practice for the
  same `request_id`, but a safety belt) from clearing the flag.
- **Idempotent.** Repeated ingests of the same transcripts converge.

The OTLP write path is unchanged — it inserts with `is_subagent = 0` (its
default), and the post-session JSONL ingest later upgrades the subagent rows.

## Backend

### Aggregation contract

`aggregate_model_efficiency` in `src/api/efficiency.rs` is extended:

```rust
pub fn aggregate_model_efficiency(
    cost_rows:   &[(String, String, f64, bool)],  // (session_id, model, cost_usd, is_subagent)
    output_rows: &[(String, i64, bool)],          // (session_id, output_tokens, is_subagent)
) -> Vec<ModelEfficiencyRow>;
```

`ModelEfficiencyRow` gains a `role: String` field with value `"main"` or
`"subagent"`.

The aggregation runs in two passes, one per role, and concatenates the
results:

**Main pass (`is_subagent = false`):** unchanged from the current
dominant-family-per-session logic. Build `session → family → cost`, pick
dominant family, attribute whole-half cost + output to that family bucket.
Emit rows with `role = "main"`.

**Subagent pass (`is_subagent = true`):** pure per-family aggregation. For
each `is_subagent = true` cost row, add to `family → (sessions_seen,
total_cost, output_tokens)` directly, where `sessions_seen` counts distinct
session_ids per family bucket. No dominant-family attribution — a subagent
invocation typically uses one model, so `aggregate_by_actual_family ==
dominant_family_per_invocation` in the overwhelming majority of cases, and
attributing by actual family avoids the awkward question "what is a subagent
session?". Emit rows with `role = "subagent"`.

All rows are concatenated and sorted by `total_cost_usd` descending. A session
that ran both main and subagent activity contributes to one main row (its
dominant family) and one or more subagent rows (one per family the subagent
calls used).

### SQL — handler

The two SQL queries in `v2_model_efficiency` change in one place only:
`SELECT … is_subagent FROM cost_entries / token_usage`. The handler now
collects 4-tuples / 3-tuples and passes both to the extended aggregator. The
filter behaviour is unchanged.

### Cache-efficiency endpoint

`v2_cache_efficiency` is **not** changed in this spec. Cache savings are
already correct regardless of role — the math sums per model. A per-role cache
breakdown is interesting but not part of this scope.

## Frontend

### DTO

`V2ModelEfficiency` (in `web/src/app/core/api.service.ts`) gains:

```ts
role: 'main' | 'subagent';
```

### Table

The model-efficiency table in `efficiency.component.html` grows one column —
**Role** — placed between Family and Sessions:

| Family | Role | Sessions | Cost / session | $ / 1k output | Total |

`Role` renders as a small uppercase badge. `main` uses `text-muted`;
`subagent` uses `text-accent/80` to lift the new rows visually:

```html
<span class="text-[10px] uppercase tracking-wider"
      [class]="m.role === 'subagent' ? 'text-accent/80' : 'text-muted'">
  {{ m.role }}
</span>
```

`@for` track key changes from `m.family` to a composite — either
`m.family + ':' + m.role` or a separate `id` field — so Angular renders
distinct rows for the two roles of the same family.

The empty state and existing test cases continue to work; the new spec test
asserts a `subagent` row renders when the API returns one.

## Backfill of existing data

No migration-time backfill is performed. The user's existing 12k cost rows
(and 47k token-usage rows) sit at `is_subagent = 0` after the schema add.

**They heal on the next JSONL ingest run**:

- The hourly session-end JSONL ingest (`kind = "session_end"`) re-reads each
  closed session's transcripts and runs the upsert. Subagent records flip
  their rows to `is_subagent = 1`.
- A user-initiated full backfill (Settings → "Backfill JSONL") does the same,
  wholesale.

A line in `docs/features.md` and a one-line release-note will tell users to
re-run the backfill if they want the historical data to show subagent rows
immediately.

## Edge cases

- **Sessions with no subagents:** no rows ever flip to `is_subagent = 1`; the
  table looks identical to the pre-change behaviour. No regression.
- **Subagent activity in OTLP-only data:** stays merged into `main`. The
  user's UI doesn't lie — it just doesn't surface the breakdown. The Behaviour
  page's "Experimental" banner principle is the same trade-off.
- **A session that is entirely subagent** (rare — e.g. the user invoked the
  Task tool as the very first turn): contributes only to the subagent rows.
  The main pass sees zero cost for that session and skips it. Correct.
- **A subagent that called another subagent:** Claude Code marks both levels
  `isSidechain: true`. They sum into the subagent bucket. Correct — nested
  subagents are still subagents.
- **`isSidechain` on a main-transcript record (would be malformed):** treated
  as subagent. The row's `is_subagent` becomes 1. This is the conservative
  interpretation of upstream data; if Claude Code ever stamps the flag
  somewhere unexpected, the row joins the subagent rollup rather than the
  main one — easier to spot than the inverse.

## Testing

TDD throughout; Rust under `cargo test --features test-support`, Angular under
`npm test`.

**Rust unit:**
- `record.rs`: `isSidechain: true` → `is_sidechain == true`; absent → `false`.
- `reducer.rs`: an assistant record with `isSidechain: true` produces
  `TokenUsage { is_subagent: true, .. }` and `CostEntry { is_subagent: true, .. }`.
- `efficiency.rs` aggregator: a fixture with one session that has both main
  (`is_subagent = false`) and subagent (`is_subagent = true`) cost rows
  produces two rows — `main` (dominant family of the main half) and
  `subagent` (per-family).
- A subagent-only session contributes only the subagent row.

**Rust integration:**
- `tests/api_reports.rs`: seed a session via `seed_session` plus a direct INSERT
  of `is_subagent = 1` cost/token rows for a different model (since the test
  helper doesn't yet model subagent rows — add an `is_subagent` option to
  `SeedOpts` as part of the work). GET `/api/v2/model-efficiency`; assert one
  `main` row and one `subagent` row with the expected families.

**Rust ingest:**
- Seed an existing `cost_entries` row with `is_subagent = 0` and a known
  `request_id`. Trigger an ingest path that processes a JSONL record carrying
  the same `request_id` and `isSidechain: true`. Assert the row is now
  `is_subagent = 1`. A second ingest of the same record is a no-op.

**Angular:**
- `efficiency.component.spec.ts`: stub `modelEfficiency` to return one `main`
  and one `subagent` row, assert both render with the role badge visible.

## Privacy & safety

No new listeners, no new outbound calls, no new payload exposure. The schema
addition is one boolean column on each of two existing tables — purely
local. The four privacy guarantees in `docs/architecture.md` are unaffected.

## Files touched

**Rust**
- `src-tauri/src/db/migrations.rs` — new migration adding `is_subagent` to
  both tables.
- `src-tauri/src/jsonl/record.rs` — `is_sidechain` field on `JsonlRecord`.
- `src-tauri/src/jsonl/reducer.rs` — thread `is_sidechain` into the two
  derived-event variants; existing `assistant_task_tool_emits_subagent` test
  unaffected (it covers a different event).
- `src-tauri/src/otlp/ingestor.rs` (and any JSONL-specific writer) — switch
  the two `INSERT OR IGNORE` writes to the `ON CONFLICT … DO UPDATE` form
  shown above. Both the OTLP write path (always `is_subagent = false`) and
  the JSONL write path use the same SQL; only the bound value differs.
- `src-tauri/src/api/efficiency.rs` — extend the aggregator to two passes
  per role.
- `src-tauri/src/api/dto.rs` — `role` field on `ModelEfficiencyRow`.
- `src-tauri/src/api/routes.rs` — `v2_model_efficiency` selects `is_subagent`
  and passes 4-tuples / 3-tuples to the aggregator.
- `src-tauri/tests/common/mod.rs` — `SeedOpts` gains an `is_subagent: bool`
  option used by the integration tests.
- `src-tauri/tests/api_reports.rs` — one new test (`v2_model_efficiency_splits_main_and_subagent`).

**Angular**
- `web/src/app/core/api.service.ts` — `role` on `V2ModelEfficiency`.
- `web/src/app/features/efficiency/efficiency.component.html` — new Role
  column / badge.
- `web/src/app/features/efficiency/efficiency.component.spec.ts` — one new
  test for the role badge.

**Docs**
- `docs/features.md` — extend the Efficiency section's bullet on model
  cost-efficiency to mention the role split.
