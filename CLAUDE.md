# CLAUDE.md

> Instructions for Claude Code working in this repo. Human contributors: see [`CONTRIBUTING.md`](CONTRIBUTING.md).

Andon is a single-binary Tauri 2 desktop app that ingests Claude Code OpenTelemetry, persists it to embedded SQLite, and renders an Angular dashboard. Everything runs on `127.0.0.1`. No cloud, no auth, no outbound network (except the opt-in OTel forwarder).

## Where to look first

| For… | Read |
|---|---|
| What the app is and how to install it | [`README.md`](README.md) |
| System design, ports, schema, ingestion path | [`docs/architecture.md`](docs/architecture.md) |
| Prerequisites and build instructions | [`docs/building.md`](docs/building.md) |
| What each dashboard page does | [`docs/features.md`](docs/features.md) |
| Contributing rules (Rust + Angular) | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| Cutting a release | [`docs/releasing.md`](docs/releasing.md) |
| In-flight design + plans (superpowers workflow) | [`docs/superpowers/specs/`](docs/superpowers/specs) and [`docs/superpowers/plans/`](docs/superpowers/plans) |

Don't duplicate what those files say — link to them.

## Quick commands (Windows / PowerShell)

Each block assumes you start at the repo root. `cd` back (or open a fresh shell) between blocks.

```powershell
# Dev (Rust backend + embedded SPA, hot reload)
cargo tauri dev

# Build the SPA only
cd web; npm install; npm run build

# Production binary + installers
cargo tauri build

# Tests
cd src-tauri; cargo test --features test-support   # Rust unit + integration tests
cd web; npm test                      # Angular tests (Vitest, CI mode by default)

# Smoke-test the OTLP receivers against a running app
# (start `cargo tauri dev` first — these need the listeners bound on :4317 / :4318)
cd scripts
npm install                             # one-time for the gRPC smoke deps
node smoke_grpc.js
python smoke_otlp.py                    # no deps; uses stdlib
```

Tagged builds follow [`docs/releasing.md`](docs/releasing.md) — do not invent a new flow.

## Repo map

```
src-tauri/         Rust backend (Tauri shell, OTLP receivers, axum API, SQLite)
  src/otlp/        gRPC :4317 + HTTP :4318 receivers, ingestor, forwarder
  src/api/         axum routes + DTOs (:8765)
  src/db/          rusqlite pool, migrations, queries
  src/reports/     standalone HTML session/diagnostic reports
  src/{integration,autostart,diagnostics,settings,git_query,repo_inference}.rs
  templates/       MiniJinja templates for reports
web/               Angular 21 SPA (standalone components, signals)
  src/app/features/{overview,sessions,files,diagnostics,settings}
docs/              Architecture, building, features, pitch
docs/superpowers/  Active design specs + implementation plans
scripts/           OTLP smoke scripts (gRPC via Node, HTTP via stdlib Python)
```

## Non-negotiable rules

Full rationale lives in [`CONTRIBUTING.md`](CONTRIBUTING.md). Short list for in-session reference:

**Rust**
- No `unwrap()` / `expect()` outside `main.rs` setup. Use `anyhow::Result` at the boundary, `thiserror` for domain errors.
- All DB writes go through the `Ingestor`. Never hold an `rusqlite` connection across `.await`.
- `tracing::instrument` on public async fns in `otlp/`, `api/`, `db/`.
- `serde` for every JSON payload — never hand-write JSON strings.
- OTLP receivers always return `Ok` to the client. Log ingestion failures, never surface them.

**Angular**
- Standalone components only. No NgModules. No NgRx.
- Signals for all state: `signal()`, `computed()`, `effect()`. No `Subject` / `Observable` in feature code.
- `inject()`, `ChangeDetectionStrategy.OnPush`, `@if` / `@for` / `@switch` (never `*ngIf` / `*ngFor`).
- Tailwind utilities first. Custom CSS only when utilities don't cover the case.
- Master-detail nav uses parent layout + `position: absolute; inset: 0` overlay. Do not destroy summary components when navigating to detail.

**Both**
- US English everywhere (color, behavior, organize).
- Mermaid for diagrams in markdown. ASCII art only in CLI / console output.
- Conventional Commits (no emojis): `type(scope): subject`.
- TDD: failing test first, then implementation, then refactor.
- SOLID is non-negotiable for new classes / modules.

## Privacy guarantees the code must keep

1. All listeners bind to `127.0.0.1` only — never `0.0.0.0`.
2. Raw user prompts are never persisted, even if `OTEL_LOG_USER_PROMPTS=1` upstream.
3. No outbound network calls except the opt-in OTel forwarder.
4. No "telemetry of telemetry" — the app does not phone home.

If a change touches receivers, the API surface, or `~/.claude/settings.json` patching, re-read [`docs/architecture.md`](docs/architecture.md) §"Privacy & safety rules" before merging.

## Workflow for non-trivial work

1. **Spec** → write a design doc under `docs/superpowers/specs/YYYY-MM-DD-<slug>-design.md`.
2. **Plan** → write a step-by-step plan under `docs/superpowers/plans/YYYY-MM-DD-<slug>.md`.
3. **Implement** on a short-lived feature branch off `main`. Squash-merge via PR.
4. **Verify** against the Definition of Done in [`CONTRIBUTING.md`](CONTRIBUTING.md).

Bug fixes and one-line changes skip the spec/plan — go straight to a PR.

## Out of scope (do not add without asking)

Multi-user / multi-machine aggregation · cloud sync · auth · mobile clients · HTTPS (it's localhost) · update checker / auto-updater · CSV/Excel export · user-defined queries.

The OTel **forwarder** is in scope and shipped — re-emits to one downstream HTTP/protobuf endpoint, opt-in, off by default.
