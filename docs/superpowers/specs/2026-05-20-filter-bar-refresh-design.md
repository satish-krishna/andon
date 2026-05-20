# Filter-bar Refresh button — Design

> Status: draft 2026-05-20 · author: SatishKrishna Pilla

## Motivation

Andon ingests Claude Code telemetry live, but the filter-bearing dashboard pages
(Overview, Sessions, Files) only re-fetch when a *filter* changes — each page's
fetch runs inside a constructor `effect()` that reacts to `FilterService`
signals (`window()`, `modelsCsv()`, `reposCsv()`). If new data arrives while the
user sits on a page with filters unchanged, the page does not update. The user
wants an on-demand **Refresh** button that re-fetches the current page using the
filters already selected — Refresh must not reset the filters.

## Non-goals

- No filter persistence across app restart / window reload. `FilterService` is a
  root singleton, so filters already survive in-app navigation; persisting to
  `localStorage` is out of scope.
- No loading spinner / in-flight state. Refresh is a plain button; "minor
  enhancement."
- No Refresh control on Behaviour or Diagnostics. Behaviour has no filters;
  Diagnostics already auto-polls and has its own "Re-check" button.
- No change to what the pages fetch or how filters are applied.

## Mechanism — a refresh trigger signal

The pages re-fetch only when a signal read inside their fetch `effect()` changes.
To re-fetch on demand without mutating a filter, add a trigger the same effect
also depends on.

`FilterService` gains:

```ts
readonly refreshTick = signal(0);

refresh() {
  this.refreshTick.update((n) => n + 1);
}
```

Each filter-bearing page's fetch `effect()` adds one tracked read of
`this.filter.refreshTick();`. Calling a signal getter inside an effect registers
a dependency whether or not the value is used, so incrementing `refreshTick`
re-runs the effect. The effect re-reads the filter signals unchanged, so the
fetch uses the current filter selections.

**The "retain filters" guarantee is structural:** Refresh and the filters flow
through the *same* effect. `refresh()` writes only `refreshTick` — never a filter
signal — so a Refresh can never alter `range`, `customRange`, `models`,
`search`, or `repos`.

## UI

The Refresh button lives in `FilterBarComponent` (`web/src/app/shared/filter-bar.component.ts`),
so it appears on all three pages that embed `<app-filter-bar>` — Overview,
Sessions, Files.

It sits on the second row (the Model row), right-aligned, alongside the existing
Clear action. The current row-2 right side is a single conditional Clear button;
it becomes a right-aligned group:

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

- **Refresh** — always visible, styled exactly like the existing Clear action
  (muted text-button). Icon: `refresh-cw` (already registered in
  `web/src/app/core/icons.ts`; used by the Diagnostics "Re-check" button).
- **Clear** — behaviour unchanged: still only rendered when `hasActiveFilters()`.
  Its `ml-auto` moves to the wrapping group.

## Per-page wiring

Each of the three components already has a constructor `effect()` that fetches.
Add one line — a tracked `refreshTick` read — inside each:

- `web/src/app/features/overview/overview.component.ts` — the effect that reads
  `filter.window()` / `filter.modelsCsv()`.
- `web/src/app/features/sessions/sessions.component.ts` — the effect that reads
  `filter.window()` / `filter.modelsCsv()` / `filter.reposCsv()`.
- `web/src/app/features/files/files.component.ts` — the effect that reads
  `filter.window()` / `filter.reposCsv()`.

The read is `this.filter.refreshTick();` as a bare statement with an explanatory
comment. It does not change any other behaviour of the effect.

## Files touched

| File | Change |
|---|---|
| `web/src/app/core/filter.service.ts` | Add `refreshTick` signal + `refresh()` method. |
| `web/src/app/shared/filter-bar.component.ts` | Add the Refresh button; wrap row-2 right side in a group. |
| `web/src/app/features/overview/overview.component.ts` | One `refreshTick()` read in the fetch effect. |
| `web/src/app/features/sessions/sessions.component.ts` | One `refreshTick()` read in the fetch effect. |
| `web/src/app/features/files/files.component.ts` | One `refreshTick()` read in the fetch effect. |

**Not touched:** API, DTOs, routes, the Rust backend, other pages.

## Testing

- **`FilterService` unit test** — `refresh()` increments `refreshTick`: read the
  initial value, call `refresh()`, assert it incremented; call again, assert it
  incremented again. Also assert `refresh()` leaves the filter signals
  untouched (`range`, `models`, `search`, `repos` unchanged after a `refresh()`).
- **No page-level tests.** The three pages have no fetch/effect tests today;
  adding HTTP-mocking effect tests for just this one line would be inconsistent
  with the codebase. The effect-rerun behaviour is a standard Angular signals
  guarantee.
- Existing `npm run build` + `npm test` (currently 14/14) must stay green.

## Risks

- **Effect dependency not registered.** If the `refreshTick()` read were placed
  outside the effect's synchronous body (e.g. inside a nested callback), it
  would not be tracked. Mitigation: the read is a top-level statement in the
  effect body, exactly where the existing `filter.window()` reads are.
- **Double fetch.** Changing a filter and clicking Refresh are independent
  triggers; each re-runs the effect once. There is no debounce, consistent with
  the current filter-driven behaviour. Acceptable for a localhost app.
