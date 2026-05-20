# Filter-bar Refresh button — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Refresh button to the shared filter bar that re-fetches the current page (Overview / Sessions / Files) using the filters already selected, without resetting them.

**Architecture:** `FilterService` gains a `refreshTick` counter signal and a `refresh()` method. The Refresh button calls `refresh()`. Each filter-bearing page's fetch `effect()` adds a tracked read of `refreshTick()`, so bumping it re-runs the effect; the effect re-reads the filter signals unchanged, so filters are retained by construction.

**Tech Stack:** Angular 21 (standalone components, signals), Tailwind 4, Vitest.

**Spec:** [`docs/superpowers/specs/2026-05-20-filter-bar-refresh-design.md`](../specs/2026-05-20-filter-bar-refresh-design.md)

**Branch:** `feature/jsonl-ingest` (current branch).

---

## File structure

### Modify
- `web/src/app/core/filter.service.ts` — `refreshTick` signal + `refresh()` method.
- `web/src/app/core/filter.service.spec.ts` — unit test for `refresh()`.
- `web/src/app/shared/filter-bar.component.ts` — Refresh button in the inline template.
- `web/src/app/shared/filter-bar.component.spec.ts` — `RefreshCw` icon in the test pick + a Refresh-button test.
- `web/src/app/features/overview/overview.component.ts` — `refreshTick()` read in the fetch effect.
- `web/src/app/features/sessions/sessions.component.ts` — `refreshTick()` read in the fetch effect.
- `web/src/app/features/files/files.component.ts` — `refreshTick()` read in the fetch effect.

### Not touched
- API, DTOs, routes, the Rust backend, other pages.

---

## Task 1: `FilterService.refresh()` + `refreshTick`

**Files:**
- Modify: `web/src/app/core/filter.service.ts`.
- Test: `web/src/app/core/filter.service.spec.ts`.

- [ ] **Step 1: Write the failing test**

In `web/src/app/core/filter.service.spec.ts`, append this test inside the `describe('FilterService', () => { ... })` block (before its closing `});`):

```ts
  it('refresh() increments refreshTick and leaves filters untouched', () => {
    const s = createService();
    const before = s.refreshTick();
    s.refresh();
    expect(s.refreshTick()).toBe(before + 1);
    s.refresh();
    expect(s.refreshTick()).toBe(before + 2);
    // refresh() must never behave like clearFilters() — filters stay as they were.
    expect(s.range()).toBe('month');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.search()).toBe('');
    expect(s.repos()).toEqual([]);
    expect(s.hasActiveFilters()).toBe(false);
  });
```

- [ ] **Step 2: Run to verify it fails**

```powershell
cd web; npm test
```

Expected: the new test FAILS to compile / run — `refreshTick` and `refresh` do not exist on `FilterService`.

- [ ] **Step 3: Implement `refreshTick` + `refresh()`**

In `web/src/app/core/filter.service.ts`, add the `refreshTick` signal immediately after the `repos` signal. The signals block currently is:

```ts
  readonly range = signal<RangePreset>(DEFAULT_RANGE);
  readonly customRange = signal<CustomRange | null>(null);
  readonly models = signal<Set<string>>(new Set(ALL_MODELS));
  readonly search = signal<string>('');
  readonly repos = signal<string[]>([]);
```

Change it to:

```ts
  readonly range = signal<RangePreset>(DEFAULT_RANGE);
  readonly customRange = signal<CustomRange | null>(null);
  readonly models = signal<Set<string>>(new Set(ALL_MODELS));
  readonly search = signal<string>('');
  readonly repos = signal<string[]>([]);

  /** Bumped by refresh() so filter-driven page effects re-fetch on demand. */
  readonly refreshTick = signal(0);
```

Then add the `refresh()` method immediately after the existing `clearFilters()` method. `clearFilters()` currently is:

```ts
  clearFilters() {
    this.range.set(DEFAULT_RANGE);
    this.customRange.set(null);
    this.models.set(new Set(ALL_MODELS));
    this.search.set('');
    this.repos.set([]);
  }
```

Add directly after it:

```ts
  /** Force a re-fetch on pages whose data effect reads refreshTick(). */
  refresh() {
    this.refreshTick.update((n) => n + 1);
  }
```

- [ ] **Step 4: Run to verify it passes**

```powershell
cd web; npm test
```

Expected: all tests pass, including the new `refresh() increments refreshTick and leaves filters untouched`.

- [ ] **Step 5: Commit**

```powershell
cd D:/Repos/andon
git add web/src/app/core/filter.service.ts web/src/app/core/filter.service.spec.ts
git commit -m "feat(web): FilterService.refresh() and refreshTick signal"
```

---

## Task 2: Refresh button in `FilterBarComponent`

**Files:**
- Modify: `web/src/app/shared/filter-bar.component.ts` (inline template).
- Test: `web/src/app/shared/filter-bar.component.spec.ts`.

- [ ] **Step 1: Update the test setup and add the failing test**

In `web/src/app/shared/filter-bar.component.spec.ts`:

a) Change the lucide import line. It currently is:

```ts
import { Calendar, Layers, X, LucideAngularModule } from 'lucide-angular';
```

Change to (add `RefreshCw`):

```ts
import { Calendar, Layers, RefreshCw, X, LucideAngularModule } from 'lucide-angular';
```

b) In the `setup()` function, the `importProvidersFrom` line currently is:

```ts
      importProvidersFrom(LucideAngularModule.pick({ Calendar, Layers, X })),
```

Change to (add `RefreshCw` — the new button renders a `refresh-cw` icon, and `LucideAngularComponent` throws on `ngOnChanges` for any icon not picked):

```ts
      importProvidersFrom(LucideAngularModule.pick({ Calendar, Layers, RefreshCw, X })),
```

c) Add a `refreshButton` query helper next to the existing `clearButton` helper:

```ts
function refreshButton(fixture: ComponentFixture<FilterBarComponent>): HTMLElement | null {
  return fixture.nativeElement.querySelector('[data-testid="refresh-data"]');
}
```

d) Append these two tests inside the `describe('FilterBarComponent', () => { ... })` block (before its closing `});`):

```ts
  it('Refresh button is always visible, including with no active filters', () => {
    const { fixture } = setup();
    // Default state has no active filters (Clear is hidden) — Refresh still shows.
    expect(clearButton(fixture)).toBeFalsy();
    expect(refreshButton(fixture)).toBeTruthy();
  });

  it('clicking Refresh bumps refreshTick without activating any filter', () => {
    const { fixture, filter } = setup();
    const btn = refreshButton(fixture);
    expect(btn).toBeTruthy();
    const before = filter.refreshTick();
    btn!.click();
    fixture.detectChanges();
    expect(filter.refreshTick()).toBe(before + 1);
    expect(filter.hasActiveFilters()).toBe(false);
  });
```

- [ ] **Step 2: Run to verify it fails**

```powershell
cd web; npm test
```

Expected: the two new `FilterBarComponent` tests FAIL — `refreshButton(fixture)` returns `null` because the button does not exist yet.

- [ ] **Step 3: Add the Refresh button to the template**

In `web/src/app/shared/filter-bar.component.ts`, the inline template's second row currently ends with this conditional Clear button:

```html
        @if (filter.hasActiveFilters()) {
          <button class="ml-auto text-muted hover:text-text font-mono text-[11px] flex items-center gap-1"
                  data-testid="clear-filters"
                  aria-label="Clear filters"
                  (click)="filter.clearFilters()">
            <lucide-icon name="x" class="w-3 h-3"></lucide-icon>Clear
          </button>
        }
```

Replace that entire `@if` block with a right-aligned group holding an always-visible Refresh button and the existing conditional Clear button (note: `ml-auto` moves from the Clear button to the wrapping `<div>`):

```html
        <div class="ml-auto flex items-center gap-3">
          <button class="text-muted hover:text-text font-mono text-[11px] flex items-center gap-1"
                  data-testid="refresh-data"
                  aria-label="Refresh data"
                  (click)="filter.refresh()">
            <lucide-icon name="refresh-cw" class="w-3 h-3"></lucide-icon>Refresh
          </button>
          @if (filter.hasActiveFilters()) {
            <button class="text-muted hover:text-text font-mono text-[11px] flex items-center gap-1"
                    data-testid="clear-filters"
                    aria-label="Clear filters"
                    (click)="filter.clearFilters()">
              <lucide-icon name="x" class="w-3 h-3"></lucide-icon>Clear
            </button>
          }
        </div>
```

No TypeScript change to the component class — `filter` is already injected and `filter.refresh()` is called directly from the template.

- [ ] **Step 4: Run to verify it passes**

```powershell
cd web; npm test
```

Expected: all tests pass, including the four existing `FilterBarComponent` tests and the two new ones.

- [ ] **Step 5: Commit**

```powershell
cd D:/Repos/andon
git add web/src/app/shared/filter-bar.component.ts web/src/app/shared/filter-bar.component.spec.ts
git commit -m "feat(web): Refresh button in the shared filter bar"
```

---

## Task 3: Wire `refreshTick` into the three page fetch effects

**Files:**
- Modify: `web/src/app/features/overview/overview.component.ts`.
- Modify: `web/src/app/features/sessions/sessions.component.ts`.
- Modify: `web/src/app/features/files/files.component.ts`.

Each component has exactly one `effect(() => {` in its constructor whose first statement is `const w = this.filter.window();`. Adding a tracked read of `refreshTick()` as the effect's first statement makes the effect re-run when the Refresh button is clicked. No new tests — the three pages have no fetch/effect tests today, and effect re-run on a signal change is a standard Angular guarantee (the spec's Testing section documents this decision).

- [ ] **Step 1: Overview**

In `web/src/app/features/overview/overview.component.ts`, find:

```ts
    effect(() => {
      const w = this.filter.window();
```

Change to:

```ts
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      const w = this.filter.window();
```

- [ ] **Step 2: Sessions**

In `web/src/app/features/sessions/sessions.component.ts`, find:

```ts
    effect(() => {
      const w = this.filter.window();
```

Change to:

```ts
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      const w = this.filter.window();
```

- [ ] **Step 3: Files**

In `web/src/app/features/files/files.component.ts`, find:

```ts
    effect(() => {
      const w = this.filter.window();
```

Change to:

```ts
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      const w = this.filter.window();
```

- [ ] **Step 4: Build and test**

```powershell
cd web; npm run build
cd web; npm test
```

Expected: build succeeds (only the pre-existing `NG8113: DatePipe is not used within the template of FilesComponent` warning). All tests pass.

- [ ] **Step 5: Commit**

```powershell
cd D:/Repos/andon
git add web/src/app/features/overview/overview.component.ts web/src/app/features/sessions/sessions.component.ts web/src/app/features/files/files.component.ts
git commit -m "feat(web): Overview/Sessions/Files re-fetch on filter-bar Refresh"
```

---

## Task 4: Verification + push

- [ ] **Step 1: Full web build + tests**

```powershell
cd web; npm run build
cd web; npm test
```

Expected: build succeeds; all tests pass — the original 14 plus the 3 new ones (1 in `filter.service.spec.ts`, 2 in `filter-bar.component.spec.ts`) = 17.

- [ ] **Step 2: Push**

```powershell
cd D:/Repos/andon
git push
```

This pushes the three commits onto `feature/jsonl-ingest`.

---

## Self-review checklist (run before opening/updating the PR)

1. **Spec coverage:**
   - `refreshTick` signal + `refresh()` (spec §Mechanism) → Task 1.
   - Refresh button in the shared filter bar, always visible, alongside Clear (spec §UI) → Task 2.
   - `refreshTick()` read in each of the three page effects (spec §Per-page wiring) → Task 3.
   - `FilterService` unit test for `refresh()` (spec §Testing) → Task 1, Step 1.
2. **No placeholders:** every step has concrete code or a concrete command.
3. **Type consistency:** `refreshTick` is a `signal(0)` (number) everywhere; `refresh()` is the method name in `FilterService`, the template, and the tests. `data-testid="refresh-data"` matches between the template (Task 2 Step 3) and the test helper (Task 2 Step 1c).
4. **Retain-filters guarantee:** `refresh()` writes only `refreshTick` — never a filter signal — so a Refresh can never alter `range` / `customRange` / `models` / `search` / `repos`. Task 1's test asserts this explicitly.
