# Tape Day-Select Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Overview's tape (the per-day cost bar chart) clickable so clicking a current-month day-bar narrows the whole dashboard to that single day.

**Architecture:** Frontend-only. A selected day is a 1-day `custom` filter window — the Overview's existing filter `effect()` already re-fetches every tile from `filter.window()`, so narrowing the window *is* "update the overview." `FilterService` gains a one-shot `selectDay()` setter; a new pure helper derives which tape day the filter currently isolates; `OverviewComponent` wires the click and the highlight.

**Tech Stack:** Angular 21 (standalone components, signals — `signal`/`computed`/`effect`), TypeScript, Tailwind v4, Vitest.

**Design spec:** `docs/superpowers/specs/2026-05-21-tape-day-select-design.md`

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `web/src/app/core/filter.service.ts` | Filter state (range, custom window, models) | Add `selectDay()`; add a single-day case to `rangeLabel` |
| `web/src/app/core/filter.service.spec.ts` | FilterService unit tests | Add tests for the two changes above |
| `web/src/app/features/overview/tape-selection.ts` | Pure helpers mapping filter state ⇄ tape day index | **New file** — `selectedTapeDay`, `tapeDayDate` |
| `web/src/app/features/overview/tape-selection.spec.ts` | Unit tests for the pure helpers | **New file** |
| `web/src/app/features/overview/overview.component.ts` | Overview page controller | Add `selectedDayIndex` computed + `onTapeDayClick` method |
| `web/src/app/features/overview/overview.component.html` | Overview template | Tape day-bar gets click handler + selected highlight |

Three tasks, in order. Tasks 1 and 2 are pure logic and are fully unit-tested (TDD). Task 3 wires them into the component and template; `OverviewComponent` has no unit-test harness in this repo (it is a large data-fetching component), so Task 3 is verified by `npm run build` plus a manual check — consistent with the existing codebase.

**Notes for an engineer new to this codebase:**
- `npm test` runs `vitest run` (CI mode, no watch). Vitest globals (`describe`/`it`/`expect`) are enabled — spec files do **not** import them. Match that style.
- To run one spec file: `npm test -- <filename-fragment>` (Vitest treats a positional arg as a filename filter).
- All commands below assume you start at the repo root, `D:\Repos\andon`. The shell is PowerShell; `cd web` then a command on the next line, or `cd web; npm test` on one line.
- Conventional Commits, no emojis. Every commit ends with the trailer shown in the commit steps.
- US English in all code, comments, and identifiers.

---

## Task 1: `FilterService.selectDay()` and single-day `rangeLabel`

A selected tape day is a 1-day `custom` window. `FilterService` already models custom windows (`customRange` signal, `range` set to `'custom'`) but has no one-shot setter — `enterCustomMode` + `setCustomFrom`/`setCustomTo` is a multi-call flow built for the date-input UI. Add `selectDay(day: Date)` that sets both signals in one call. Also fix `rangeLabel`: a 1-day custom window currently renders the redundant `custom · May 15 – May 15`; make it render `custom · May 15`.

**Files:**
- Modify: `web/src/app/core/filter.service.ts`
- Test: `web/src/app/core/filter.service.spec.ts`

- [ ] **Step 1: Write the failing tests**

In `web/src/app/core/filter.service.spec.ts`, add these three tests inside the existing `describe('FilterService', () => { ... })` block — place them just before the block's closing `});` (after the existing `refresh()` test):

```typescript
  it('selectDay sets a single-day custom window', () => {
    const s = createService();
    s.selectDay(new Date(2026, 4, 15)); // May 15, 2026 (month is 0-based)
    expect(s.range()).toBe('custom');
    const w = s.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    expect(from.getFullYear()).toBe(2026);
    expect(from.getMonth()).toBe(4);
    expect(from.getDate()).toBe(15);
    expect(from.getHours()).toBe(0);
    expect(to.getDate()).toBe(15);
    expect(to.getHours()).toBe(23);
  });

  it('rangeLabel for a single-day custom window omits the duplicate date', () => {
    const s = createService();
    s.selectDay(new Date(2026, 4, 15));
    const label = s.rangeLabel();
    expect(label.startsWith('custom · ')).toBe(true);
    expect(label).not.toContain('–'); // en-dash range separator
  });

  it('rangeLabel keeps the date range for a multi-day custom window', () => {
    const s = createService();
    s.customRange.set({
      fromMs: new Date(2026, 4, 1).getTime(),
      toMs: new Date(2026, 4, 20).getTime(),
    });
    s.setRange('custom');
    expect(s.rangeLabel()).toContain('–');
  });
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd web; npm test -- filter.service`
Expected: FAIL — `s.selectDay is not a function` for the first two tests. (The third test may already pass; that is fine — it is a regression guard for the `rangeLabel` change in Step 3.)

- [ ] **Step 3: Implement `selectDay` and the `rangeLabel` single-day case**

In `web/src/app/core/filter.service.ts`, change the `rangeLabel` computed's `custom` case. The current code is:

```typescript
      case 'custom':
        return `custom · ${fmt(from)} – ${fmt(to)}`;
```

Replace it with:

```typescript
      case 'custom': {
        // A single-day custom window (e.g. a tape day-select) has from and to
        // on the same calendar day — drop the redundant second date.
        if (from.toDateString() === to.toDateString()) {
          return `custom · ${fmt(from)}`;
        }
        return `custom · ${fmt(from)} – ${fmt(to)}`;
      }
```

Then add the `selectDay` method. Place it immediately after the `setCustomTo` method and before `toggleModel`:

```typescript
  /**
   * Narrow the filter to a single day — a 1-day `custom` window spanning that
   * day's start-of-day to end-of-day. Used by the Overview tape's day-select.
   */
  selectDay(day: Date) {
    this.customRange.set({
      fromMs: startOfDay(day).getTime(),
      toMs: endOfDay(day).getTime(),
    });
    this.range.set('custom');
  }
```

(`startOfDay` and `endOfDay` are the module-private helpers already defined at the bottom of this file — they return a `Date`, so `.getTime()` converts to the epoch-ms the `customRange` signal stores.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd web; npm test -- filter.service`
Expected: PASS — all tests in `filter.service.spec.ts`, including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/core/filter.service.ts web/src/app/core/filter.service.spec.ts
git commit -m "$(cat <<'EOF'
feat(filter): add selectDay for one-shot single-day windows

selectDay sets a 1-day custom window in one call; rangeLabel renders
such a window as `custom · May 15` instead of the redundant
`custom · May 15 – May 15`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `tape-selection.ts` pure helpers

The tape needs to know which day-bar (if any) the current filter isolates — to highlight it and to implement the click-to-deselect toggle. That is a pure function of `(range, window, tapeMonth)`. Extract it into a standalone pure module with no Angular imports, mirroring the existing `web/src/app/features/overview/budget-indicator.ts` precedent. This keeps the logic unit-testable without a component harness.

Two functions:
- `selectedTapeDay(range, filterWindow, tapeMonth)` — the 0-based index of the isolated tape day, or `null`.
- `tapeDayDate(tapeMonth, index)` — the local `Date` for a day index, used to build the window passed to `selectDay`.

**Files:**
- Create: `web/src/app/features/overview/tape-selection.ts`
- Test: `web/src/app/features/overview/tape-selection.spec.ts` (create)

- [ ] **Step 1: Write the failing tests**

Create `web/src/app/features/overview/tape-selection.spec.ts` with this exact content:

```typescript
import { selectedTapeDay, tapeDayDate } from './tape-selection';

// Build a 1-day window the way FilterService.selectDay does, for the given
// local calendar day (month is 0-based, matching the Date constructor).
function dayWindow(year: number, month: number, day: number) {
  return {
    fromMs: new Date(year, month, day, 0, 0, 0, 0).getTime(),
    toMs: new Date(year, month, day, 23, 59, 59, 999).getTime(),
  };
}

describe('selectedTapeDay', () => {
  it('returns the 0-based index for a single-day custom window in the tape month', () => {
    expect(selectedTapeDay('custom', dayWindow(2026, 4, 15), '2026-05')).toBe(14);
  });

  it('returns null when the range is not custom', () => {
    expect(selectedTapeDay('month', dayWindow(2026, 4, 15), '2026-05')).toBeNull();
  });

  it('returns null for a multi-day custom window', () => {
    const w = {
      fromMs: new Date(2026, 4, 1).getTime(),
      toMs: new Date(2026, 4, 20).getTime(),
    };
    expect(selectedTapeDay('custom', w, '2026-05')).toBeNull();
  });

  it('returns null when the selected day is outside the tape month', () => {
    // April 15 selected while the tape shows May
    expect(selectedTapeDay('custom', dayWindow(2026, 3, 15), '2026-05')).toBeNull();
  });

  it('returns null when the tape month has not loaded', () => {
    expect(selectedTapeDay('custom', dayWindow(2026, 4, 15), null)).toBeNull();
  });
});

describe('tapeDayDate', () => {
  it('builds the local date for a 0-based day index of the tape month', () => {
    const d = tapeDayDate('2026-05', 14);
    expect(d.getFullYear()).toBe(2026);
    expect(d.getMonth()).toBe(4); // May, 0-based
    expect(d.getDate()).toBe(15);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd web; npm test -- tape-selection`
Expected: FAIL — cannot resolve module `./tape-selection` (the file does not exist yet).

- [ ] **Step 3: Create the implementation**

Create `web/src/app/features/overview/tape-selection.ts` with this exact content:

```typescript
import type { CustomRange, RangePreset } from '../../core/filter.service';

/**
 * The 0-based tape day-bar index that the current filter isolates, or null.
 *
 * Returns an index only when the filter is a single-day `custom` window AND
 * that day falls inside `tapeMonth`. A non-custom range, a multi-day custom
 * range, or a single day outside the displayed month all yield null — no tape
 * day is highlighted.
 *
 * @param range       the filter's current range preset
 * @param filterWindow the filter's resolved window (`filter.window()`)
 * @param tapeMonth   the tape's month as `"YYYY-MM"`, or null before it loads
 */
export function selectedTapeDay(
  range: RangePreset,
  filterWindow: CustomRange,
  tapeMonth: string | null,
): number | null {
  if (range !== 'custom' || tapeMonth === null) return null;
  const from = new Date(filterWindow.fromMs);
  const to = new Date(filterWindow.toMs);
  // A selected day is exactly one calendar day wide.
  if (from.toDateString() !== to.toDateString()) return null;
  const ym = `${from.getFullYear()}-${String(from.getMonth() + 1).padStart(2, '0')}`;
  if (ym !== tapeMonth) return null;
  return from.getDate() - 1;
}

/** The local `Date` for 0-based day `index` of `tapeMonth` (`"YYYY-MM"`). */
export function tapeDayDate(tapeMonth: string, index: number): Date {
  const [year, month] = tapeMonth.split('-').map(Number);
  return new Date(year, month - 1, index + 1);
}
```

(`CustomRange` and `RangePreset` are exported types from `filter.service.ts`. They are imported with `import type` — erased at compile time, so this module pulls in no Angular runtime code, matching `budget-indicator.ts`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd web; npm test -- tape-selection`
Expected: PASS — all 6 tests in `tape-selection.spec.ts`.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/overview/tape-selection.ts web/src/app/features/overview/tape-selection.spec.ts
git commit -m "$(cat <<'EOF'
feat(overview): add tape-selection pure helpers

selectedTapeDay maps filter state to the isolated tape day index;
tapeDayDate maps a day index back to a Date. Pure module, no Angular
imports, mirroring budget-indicator.ts.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Wire the tape into `OverviewComponent`

Connect the helpers from Tasks 1 and 2 to the template: derive the selected day index, handle clicks on a day-bar (select, or toggle off if it is already selected), and render the selected highlight. Future days (no data yet) stay non-clickable.

**Files:**
- Modify: `web/src/app/features/overview/overview.component.ts`
- Modify: `web/src/app/features/overview/overview.component.html`

`OverviewComponent` has no unit-test harness in this repo (it is a large data-fetching component — the design spec confirms this is intentional). This task is verified by a clean `npm run build` and a manual check.

- [ ] **Step 1: Add the import to `overview.component.ts`**

In `web/src/app/features/overview/overview.component.ts`, the current line 19 is:

```typescript
import { budgetTextClass, budgetBarClass } from './budget-indicator';
```

Add this line immediately after it:

```typescript
import { selectedTapeDay, tapeDayDate } from './tape-selection';
```

(`computed` is already imported from `@angular/core` on line 2 — no change needed there.)

- [ ] **Step 2: Add the `selectedDayIndex` computed**

In the same file, the `prevMax` computed ends like this (around line 73):

```typescript
  prevMax = computed(() => {
    const t = this.tape();
    if (!t) return 1;
    return Math.max(1, ...t.previous);
  });
```

Immediately after that closing `});`, add:

```typescript
  // The 0-based tape day-bar the filter currently isolates, or null.
  // Drives the tape highlight and the click-to-deselect toggle.
  selectedDayIndex = computed(() =>
    selectedTapeDay(this.filter.range(), this.filter.window(), this.tape()?.month ?? null),
  );
```

- [ ] **Step 3: Add the `onTapeDayClick` method**

In the same file, the `modelLabel` method ends like this (around line 93):

```typescript
  /** Display name: claude-opus-4-7 → "opus 4.7" */
  modelLabel(m: string | null): string {
    if (!m) return '—';
    return m
      .replace(/^claude-/, '')
      .replace(/-(\d+)-(\d+)(?:-\d+)?$/, ' $1.$2');
  }
```

Immediately after that closing `}`, add:

```typescript
  /**
   * Click on tape day-bar `i` (0-based). Future days have no data and are
   * ignored. Clicking the already-selected day toggles back to "This month";
   * clicking any other past-or-today day narrows the filter to that day.
   */
  onTapeDayClick(i: number) {
    const t = this.tape();
    if (!t) return;
    // Future days are not selectable — mirrors the template's `today_day ?? 31`.
    if (i + 1 > (t.today_day ?? 31)) return;
    if (i === this.selectedDayIndex()) {
      this.filter.setRange('month'); // toggle the selection off
    } else {
      this.filter.selectDay(tapeDayDate(t.month, i));
    }
  }
```

- [ ] **Step 4: Make the tape day-bar column clickable**

In `web/src/app/features/overview/overview.component.html`, find the current-month day-bar column. The current code (around line 123) is:

```html
          @for (cost of t.current; track $index; let i = $index) {
            <div class="flex-1 flex flex-col h-full cursor-pointer min-w-0 group relative">
```

Replace that `<div>` opening tag with:

```html
          @for (cost of t.current; track $index; let i = $index) {
            <div class="flex-1 flex flex-col h-full min-w-0 group relative"
                 [class.cursor-pointer]="i + 1 <= (t.today_day ?? 31)"
                 (click)="onTapeDayClick(i)">
```

(`cursor-pointer` moves out of the static class list into a binding so only past-or-today bars show the pointer cursor; future bars get the default cursor.)

- [ ] **Step 5: Add the selected highlight to the day-bar**

In the same file, the inner bar `<div>` (immediately inside the column, around lines 124-128) currently is:

```html
              <div class="mt-auto"
                   [style.height.%]="(cost / tapeMax_()) * 100 || (i + 1 > (t.today_day ?? 31) ? 100 : 8)"
                   [class]="i + 1 === t.today_day ? 'bg-accent border-t border-yellow-200' :
                            (i + 1 > (t.today_day ?? 31) ? 'border-t border-l border-r border-dashed border-border-bright' :
                            'bg-accent/40 border-t border-accent/70 group-hover:bg-accent')"></div>
```

Replace the entire `[class]` binding (keep `class="mt-auto"` and the `[style.height.%]` line unchanged) so a **selected** branch is checked first:

```html
              <div class="mt-auto"
                   [style.height.%]="(cost / tapeMax_()) * 100 || (i + 1 > (t.today_day ?? 31) ? 100 : 8)"
                   [class]="i === selectedDayIndex() ? 'bg-accent ring-2 ring-yellow-200' :
                            i + 1 === t.today_day ? 'bg-accent border-t border-yellow-200' :
                            (i + 1 > (t.today_day ?? 31) ? 'border-t border-l border-r border-dashed border-border-bright' :
                            'bg-accent/40 border-t border-accent/70 group-hover:bg-accent')"></div>
```

(The selected day gets a full `ring-2 ring-yellow-200` outline — distinct from `today`'s top-border-only treatment and from the hover state. `yellow-200` is already used by the `today` branch, so it is a valid color in this Tailwind config. The selected branch is checked first so a selected *today* shows the selection ring.)

- [ ] **Step 6: Emphasize the selected day's number label**

In the same file, the day-number row (around lines 141-152) currently is:

```html
          @for (cost of t.current; track $index; let i = $index) {
            <div class="flex-1 text-center text-[9px] font-mono"
                 [class]="i + 1 === t.today_day ? 'text-accent' : 'text-muted'">
              @if (i + 1 === t.today_day) {
                {{ i + 1 }}↑
              } @else if (i === 0 || (i + 1) % 5 === 0 || i + 1 === t.days_in_month) {
                {{ i + 1 }}
              } @else {
                ·
              }
            </div>
          }
```

Replace that whole `@for` block with:

```html
          @for (cost of t.current; track $index; let i = $index) {
            <div class="flex-1 text-center text-[9px] font-mono"
                 [class]="i + 1 === t.today_day || i === selectedDayIndex() ? 'text-accent' : 'text-muted'">
              @if (i + 1 === t.today_day) {
                {{ i + 1 }}↑
              } @else if (i === selectedDayIndex()) {
                {{ i + 1 }}
              } @else if (i === 0 || (i + 1) % 5 === 0 || i + 1 === t.days_in_month) {
                {{ i + 1 }}
              } @else {
                ·
              }
            </div>
          }
```

(The selected day's number is now always shown — not collapsed to a `·` dot — and rendered in `text-accent`, consistent with how `today_day` is emphasized. When the selected day *is* today, the first branch still wins and shows `{{ i + 1 }}↑`.)

- [ ] **Step 7: Build to verify it compiles**

Run: `cd web; npm run build`
Expected: `Application bundle generation complete.` with no errors. The build prints one pre-existing unrelated warning — `NG8113: DatePipe is not used within the template of FilesComponent`. That warning is expected and is **not** caused by this change. There must be no *new* warnings mentioning `OverviewComponent`, `overview.component`, or `tape-selection`.

- [ ] **Step 8: Manual check**

If `andon.exe` is running, ask the user to close it first (the running binary can lock the build output). Then:

Run: `cargo tauri dev` (from the repo root)

In the app, on the Overview page:
1. Click a past day-bar on the tape → the tape highlights that bar with a ring, its number turns accent-colored, the filter bar shows "Custom", and every filtered tile (Cost, Sessions, Tokens, Cost by model, Invocations by model, Accept rate, Top repos, Active time, Recent sessions) updates to that single day.
2. Click the same day again → returns to "This month" (the highlight clears, tiles show month-to-date).
3. Click "today" on the tape → selects today; the bar shows the selection ring.
4. Hover a future day-bar → no pointer cursor; clicking it does nothing.

Report the result. If anything looks off, stop and flag it before committing.

- [ ] **Step 9: Commit**

```bash
git add web/src/app/features/overview/overview.component.ts web/src/app/features/overview/overview.component.html
git commit -m "$(cat <<'EOF'
feat(overview): make the tape clickable to filter by day

Clicking a current-month tape day-bar narrows the whole Overview to
that single day; clicking the selected day again returns to the month.
The selected bar is highlighted with a ring. Future days are not
selectable.

Closes the tape day-select design spec.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final Verification

After all three tasks:

- [ ] **Run the full frontend test suite**

Run: `cd web; npm test`
Expected: all suites pass, including the new `tape-selection.spec.ts` and the additions to `filter.service.spec.ts`.

- [ ] **Confirm the build is clean**

Run: `cd web; npm run build`
Expected: `Application bundle generation complete.`, only the pre-existing unrelated `NG8113` `DatePipe`/`FilesComponent` warning.

Then complete the branch with the `superpowers:finishing-a-development-branch` skill.

---

## Self-Review Notes

- **Spec coverage:** Spec §1 → Task 1 (`selectDay`, `rangeLabel`). Spec §2 → Task 2 (`tape-selection.ts`). Spec §3 → Task 3 Steps 1-3 (`selectedDayIndex`, `onTapeDayClick`). Spec §4 → Task 3 Steps 4-6 (click handler, `cursor-pointer` conditional, selected highlight). Spec "Testing" → Task 1 Step 1, Task 2 Step 1, Task 3 Step 8. Spec edge cases (future days, clicking today, multi-day custom, no tape data) → covered by `onTapeDayClick`'s future guard, the selected-first branch order, `selectedTapeDay`'s single-day check, and `tape()` being `null` rendering no bars.
- **Type consistency:** `selectedTapeDay(range, filterWindow, tapeMonth)` / `tapeDayDate(tapeMonth, index)` signatures are identical in Task 2's implementation, its tests, and Task 3's call sites. `selectDay(day: Date)` is consistent between Task 1 and the `tapeDayDate(...)` → `selectDay(...)` call in `onTapeDayClick`.
- **No placeholders:** every step shows complete code or an exact command with expected output.
