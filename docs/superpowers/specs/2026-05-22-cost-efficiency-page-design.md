# Cost-efficiency page — Design

> Status: draft 2026-05-22 · author: SatishKrishna Pilla
> Branch: `feature/efficiency-page` (to be created off `main`)
> Covers GitHub issues **#19** (surface prompt-cache savings) and **#20** (model
> cost-efficiency view).

## Motivation

Andon already captures every number these two issues want — it just doesn't make any
of it answer a question.

- **Cache (#19).** `v2_kpis` computes `cacheRead` and `cacheCreation` token totals and
  drops their sum into one muted "Cache" figure in the Overview Tokens tile. Prompt
  caching is the single largest cost lever in Claude Code, and right now the dashboard
  says nothing about whether it is working.
- **Model mix (#20).** The Behaviour page shows *how often* each model is used. It
  never shows what each model *costs per unit of work* — so it raises the question
  "should that have been an Opus job or a Haiku job?" without answering it.

Both are the same underlying question: **am I spending my tokens well?** This feature
gives that question a home.

## Goal

A new, filterable **Efficiency** page that answers two questions at a glance:

1. **Is prompt caching paying off?** — a cache hit ratio and the *net* dollars saved,
   with the gross saving and the cache-creation premium broken out so the mechanic is
   visible, not hidden.
2. **Which model family is cost-effective for the work I do?** — cost per session and
   cost per 1k output tokens, per model family.

## Non-goals

- No trend lines / sparklines — both issues are point-in-time ("this period").
- No per-family breakdown of cache savings — the cache section stays aggregate.
- No changes to the Overview or Behaviour pages. This feature is purely additive.
- No expand-to-exact-model drill-down in the model table (families only).
- No new schema, no migration — every figure derives from existing tables.

## Decisions (resolved during brainstorming)

| # | Decision | Choice |
|---|---|---|
| 1 | Placement | New dedicated, filterable `/efficiency` page (not Overview tiles, not the Behaviour page). Cost-efficiency is a *cost* concern and deserves the filter bar Behaviour lacks. |
| 2 | Cache "$ saved" | **Net** headline (gross − creation premium), with gross and overhead shown as a breakdown. |
| 3 | Model granularity | Collapsed **family** — `opus` / `sonnet` / `haiku`, plus `other` for anything unrecognized. |
| 4 | Session→family attribution | **Dominant family** — each session is bucketed by the family that spent the most in it; the session's *whole* cost and output land in that one bucket. |
| 5 | Page layout | KPI strip (3 tiles) + model-efficiency table. |

## Page & navigation

A lazy-loaded standalone feature at `web/src/app/features/efficiency/`:

- Route `/efficiency` in `app.routes.ts`.
- Nav item in `app.component.html`, placed **immediately after Overview** (icon:
  `gauge`).
- Standard crumb header and `<app-filter-bar />`. Window + model-chip filtering behave
  exactly as on Overview.

## Backend

Two new endpoints in `src-tauri/src/api/routes.rs`, registered on the existing router.
Both accept the standard `FilterQuery` (`from`, `to`, `models`), carry
`#[tracing::instrument]`, return `serde`-derived DTOs from `src-tauri/src/api/dto.rs`,
and on any internal failure degrade to zeros rather than erroring (consistent with the
other `v2_*` handlers). No `unwrap`/`expect`.

### Period semantics

Both endpoints scope a period by **entry timestamp**, not `sessions.started_at` — the
same convention `v2_cost_by_model` and the tape already use. A session straddling the
period boundary contributes only its in-window rows. The model filter, when active,
restricts `token_usage` / `cost_entries` rows by `model` before any aggregation.

### `GET /api/v2/cache-efficiency` — issue #19

Handler `v2_cache_efficiency`.

**Token totals.** Reuse the existing `sum_tokens` helper for `input`, `output`,
`cacheRead`, `cacheCreation` over `[from, to)` with the model filter.

**Hit ratio.** The share of *prompt* tokens served from cache:

```
hit_ratio = cache_read / (input + cache_create + cache_read)
```

Output is excluded — it is not part of the prompt. Denominator `0` → `hit_ratio = 0`.

**Savings.** Cache-read and cache-creation are priced differently per model, so savings
are computed per model and summed. Query:

```sql
SELECT model, token_type, SUM(count)
FROM token_usage
WHERE timestamp >= ?1 AND timestamp < ?2 {model filter}
GROUP BY model, token_type
```

For each model resolved by `pricing::lookup(model)`:

```
gross             += cache_read   × (input_rate      − cache_read_rate)   / 1e6
creation_overhead += cache_create × (cache_create_rate − input_rate)      / 1e6
net                = gross − creation_overhead
```

**Counterfactual** (the reasoning the numbers encode): if prompt caching were off,
every cache-read token would instead be billed as fresh input, and every
cache-creation token would *also* be billed as fresh input (no creation premium).
`gross` is the discount won on reads; `creation_overhead` is the premium paid to write
the cache; `net` is the true saving.

Worked example, opus-4-7 (`input 15`, `cache_read 1.50`, `cache_create 18.75` per
Mtok): `1.0M` cache-read + `1.0M` cache-create →
`gross = 13.50`, `creation_overhead = 3.75`, `net = 9.75`.

**Un-priced models.** A model absent from `pricing::TABLE` cannot be priced; its tokens
are excluded from `gross`/`overhead`/`net` but still counted in the token totals and
the hit ratio. The sum of cache-read + cache-create tokens on un-priced models is
returned as `unpriced_cache_tokens` so the UI can footnote the omission honestly.

**Comparison.** Previous-period `hit_ratio` and `net` are computed over
`prev_period_window(from, to)` (existing helper) to drive the tile deltas.

Response shape:

```json
{
  "hit_ratio": 0.68,
  "hit_ratio_prev": 0.61,
  "tokens": { "input": 1900000, "output": 540000, "cache_read": 3400000, "cache_create": 700000 },
  "savings": { "net": 42.18, "gross": 58.90, "creation_overhead": 16.72 },
  "net_prev": 35.04,
  "unpriced_cache_tokens": 0
}
```

DTO: `CacheEfficiency { hit_ratio: f64, hit_ratio_prev: f64, tokens: CacheTokenTotals,
savings: CacheSavings, net_prev: f64, unpriced_cache_tokens: i64 }`, with
`CacheTokenTotals { input, output, cache_read, cache_create: i64 }` and
`CacheSavings { net, gross, creation_overhead: f64 }`. All money rounded with the
existing `round4`.

### `GET /api/v2/model-efficiency` — issue #20

Handler `v2_model_efficiency`.

**Family classifier.** A `model_family(&str) -> &'static str` helper: case-insensitive
substring match → `"opus"` / `"sonnet"` / `"haiku"`, else `"other"`. This is the Rust
mirror of the frontend's `MODEL_COLOR_TABLE` substring approach.

**Per-session cost by family.**

```sql
SELECT session_id, model, SUM(cost_usd)
FROM cost_entries
WHERE timestamp >= ?1 AND timestamp < ?2 {model filter}
GROUP BY session_id, model
```

In Rust, fold the rows into, per `session_id`, a `{family → cost}` map and a running
session total.

**Per-session output tokens.**

```sql
SELECT session_id, SUM(count)
FROM token_usage
WHERE token_type = 'output' AND timestamp >= ?1 AND timestamp < ?2 {model filter}
GROUP BY session_id
```

**Bucketing.** For each session, the **dominant family** is the family with the highest
cost. Tie-break (rare): the first family in the fixed order `[opus, sonnet, haiku,
other]`. The session's *total* cost and *total* output tokens are added to the dominant
family's bucket, and the bucket's session count is incremented. A session with cost
entries but no output rows contributes `0` output.

**Row metrics**, per family bucket:

```
cost_per_session   = total_cost / sessions
cost_per_1k_output = output_tokens > 0 ? total_cost / output_tokens × 1000 : 0
```

Rows sorted by `total_cost` descending. Families with no sessions in the period are
omitted.

Response: a JSON array of
`ModelEfficiencyRow { family: String, sessions: i64, total_cost_usd: f64,
cost_per_session: f64, output_tokens: i64, cost_per_1k_output: f64 }`.

```json
[
  { "family": "opus",   "sessions": 38, "total_cost_usd": 69.92, "cost_per_session": 1.84, "output_tokens": 98480, "cost_per_1k_output": 0.71 },
  { "family": "sonnet", "sessions": 12, "total_cost_usd": 5.04,  "cost_per_session": 0.42, "output_tokens": 29647, "cost_per_1k_output": 0.17 },
  { "family": "haiku",  "sessions": 7,  "total_cost_usd": 0.63,  "cost_per_session": 0.09, "output_tokens": 15750, "cost_per_1k_output": 0.04 }
]
```

## Frontend

`EfficiencyComponent` — standalone, `ChangeDetectionStrategy.OnPush`, signals only,
`inject()` for `FilterService` and `ApiService`. No `Observable`/`Subject` in feature
code beyond the `ApiService` HTTP calls themselves.

- A constructor `effect()` refetches both endpoints when `filter.window()`,
  `filter.modelsCsv()`, or `filter.refreshTick()` change — the same pattern as
  `OverviewComponent`.
- State: `cache = signal<V2CacheEfficiency | null>(null)` and
  `models = signal<V2ModelEfficiency[]>([])`.
- `ApiService` gains `cacheEfficiency(args)` and `modelEfficiency(args)`, plus the
  matching DTO interfaces in `core/`.
- Delta formatting mirrors `OverviewComponent`'s `fmtDelta` / `deltaClass`. The
  hit-ratio tile shows a **percentage-point** delta (e.g. `+7 pts`); the net-savings
  tile shows a percent delta like the existing KPIs. If duplication grates, extract a
  small shared `delta` util — optional, not required.
- A `familyColor(family)` helper: `opus #facc15`, `sonnet #60a5fa`, `haiku #34d399`,
  `other` → muted/fallback. Same palette as `MODEL_COLOR_TABLE`.

**Layout** (option B from the brainstorm). Tailwind utilities first, reusing the
`panel` / `panel-title` / `panel-body` classes:

- A 3-tile KPI strip:
  1. **Cache hit ratio** — large percent + a horizontal fill bar + pt-delta.
  2. **Net cache savings** — large `$` (green) + `gross … − premium …` sub-line +
     percent delta. When `unpriced_cache_tokens > 0`, a small footnote: "excludes
     N tokens on un-priced models".
  3. **Cache tokens** — total read+create, with a `read · create` split sub-line.
- A **Model cost-efficiency** table panel: columns *Family · Sessions · Cost / session
  · $ / 1k output · Total*, a family-color dot per row.
- Empty states: tiles render `—`; the table renders "No data" (matching the existing
  "No data" treatment on the Overview Cost-by-model panel).

## Edge cases

- Every division (`hit_ratio`, `cost_per_session`, `cost_per_1k_output`, delta
  percentages) is zero-guarded.
- Un-priced models: excluded from cache savings, surfaced via `unpriced_cache_tokens`.
  They still appear in the model table (cost_entries always carries `cost_usd`), most
  likely under the `other` family.
- An empty period yields a well-formed all-zero `CacheEfficiency` and an empty model
  array — never an error.
- The model filter narrowing dominant-family computation to the filtered models is
  intended behavior: the filter is global and user-initiated.

## Testing

TDD — failing test first, then implementation. Rust tests run under
`cargo test --features test-support`.

**Rust unit:**
- `model_family` — `claude-opus-4-7`, a date-suffixed id, sonnet, haiku, and a
  non-Claude id → `other`.
- Cache savings math — known token counts assert `gross`, `creation_overhead`, `net`
  (e.g. the opus worked example above).
- Hit-ratio denominator — `input 100 + cache_create 100 + cache_read 300` → `0.6`.
- Dominant-family bucketing — a session with opus cost > haiku cost routes the *whole*
  session (cost + output) into the opus bucket; verify the tie-break order.

**Rust integration / snapshot** (`src-tauri/tests/api_reports.rs` style): seed a DB via
`test-support`, hit `/api/v2/cache-efficiency` and `/api/v2/model-efficiency`, assert
the JSON, and add shape snapshots alongside the existing `v2_kpis` snapshot.

**Angular** (Vitest): `EfficiencyComponent` renders the three tiles and the table from
a mocked `ApiService`, and shows the empty state when both responses are empty.

## Privacy & safety

No new listeners, no new outbound calls, no schema change. Both endpoints are read-only
aggregations served on the existing `127.0.0.1:8765` API. No raw prompt content is
touched. The four privacy guarantees in `docs/architecture.md` are unaffected.

## Files touched

**Rust**
- `src-tauri/src/api/routes.rs` — two handlers + route registration; `model_family`
  helper.
- `src-tauri/src/api/dto.rs` — `CacheEfficiency`, `CacheTokenTotals`, `CacheSavings`,
  `ModelEfficiencyRow`.
- `src-tauri/tests/api_reports.rs` (+ a new `.snap`) — endpoint coverage.

**Angular**
- `web/src/app/features/efficiency/efficiency.component.{ts,html}` — new.
- `web/src/app/features/efficiency/efficiency.component.spec.ts` — new.
- `web/src/app/core/api.service.ts` (+ DTO interfaces) — two new methods.
- `web/src/app/app.routes.ts` — `/efficiency` route.
- `web/src/app/app.component.html` — nav item.

**Docs**
- `docs/features.md` — a short section for the new page.
