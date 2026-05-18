# Contributing to Andon

Thanks for contributing. Andon is a single-binary Tauri 2 desktop app: Rust backend, Angular 21 frontend, embedded SQLite, OTLP receivers. This guide covers the development process — patterns, commands, and the bar for "done".

## Before you start

Read in this order:

1. [`README.md`](README.md) — what andon is and how it ships.
2. [`docs/architecture.md`](docs/architecture.md) — process model, ports, schema, ingestion path.
3. [`docs/building.md`](docs/building.md) — prerequisites and build commands.
4. [`docs/features.md`](docs/features.md) — what every dashboard page does.
5. [`CLAUDE.md`](CLAUDE.md) — the short version of the rules below, scoped for AI assistants.

In-flight work lives in [`docs/superpowers/specs/`](docs/superpowers/specs) (design docs) and [`docs/superpowers/plans/`](docs/superpowers/plans) (step-by-step implementation plans).

## Table of contents

- [Prerequisites](#prerequisites)
- [Getting started](#getting-started)
- [Repo layout](#repo-layout)
- [Branching, commits, and PRs](#branching-commits-and-prs)
- [Rust guidelines](#rust-guidelines)
- [Angular guidelines](#angular-guidelines)
- [Testing](#testing)
- [Security and privacy](#security-and-privacy)
- [Release process](#release-process)
- [Definition of Done](#definition-of-done)

## Prerequisites

Full details in [`docs/building.md`](docs/building.md). Summary:

- **Rust** stable, MSVC toolchain: `rustup default stable-x86_64-pc-windows-msvc`
- **Visual Studio Build Tools 2022** with "Desktop development with C++" (required by `rusqlite` bundled SQLite)
- **Node.js 20+**
- **Tauri CLI**: `cargo install tauri-cli --version "^2.0" --locked`

Andon ships Windows-only today. macOS and Linux builds are possible (Tauri supports them) but not in the release pipeline.

## Getting started

```powershell
# One-time
cd web; npm install; cd ..

# Day-to-day: backend + SPA with hot reload
cargo tauri dev
```

The tray icon appears; in dev mode the main window opens automatically. Closing the window hides to tray; "Quit" from the tray menu shuts the runtime down cleanly.

To run the SPA standalone against a running backend (handy for pure frontend work):

```powershell
cd web; npm run build
python -m http.server -d dist/web/browser
# Visit http://localhost:8000 — CORS is Any on the API at :8765
```

## Repo layout

```
andon/
├── src-tauri/                  # Rust backend + Tauri config
│   ├── src/
│   │   ├── otlp/               # gRPC :4317 + HTTP :4318 receivers, ingestor, forwarder
│   │   ├── api/                # axum routes + DTOs (:8765)
│   │   ├── db/                 # rusqlite pool, migrations, queries
│   │   ├── reports/            # standalone HTML report rendering
│   │   ├── lib.rs, main.rs
│   │   └── {integration, autostart, diagnostics, settings,
│   │         git_query, repo_inference, config}.rs
│   ├── templates/              # MiniJinja templates for reports
│   └── tauri.conf.json
├── web/                        # Angular 21 SPA (standalone, signals)
│   └── src/app/
│       ├── core/               # ApiService, models
│       ├── shared/             # cross-feature components
│       └── features/{overview, sessions, files, diagnostics, settings}/
├── docs/                       # Architecture, building, features, pitch
│   └── superpowers/            # Active specs + plans
├── scripts/                    # Smoke scripts, release helpers
└── README.md, CLAUDE.md, CONTRIBUTING.md
```

## Branching, commits, and PRs

- **Branches** are short-lived. Off `main`, named `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, `chore/<slug>`. Finish the slice, open a PR, merge, delete.
- **Rebase on `main`** before opening the PR.
- **Squash-merge** when merging — keep the main history one commit per PR.

### Conventional Commits

`type(scope): subject` — no emojis, US English, imperative mood.

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `style`, `perf`, `build`, `ci`, `chore`.

Scopes that match the repo: `api`, `otlp`, `db`, `ingestor`, `forwarder`, `reports`, `integration`, `web`, `overview`, `sessions`, `files`, `diagnostics`, `settings`, `tray`, `build`, `release`.

Examples (see `git log` for more):

```
feat(api): add cost endpoint
fix(ingestor): handle missing session id
fix(api): return valid Claude Code hook output envelope (#6)
chore: bump version to 0.4.2 (hook response envelope)
```

### Pull requests

- Title in Conventional Commits form. Body describes *why*, links the spec/plan if there is one, and shows screenshots for UI changes.
- Include the smoke-test steps you ran (especially for OTLP / hook changes).
- Link the issue with `Closes #<n>`.

## Rust guidelines

### Engineering discipline

- **SOLID** for any new module, class, or trait. Single responsibility wins ties.
- **TDD**. Failing test first, then implementation, then refactor. No production code without a test driving it.
- **No `unwrap()` / `expect()`** outside `src-tauri/src/main.rs` setup. If a path is genuinely infallible, prove it with the type system.

### Error handling

- Use `anyhow::Result<T>` at the application boundary (handlers, top-level fns).
- Use `thiserror` for domain errors with stable variants worth matching on.
- Receivers (`otlp::grpc_server`, `otlp::http_server`) **always return `Ok`** to the client. Log the failure via `tracing::error!` — never propagate ingestion errors to Claude Code.

### Async / concurrency

- `tokio` for everything async. Do not introduce another runtime.
- The `rusqlite` pool is wrapped in `Arc<r2d2::Pool>`. **Never hold a connection across `.await`** — get it, use it synchronously, drop it.
- If `r2d2_sqlite` causes friction for a specific call site, a `tokio::sync::Mutex<Connection>` is a fine fallback. Performance is not the bottleneck for a single-user tool.

### Modules & ownership

- All DB writes go through the `Ingestor`. UI / API code never opens a connection directly.
- New OTLP metrics: extend the typed table if it's a known metric, otherwise let it flow into `metrics_raw`. Don't add ad-hoc tables without updating `docs/architecture.md`.
- Resource attributes (`session.id`, `user.account_uuid`, `organization.id`, `service.version`, `host.arch`, `os.type`, `terminal.type`) are denormalised onto every row at ingest time. Don't re-derive them downstream.

### Observability

- `tracing::instrument` on every public async fn in `otlp/`, `api/`, `db/`.
- Logs go to `~/.andon/log.txt` (daily rotation via `tracing-appender`).
- Use structured fields (`tracing::info!(session_id = %id, "…")`), not formatted strings.

### Serialisation

- `serde` for every JSON payload, in both directions. **Never hand-write JSON strings.**
- DTOs live in `src-tauri/src/api/dto.rs`. Response shapes are flat and chart-ready — the SPA shouldn't have to reshape.

### Naming / style

- `cargo fmt` on save. `cargo clippy --all-targets -- -D warnings` must pass.
- Module file naming: `snake_case.rs`; modules with submodules use a folder + `mod.rs`.
- US English in identifiers, comments, and docs.

## Angular guidelines

Stack: Angular 21, standalone components, signals, Tailwind 4, `lucide-angular`. Charts are hand-rolled SVG + CSS (themable, deterministic). `@spartan-ng/brain` is declared in `package.json` but currently unused — don't reach for it without a deliberate decision. `chart.js` / `ng2-charts` are present for legacy callers; do not introduce new usages.

### Component patterns

```typescript
@Component({
  selector: 'app-overview',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, /* feature deps */],
  template: `
    @if (store.loading()) {
      <p>Loading…</p>
    } @else if (store.error()) {
      <p>Error: {{ store.error() }}</p>
    } @else {
      @for (row of store.rows(); track row.id) {
        <app-row [row]="row" />
      }
    }
  `,
})
export class OverviewComponent {
  protected store = inject(OverviewStore);
}
```

Rules:

- **Standalone components only.** No NgModules anywhere.
- `inject()` for DI. No constructor injection.
- `ChangeDetectionStrategy.OnPush` on every component.
- **Signals for all state.** `signal()`, `computed()`, `effect()`. No `Subject` / `BehaviorSubject` / `Observable` in feature code. RxJS is allowed only inside the `HttpClient` boundary — convert to signals at the edge.
- Built-in control flow: `@if`, `@for`, `@switch`. Never `*ngIf` / `*ngFor`.
- Lazy-load feature routes with `loadComponent` / `loadChildren`.

### State

- Per-feature stores are plain `Injectable({ providedIn: 'root' })` services exposing `signal()`s + `computed()`s. No NgRx, no NGXS.
- One `ApiService` wraps fetches against `http://127.0.0.1:8765`. Feature stores call into it.

### Styling

- Tailwind utilities first.
- Custom CSS only for SpartanNG overrides. Keep it scoped to the component.
- **Layout is grid-based**, not flexbox-only. Master-detail navigation uses the parent layout + `position: absolute; inset: 0` overlay pattern — the summary list is *not* destroyed when navigating to a detail view.

### Charts

- Hand-rolled SVG + CSS components in `web/src/app/shared/`. They take signal inputs and re-render reactively.
- If you need a chart type that doesn't exist, build it as another small SVG component — do not pull in Chart.js for new code.

### Naming / style

- File naming: `kebab-case.component.ts`, `kebab-case.service.ts`.
- Component selectors: `app-<feature>-<part>`.
- US English in identifiers and templates.

## Testing

- **Rust**: `cd src-tauri; cargo test` for unit + integration tests (the crate lives in `src-tauri/`, not at the repo root). Integration tests run against the real SQLite (file-backed under `tempfile`) — do not mock the DB.
- **Angular**: `cd web; npm test -- --watch=false` for Karma + Jasmine.
- **Smoke**: `node scripts/smoke_grpc.js` and `python scripts/smoke_otlp.py` exercise the live OTLP receivers. Run these before merging anything that touches `otlp/` or the `Ingestor`.
- **TDD is non-negotiable** — write the failing test first.

A formal test harness is being built; see [`docs/superpowers/plans/2026-05-18-test-harness-phase-1.md`](docs/superpowers/plans/2026-05-18-test-harness-phase-1.md) for current state.

## Security and privacy

These are user-visible promises the README makes. Do not weaken them.

1. **Loopback only.** All three listeners (`:4317`, `:4318`, `:8765`) bind to `127.0.0.1`. Never `0.0.0.0`.
2. **No raw prompts.** Even if `OTEL_LOG_USER_PROMPTS=1` is set upstream, andon must not persist or log prompt bodies.
3. **No outbound calls** except the opt-in OTel forwarder (off by default, configured under Settings → OTel forwarder).
4. **DB file is user-only.** `~/.andon/data.db` is read/write for the current user only.
5. **No telemetry of telemetry.** The app must not phone home — no update checks, no error reporting endpoint, nothing.

Anything that changes `~/.claude/settings.json` (the integration patcher, the hook installer) must keep its backup-first behaviour and never overwrite a foreign OTLP endpoint configuration.

## Release process

Manual + local — no CI. Full steps (commands, artefact paths, release-notes skeleton) in [`docs/releasing.md`](docs/releasing.md). Summary:

1. Squash-merge the PR into `main`.
2. Bump `version` in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` on `main` as a separate commit.
3. `cargo tauri build` to produce the NSIS `.exe`, MSI installer, and (renamed) portable binary.
4. Tag (`vX.Y.Z`), push the tag, then `gh release create` attaching the three artefacts.

Do not invent a new release flow without updating [`docs/releasing.md`](docs/releasing.md) first.

## Definition of Done

Before opening a PR, all of these must pass:

**Build & quality**
- [ ] `cargo build --release` succeeds
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] `cd web; npm run build` succeeds
- [ ] No `unwrap()` / `expect()` introduced outside `main.rs` setup

**Tests**
- [ ] `cd src-tauri; cargo test` passes
- [ ] `cd web; npm test -- --watch=false` passes
- [ ] New code has tests driving it (TDD)
- [ ] If `otlp/` or `Ingestor` changed: relevant smoke script run locally

**Behaviour**
- [ ] `cargo tauri dev` launches; tray icon appears; closing the window hides to tray
- [ ] For UI changes: screenshots attached to the PR
- [ ] For schema changes: migration tested against an existing `~/.andon/data.db`

**Privacy**
- [ ] No new outbound network call (forwarder excluded)
- [ ] No new bind to `0.0.0.0`
- [ ] No raw prompt content logged or stored

**Process**
- [ ] Branch rebased on latest `main`
- [ ] Conventional Commits in commit history
- [ ] Linked spec / plan in PR body if non-trivial
- [ ] `docs/` updated if architecture, schema, or ports changed

**Sanity check**
- [ ] Is this the simplest change that works?
- [ ] Would the next contributor understand this in six months?
- [ ] Did I fix the root cause, not the symptom?

## Useful links

- [Tauri 2 docs](https://tauri.app/v2/)
- [OpenTelemetry Protocol](https://opentelemetry.io/docs/specs/otlp/)
- [opentelemetry-proto crate](https://docs.rs/opentelemetry-proto)
- [Angular signals](https://angular.dev/guide/signals)
- [SpartanNG](https://www.spartan.ng/)
- [Tailwind v4](https://tailwindcss.com/)

Thanks for helping make andon better.
