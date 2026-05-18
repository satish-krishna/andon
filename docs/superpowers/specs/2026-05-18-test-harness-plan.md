# Test Harness Plan — Web + Tauri

**Date:** 2026-05-18
**Status:** Planned. To be executed in a dedicated session after `fix/filters` lands.
**Estimated effort:** ~7–9 focused days, sequenceable into phased PRs.

This document enumerates every module that needs coverage, the fixture strategy, and an estimated effort breakdown. A future session executes against this plan.

---

## Goals

- Catch regressions in filter logic, ingestor metric handlers, and API route SQL.
- Provide a fixture toolkit so future tests are cheap to write.
- Wire CI so every PR runs `cargo test` and `ng test --watch=false`.

## Non-goals

- 100% line coverage. Prioritize logic that has been historically buggy or that a misuse of would silently corrupt data.
- UI snapshot tests for every component.
- Visual regression tooling.

---

## Rust backend (`src-tauri/`)

### Harness setup (~0.5 day)

- `tests/common/mod.rs` with:
  - `fixture_pool()`: in-memory SQLite pool with migrations applied.
  - `seed_session(pool, opts)`: insert a session + sample token/cost/decision/file rows.
  - `sample_export_metrics(...)`: build an `ExportMetricsServiceRequest` with arbitrary resource attrs and metric data points.
  - `sample_export_logs(...)`: same for logs.
- Verify `rusqlite` `bundled` feature + WAL mode works in-memory (`:memory:` doesn't support WAL — use a temp file via `tempfile` crate).

### Module-by-module

| Module | What to cover | Effort |
|---|---|---|
| `api/routes.rs` — `FilterQuery` | `model_clause` substring SQL + params; `window()` defaults; `model_list` parsing edge cases (empty, whitespace, trailing comma). | 0.25d |
| `api/routes.rs` — endpoints | One integration test per endpoint (24+) against a seeded in-memory DB. Assert JSON shape + aggregate math. Group by feature: overview (5), sessions (3), files (2), reports/tape (6), settings (2), git (3), diagnostics (3). | 1.5–2d |
| `otlp/ingestor.rs` | One test per known metric (`session.count`, `lines_of_code.count`, `pull_request.count`, `commit.count`, `cost.usage`, `token.usage`, `code_edit_tool.decision`, `active_time.total`) asserting the correct typed row is written. Plus: unknown metric → `metrics_raw`. Plus: missing `session.id` is stored but flagged in logs (don't panic). | 1d |
| `otlp/grpc_server.rs` + `otlp/http_server.rs` | Smoke test: feed a real `ExportMetricsServiceRequest` through both transports, assert the same rows land. Verify `127.0.0.1` bind. Verify both return `Ok` even when ingestor errors internally. | 0.5d |
| `reports/model.rs` + other report builders | Unit tests on aggregation math with fixed seed data. | 0.5d |
| `db/migrations.rs` | Applying migrations twice is idempotent. Schema matches the documented shape (introspect `sqlite_master`). | 0.25d |
| `repo_inference.rs` + `git_query.rs` | Build a tempdir git repo with known history; assert inference picks the right repo + recent commits. | 0.5d |
| `settings.rs` + `autostart.rs` + `config.rs` | Read/write round-trip; missing-file defaults; corrupt-file fallback. Autostart: behavior gated by OS, so test the pure-Rust parts (path computation, registry key string, plist content). | 0.5d |
| `integration.rs` + `diagnostics.rs` | One smoke each; mainly assert no panic on empty DB. | 0.25d |

**Subtotal: ~4–5 days**

### CI for Rust

- GitHub Actions matrix: ubuntu-latest, macos-latest, windows-latest.
- `cargo test --workspace --all-features`.
- Cache `~/.cargo` and `target/`.

---

## Web frontend (`web/`)

**Stack:** Vitest + ng-mocks + Spectator. Replaces the default Angular 21 Jasmine/Karma scaffold.

### Harness setup (~1 day, +0.5 over Jasmine/Karma)

The extra half-day covers swapping the runner and proving the stack works against Angular 21's standalone-only components and signals.

**Dependencies to add:**

- `vitest`, `@analogjs/vitest-angular` (Vitest preset that handles Angular's TestBed bootstrap).
- `@ngneat/spectator` (component / service test wrappers; works with Vitest via its `jest`-compatible matchers — verify version ≥ 19 for Angular 21 standalone support).
- `ng-mocks` (mock components, directives, pipes, providers — pairs well with Spectator for shallow renders).
- `jsdom` (Vitest's browser env; no headless Chrome needed in CI).
- `@vitest/coverage-v8` (coverage reporting).

**Project changes:**

- Remove `jasmine-core`, `@types/jasmine`, `karma*` from `package.json`.
- Delete `karma.conf.js` and `web/src/test.ts` if present.
- Add `vitest.config.ts` with the Analog preset.
- Add `setup-vitest.ts` registering `zone.js/testing` and Spectator's globals.
- Update `tsconfig.spec.json` to point at Vitest types instead of Jasmine.
- `npm run test` → `vitest run`; `npm run test:watch` → `vitest`.

**`web/src/testing/` folder:**

- `signal-helpers.ts`: utilities for asserting signal/computed values across tick.
- `api-fixtures.ts`: typed sample responses for every endpoint.
- `filter-builder.ts`: build a `FilterService` in known states.
- `spectator-factories.ts`: re-export `createComponentFactory` / `createServiceFactory` pre-wired with `MockProvider`/`MockComponent` helpers from ng-mocks.

**Patterns:**

- Service tests: `createServiceFactory({ service: FilterService })` + Vitest `expect()`.
- HTTP tests: Spectator's `HttpClientSpectator` wrapping `HttpTestingController` — same assertions, less boilerplate.
- Component tests: `createComponentFactory({ component: OverviewComponent, declarations: [MockComponent(ChartComponent)], providers: [MockProvider(ApiService)] })` for shallow renders with ng-mocks stubs.

**Risks to confirm during setup:**

- Spectator + Vitest + Angular 21: officially supported combo is recent; verify on a single throwaway test before scaling out. If incompatible, fall back to Spectator + Jest (still keeps the ng-mocks/Spectator ergonomics).
- ng2-charts + jsdom: Chart.js needs a canvas shim (`canvas` npm pkg or `jest-canvas-mock` equivalent). Confirm during overview component tests.

### Module-by-module

| Module | What to cover | Effort |
|---|---|---|
| `core/filter.service.ts` | Every preset's `window()` math (today/week/month/30d/custom); `modelsCsv` with all/partial/none selected; `enterCustomMode` seeding; `setCustomFrom/To` clamping; `hasActiveFilters` truth table; `clearFilters` resets. | 0.5d |
| `shared/filter-bar.component.ts` | Chip click toggles model; range click switches preset; "Custom…" reveals date inputs; date input change updates `customRange`; Clear button visibility tracks `hasActiveFilters`. | 0.25d |
| `core/api.service.ts` | Each method builds the correct URL incl. `models=`, `from=`, `to=`, `repos=`, `search=` query params. Use Spectator's `HttpClientSpectator`. | 0.5d |
| `features/overview/overview.component.ts` | Renders all five visualizations when API returns data; empty states when API returns empty; re-fetches when `FilterService` state changes (effect-driven). | 0.5d |
| `features/sessions/sessions.component.ts` + detail | List renders, pagination works, detail page fetches by id, timeline renders tool decisions in order. | 0.5d |
| `features/files/files.component.ts` | Heatmap renders with sample data; sized by edit count; colored by accept rate. | 0.25d |
| `features/settings/settings.component.ts` | DB location displays; row counts render; "Open data folder" calls Tauri command (mock). | 0.25d |
| `shared/*` other components | Smoke render each. | 0.25d |

**Subtotal: ~3–3.5 days** (includes the +0.5d Vitest/Spectator setup tax)

### CI for web

- Same GitHub Actions workflow as Rust.
- `npm ci && npm run build && npm run test`.
- Vitest runs headless in `jsdom` — no Chrome dependency in CI.
- Cache `node_modules` keyed on `package-lock.json`.

---

## E2E smoke (optional, ~0.5 day)

Playwright, one happy-path test:

1. Boot the Angular dev server with a mocked API (or a pre-seeded SQLite + the real Tauri binary in `cargo tauri dev`).
2. Visit `/`, assert overview cards render.
3. Click "Sessions", assert list.
4. Click a row, assert detail.
5. Apply a custom date range, assert overview re-fetches.

Adds ~200MB Playwright dep. Skip if that's unacceptable.

---

## Phasing recommendation

If the future session wants to ship incrementally instead of one huge PR:

1. **Phase 1 (~2 days):** Harness setup (Rust + web) + filter-area tests (`FilterService`, `FilterQuery`, `filter-bar`). Wires CI.
2. **Phase 2 (~3 days):** Full Rust coverage (ingestor + all API routes + reports + repo/git + settings).
3. **Phase 3 (~3 days):** Full web coverage (services + every feature component) + optional E2E.

Each phase is a reviewable PR on its own branch.

---

## Open questions for the future session

- Use `mockall` for Rust trait mocking or hand-roll fakes? (Recommendation: hand-roll — only the ingestor would benefit, and it's small.)
- Snapshot testing for JSON DTOs (e.g. `insta`)? Useful for endpoint shape stability. (Recommendation: yes — `insta` is low-friction.)
- Coverage thresholds in CI (`cargo-llvm-cov` for Rust, `@vitest/coverage-v8` for web)? (Recommendation: report but don't fail the build initially; set a floor later.)
- Spectator + Vitest + Angular 21 compatibility — confirm during harness setup; documented fallback is Spectator + Jest.
