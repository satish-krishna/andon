# Recipe: add a dashboard page

Adding a page means threading one vertical slice from SQLite to the sidebar. Every step names the reference file to copy. Work test-first and check the Definition of Done in [`CONTRIBUTING.md`](../CONTRIBUTING.md) before you open a PR.

Read [`docs/architecture.md`](architecture.md) first if you are touching ingestion, ports, or the schema.

## 1. Backend: query

Add the read query to `src-tauri/src/db/queries.rs`. Queries take a connection and return plain Rust values; they never know about HTTP. All DB writes go through the `Ingestor` — a page is read-only, so you should not be writing here at all.

Never hold an `rusqlite` connection across `.await`: get it, use it synchronously, drop it.

## 2. Backend: DTO

Add the response shape to `src-tauri/src/api/dto.rs` as a `#[derive(Serialize)]` struct, like `OverviewToday`. Keep it flat and chart-ready — the SPA should not have to reshape the response.

Never hand-write JSON strings. `serde` handles every payload in both directions.

## 3. Backend: handler and route

Add the handler and register it in `src-tauri/src/api/routes.rs`, next to the existing `.route("/api/...", get(...))` lines.

If the page is filterable, take `FilterArgs` and build the `WHERE` clause through `src-tauri/src/api/filter.rs` rather than assembling SQL inline — that is what keeps date, model, and repo filtering consistent across pages. Filterable endpoints live under `/api/v2/`.

Put `tracing::instrument` on the handler and use structured fields.

## 4. Backend: migration, only if the schema changes

Add a numbered migration in `src-tauri/src/db/migrations.rs`.

If the new object must exist for a feature to work at all, read the `ensure_required_objects()` comment in that file before touching it. Only idempotent DDL (`CREATE ... IF NOT EXISTS`) may go in the self-heal block — an `ALTER TABLE ADD COLUMN` there will error on every subsequent start.

Test the migration against an existing `~/.andon/data.db`, not just a fresh one.

## 5. Backend: tests

```powershell
cd src-tauri; cargo test --features test-support
```

Integration tests run against real file-backed SQLite under `tempfile`. Do not mock the database.

## 6. Frontend: types

Add the TypeScript interface to `web/src/app/core/models.ts`, mirroring the DTO field-for-field, snake_case included.

These types are hand-written and can drift from the Rust DTO silently. Until that is generated, treat the DTO and the interface as one change — never edit one without the other in the same diff.

## 7. Frontend: ApiService method

Add a method to `web/src/app/core/api.service.ts` returning `Observable<T>`. Filterable endpoints take `FilterArgs` and go through the existing `toParams()` helper.

`ApiService` is the HTTP boundary. It is the only place in the frontend allowed to hold an Observable.

## 8. Frontend: component

Create `web/src/app/features/<name>/<name>.component.ts` and a sibling `.html`. Copy the shape of `features/behaviour/behaviour.component.ts`:

- `standalone: true`
- `changeDetection: ChangeDetectionStrategy.OnPush`
- `inject()` for dependencies, never constructor injection
- `signal()` for state, `computed()` for derived values
- `@if` / `@for` in the template, never `*ngIf` / `*ngFor`
- Tailwind utilities first; custom CSS only where utilities do not reach

**Convert Observables to signals at the edge.** Use `toSignal()` from `@angular/core/rxjs-interop`, not `.subscribe()` in the constructor:

```typescript
readonly modelMix = toSignal(this.api.modelMix(), { initialValue: null });
```

Existing components subscribe in their constructors. That predates the current rule, is scheduled for change, and is not the pattern to copy.

## 9. Frontend: filter-driven refetch

If the page responds to the filter bar, read `filter.refreshTick()` inside an `effect()` so the Refresh button re-fetches without changing filter state:

```typescript
effect(() => {
  this.filter.refreshTick();
  const w = this.filter.window();
  // fetch with w.fromMs, w.toMs, this.filter.modelsCsv()
});
```

`refresh()` must never behave like `clearFilters()`.

## 10. Frontend: route and navigation

Add a lazy route to `web/src/app/app.routes.ts` using `loadComponent`, then a `nav-link` anchor to `web/src/app/app.component.html` alongside the existing ones.

Master-detail pages use the parent layout plus a `position: absolute; inset: 0` overlay — do not destroy the summary component when navigating to a detail view.

## 11. Charts

Hand-rolled SVG and CSS components in `web/src/app/shared/`, taking signal inputs. Do not add Chart.js for new code.

## 12. Frontend: tests

```powershell
cd web; npm test
```

Vitest, CI mode by default. Use bare `TestBed` plus `TestBed.createComponent` — `@ngneat/spectator`'s `createComponentFactory` is incompatible with the current `@analogjs/vitest-angular` zone setup.

## 13. Docs

Update [`docs/features.md`](features.md) with what the page does. Update [`docs/architecture.md`](architecture.md) only if you changed the schema, a port, or the ingestion path.

## 14. Privacy check before the PR

Every page ships under the same guarantees: loopback-only listeners, no raw prompt content persisted or logged, no outbound network call outside the opt-in forwarder. If your slice touches receivers, the API surface, or `~/.claude/settings.json`, re-read the privacy rules in [`docs/architecture.md`](architecture.md) before merging.
