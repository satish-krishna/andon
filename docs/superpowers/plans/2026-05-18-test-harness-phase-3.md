# Test Harness Phase 3 — Full Web Coverage + Optional E2E

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cover the Angular web layer: `ApiService` URL building, every feature component, shared components smoke tests, and an optional Playwright happy-path E2E.

**Architecture:** Build on the Vitest + Spectator + ng-mocks harness from Phase 1. Service tests use Spectator's `HttpClientSpectator` wrapping `HttpTestingController`. Component tests use shallow renders with `MockComponent(ChartComponent)` / `MockProvider(ApiService)`. Tests are signal-driven — assert via `read(signal)` from `signal-helpers.ts`.

**Tech Stack:** Phase 1 harness + Chart.js canvas shim (`jest-canvas-mock` equivalent for Vitest) if any component test renders a chart. Optional: Playwright for E2E.

**Branch:** `tests/phase-3-web-coverage` off `main`.

**Prereq:** Phase 1 merged; harness folder `web/src/testing/` exists; Vitest runs.

---

## File Structure

**New tests (each one a `.spec.ts` next to the file under test):**
- `web/src/app/core/api.service.spec.ts`
- `web/src/app/features/overview/overview.component.spec.ts`
- `web/src/app/features/sessions/sessions.component.spec.ts`
- `web/src/app/features/sessions/session-detail.component.spec.ts` (adapt to actual file name)
- `web/src/app/features/files/files.component.spec.ts`
- `web/src/app/features/settings/settings.component.spec.ts`
- `web/src/app/features/diagnostics/diagnostics.component.spec.ts`
- `web/src/app/shared/panel.component.spec.ts`
- `web/src/app/shared/empty.component.spec.ts`
- `web/src/app/app.component.spec.ts` (smoke render)

**Modified:**
- `web/src/testing/api-fixtures.ts` — add a typed sample for every endpoint as tests grow.
- `web/vitest.config.ts` — add Chart.js canvas mock setup if needed.
- `web/package.json` — add `vitest-canvas-mock` (or equivalent) if charts barf.

**Optional E2E:**
- `e2e/playwright.config.ts`
- `e2e/happy-path.spec.ts`
- `.github/workflows/ci.yml` — add a Playwright job if E2E is in.

---

## Open Questions to Resolve During Execution

- **Chart.js + jsdom:** ng2-charts renders into a canvas. jsdom doesn't have one. Confirm during Task 3 (Overview). Fix options: install `vitest-canvas-mock` (or import `'jest-canvas-mock'` from setup), or shallow-mock the chart component via `MockComponent(BaseChartDirective)`. Prefer the latter — we're not testing Chart.js, only that we hand it the right data.
- **HttpClientSpectator availability:** depends on Spectator version's compatibility with Vitest. If unavailable, fall back to `provideHttpClientTesting()` + `HttpTestingController` directly. Same assertions, more boilerplate.
- **E2E:** spec marks it optional. Skip unless the user opts in — adds ~200MB Playwright dep.

---

## Task 1: Branch and Chart.js canvas decision

**Files:**
- Modify: `web/vitest.config.ts` and/or `web/setup-vitest.ts` if mocking globally.

- [ ] **Step 1: Branch**

```bash
git checkout main && git pull
git checkout -b tests/phase-3-web-coverage
```

- [ ] **Step 2: Spike a chart-component test to surface the canvas problem now, not later**

Throwaway `web/src/_chart_probe.spec.ts`:

```ts
import { Component } from '@angular/core';
import { BaseChartDirective } from 'ng2-charts';
import { TestBed } from '@angular/core/testing';

@Component({
  standalone: true,
  imports: [BaseChartDirective],
  template: `<canvas baseChart [data]="data" [type]="'bar'"></canvas>`,
})
class ProbeCmp {
  data = { labels: ['a'], datasets: [{ data: [1] }] };
}

it('chart renders or fails loudly', () => {
  const f = TestBed.createComponent(ProbeCmp);
  f.detectChanges();
  expect(f.nativeElement.querySelector('canvas')).toBeTruthy();
});
```

```bash
cd web && npm test -- _chart_probe
```

- [ ] **Step 3: Based on result, choose strategy:**
  - **Works as-is:** delete the probe, move on.
  - **Canvas error:** install `vitest-canvas-mock`, add `import 'vitest-canvas-mock'` to `setup-vitest.ts`. Re-run.
  - **Still flaky:** standardize on shallow-mocking `BaseChartDirective` via `MockDirective(BaseChartDirective)` in every component test. Document this in `web/src/testing/spectator-factories.ts` so all future tests follow suit.

- [ ] **Step 4: Delete probe; commit setup change if any**

```bash
cd web && git rm src/_chart_probe.spec.ts
git add web/
git commit -m "test(web): resolve Chart.js + jsdom canvas strategy"
```

---

## Task 2: `ApiService` — URL building per endpoint

**Files:** Create `web/src/app/core/api.service.spec.ts`

- [ ] **Step 1: Read `web/src/app/core/api.service.ts`** and enumerate every public method. Group by endpoint family.

- [ ] **Step 2: Test template (per method)**

```ts
import { TestBed } from '@angular/core/testing';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideHttpClient } from '@angular/common/http';
import { ApiService } from './api.service';

describe('ApiService', () => {
  let api: ApiService;
  let http: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [ApiService, provideHttpClient(), provideHttpClientTesting()],
    });
    api = TestBed.inject(ApiService);
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => http.verify());

  it('overviewToday hits /api/overview/today', () => {
    api.overviewToday().subscribe();
    http.expectOne('/api/overview/today').flush({});
  });

  it('costByDay forwards models, from, to', () => {
    api.costByDay({ models: 'opus,sonnet', from: 100, to: 200 }).subscribe();
    const req = http.expectOne(
      (r) => r.url === '/api/overview/cost-by-day'
        && r.params.get('models') === 'opus,sonnet'
        && r.params.get('from') === '100'
        && r.params.get('to') === '200'
    );
    req.flush({});
  });

  // ...one assertion per public method, including:
  // sessions list with search/repos/limit, sessionDetail by id,
  // filesHeatmap?days=N, v2/* family, settings GET/PUT, diagnostics, etc.
});
```

- [ ] **Step 3: Cover EVERY public method.** Method count drives the spec length — don't shortcut. Each method needs at least one test asserting URL + non-default params.

- [ ] **Step 4: Run + commit**

```bash
cd web && npm test -- api.service
git add . && git commit -m "test(web): cover ApiService URL building for every endpoint"
```

---

## Task 3: Overview component

**Files:** Create `web/src/app/features/overview/overview.component.spec.ts`

- [ ] **Step 1: Inspect `overview.component.ts`** to learn which child components exist (charts, panels) and what `ApiService` methods it calls.

- [ ] **Step 2: Build a Spectator factory that mocks children and the API**

```ts
import { createComponentFactory, MockComponent, MockProvider } from '../../../testing/spectator-factories';
import { ApiService } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { OverviewComponent } from './overview.component';
import { BaseChartDirective } from 'ng2-charts';
import { of } from 'rxjs';
import { sampleOverviewToday } from '../../../testing/api-fixtures';

describe('OverviewComponent', () => {
  const apiMock = {
    overviewToday: () => of(sampleOverviewToday),
    costByDay: () => of({ days: [], models: [] }),
    tokensByDay: () => of({ days: [], series: {} }),
    acceptByLanguage: () => of([]),
    activeTimeToday: () => of({ user: 0, cli: 0 }),
  };

  const create = createComponentFactory({
    component: OverviewComponent,
    providers: [
      FilterService,
      { provide: ApiService, useValue: apiMock },
    ],
    declarations: [MockComponent /*MockDirective*/ (BaseChartDirective as any)],
    detectChanges: false,
  });

  it('renders all five visualizations when API returns data', () => {
    const spec = create();
    spec.detectChanges();
    // Adapt selectors to actual markup — e.g. data-test attrs you may need to add.
    expect(spec.queryAll('[data-viz]').length).toBe(5);
  });

  it('shows empty state when API returns no data', () => {
    // override providers to return empty arrays/zeros
    const empty = { ...apiMock, overviewToday: () => of({ cost_usd: 0, sessions: 0, accept_rate: 0 }) };
    const spec = create({ providers: [{ provide: ApiService, useValue: empty }] });
    spec.detectChanges();
    expect(spec.query('app-empty')).toBeTruthy();
  });

  it('re-fetches when FilterService changes', () => {
    const overviewSpy = vi.fn().mockReturnValue(of(sampleOverviewToday));
    const spec = create({ providers: [{ provide: ApiService, useValue: { ...apiMock, overviewToday: overviewSpy } }] });
    spec.detectChanges();
    const calls0 = overviewSpy.mock.calls.length;
    spec.inject(FilterService).setRange('today');
    spec.detectChanges();
    expect(overviewSpy.mock.calls.length).toBeGreaterThan(calls0);
  });
});
```

If the component needs `data-viz` attributes to make assertions tractable, add them to the template — that's a reasonable production change to support testability.

- [ ] **Step 3: Run + commit**

```bash
cd web && npm test -- overview.component
git add . && git commit -m "test(web): cover OverviewComponent rendering + filter re-fetch"
```

---

## Task 4: Sessions list + detail

**Files:**
- Create: `web/src/app/features/sessions/sessions.component.spec.ts`
- Create: `web/src/app/features/sessions/<detail-file>.spec.ts` (match the actual filename)

- [ ] **List:** assert list renders rows from mocked API, pagination control updates query params on API mock, search input updates `FilterService.search`.
- [ ] **Detail:** assert API call uses route param `id`, timeline renders tool decisions in chronological order, files list renders.
- [ ] Commit `test(web): cover sessions list and detail`.

---

## Task 5: Files heatmap

**Files:** Create `web/src/app/features/files/files.component.spec.ts`

- [ ] Mock `ApiService.filesHeatmap()` returning known rows. Assert each row renders, sized attribute matches edit count, color/class matches accept-rate bucket.
- [ ] Commit `test(web): cover Files heatmap rendering`.

---

## Task 6: Settings

**Files:** Create `web/src/app/features/settings/settings.component.spec.ts`

- [ ] Mock `ApiService` for settings + stats. Assert DB location string shows, table row-count list renders, "Open data folder" button calls the right API method (mock and assert).
- [ ] Commit `test(web): cover Settings rendering and open-folder action`.

---

## Task 7: Diagnostics

**Files:** Create `web/src/app/features/diagnostics/diagnostics.component.spec.ts`

- [ ] Mirror Settings pattern: mock the diagnostics endpoints, assert rendered output.
- [ ] Commit `test(web): cover Diagnostics rendering`.

---

## Task 8: Shared components smoke

**Files:**
- Create: `web/src/app/shared/panel.component.spec.ts`
- Create: `web/src/app/shared/empty.component.spec.ts`
- Create: `web/src/app/app.component.spec.ts`

- [ ] **Panel:** renders projected content, applies title input.
- [ ] **Empty:** renders the message input and an icon.
- [ ] **App root:** mounts router-outlet, routes registered, FilterBar renders.
- [ ] Commit `test(web): smoke shared components and app root`.

---

## Task 9: (Optional) Playwright E2E happy path

**Files (only if user opts in):**
- Create: `e2e/package.json`, `e2e/playwright.config.ts`, `e2e/happy-path.spec.ts`
- Modify: `.github/workflows/ci.yml` to add a Playwright job

- [ ] **Step 1: Confirm with the user before adding Playwright** (~200MB dep). If skipped, mark this task N/A and proceed to Task 10.

- [ ] **Step 2: Boot the Angular dev server against the real Tauri binary backed by a pre-seeded SQLite.** Easiest path: a `BeforeAll` that copies a fixture `.db` to `~/.andon/data.db`, starts `npm start` and `cargo tauri dev` (or just `cargo run --bin andon`), waits for `:8765/api/health`.

- [ ] **Step 3: Test script**

```ts
test('happy path: overview → sessions → detail → custom range', async ({ page }) => {
  await page.goto('http://localhost:4200/');
  await expect(page.getByTestId('viz-cost-today')).toBeVisible();
  await page.getByRole('link', { name: /sessions/i }).click();
  await expect(page).toHaveURL(/\/sessions/);
  await page.locator('table tbody tr').first().click();
  await expect(page).toHaveURL(/\/sessions\//);

  await page.goto('http://localhost:4200/');
  await page.getByRole('button', { name: /custom/i }).click();
  // ...assert overview re-fetches (network spy or visible re-render).
});
```

- [ ] **Step 4: Commit + add to CI**

---

## Task 10: PR + CI

- [ ] **Step 1: Push + open PR**

```bash
git push -u origin tests/phase-3-web-coverage
gh pr create --title "Phase 3: full web test coverage" --body "Covers ApiService URL building for every endpoint; smoke + behavior tests for every feature component and shared component."
```

- [ ] **Step 2: Watch CI; fix until green.**

- [ ] **Step 3: Mark ready, request review.**

---

## Done When

- Every public `ApiService` method has at least one URL/param test.
- Every component under `features/` has at least one smoke render + at least one behavior test.
- `npm test` runs the full web suite green in CI.
- (Optional) Playwright happy-path test passes in CI.
