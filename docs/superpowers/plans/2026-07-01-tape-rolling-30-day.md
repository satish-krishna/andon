# Rolling-30-Day Tape Ribbon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Overview two-row calendar tape with a single rolling 30-day ribbon whose bars light up for the days inside the active date filter and show exact date + cost on hover.

**Architecture:** The `/api/v2/tape` endpoint is reshaped to return an explicit 30-point `{date, cost}` series ending today, built with the existing `last_n_days_bounds` / `day_labels` / `day_index_for` rolling-window helpers (local-day bucketing) plus the tape's existing model LIKE filter. The Angular Overview component renders one 30-bar row; a pure `dateInWindow` helper decides per-bar highlighting from `filter.window()`, so highlighting reacts to the filter with no refetch.

**Tech Stack:** Rust (axum, rusqlite, chrono, serde), Angular 21 (standalone, signals), Tailwind, Vitest.

## Global Constraints

- Rust: no `unwrap()` / `expect()` outside `main.rs` setup or `#[cfg(test)]` code; `anyhow::Result` at boundaries.
- Rust: `serde` for every JSON payload — no hand-written JSON strings.
- Angular: standalone components, signals only (`signal`/`computed`/`effect`), `inject()`, `OnPush`, `@if`/`@for` (never `*ngIf`/`*ngFor`).
- Angular: Tailwind utilities first; custom CSS only when utilities don't cover it.
- US English everywhere (color, behavior).
- Conventional Commits, no emojis: `type(scope): subject`.
- TDD: failing test first, then implementation.
- All work on branch `feature/tape-rolling-30-day` (already checked out).
- Tests: `cd src-tauri; cargo test --features test-support` · `cd web; npm test` · `cd web; npm run build`.

---

### Task 1: Backend — rolling-30-day tape endpoint

Reshape `/api/v2/tape` to return `{ days: [{date, cost}] }` (exactly `days` points, oldest → today), drop the dead `month` param, keep the `models` filter, and replace the hand-rolled `json!` with a typed serde DTO. Delete `tape_for_month` (month-anchored, now unused).

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (replace `TapeQuery` at ~1444-1448, `v2_tape` at ~1450-1491, and `tape_for_month` at ~1493-1542; add tests in the `mod tests` block at ~2822)

**Interfaces:**
- Consumes: existing `last_n_days_bounds(days: i64) -> (i64, i64)`, `day_labels(days: i64) -> Vec<String>`, `day_index_for(ts_ms: i64, days: i64) -> Option<usize>`, `round4`, `default_days() -> i64`, `FilterQuery { from, to, models }` + `FilterQuery::model_clause(col) -> (String, Vec<String>)`, `crate::db::migrations::apply(&mut Connection)`.
- Produces:
  - `struct TapePoint { date: String, cost: f64 }` (serde `Serialize`)
  - `struct TapeResponse { days: Vec<TapePoint> }` (serde `Serialize`)
  - `fn tape_last_n_days(conn: &rusqlite::Connection, days: i64, models: &FilterQuery) -> Vec<TapePoint>`
  - Route `GET /api/v2/tape?days=<1..365, default 30>&models=<csv>` → `TapeResponse`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests { ... }` block at the bottom of `src-tauri/src/api/routes.rs` (before its closing `}`):

```rust
    fn tape_conn() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut conn).expect("apply migrations");
        conn
    }

    // Local noon `days_ago` days back — noon avoids DST/midnight day-boundary flakiness.
    fn noon_ms_days_ago(days_ago: u64) -> i64 {
        let d = Local::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(days_ago))
            .expect("valid date");
        Local
            .from_local_datetime(&d.and_hms_opt(12, 0, 0).expect("valid noon"))
            .single()
            .expect("unambiguous local datetime")
            .timestamp_millis()
    }

    fn insert_cost(conn: &rusqlite::Connection, ts_ms: i64, model: &str, cost: f64) {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES ('s', ?1, ?2, ?3)",
            rusqlite::params![ts_ms, model, cost],
        )
        .expect("insert cost_entries row");
    }

    #[test]
    fn tape_last_n_days_bins_by_local_day_and_excludes_older() {
        let conn = tape_conn();
        insert_cost(&conn, noon_ms_days_ago(0), "claude-opus-4-8", 1.0); // today
        insert_cost(&conn, noon_ms_days_ago(1), "claude-sonnet-5", 2.0); // yesterday
        insert_cost(&conn, noon_ms_days_ago(29), "claude-haiku-4-5", 4.0); // oldest in-window
        insert_cost(&conn, noon_ms_days_ago(30), "claude-opus-4-8", 8.0); // out of window

        let models = FilterQuery { from: None, to: None, models: None };
        let pts = tape_last_n_days(&conn, 30, &models);

        assert_eq!(pts.len(), 30, "always exactly `days` points");
        assert_eq!(pts[29].cost, 1.0, "today is the last bar");
        assert_eq!(pts[28].cost, 2.0, "yesterday is second-to-last");
        assert_eq!(pts[0].cost, 4.0, "29 days ago is the first bar");
        let total: f64 = pts.iter().map(|p| p.cost).sum();
        assert_eq!(total, 7.0, "30-days-ago row is excluded from the window");
        assert!(pts[0].date < pts[29].date, "dates ascend oldest -> today");
    }

    #[test]
    fn tape_last_n_days_applies_model_filter() {
        let conn = tape_conn();
        insert_cost(&conn, noon_ms_days_ago(0), "claude-opus-4-8", 5.0);
        insert_cost(&conn, noon_ms_days_ago(0), "claude-haiku-4-5", 9.0);

        let models = FilterQuery { from: None, to: None, models: Some("opus".into()) };
        let pts = tape_last_n_days(&conn, 30, &models);

        assert_eq!(pts[29].cost, 5.0, "only opus rows counted");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri; cargo test --features test-support tape_last_n_days`
Expected: FAIL to compile — `cannot find function tape_last_n_days` / `TapePoint`.

- [ ] **Step 3: Implement the DTO, handler, and query**

In `src-tauri/src/api/routes.rs`, ensure `Serialize` is in scope (top of file — if the imports read `use serde::Deserialize;`, change to `use serde::{Deserialize, Serialize};`).

Replace the `TapeQuery` struct (~1444-1448) and the `v2_tape` handler (~1450-1491) with:

```rust
#[derive(Deserialize)]
struct TapeQuery {
    #[serde(default = "default_days")]
    days: i64,
    models: Option<String>,
}

#[derive(Serialize)]
struct TapePoint {
    date: String, // "YYYY-MM-DD", local
    cost: f64,
}

#[derive(Serialize)]
struct TapeResponse {
    days: Vec<TapePoint>, // exactly `days` points, oldest -> today
}

async fn v2_tape(
    State(state): State<ApiState>,
    Query(q): Query<TapeQuery>,
) -> Result<Json<TapeResponse>, ApiError> {
    let days = q.days.clamp(1, 365);
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let models = FilterQuery {
        from: None,
        to: None,
        models: q.models.clone(),
    };
    let points = tape_last_n_days(&conn, days, &models);
    Ok(Json(TapeResponse { days: points }))
}
```

Replace the entire `tape_for_month` function (~1493-1542) with:

```rust
fn tape_last_n_days(
    conn: &rusqlite::Connection,
    days: i64,
    models: &FilterQuery,
) -> Vec<TapePoint> {
    let labels = day_labels(days); // oldest -> today, "YYYY-MM-DD"
    let (from, _to) = last_n_days_bounds(days);
    let mut bins = vec![0f64; days as usize];

    let (m_sql, m_vals) = models.model_clause("model");
    let sql = format!(
        "SELECT timestamp, cost_usd FROM cost_entries WHERE timestamp >= ?{m_sql}"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(refs.as_slice(), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        }) {
            for (ts_ms, cost) in rows.flatten() {
                if let Some(idx) = day_index_for(ts_ms, days) {
                    bins[idx] += cost;
                }
            }
        }
    }

    labels
        .into_iter()
        .zip(bins)
        .map(|(date, cost)| TapePoint { date, cost: round4(cost) })
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri; cargo test --features test-support tape_last_n_days`
Expected: PASS (2 tests). Then `cargo build` to confirm no unused-import / dead-code errors from the removed `tape_for_month`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): serve tape as rolling 30-day dated series"
```

---

### Task 2: Frontend — `dateInWindow` highlight helper

A pure, testable predicate deciding whether a tape bar's local date overlaps the filter window. This is where the highlight logic lives so it can be unit-tested without a DOM.

**Files:**
- Create: `web/src/app/features/overview/tape-window.ts`
- Test: `web/src/app/features/overview/tape-window.spec.ts`

**Interfaces:**
- Produces: `dateInWindow(date: string, fromMs: number, toMs: number): boolean` where `date` is `"YYYY-MM-DD"` local.

- [ ] **Step 1: Write the failing test**

Create `web/src/app/features/overview/tape-window.spec.ts`:

```ts
import { dateInWindow } from './tape-window';

// A 1-day window the way FilterService.selectDay builds it.
function dayWindow(year: number, month: number, day: number) {
  return {
    fromMs: new Date(year, month, day, 0, 0, 0, 0).getTime(),
    toMs: new Date(year, month, day, 23, 59, 59, 999).getTime(),
  };
}

describe('dateInWindow', () => {
  it('lights a date inside a multi-day window', () => {
    const w = { fromMs: new Date(2026, 5, 1).getTime(), toMs: new Date(2026, 5, 30, 23, 59, 59, 999).getTime() };
    expect(dateInWindow('2026-06-15', w.fromMs, w.toMs)).toBe(true);
  });

  it('does not light a date before the window', () => {
    const w = dayWindow(2026, 6, 5); // Jul 5 only
    expect(dateInWindow('2026-07-04', w.fromMs, w.toMs)).toBe(false);
  });

  it('does not light a date after the window', () => {
    const w = dayWindow(2026, 6, 5);
    expect(dateInWindow('2026-07-06', w.fromMs, w.toMs)).toBe(false);
  });

  it('lights the single day of a single-day window', () => {
    const w = dayWindow(2026, 6, 5); // month is 0-based => July
    expect(dateInWindow('2026-07-05', w.fromMs, w.toMs)).toBe(true);
  });

  it('lights a boundary day whose midnight equals the window end', () => {
    // window ends at Jul 1 end-of-day; the Jul 1 bar must be lit
    const w = { fromMs: new Date(2026, 5, 2).getTime(), toMs: new Date(2026, 6, 1, 23, 59, 59, 999).getTime() };
    expect(dateInWindow('2026-07-01', w.fromMs, w.toMs)).toBe(true);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web; npm test -- tape-window`
Expected: FAIL — cannot resolve `./tape-window`.

- [ ] **Step 3: Write the implementation**

Create `web/src/app/features/overview/tape-window.ts`:

```ts
/**
 * True when the tape bar for local calendar date `date` ("YYYY-MM-DD") overlaps
 * the filter window [fromMs, toMs]. Each bar spans its own local day; overlap,
 * not containment, so day-aligned window edges still light the edge bars.
 */
export function dateInWindow(date: string, fromMs: number, toMs: number): boolean {
  const [y, m, d] = date.split('-').map(Number);
  const dayStart = new Date(y, m - 1, d, 0, 0, 0, 0).getTime();
  const dayEnd = new Date(y, m - 1, d, 23, 59, 59, 999).getTime();
  return dayStart <= toMs && dayEnd >= fromMs;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web; npm test -- tape-window`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/overview/tape-window.ts web/src/app/features/overview/tape-window.spec.ts
git commit -m "feat(overview): add tape date-in-window highlight predicate"
```

---

### Task 3: Frontend — reshape API + component + template, delete old tape

Point the API service at the new shape, rewrite the component's tape logic to a single rolling ribbon with window highlighting and click-to-select, replace the two-row template, and delete the obsolete `tape-selection` files.

**Files:**
- Modify: `web/src/app/core/api.service.ts` (`V2Tape` interface ~59-65, `tape()` method ~225-230)
- Modify: `web/src/app/features/overview/overview.component.ts` (imports, `tape` signal, tape computeds/methods ~62-136, effect fetch ~146, template aliases ~181-182)
- Modify: `web/src/app/features/overview/overview.component.html` (tape section ~108-164)
- Delete: `web/src/app/features/overview/tape-selection.ts`
- Delete: `web/src/app/features/overview/tape-selection.spec.ts`

**Interfaces:**
- Consumes: `dateInWindow` (Task 2); `filter.window()`, `filter.selectDay(date: Date)`, `filter.setRange('month')` from `FilterService`.
- Produces: new `V2Tape` shape `{ days: { date: string; cost: number }[] }`; `api.tape(models?: string)`.

- [ ] **Step 1: Reshape the API service**

In `web/src/app/core/api.service.ts`, replace the `V2Tape` interface (~59-65) with:

```ts
export interface V2TapePoint {
  date: string; // "YYYY-MM-DD", local
  cost: number;
}

export interface V2Tape {
  days: V2TapePoint[];
}
```

Replace the `tape()` method (~225-230) with:

```ts
  tape(models?: string): Observable<V2Tape> {
    let p = new HttpParams();
    if (models) p = p.set('models', models);
    return this.http.get<V2Tape>(`${BASE}/api/v2/tape`, { params: p });
  }
```

- [ ] **Step 2: Rewrite the component tape logic**

In `web/src/app/features/overview/overview.component.ts`:

Remove the import of the deleted helper (line ~20): delete `import { selectedTapeDay, tapeDayDate } from './tape-selection';` and add `import { dateInWindow } from './tape-window';`.

Replace the tape computeds and methods (`tapeMax` ~62-66, `prevMax` ~70-74, `selectedDayIndex` ~78-80, `onTapeDayClick` ~107-117, `tapeBarClass` ~124-136) with:

```ts
  // Scale bars to the tallest day in the 30-day window.
  tapeMax = computed(() => {
    const t = this.tape();
    if (!t || t.days.length === 0) return 1;
    return Math.max(1, ...t.days.map((d) => d.cost));
  });

  // Range label for the panel title, e.g. "Jun 2 – Jul 1".
  tapeRangeLabel = computed(() => {
    const t = this.tape();
    if (!t || t.days.length === 0) return '';
    const fmt = (iso: string) => {
      const [y, m, d] = iso.split('-').map(Number);
      return new Date(y, m - 1, d).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    };
    return `${fmt(t.days[0].date)} – ${fmt(t.days[t.days.length - 1].date)}`;
  });

  /** True when bar `date` ("YYYY-MM-DD") falls inside the active filter window. */
  inWindow(date: string): boolean {
    const w = this.filter.window();
    return dateInWindow(date, w.fromMs, w.toMs);
  }

  /** True when the filter is a single-day custom window on exactly this date. */
  private isSoleSelectedDay(date: string): boolean {
    if (this.filter.range() !== 'custom') return false;
    const w = this.filter.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    if (from.toDateString() !== to.toDateString()) return false;
    const [y, m, d] = date.split('-').map(Number);
    return from.getFullYear() === y && from.getMonth() + 1 === m && from.getDate() === d;
  }

  /**
   * Click tape bar for `date`. Clicking the already-isolated single day toggles
   * back to "This month"; any other day narrows the filter to that day.
   */
  onTapeDayClick(date: string) {
    if (this.isSoleSelectedDay(date)) {
      this.filter.setRange('month');
      return;
    }
    const [y, m, d] = date.split('-').map(Number);
    this.filter.selectDay(new Date(y, m - 1, d));
  }

  /** Tailwind classes for the tape bar on `date`. Today gets a top marker. */
  tapeBarClass(date: string): string {
    const isToday = date === this.tape()?.days.at(-1)?.date;
    if (this.inWindow(date)) {
      return isToday
        ? 'bg-accent border-t border-yellow-200'
        : 'bg-accent group-hover:bg-accent';
    }
    return isToday
      ? 'bg-accent/30 border-t border-yellow-200 group-hover:bg-accent/60'
      : 'bg-accent/30 group-hover:bg-accent/60';
  }
```

Update the fetch call in the constructor `effect` (line ~146): change `this.api.tape(undefined, models).subscribe(...)` to:

```ts
      this.api.tape(models).subscribe((v) => this.tape.set(v));
```

Remove the now-dead template aliases (lines ~181-182): delete `tapeMax_ = this.tapeMax;` and `prevMax_ = this.prevMax;`.

- [ ] **Step 3: Replace the template tape section**

In `web/src/app/features/overview/overview.component.html`, replace the whole tape `@if` block (~108-164) with:

```html
  <!-- THE TAPE: rolling 30 days -->
  @if (tape(); as t) {
    <section class="panel">
      <div class="panel-title">
        <span>{{ tapeRangeLabel() }} · The tape</span>
        @if (t.days.length) {
          <span class="font-mono text-text flex items-center gap-1.5">
            <lucide-icon name="clock" class="w-3 h-3"></lucide-icon>
            Today · ${{ t.days[t.days.length - 1].cost | number : '1.2-2' }}
          </span>
        }
      </div>
      <div class="panel-body">
        <div class="flex items-end gap-[2px] h-[68px]">
          @for (pt of t.days; track pt.date) {
            <div class="flex-1 flex flex-col h-full min-w-0 group relative cursor-pointer"
                 (click)="onTapeDayClick(pt.date)">
              <div class="mt-auto"
                   [style.height.%]="(pt.cost / tapeMax()) * 100 || 8"
                   [class]="tapeBarClass(pt.date)"></div>
              <div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-1 bg-panel-2 border border-border-bright px-2 py-1 text-[10px] font-mono whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none z-20 rounded-sm">
                {{ pt.date }}<br>${{ pt.cost | number : '1.2-2' }}
              </div>
            </div>
          }
        </div>
        <div class="flex gap-[2px] mt-1">
          @for (pt of t.days; track pt.date; let i = $index) {
            <div class="flex-1 text-center text-[9px] font-mono"
                 [class]="inWindow(pt.date) ? 'text-accent' : 'text-muted'">
              @if (i === t.days.length - 1) {
                {{ +pt.date.slice(-2) }}↑
              } @else if (i === 0 || (i + 1) % 5 === 0) {
                {{ +pt.date.slice(-2) }}
              } @else {
                ·
              }
            </div>
          }
        </div>
        <div class="mt-3 flex items-center gap-4 text-[11px] text-muted">
          <span class="flex items-center gap-1.5"><span class="inline-block w-3 h-2 bg-accent"></span>In filter</span>
          <span class="flex items-center gap-1.5"><span class="inline-block w-3 h-2 bg-accent/30"></span>Outside filter</span>
          <span class="flex items-center gap-1.5"><span class="inline-block w-3 h-2 border-t border-yellow-200"></span>Today</span>
        </div>
      </div>
    </section>
  }
```

- [ ] **Step 4: Delete the obsolete tape-selection files**

```bash
git rm web/src/app/features/overview/tape-selection.ts web/src/app/features/overview/tape-selection.spec.ts
```

- [ ] **Step 5: Build and test**

Run: `cd web; npm test`
Expected: PASS — the `tape-selection` suite is gone; `tape-window` and all other suites pass.

Run: `cd web; npm run build`
Expected: builds with no TypeScript errors (confirms no lingering references to `V2Tape.current`/`previous`, `today_day`, `days_in_month`, `prevMax_`, `tapeMax_`, `selectedDayIndex`, or the deleted imports).

- [ ] **Step 6: Commit**

```bash
git add web/src/app/core/api.service.ts web/src/app/features/overview/overview.component.ts web/src/app/features/overview/overview.component.html
git commit -m "feat(overview): render tape as rolling 30-day ribbon with filter highlight"
```

---

### Task 4: Manual verification

**Files:** none (verification only).

- [ ] **Step 1: Run the app**

Run: `cargo tauri dev` from the repo root. Open the Overview page.

- [ ] **Step 2: Verify behavior**

Confirm:
- The tape is a single 30-bar row ending today; no previous-month row.
- Hovering any bar shows `YYYY-MM-DD` + `$cost`.
- Switching the filter (Today / This week / This month / Last 30d / Custom) re-lights the bars without a full reload flicker — only the highlight changes for windows within the 30 days.
- Clicking a bar narrows the filter to that day (bars re-light to just that one); clicking it again returns to This month.
- With a model chip deselected, bar heights drop to reflect the filtered cost.

- [ ] **Step 3: Commit (if any tweaks were needed)**

Only if Step 2 required fixes. Otherwise this task closes with no commit.

---

## Self-Review

**Spec coverage:**
- Single 30-day strip, exact local dates, ending today → Task 1 (`tape_last_n_days` + `day_labels`), Task 3 template.
- Highlight window across bars → Task 2 (`dateInWindow`) + Task 3 (`inWindow`, `tapeBarClass`).
- Hover date + cost → Task 3 template tooltip.
- Click-to-select-day preserved → Task 3 (`onTapeDayClick`, `isSoleSelectedDay`).
- Typed serde DTO replacing `json!` → Task 1 (`TapeResponse`/`TapePoint`).
- Reuse rolling helpers, keep model LIKE filter → Task 1 (`tape_last_n_days`).
- Deletions (previous row, `prevMax`, `tape-selection.ts`, `month` param) → Task 1 (`month`, `tape_for_month`) + Task 3 (`prevMax`, template row, `tape-selection` files).
- Rust test across window edges incl. exclusion → Task 1 tests. (Note: fixed Feb→March calendar test is intentionally omitted — `day_index_for` uses `Local::now()` with no injectable clock, and the rolling helpers do pure date subtraction so there is no day-of-month boundary special-case to exercise; the relative-to-today seeding covers the 29-in / 30-out edge that motivated Lane B.)
- Angular test on `inWindow` logic → Task 2.
- Privacy: read-only aggregate, no new listener/outbound → unchanged by all tasks.

**Placeholder scan:** none — every code step carries complete code.

**Type consistency:** `tape_last_n_days(&conn, days, &models) -> Vec<TapePoint>` and `TapePoint { date, cost }` used identically in Task 1 impl and tests. `V2Tape { days: V2TapePoint[] }` produced in Task 3 Step 1 and consumed in Steps 2-3. `dateInWindow(date, fromMs, toMs)` defined in Task 2, called in Task 3 `inWindow`.
