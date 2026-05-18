# Filter Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the two filter bugs in the andon dashboard: model chips that produce all-or-nothing results, and a "Custom…" range with no date picker UI.

**Architecture:** Three surgical changes — backend `FilterQuery::model_clause` switches to substring `LIKE` matching against family tokens (opus/sonnet/haiku); frontend `FilterService` gains seeded custom-range support and treats non-default ranges as active filters; `filter-bar` component reveals two native `<input type="date">` controls when Custom is active. No schema, no DTO, no endpoint changes. Tests are explicitly deferred to a future session per `docs/superpowers/specs/2026-05-18-test-harness-plan.md`.

**Tech Stack:** Rust (rusqlite, axum, tokio), Angular 21 standalone, signals, Tailwind, lucide-angular.

**Reference spec:** `docs/superpowers/specs/2026-05-18-filter-fixes-design.md`

**Branch:** `fix/filters` (already created and checked out; specs already committed there).

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src-tauri/src/api/routes.rs` | Modify | `FilterQuery::model_clause` substring matching; one inline call site (line ~1635) routed through the same helper. |
| `web/src/app/core/filter.service.ts` | Modify | Add `enterCustomMode()`, `setCustomFrom`, `setCustomTo`; fix `window()` custom branch; update `hasActiveFilters` and `clearFilters`. |
| `web/src/app/shared/filter-bar.component.ts` | Modify | Wire "Custom…" chip to `enterCustomMode()`; render two date inputs when range is custom. |
| `web/src/styles.css` | Modify | One CSS rule to invert the native date-picker indicator for the dark theme. |

---

## Task 1: Backend — substring model filter

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (struct impl at ~1186–1210, call site at ~1635)

- [ ] **Step 1: Replace `FilterQuery::model_clause` with substring matching**

Open `src-tauri/src/api/routes.rs`. Locate the existing impl (around lines 1186–1210):

```rust
fn model_clause(&self, col: &str) -> (String, Vec<String>) {
    let models = self.model_list();
    if models.is_empty() {
        (String::new(), vec![])
    } else {
        let placeholders = models.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        (format!(" AND {col} IN ({placeholders})"), models)
    }
}
```

Replace the body with substring matching against family tokens:

```rust
fn model_clause(&self, col: &str) -> (String, Vec<String>) {
    let models = self.model_list();
    if models.is_empty() {
        return (String::new(), vec![]);
    }
    let likes: Vec<String> = models.iter().map(|m| format!("%{}%", m.to_lowercase())).collect();
    let ored = likes
        .iter()
        .map(|_| format!("LOWER({col}) LIKE ?"))
        .collect::<Vec<_>>()
        .join(" OR ");
    (format!(" AND ({ored})"), likes)
}
```

- [ ] **Step 2: Route the inline call site through `model_clause`**

In the same file, around line 1635, find the inline `model_filter_sql` builder inside the sessions list endpoint:

```rust
let model_filter_sql = if model_list.is_empty() {
    String::new()
} else {
    let placeholders = model_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    format!(
        " AND EXISTS (SELECT 1 FROM cost_entries c
                      WHERE c.session_id = s.session_id AND c.model IN ({placeholders}))"
    )
};
```

This uses `IN (...)` against `cost_entries.model` (joined via `EXISTS`). Replace it with a substring version built off the same helper output. Add this inline:

```rust
let (model_inner_sql, model_inner_vals) = filt.model_clause("c.model");
let model_filter_sql = if model_inner_sql.is_empty() {
    String::new()
} else {
    // model_inner_sql starts with " AND (...)". Strip the leading " AND " for embedding.
    let inner = model_inner_sql.trim_start_matches(" AND ");
    format!(
        " AND EXISTS (SELECT 1 FROM cost_entries c
                      WHERE c.session_id = s.session_id AND {inner})"
    )
};
```

Then find where the old `model_list` was bound to query parameters in this function (search downward for `model_list` usages and the `params!`/`rusqlite::params_from_iter` call that includes it). Replace any iteration over `model_list` with iteration over `model_inner_vals`. If `model_list` is no longer used after this change, delete the `let model_list = filt.model_list();` line at the top of the function.

- [ ] **Step 3: Build the backend**

Run from repo root:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: builds cleanly. If a parameter-count mismatch panic occurs at runtime, the binding update in Step 2 was missed — re-read the function and align bound params with the new `model_inner_vals`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "fix(api): substring-match model filter so chip selections actually filter"
```

---

## Task 2: FilterService — custom range support + active-filter semantics

**Files:**
- Modify: `web/src/app/core/filter.service.ts` (entire file rewrite for clarity)

- [ ] **Step 1: Rewrite `filter.service.ts`**

Open `web/src/app/core/filter.service.ts` and replace its contents with:

```typescript
import { Injectable, computed, signal } from '@angular/core';

export type RangePreset = 'today' | 'week' | 'month' | '30d' | 'custom';

export interface CustomRange {
  fromMs: number;
  toMs: number;
}

// Family tokens; backend matches via substring on the stored full model ID
// (e.g. "claude-opus-4-5-20251001" matches "opus").
const ALL_MODELS = ['opus', 'sonnet', 'haiku'];
const DEFAULT_RANGE: RangePreset = 'month';

@Injectable({ providedIn: 'root' })
export class FilterService {
  readonly range = signal<RangePreset>(DEFAULT_RANGE);
  readonly customRange = signal<CustomRange | null>(null);
  readonly models = signal<Set<string>>(new Set(ALL_MODELS));
  readonly search = signal<string>('');
  readonly repos = signal<string[]>([]);

  readonly window = computed<{ fromMs: number; toMs: number }>(() => {
    const r = this.range();
    if (r === 'custom') {
      const cr = this.customRange();
      if (cr) return cr;
      // Custom selected but no range yet — fall back to current month.
      return monthToToday();
    }
    const now = new Date();
    const todayEnd = endOfDay(now);
    switch (r) {
      case 'today':
        return { fromMs: startOfDay(now).getTime(), toMs: todayEnd.getTime() };
      case 'week': {
        const start = new Date(now);
        const dow = (start.getDay() + 6) % 7; // Monday = 0
        start.setDate(start.getDate() - dow);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
      case 'month':
        return monthToToday();
      case '30d': {
        const start = new Date(now);
        start.setDate(start.getDate() - 29);
        return { fromMs: startOfDay(start).getTime(), toMs: todayEnd.getTime() };
      }
    }
  });

  readonly modelsCsv = computed(() => {
    const s = this.models();
    return s.size === ALL_MODELS.length ? '' : [...s].join(',');
  });

  readonly reposCsv = computed(() => {
    const r = this.repos();
    return r.length ? r.join(',') : '';
  });

  readonly hasActiveFilters = computed(() => {
    return (
      this.range() !== DEFAULT_RANGE ||
      this.models().size !== ALL_MODELS.length ||
      this.search() !== '' ||
      this.repos().length > 0
    );
  });

  readonly rangeLabel = computed(() => {
    const r = this.range();
    const w = this.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    const fmt = (d: Date) => d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    switch (r) {
      case 'today':
        return `today · ${fmt(from)}`;
      case 'week':
        return `this week · ${fmt(from)} – today`;
      case 'month': {
        const monthName = from.toLocaleDateString(undefined, { month: 'long' });
        const dayOfMonth = new Date().getDate();
        const daysInMonth = new Date(from.getFullYear(), from.getMonth() + 1, 0).getDate();
        return `${monthName} · day ${dayOfMonth} of ${daysInMonth}`;
      }
      case '30d':
        return `last 30d · ${fmt(from)} – ${fmt(to)}`;
      case 'custom':
        return `custom · ${fmt(from)} – ${fmt(to)}`;
    }
  });

  setRange(r: RangePreset) {
    this.range.set(r);
  }

  enterCustomMode() {
    // Seed customRange from whatever window is currently active so the
    // date inputs are never blank when Custom is first opened.
    const seed = this.window();
    this.customRange.set({ fromMs: seed.fromMs, toMs: seed.toMs });
    this.range.set('custom');
  }

  setCustomFrom(ms: number) {
    const cur = this.customRange() ?? this.window();
    const next = ms > cur.toMs ? { fromMs: cur.toMs, toMs: ms } : { fromMs: ms, toMs: cur.toMs };
    this.customRange.set(next);
  }

  setCustomTo(ms: number) {
    const cur = this.customRange() ?? this.window();
    const next = ms < cur.fromMs ? { fromMs: ms, toMs: cur.fromMs } : { fromMs: cur.fromMs, toMs: ms };
    this.customRange.set(next);
  }

  toggleModel(m: string) {
    const next = new Set(this.models());
    if (next.has(m)) next.delete(m);
    else next.add(m);
    this.models.set(next);
  }

  setSearch(s: string) {
    this.search.set(s);
  }

  clearFilters() {
    this.range.set(DEFAULT_RANGE);
    this.customRange.set(null);
    this.models.set(new Set(ALL_MODELS));
    this.search.set('');
    this.repos.set([]);
  }

  allModels(): readonly string[] {
    return ALL_MODELS;
  }
}

function monthToToday(): { fromMs: number; toMs: number } {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), 1);
  return { fromMs: start.getTime(), toMs: endOfDay(now).getTime() };
}

function startOfDay(d: Date): Date {
  const x = new Date(d);
  x.setHours(0, 0, 0, 0);
  return x;
}

function endOfDay(d: Date): Date {
  const x = new Date(d);
  x.setHours(23, 59, 59, 999);
  return x;
}
```

- [ ] **Step 2: Type-check the web app**

Run from repo root:

```bash
cd web && npx tsc --noEmit
```

Expected: no errors. (If `npx tsc` is slow, `npm run build` works too but is heavier.)

- [ ] **Step 3: Commit**

```bash
git add web/src/app/core/filter.service.ts
git commit -m "fix(web): repair custom date range and seed picker from current window"
```

---

## Task 3: Filter-bar — Custom chip + inline date inputs

**Files:**
- Modify: `web/src/app/shared/filter-bar.component.ts`
- Modify: `web/src/styles.css`

- [ ] **Step 1: Update `filter-bar.component.ts`**

Open `web/src/app/shared/filter-bar.component.ts` and replace its contents with:

```typescript
import { Component, inject } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { FilterService, RangePreset } from '../core/filter.service';

@Component({
  selector: 'app-filter-bar',
  imports: [LucideAngularModule],
  template: `
    <div class="sticky top-0 z-10 bg-panel/90 backdrop-blur-sm border-b border-border">
      <div class="px-6 py-2.5 flex items-center gap-3 flex-wrap">
        <span class="filter-label flex items-center gap-1.5">
          <lucide-icon name="calendar" class="w-3 h-3"></lucide-icon>Range
        </span>
        <div class="flex items-center gap-1.5">
          @for (r of ranges; track r.id) {
            <button class="filter-chip"
                    [attr.data-active]="filter.range() === r.id ? 'true' : null"
                    (click)="onRangeClick(r.id)">{{ r.label }}</button>
          }
        </div>
        @if (filter.range() === 'custom') {
          <div class="flex items-center gap-2 ml-1">
            <input type="date"
                   class="filter-date-input"
                   [value]="customFromIso()"
                   (change)="onFromChange($any($event.target).value)" />
            <span class="text-muted text-[11px] font-mono">–</span>
            <input type="date"
                   class="filter-date-input"
                   [value]="customToIso()"
                   (change)="onToChange($any($event.target).value)" />
          </div>
        }
        <span class="ml-auto text-[11px] font-mono text-muted">{{ filter.rangeLabel() }}</span>
      </div>
      <div class="px-6 py-2.5 flex items-center gap-3 border-t border-border/50">
        <span class="filter-label flex items-center gap-1.5">
          <lucide-icon name="layers" class="w-3 h-3"></lucide-icon>Model
        </span>
        <div class="flex items-center gap-1.5">
          @for (m of filter.allModels(); track m) {
            <button class="filter-chip"
                    [attr.data-active]="filter.models().has(m) ? 'true' : null"
                    (click)="filter.toggleModel(m)">
              {{ m }}
            </button>
          }
        </div>
        @if (filter.hasActiveFilters()) {
          <button class="ml-auto text-muted hover:text-text font-mono text-[11px] flex items-center gap-1"
                  (click)="filter.clearFilters()">
            <lucide-icon name="x" class="w-3 h-3"></lucide-icon>Clear
          </button>
        }
      </div>
    </div>
  `,
})
export class FilterBarComponent {
  filter = inject(FilterService);
  ranges: { id: RangePreset; label: string }[] = [
    { id: 'today', label: 'Today' },
    { id: 'week', label: 'This week' },
    { id: 'month', label: 'This month' },
    { id: '30d', label: 'Last 30d' },
    { id: 'custom', label: 'Custom…' },
  ];

  onRangeClick(id: RangePreset) {
    if (id === 'custom') this.filter.enterCustomMode();
    else this.filter.setRange(id);
  }

  onFromChange(iso: string) {
    const ms = parseDateInput(iso, false);
    if (ms !== null) this.filter.setCustomFrom(ms);
  }

  onToChange(iso: string) {
    const ms = parseDateInput(iso, true);
    if (ms !== null) this.filter.setCustomTo(ms);
  }

  customFromIso(): string {
    const cr = this.filter.customRange();
    return cr ? toIsoDate(cr.fromMs) : '';
  }

  customToIso(): string {
    const cr = this.filter.customRange();
    return cr ? toIsoDate(cr.toMs) : '';
  }
}

function parseDateInput(iso: string, endOfDay: boolean): number | null {
  // iso is "YYYY-MM-DD" from <input type="date"> in the browser's locale-agnostic form.
  if (!iso) return null;
  const [y, m, d] = iso.split('-').map(Number);
  if (!y || !m || !d) return null;
  const date = new Date(y, m - 1, d);
  if (endOfDay) date.setHours(23, 59, 59, 999);
  else date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function toIsoDate(ms: number): string {
  const d = new Date(ms);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}
```

- [ ] **Step 2: Add date-input styling for the dark theme**

Open `web/src/styles.css` and append this block at the end of the file:

```css
.filter-date-input {
  background: transparent;
  border: 1px solid rgb(var(--border) / 0.6);
  color: rgb(var(--text));
  font-family: ui-monospace, monospace;
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 4px;
  color-scheme: dark;
}
.filter-date-input:focus {
  outline: none;
  border-color: rgb(var(--text) / 0.5);
}
.filter-date-input::-webkit-calendar-picker-indicator {
  filter: invert(0.8);
  cursor: pointer;
}
```

(`color-scheme: dark` is the modern way to theme the native picker; the `::-webkit-calendar-picker-indicator` rule is a fallback for Chromium.)

- [ ] **Step 3: Build the SPA**

Run from repo root:

```bash
cd web && npm run build
```

Expected: succeeds. The compiled SPA lands in `web/dist/web/browser`.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/shared/filter-bar.component.ts web/src/styles.css
git commit -m "fix(web): reveal date pickers for custom range and treat off-default range as active filter"
```

---

## Task 4: Manual verification

**Files:** none (verification only)

- [ ] **Step 1: Launch dev mode**

From repo root:

```bash
cd src-tauri && cargo tauri dev
```

Expected: window opens, tray icon appears, dashboard loads.

- [ ] **Step 2: Verify model filter actually filters**

In the running app:
1. Note the current overview cost number with all 3 chips active.
2. Deselect "sonnet" and "haiku" so only "opus" is highlighted.
3. Expected: cost number drops to the opus-only subtotal (non-zero, assuming any opus usage in the DB). Previously this produced 0.
4. Deselect all chips. Expected: numbers match the all-3-selected state (zero-selected and all-selected both mean "no filter").

- [ ] **Step 3: Verify custom range UI**

1. Click "Custom…". Expected: two date inputs appear, pre-filled with the current month start and today.
2. Change "from" to a date one week ago. Expected: overview cards update to reflect the new window; `rangeLabel` shows `custom · <from> – today`.
3. Set "from" to a date later than "to". Expected: the values silently swap; UI never shows an inverted range.
4. Switch back to "This month" chip. Expected: date inputs disappear; data restores to the month-to-date view.
5. Click "Custom…" again. Expected: pickers reappear, seeded from the current window.

- [ ] **Step 4: Verify Clear button surfaces on non-default range**

1. With all model chips active and no search, click "Today". Expected: "Clear" link appears at the right edge of the model row (previously it only appeared on model/search changes).
2. Click "Clear". Expected: range returns to "This month", chips reset to all active.

- [ ] **Step 5: Confirm no hardcoded family lists elsewhere**

```bash
git grep -n "ALL_MODELS\|opus.*sonnet.*haiku\|sonnet.*haiku.*opus"
```

Expected: only the `ALL_MODELS` definition in `filter.service.ts`. Any other hit is a leftover assumption to clean up.

- [ ] **Step 6: Build the release binary**

```bash
cd src-tauri && cargo build --release
```

Expected: builds cleanly. (Quick check that release-mode optimizations don't trip anything; the dev build already proved correctness.)

---

## Task 5: Open the pull request

**Files:** none

- [ ] **Step 1: Push the branch**

```bash
git push -u origin fix/filters
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "fix: model filter substring matching and custom date range UI" --body "$(cat <<'EOF'
## Summary
- Backend `FilterQuery::model_clause` now matches stored model IDs by family substring (`LOWER(model) LIKE '%opus%'` etc.), fixing the all-or-none behavior caused by exact-match against `opus`/`sonnet`/`haiku` family tokens.
- Frontend filter bar reveals two native `<input type="date">` controls when Custom is selected, seeded from the current window. `FilterService` adds `enterCustomMode`, `setCustomFrom`, `setCustomTo`, with clamping so `from ≤ to`.
- Non-default range now counts as an active filter, so Clear surfaces and reset works.
- Specs committed: `docs/superpowers/specs/2026-05-18-filter-fixes-design.md` and the deferred `2026-05-18-test-harness-plan.md`.

## Test plan
- [ ] Launch app; only "opus" chip active → overview cost is non-zero.
- [ ] Deselect all chips → behaves like "all selected".
- [ ] Click "Custom…" → date inputs appear, pre-filled with current month bounds.
- [ ] Pick a single past day → overview updates accordingly.
- [ ] Switch off Custom and back → pickers re-seed from current window.
- [ ] Off-default range (e.g. Today) → Clear button appears and resets to This month.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL printed. Share with the user.

---

## Self-Review

**Spec coverage:**
- Model substring matching → Task 1 (backend) + Task 2 (frontend untouched on this — chips already emit family tokens).
- Custom range UI + seeding → Tasks 2 and 3.
- `hasActiveFilters` includes range, `clearFilters` resets range/customRange → Task 2.
- Native date input dark-mode styling → Task 3 Step 2.
- Edge cases (swap on inversion, partial entry never sent, junk token returns empty) → covered by `setCustomFrom`/`setCustomTo` clamping and existing empty-state components; verified in Task 4.
- Commit plan (one per concern) → Tasks 1/2/3 each end in a single commit; matches the spec's four-commit plan (specs already committed earlier).
- Out-of-scope items (tests, popover picker, refactor) → none included. ✓

**Placeholder scan:** No TBDs, no "add error handling" hand-waves, no "similar to Task N" references. Every code step contains the actual code. ✓

**Type consistency:**
- `enterCustomMode()`, `setCustomFrom(ms: number)`, `setCustomTo(ms: number)`, `customRange()` returning `CustomRange | null` — referenced identically in Task 2 (definition) and Task 3 (consumption). ✓
- `parseDateInput(iso, endOfDay)` returns `number | null` and is consumed via the `null` check in `onFromChange`/`onToChange`. ✓
- Backend `model_clause` signature `(&self, col: &str) -> (String, Vec<String>)` unchanged. ✓
