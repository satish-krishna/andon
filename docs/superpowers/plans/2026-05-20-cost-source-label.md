# Per-session Cost-Source Label Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show, per session, whether its cost came from live OTLP telemetry or retroactive JSONL ingestion — as a "Source" column in the sessions list and a marker on the session detail page.

**Architecture:** The label is derived per session from `cost_entries.request_id` (NULL = OTLP-written, non-NULL = JSONL-written), with OTLP-wins precedence matching `reconciler::coverage_for`. The unreliable `sessions.data_source` column is deliberately not used. A SQL `CASE` expression yields `'otlp'` / `'jsonl'` / `NULL`; that value rides existing session DTOs to the Angular UI, where a shared helper module renders a dot + label.

**Tech Stack:** Rust (rusqlite, axum, serde), SQLite, `insta` snapshot tests; Angular 21 (standalone components, signals), Tailwind, Vitest.

**Spec:** `docs/superpowers/specs/2026-05-20-cost-source-label-design.md`

**Conventions:**
- Work on the existing branch `feature/jsonl-ingest`.
- Rust commands run from `src-tauri/`. Web commands run from `web/`.
- Every commit message ends with the trailer `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- Conventional Commits, no emojis. TDD: failing test first.

> **Note on the spec:** the spec's Frontend section names `models.ts SessionSummary`
> as the list-table type. The sessions *list* is actually backed by the `V2Session`
> type in `api.service.ts`; `SessionSummary` (in `models.ts`) backs the *detail* page.
> This plan touches both, which fully satisfies the spec's intent (label on list +
> detail). The spec also said the helpers live "in the component" — this plan extracts
> them to a shared `core/cost-source.ts` so the list and detail components do not
> duplicate logic. Consequently the spec's UI-render test becomes a focused unit
> test of that shared module (Task 2); the template wiring is covered by the
> type-checked `npm run build` in Tasks 3 and 4.

---

### Task 1: Backend — derive `cost_source` for sessions

**Files:**
- Modify: `src-tauri/src/api/dto.rs`
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/tests/api_sessions.rs`
- Modify: `src-tauri/tests/snapshots/` (regenerated `.snap` files)

- [ ] **Step 1: Write the failing test**

Add to the end of `src-tauri/tests/api_sessions.rs`:

```rust
// ---------------------------------------------------------------------------
// 9. cost_source_reflects_request_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cost_source_reflects_request_id() {
    let (pool, _db_dir) = common::fixture_pool();

    // OTLP session: seed_session inserts a cost row with request_id = NULL.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-otlp".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 1.0,
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );

    // JSONL session: seeded with no cost row, then a cost row carrying a
    // non-NULL request_id is inserted directly (as JSONL ingest would).
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-jsonl".into(),
            started_at_ms: Some(anchor_ms()),
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO cost_entries (session_id, request_id, timestamp, model, cost_usd) \
             VALUES ('src-jsonl', 'req_abc', 0, 'claude-opus-4-7', 0.5)",
            [],
        )
        .unwrap();
    }

    // No-cost session: seed_session with cost_usd = 0.0 inserts no cost row.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "src-none".into(),
            started_at_ms: Some(anchor_ms()),
            model: "claude-opus-4-7".into(),
            ..Default::default()
        },
    );

    let from = ms_ago(5);
    let to = anchor_ms() + 1000;

    // --- v2 list endpoint ---
    let (router, _rd) = common::test_router(&pool);
    let (status, body) = get_json(router, &format!("/api/v2/sessions?from={from}&to={to}")).await;
    assert_eq!(status, StatusCode::OK);
    let pick = |id: &str| -> Value {
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["session_id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(pick("src-otlp")["cost_source"], "otlp");
    assert_eq!(pick("src-jsonl")["cost_source"], "jsonl");
    assert!(
        pick("src-none")["cost_source"].is_null(),
        "a session with no cost rows has a null cost_source"
    );

    // --- detail endpoint exposes the same field ---
    let (router2, _rd2) = common::test_router(&pool);
    let (status2, detail) = get_json(router2, "/api/sessions/src-jsonl").await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(detail["session"]["cost_source"], "jsonl");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features test-support --test api_sessions cost_source_reflects_request_id`
Expected: FAIL — `cost_source` is absent from the JSON, so `pick("src-otlp")["cost_source"]` is `Null` and `assert_eq!(Null, "otlp")` fails.

- [ ] **Step 3: Add the `cost_source` field to `SessionSummary`**

In `src-tauri/src/api/dto.rs`, in the `SessionSummary` struct, add this field immediately after `repo_name` (mirroring the `#[serde(default)]` of the surrounding repo fields):

```rust
    #[serde(default)] pub cost_source: Option<String>,
```

- [ ] **Step 4: Add `cost_source` to the v2 sessions-list query**

In `src-tauri/src/api/routes.rs`, in the `sql` string built for the v2 sessions list
(the query selecting `top_model`, `api_calls`, `decisions`), the `SELECT` list ends with
`s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name`. Replace that line with:

```rust
                s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name,
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source
```

Then, in the `query_map` closure for that query, after the `"repo_name": r.get::<_, Option<String>>(20)?,` line, add:

```rust
            "cost_source":     r.get::<_, Option<String>>(21)?,
```

- [ ] **Step 5: Add `cost_source` to the `list_sessions` and `session_detail` queries**

In `src-tauri/src/api/routes.rs`, the `list_sessions` function and the `session_detail`
function each run a query whose `SELECT` ends with
`s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name`. In **both** queries,
replace that ending with:

```sql
                s.cwd, s.repo_root, s.repo_remote, s.repo_branch, s.repo_name,
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source
```

In **both** `Ok(SessionSummary { ... })` constructions, add this field after `repo_name: r.get(15)?,`:

```rust
                cost_source: r.get::<_, Option<String>>(16)?,
```

- [ ] **Step 6: Run the new test to verify it passes**

Run: `cargo test --features test-support --test api_sessions cost_source_reflects_request_id`
Expected: PASS.

- [ ] **Step 7: Regenerate the affected snapshots**

Adding `cost_source` adds a key to three `insta` snapshots (`list_sessions_shape`,
`v2_sessions_shape`, `session_detail_shape`). Regenerate them:

Run: `cargo insta accept`
(If `cargo insta` is not found, install it once: `cargo install cargo-insta`, then retry.)

This updates the `.snap` files under `src-tauri/tests/snapshots/`. The seeded sessions
in those tests carry NULL-`request_id` cost rows, so the new key serializes as
`"cost_source": "otlp"` — leave it unredacted; it is deterministic and worth asserting.

- [ ] **Step 8: Run the full `api_sessions` suite to verify it passes**

Run: `cargo test --features test-support --test api_sessions`
Expected: PASS — all tests, including the regenerated snapshot tests.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/api/dto.rs src-tauri/src/api/routes.rs src-tauri/tests/api_sessions.rs src-tauri/tests/snapshots/
git commit -m "feat(api): derive cost_source (OTLP/JSONL) for sessions" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Web — shared cost-source helper module

**Files:**
- Create: `web/src/app/core/cost-source.ts`
- Create: `web/src/app/core/cost-source.spec.ts`

- [ ] **Step 1: Write the failing test**

Create `web/src/app/core/cost-source.spec.ts`:

```ts
import { describe, it, expect } from 'vitest';

import { sourceLabel, sourceDotClass, sourceTooltip } from './cost-source';

describe('cost-source helpers', () => {
  it('labels each source', () => {
    expect(sourceLabel('otlp')).toBe('OTLP');
    expect(sourceLabel('jsonl')).toBe('JSONL');
    expect(sourceLabel(null)).toBe('—');
  });

  it('maps each source to a dot background class', () => {
    expect(sourceDotClass('otlp')).toBe('bg-ok');
    expect(sourceDotClass('jsonl')).toBe('bg-warn');
    expect(sourceDotClass(null)).toBe('');
  });

  it('describes each source in a tooltip', () => {
    expect(sourceTooltip('otlp')).toContain('OpenTelemetry');
    expect(sourceTooltip('jsonl')).toContain('JSONL');
    expect(sourceTooltip(null)).toBe('No cost recorded');
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- cost-source`
Expected: FAIL — cannot resolve `./cost-source` (the module does not exist yet).

- [ ] **Step 3: Create the helper module**

Create `web/src/app/core/cost-source.ts`:

```ts
/** Provenance of a session's cost: live OTLP telemetry, or retroactive JSONL
 *  ingest. `null` when the session has no cost rows at all. */
export type CostSource = 'otlp' | 'jsonl' | null;

/** Short column label for a cost source. */
export function sourceLabel(s: CostSource): string {
  if (s === 'otlp') return 'OTLP';
  if (s === 'jsonl') return 'JSONL';
  return '—';
}

/** Tailwind background class for the source indicator dot. */
export function sourceDotClass(s: CostSource): string {
  if (s === 'otlp') return 'bg-ok';
  if (s === 'jsonl') return 'bg-warn';
  return '';
}

/** Hover text explaining where a session's cost figure came from. */
export function sourceTooltip(s: CostSource): string {
  if (s === 'otlp') return 'Cost from live OpenTelemetry';
  if (s === 'jsonl') return 'Cost priced retroactively from JSONL transcripts';
  return 'No cost recorded';
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `web/`): `npm test -- cost-source`
Expected: PASS — all three cases.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/core/cost-source.ts web/src/app/core/cost-source.spec.ts
git commit -m "feat(web): cost-source label helpers" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Web — Source column in the sessions list

**Files:**
- Modify: `web/src/app/core/api.service.ts`
- Modify: `web/src/app/features/sessions/sessions.component.ts`
- Modify: `web/src/app/features/sessions/sessions.component.html`

- [ ] **Step 1: Add `cost_source` to the `V2Session` type**

In `web/src/app/core/api.service.ts`, add an import near the top (next to the other
`./` imports such as the `models` import):

```ts
import { CostSource } from './cost-source';
```

Then, in the `V2Session` interface, add this field immediately after `cwd: string | null;`:

```ts
  cost_source: CostSource;
```

- [ ] **Step 2: Expose the helpers on the sessions component**

In `web/src/app/features/sessions/sessions.component.ts`, add an import alongside the
other `../../core/` imports:

```ts
import { sourceLabel, sourceDotClass, sourceTooltip } from '../../core/cost-source';
```

Then, inside the `SessionsComponent` class, add these three field bindings next to the
existing `Math = Math;` line (the same re-export idiom):

```ts
  sourceLabel = sourceLabel;
  sourceDotClass = sourceDotClass;
  sourceTooltip = sourceTooltip;
```

- [ ] **Step 3: Add the Source column header**

In `web/src/app/features/sessions/sessions.component.html`, the table header has a
`Model` column followed by a `Cost` column:

```html
              <th class="px-2 py-2 font-normal">Model</th>
              <th class="px-2 py-2 font-normal text-right">Cost</th>
```

Insert a `Source` header between them:

```html
              <th class="px-2 py-2 font-normal">Model</th>
              <th class="px-2 py-2 font-normal">Source</th>
              <th class="px-2 py-2 font-normal text-right">Cost</th>
```

- [ ] **Step 4: Add the Source column cell**

In the same file, the row body has the Model cell immediately before the Cost cell:

```html
                  <td class="px-2 py-2">
                    <span class="inline-flex items-center gap-1.5">
                      <span class="inline-block w-2 h-2 rounded-full" [style.background-color]="modelColor(s.top_model)"></span>
                      {{ modelLabel(s.top_model) }}
                    </span>
                  </td>
                  <td class="px-2 py-2 text-right tabular-nums">${{ s.cost_usd | number : '1.2-2' }}</td>
```

Insert the Source cell between the Model `</td>` and the Cost `<td>`:

```html
                  <td class="px-2 py-2">
                    <span class="inline-flex items-center gap-1.5">
                      <span class="inline-block w-2 h-2 rounded-full" [style.background-color]="modelColor(s.top_model)"></span>
                      {{ modelLabel(s.top_model) }}
                    </span>
                  </td>
                  <td class="px-2 py-2">
                    @if (s.cost_source) {
                      <span class="inline-flex items-center gap-1.5" [title]="sourceTooltip(s.cost_source)">
                        <span class="inline-block w-2 h-2 rounded-full" [ngClass]="sourceDotClass(s.cost_source)"></span>
                        {{ sourceLabel(s.cost_source) }}
                      </span>
                    } @else {
                      <span class="text-muted" [title]="sourceTooltip(null)">—</span>
                    }
                  </td>
                  <td class="px-2 py-2 text-right tabular-nums">${{ s.cost_usd | number : '1.2-2' }}</td>
```

- [ ] **Step 5: Widen the spanning cells from 10 to 11 columns**

The table now has 11 columns. In the same file there are exactly two cells with
`colspan="10"` — the group-label row and the expanded-detail row. Change **both**
occurrences of `colspan="10"` to `colspan="11"`.

- [ ] **Step 6: Verify the web build**

Run (from `web/`): `npm run build`
Expected: build succeeds with no type errors.

- [ ] **Step 7: Commit**

```bash
git add web/src/app/core/api.service.ts web/src/app/features/sessions/sessions.component.ts web/src/app/features/sessions/sessions.component.html
git commit -m "feat(web): Source column in the sessions list" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Web — cost-source marker on the session detail page

**Files:**
- Modify: `web/src/app/core/models.ts`
- Modify: `web/src/app/features/sessions/session-detail.component.ts`

- [ ] **Step 1: Add `cost_source` to the `SessionSummary` model**

In `web/src/app/core/models.ts`, add an import at the top of the file:

```ts
import { CostSource } from './cost-source';
```

Then, in the `SessionSummary` interface, add this field immediately after `repo_name`:

```ts
  cost_source: CostSource;
```

- [ ] **Step 2: Expose the helpers on the detail component**

In `web/src/app/features/sessions/session-detail.component.ts`, add an import alongside
the existing `../../core/` imports:

```ts
import { sourceLabel, sourceDotClass, sourceTooltip } from '../../core/cost-source';
```

Then, inside the `SessionDetailComponent` class, add these three field bindings (e.g.
just below the `detail` / `reportExists` / `reportBusy` signals):

```ts
  sourceLabel = sourceLabel;
  sourceDotClass = sourceDotClass;
  sourceTooltip = sourceTooltip;
```

- [ ] **Step 3: Add the marker to the Cost panel**

In the same file's inline `template`, the Cost panel currently reads:

```html
          <app-panel title="Cost">
            <div class="text-2xl font-mono">$ {{ d.session.cost_usd | number : '1.4-4' }}</div>
          </app-panel>
```

Replace it with:

```html
          <app-panel title="Cost">
            <div class="text-2xl font-mono">$ {{ d.session.cost_usd | number : '1.4-4' }}</div>
            @if (d.session.cost_source) {
              <div class="mt-1 inline-flex items-center gap-1.5 text-xs text-muted"
                   [title]="sourceTooltip(d.session.cost_source)">
                <span class="inline-block w-2 h-2 rounded-full" [ngClass]="sourceDotClass(d.session.cost_source)"></span>
                {{ sourceLabel(d.session.cost_source) }}
              </div>
            }
          </app-panel>
```

- [ ] **Step 4: Verify the web build**

Run (from `web/`): `npm run build`
Expected: build succeeds with no type errors.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/core/models.ts web/src/app/features/sessions/session-detail.component.ts
git commit -m "feat(web): cost-source marker on the session detail page" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Final verification

**Files:** none — verification only.

- [ ] **Step 1: Full Rust suite**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — entire suite green, including `cost_source_reflects_request_id` and the
regenerated `api_sessions` snapshots.

- [ ] **Step 2: Rust lint**

Run (from `src-tauri/`): `cargo clippy --features test-support --all-targets`
Expected: no new warnings attributable to `dto.rs` or `routes.rs`.

- [ ] **Step 3: Web build and tests**

Run (from `web/`): `npm run build`
Expected: build succeeds.

Run (from `web/`): `npm test`
Expected: PASS — the full Vitest suite, including `cost-source.spec.ts`.

- [ ] **Step 4: Manual smoke (optional but recommended)**

Delete the dev `data.db`, run `cargo tauri dev`, ingest JSONL history, and open the
Sessions page. Confirm the Source column shows `JSONL` (amber dot) for backfilled
sessions and `OTLP` (green dot) for any live-telemetry sessions, and that the session
detail page shows the matching marker under the Cost figure.

---

## Notes for the implementer

- **Why derive from `request_id`, not `sessions.data_source`:** `upsert_session` (OTLP
  path) never sets `data_source`, so OTLP sessions have `data_source = NULL`; JSONL
  ingest sets `'jsonl'` via `INSERT OR IGNORE`. The column reflects insert order, not
  cost provenance. `cost_entries.request_id` is the trustworthy per-row signal.
- **OTLP-wins precedence:** if a session somehow has both NULL and non-NULL
  `request_id` cost rows, the `CASE` resolves to `'otlp'` — consistent with
  `reconciler::coverage_for`.
- **No schema migration.** `cost_entries.request_id` already exists (migration v5).
- The v1 `/api/sessions` (`list_sessions`) endpoint also gains `cost_source` — required
  because it constructs the shared `SessionSummary` struct. It is harmless even though
  the current UI consumes the v2 endpoint.
