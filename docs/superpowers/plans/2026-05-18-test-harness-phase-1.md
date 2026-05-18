# Test Harness Phase 1 — Foundation + Filter Tests + CI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up Rust and Angular test harnesses, prove they work by covering the filter logic on both sides, and wire CI to run them on every PR.

**Architecture:** Rust side uses `cargo test` with a `tests/common/` fixture helper that opens a temp-file SQLite (WAL needs real file) with migrations applied. Angular side replaces Jasmine/Karma with Vitest + `@analogjs/vitest-angular` + Spectator + ng-mocks, running headless in jsdom. GitHub Actions matrix runs both on every PR.

**Tech Stack:** Rust: `cargo test`, `tempfile`, `rusqlite`. Web: Vitest, @analogjs/vitest-angular, @ngneat/spectator (≥19), ng-mocks, jsdom, @vitest/coverage-v8. CI: GitHub Actions, ubuntu/macos/windows matrix.

**Branch:** `tests/phase-1-harness` off `main`.

---

## File Structure

**Rust (new):**
- `src-tauri/tests/common/mod.rs` — `fixture_pool()`, `seed_session()`, sample-payload builders.
- `src-tauri/tests/filter_query.rs` — unit tests for `FilterQuery` (window, model_list, model_clause).

**Web (new):**
- `web/vitest.config.ts` — Analog preset, jsdom env.
- `web/setup-vitest.ts` — `zone.js/testing` + Spectator globals.
- `web/src/testing/signal-helpers.ts`
- `web/src/testing/api-fixtures.ts`
- `web/src/testing/filter-builder.ts`
- `web/src/testing/spectator-factories.ts`
- `web/src/app/core/filter.service.spec.ts`
- `web/src/app/shared/filter-bar.component.spec.ts`

**Web (modified):**
- `web/package.json` — swap test runner deps.
- `web/tsconfig.spec.json` — Vitest types instead of Jasmine.
- Delete `web/karma.conf.js` and `web/src/test.ts` if present.
- Delete the obsolete `web/src/app/app.component.spec.ts` if it uses Jasmine APIs that don't port cleanly (or rewrite it as the smoke test in Task 9).

**Rust (modified):**
- `src-tauri/src/api/routes.rs` — expose `FilterQuery` (and `current_month_bounds` if needed) to integration tests via `pub(crate)` or `#[cfg(test)] pub`.

**CI (new):**
- `.github/workflows/ci.yml`

---

## Open Questions to Resolve Before/During Execution

- **Exposing private items for tests:** `FilterQuery` is currently private inside `routes.rs`. Two options: (a) make it `pub(crate)` and re-export through a `#[cfg(test)] pub mod test_support` module in `lib.rs`; (b) move filter parsing into its own module `api/filter.rs` and make that module public to the crate. Prefer (b) — it isolates the testable surface cleanly. If the cost looks high, fall back to (a). Pick during Task 4.
- **Spectator + Vitest + Angular 21 compatibility:** Documented fallback is Spectator + Jest. Confirm during Task 6 with a throwaway test before scaling out.

---

## Task 1: Create branch and verify clean tree

**Files:** none (git only)

- [ ] **Step 1: Confirm working tree is clean**

```bash
git status
```

Expected: `working tree clean` on `main`. If uncommitted changes exist, stash or commit them first.

- [ ] **Step 2: Create and switch to feature branch**

```bash
git checkout -b tests/phase-1-harness
```

- [ ] **Step 3: Confirm baseline tests run (so we have a baseline)**

```bash
cd src-tauri && cargo test --no-run 2>&1 | tail -20
```

Expected: compiles. There may be zero existing tests — that's fine.

---

## Task 2: Add Rust test fixture helper — `fixture_pool()`

**Files:**
- Create: `src-tauri/tests/common/mod.rs`
- Create: `src-tauri/tests/_harness_smoke.rs`

- [ ] **Step 1: Write the failing smoke test**

`src-tauri/tests/_harness_smoke.rs`:

```rust
mod common;

#[test]
fn fixture_pool_applies_migrations_and_supports_wal() {
    let (pool, _guard) = common::fixture_pool();
    let conn = pool.get().expect("checkout connection");

    // WAL mode must be active (in-memory wouldn't support it).
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("read journal_mode");
    assert_eq!(mode.to_lowercase(), "wal");

    // Migrations create the sessions table.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
            [],
            |r| r.get(0),
        )
        .expect("count sessions table");
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run to confirm it fails (no `common` module yet)**

```bash
cd src-tauri && cargo test --test _harness_smoke
```

Expected: compile error `file not found for module 'common'`.

- [ ] **Step 3: Implement `fixture_pool()`**

`src-tauri/tests/common/mod.rs`:

```rust
#![allow(dead_code)] // helpers are shared across many test files

use std::sync::Arc;

use andon_lib::db::{self, DbPool};
use tempfile::TempDir;

/// Build an isolated SQLite pool backed by a temp file (WAL needs a real file).
/// Returns the pool plus the TempDir guard — drop the guard to delete the DB.
pub fn fixture_pool() -> (Arc<DbPool>, TempDir) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("test.db");
    let pool = db::open_pool(&db_path).expect("open pool");
    db::run_migrations(&pool).expect("run migrations");
    (Arc::new(pool), dir)
}
```

Note: this assumes `andon_lib::db::open_pool(&Path) -> Result<DbPool>` and `andon_lib::db::run_migrations(&DbPool) -> Result<()>` exist and are public. If they aren't, expose them as `pub` in `src-tauri/src/db/mod.rs` and `src-tauri/src/db/migrations.rs` as part of this step. Check existing signatures with `rg "pub fn open_pool|pub fn run_migrations" src-tauri/src/db` before writing — adapt names to match.

- [ ] **Step 4: Run test to verify it passes**

```bash
cd src-tauri && cargo test --test _harness_smoke
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/
git commit -m "test: add fixture_pool harness with temp-file WAL SQLite"
```

---

## Task 3: Add `seed_session()` and sample-payload builders

**Files:**
- Modify: `src-tauri/tests/common/mod.rs`
- Modify: `src-tauri/tests/_harness_smoke.rs`

- [ ] **Step 1: Write the failing test for `seed_session`**

Append to `_harness_smoke.rs`:

```rust
#[test]
fn seed_session_inserts_session_and_related_rows() {
    let (pool, _guard) = common::fixture_pool();
    let opts = common::SeedOpts {
        session_id: "sess-1".into(),
        model: "claude-opus-4-5-20251001".into(),
        input_tokens: 100,
        output_tokens: 50,
        cost_usd: 0.42,
        decisions: vec![("accept", "rust"), ("reject", "rust")],
        ..Default::default()
    };
    common::seed_session(&pool, &opts);

    let conn = pool.get().unwrap();
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(session_count, 1);

    let token_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(token_rows, 2, "one row per token_type");

    let cost: f64 = conn
        .query_row("SELECT cost_usd FROM cost_entries WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert!((cost - 0.42).abs() < 1e-9);

    let decisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM tool_decisions WHERE session_id = ?", ["sess-1"], |r| r.get(0))
        .unwrap();
    assert_eq!(decisions, 2);
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cd src-tauri && cargo test --test _harness_smoke seed_session
```

Expected: compile error — `SeedOpts` and `seed_session` not defined.

- [ ] **Step 3: Implement `SeedOpts` and `seed_session`**

Append to `src-tauri/tests/common/mod.rs`:

```rust
use rusqlite::params;

#[derive(Default, Clone)]
pub struct SeedOpts {
    pub session_id: String,
    pub started_at_ms: Option<i64>,        // defaults to "now"
    pub ended_at_ms: Option<i64>,          // None = still open
    pub model: String,                     // full model id
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    /// Pairs of (decision, language). decision ∈ accept|reject|abort.
    pub decisions: Vec<(&'static str, &'static str)>,
    pub files: Vec<FileChange>,
}

#[derive(Clone)]
pub struct FileChange {
    pub path: &'static str,
    pub added: i64,
    pub removed: i64,
}

pub fn seed_session(pool: &Arc<DbPool>, opts: &SeedOpts) {
    let now = chrono::Utc::now().timestamp_millis();
    let started = opts.started_at_ms.unwrap_or(now);
    let conn = pool.get().expect("checkout");

    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES (?, ?, ?)",
        params![opts.session_id, started, opts.ended_at_ms],
    ).expect("insert session");

    if opts.input_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES (?, ?, ?, 'input', ?)",
            params![opts.session_id, started, opts.model, opts.input_tokens],
        ).unwrap();
    }
    if opts.output_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES (?, ?, ?, 'output', ?)",
            params![opts.session_id, started, opts.model, opts.output_tokens],
        ).unwrap();
    }
    if opts.cost_usd != 0.0 {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES (?, ?, ?, ?)",
            params![opts.session_id, started, opts.model, opts.cost_usd],
        ).unwrap();
    }
    for (decision, lang) in &opts.decisions {
        conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision, language) VALUES (?, ?, 'Edit', ?, ?)",
            params![opts.session_id, started, decision, lang],
        ).unwrap();
    }
    for f in &opts.files {
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added, lines_removed) VALUES (?, ?, ?, ?, ?)",
            params![opts.session_id, started, f.path, f.added, f.removed],
        ).unwrap();
    }
}
```

Adjust column names if migration introspection reveals drift from CLAUDE.md schema. Run `sqlite3 :memory: < <migration sql>` mentally if unsure, or just run the test and adapt to errors.

- [ ] **Step 4: Run to verify it passes**

```bash
cd src-tauri && cargo test --test _harness_smoke seed_session
```

Expected: 1 passed.

- [ ] **Step 5: Add `sample_export_metrics()` and `sample_export_logs()` builders**

Append to `src-tauri/tests/common/mod.rs`:

```rust
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, KeyValue, any_value::Value as AnyV},
    metrics::v1::{
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
        metric::Data, number_data_point::Value as NumberValue,
    },
    resource::v1::Resource,
};

pub fn kv(key: &str, val: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(AnyV::StringValue(val.into())) }),
    }
}

/// Build a `Vec<ResourceMetrics>` with one Sum-typed metric data point.
pub fn sample_sum_metric(
    resource_attrs: Vec<KeyValue>,
    metric_name: &str,
    point_attrs: Vec<KeyValue>,
    value: f64,
) -> Vec<ResourceMetrics> {
    vec![ResourceMetrics {
        resource: Some(Resource { attributes: resource_attrs, dropped_attributes_count: 0 }),
        scope_metrics: vec![ScopeMetrics {
            scope: None,
            metrics: vec![Metric {
                name: metric_name.into(),
                description: String::new(),
                unit: String::new(),
                metadata: vec![],
                data: Some(Data::Sum(Sum {
                    data_points: vec![NumberDataPoint {
                        attributes: point_attrs,
                        start_time_unix_nano: 0,
                        time_unix_nano: 1_700_000_000_000_000_000,
                        exemplars: vec![],
                        flags: 0,
                        value: Some(NumberValue::AsDouble(value)),
                    }],
                    aggregation_temporality: 2, // Cumulative
                    is_monotonic: true,
                })),
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
}
```

(Use later in Phase 2. Add a one-line compile-only test to keep it from bit-rotting):

Append to `_harness_smoke.rs`:

```rust
#[test]
fn sample_sum_metric_builds_a_single_point() {
    let rm = common::sample_sum_metric(
        vec![common::kv("session.id", "s1")],
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-opus-4-5-20251001")],
        1.23,
    );
    assert_eq!(rm.len(), 1);
}
```

- [ ] **Step 6: Run, then commit**

```bash
cd src-tauri && cargo test --test _harness_smoke
git add src-tauri/tests/common/mod.rs src-tauri/tests/_harness_smoke.rs
git commit -m "test: add seed_session and OTLP sample builders to fixture"
```

---

## Task 4: Expose `FilterQuery` for integration tests

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (or split into `src-tauri/src/api/filter.rs`)
- Modify: `src-tauri/src/api/mod.rs`

- [ ] **Step 1: Decide on (a) or (b) from the Open Questions section**

Recommendation: extract `FilterQuery`, `current_month_bounds`, and the `model_clause` helper into `src-tauri/src/api/filter.rs`. Re-export from `api/mod.rs` as `pub mod filter;` so integration tests can `use andon_lib::api::filter::FilterQuery`.

- [ ] **Step 2: Move the type and helpers verbatim, run the build**

```bash
cd src-tauri && cargo build
```

Expected: compiles. If `routes.rs` uses other private items the move touches, fix in place — don't pull additional things public.

- [ ] **Step 3: Commit the refactor on its own**

```bash
git add src-tauri/src/api/
git commit -m "refactor: move FilterQuery to api::filter module for testability"
```

---

## Task 5: Tests for `FilterQuery`

**Files:**
- Create: `src-tauri/tests/filter_query.rs`

- [ ] **Step 1: Write failing tests covering all stated behavior**

```rust
mod common;

use andon_lib::api::filter::FilterQuery;

fn fq(from: Option<i64>, to: Option<i64>, models: Option<&str>) -> FilterQuery {
    FilterQuery {
        from,
        to,
        models: models.map(|s| s.to_string()),
    }
}

#[test]
fn window_defaults_to_current_month_when_unset() {
    let q = fq(None, None, None);
    let (from, to) = q.window();
    assert!(from < to);
    assert!(to - from > 24 * 3600 * 1000, "window should span >= 1 day");
}

#[test]
fn window_uses_explicit_bounds_when_set() {
    let q = fq(Some(1000), Some(2000), None);
    assert_eq!(q.window(), (1000, 2000));
}

#[test]
fn model_list_handles_empty_whitespace_and_trailing_comma() {
    assert!(fq(None, None, None).model_list().is_empty());
    assert!(fq(None, None, Some("")).model_list().is_empty());
    assert!(fq(None, None, Some("  ,  ")).model_list().is_empty());
    assert_eq!(
        fq(None, None, Some("opus, sonnet,")).model_list(),
        vec!["opus".to_string(), "sonnet".to_string()],
    );
}

#[test]
fn model_clause_is_empty_when_no_models_set() {
    let q = fq(None, None, None);
    let (sql, params) = q.model_clause("model");
    assert!(sql.is_empty());
    assert!(params.is_empty());
}

#[test]
fn model_clause_builds_substring_or_chain_and_lowercases_params() {
    let q = fq(None, None, Some("Opus,Sonnet"));
    let (sql, params) = q.model_clause("tu.model");
    assert_eq!(
        sql,
        " AND (LOWER(tu.model) LIKE ? OR LOWER(tu.model) LIKE ?)"
    );
    assert_eq!(params, vec!["%opus%".to_string(), "%sonnet%".to_string()]);
}
```

Field visibility note: if `FilterQuery`'s fields aren't `pub`, either make them `pub` (they're a query struct, low risk) or add a `pub fn new_for_test(...)` constructor. Adapt to whichever the codebase prefers.

- [ ] **Step 2: Run, expect fail (compile or assertion)**

```bash
cd src-tauri && cargo test --test filter_query
```

Expected: at least compile success, possibly assertion failure if behavior differs. Fix the test if the actual SQL string differs in whitespace/quoting; fix the code only if a true bug surfaces.

- [ ] **Step 3: Once green, commit**

```bash
git add src-tauri/tests/filter_query.rs
git commit -m "test: cover FilterQuery window, model_list, model_clause"
```

---

## Task 6: Swap web test runner — Vitest + Spectator + ng-mocks scaffolding

**Files:**
- Modify: `web/package.json`
- Delete: `web/karma.conf.js`, `web/src/test.ts` (if present)
- Modify: `web/tsconfig.spec.json`
- Create: `web/vitest.config.ts`
- Create: `web/setup-vitest.ts`

- [ ] **Step 1: Update `package.json` deps and scripts**

Remove `jasmine-core`, `@types/jasmine`, `karma`, `karma-*`. Add as devDependencies:

- `vitest`
- `@analogjs/vitest-angular`
- `@ngneat/spectator` (verify ≥19 supports Angular 21 standalone)
- `ng-mocks`
- `jsdom`
- `@vitest/coverage-v8`

Update scripts:

```json
"test": "vitest run",
"test:watch": "vitest",
"test:coverage": "vitest run --coverage"
```

Then:

```bash
cd web && npm install
```

- [ ] **Step 2: Create `vitest.config.ts` per Analog preset**

```ts
import angular from '@analogjs/vite-plugin-angular';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [angular()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./setup-vitest.ts'],
    include: ['src/**/*.spec.ts'],
  },
});
```

- [ ] **Step 3: Create `setup-vitest.ts`**

```ts
import 'zone.js';
import 'zone.js/testing';
import { getTestBed } from '@angular/core/testing';
import {
  BrowserDynamicTestingModule,
  platformBrowserDynamicTesting,
} from '@angular/platform-browser-dynamic/testing';

getTestBed().initTestEnvironment(BrowserDynamicTestingModule, platformBrowserDynamicTesting());
```

- [ ] **Step 4: Update `tsconfig.spec.json`** — replace `"types": ["jasmine"]` with `"types": ["vitest/globals", "node"]`. Adjust `include` if Karma's `test.ts` was referenced.

- [ ] **Step 5: Delete Karma artifacts**

```bash
cd web && git rm -f karma.conf.js src/test.ts 2>$null; if (Test-Path src/app/app.component.spec.ts) { git rm -f src/app/app.component.spec.ts }
```

(PowerShell — the existing app spec is Jasmine-based and we'll rewrite a smoke version later if needed. Easier to delete than port.)

- [ ] **Step 6: Throwaway compatibility probe**

Create `web/src/_vitest_probe.spec.ts`:

```ts
import { TestBed } from '@angular/core/testing';
import { Component, signal } from '@angular/core';

@Component({ standalone: true, template: '{{ count() }}' })
class ProbeCmp {
  count = signal(7);
}

describe('vitest + angular 21 standalone probe', () => {
  it('renders a signal value', () => {
    const fixture = TestBed.createComponent(ProbeCmp);
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('7');
  });
});
```

- [ ] **Step 7: Run**

```bash
cd web && npm test
```

Expected: probe passes. If Spectator import fails, the documented fallback is Spectator + Jest. If Vitest itself fails with Angular 21 standalone, escalate before proceeding — do not silently rip Spectator out.

- [ ] **Step 8: Delete the probe and commit**

```bash
cd web && git rm src/_vitest_probe.spec.ts
git add web/
git commit -m "test(web): replace Karma/Jasmine with Vitest + Spectator + ng-mocks"
```

---

## Task 7: Web testing utilities folder

**Files:**
- Create: `web/src/testing/signal-helpers.ts`
- Create: `web/src/testing/api-fixtures.ts`
- Create: `web/src/testing/filter-builder.ts`
- Create: `web/src/testing/spectator-factories.ts`

- [ ] **Step 1: Stub each file with the documented surface**

`signal-helpers.ts`:

```ts
import { Signal } from '@angular/core';

/** Read a signal/computed; convenience to keep test bodies short. */
export function read<T>(s: Signal<T>): T {
  return s();
}
```

`api-fixtures.ts` — start small; grow as endpoints get tested:

```ts
export const sampleOverviewToday = {
  cost_usd: 1.23,
  sessions: 4,
  accept_rate: 0.8125,
};
```

`filter-builder.ts`:

```ts
import { FilterService, RangePreset } from '../app/core/filter.service';

export interface FilterFixtureOpts {
  range?: RangePreset;
  models?: string[];
  search?: string;
}

export function buildFilter(opts: FilterFixtureOpts = {}): FilterService {
  const f = new FilterService();
  if (opts.range) f.setRange(opts.range);
  if (opts.models) {
    f.allModels().forEach((m) => {
      const wanted = opts.models!.includes(m);
      const has = f.models().has(m);
      if (wanted !== has) f.toggleModel(m);
    });
  }
  if (opts.search != null) f.setSearch(opts.search);
  return f;
}
```

`spectator-factories.ts`:

```ts
export { createComponentFactory, createServiceFactory, Spectator, SpectatorService } from '@ngneat/spectator';
export { MockComponent, MockProvider, MockDirective, MockPipe } from 'ng-mocks';
```

- [ ] **Step 2: Commit (no tests yet — these exist to be imported)**

```bash
git add web/src/testing/
git commit -m "test(web): add testing/ folder with shared fixtures and factories"
```

---

## Task 8: `FilterService` tests

**Files:**
- Create: `web/src/app/core/filter.service.spec.ts`

- [ ] **Step 1: Write failing tests covering each public surface**

```ts
import { createServiceFactory } from '../../testing/spectator-factories';
import { FilterService } from './filter.service';

describe('FilterService', () => {
  const createService = createServiceFactory({ service: FilterService });

  it('starts on month range with all models selected and no active filters', () => {
    const s = createService().service;
    expect(s.range()).toBe('month');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.hasActiveFilters()).toBe(false);
    expect(s.modelsCsv()).toBe(''); // all = empty (server convention)
  });

  it('window() for "today" spans start-of-day to end-of-day', () => {
    const s = createService().service;
    s.setRange('today');
    const w = s.window();
    const from = new Date(w.fromMs);
    const to = new Date(w.toMs);
    expect(from.getHours()).toBe(0);
    expect(to.getHours()).toBe(23);
  });

  it('window() for "30d" spans 30 days inclusive of today', () => {
    const s = createService().service;
    s.setRange('30d');
    const w = s.window();
    const days = Math.round((w.toMs - w.fromMs) / 86_400_000);
    expect(days).toBeGreaterThanOrEqual(29);
    expect(days).toBeLessThanOrEqual(30);
  });

  it('enterCustomMode seeds the custom range from the prior window', () => {
    const s = createService().service;
    s.setRange('today');
    const todayWin = s.window();
    s.enterCustomMode();
    expect(s.range()).toBe('custom');
    expect(s.customRange()).toEqual({ fromMs: todayWin.fromMs, toMs: todayWin.toMs });
  });

  it('setCustomFrom clamps so from <= to', () => {
    const s = createService().service;
    s.enterCustomMode();
    const cur = s.customRange()!;
    s.setCustomFrom(cur.toMs + 1000); // attempt to set "from" past "to"
    const after = s.customRange()!;
    expect(after.fromMs).toBeLessThanOrEqual(after.toMs);
  });

  it('toggleModel refuses to deselect the last active chip', () => {
    const s = createService().service;
    const all = s.allModels();
    all.slice(1).forEach((m) => s.toggleModel(m)); // remove every chip but the first
    expect(s.models().size).toBe(1);
    s.toggleModel(all[0]); // attempt to remove the last
    expect(s.models().size).toBe(1);
  });

  it('modelsCsv is empty when all selected, csv otherwise', () => {
    const s = createService().service;
    expect(s.modelsCsv()).toBe('');
    const all = s.allModels();
    s.toggleModel(all[0]); // remove one
    expect(s.modelsCsv().split(',').length).toBe(all.length - 1);
  });

  it('hasActiveFilters reflects range / models / search', () => {
    const s = createService().service;
    expect(s.hasActiveFilters()).toBe(false);
    s.setRange('today');
    expect(s.hasActiveFilters()).toBe(true);
    s.clearFilters();
    expect(s.hasActiveFilters()).toBe(false);
    s.setSearch('foo');
    expect(s.hasActiveFilters()).toBe(true);
  });

  it('clearFilters resets all state to defaults', () => {
    const s = createService().service;
    s.setRange('today');
    s.setSearch('foo');
    s.toggleModel(s.allModels()[0]);
    s.clearFilters();
    expect(s.range()).toBe('month');
    expect(s.search()).toBe('');
    expect(s.models().size).toBe(s.allModels().length);
    expect(s.customRange()).toBeNull();
  });
});
```

- [ ] **Step 2: Run** — `cd web && npm test`. Adjust assertions against actual behavior; fix code only on a true bug.

- [ ] **Step 3: Commit**

```bash
git add web/src/app/core/filter.service.spec.ts
git commit -m "test(web): cover FilterService presets, custom range, toggle rules"
```

---

## Task 9: `FilterBarComponent` tests

**Files:**
- Create: `web/src/app/shared/filter-bar.component.spec.ts`

- [ ] **Step 1: Write failing tests**

```ts
import { createComponentFactory, MockComponent } from '../../testing/spectator-factories';
import { LucideAngularModule, LucideIconData } from 'lucide-angular';
import { FilterBarComponent } from './filter-bar.component';
import { FilterService } from '../core/filter.service';

describe('FilterBarComponent', () => {
  const create = createComponentFactory({
    component: FilterBarComponent,
    imports: [LucideAngularModule],
    providers: [FilterService],
    detectChanges: false,
  });

  it('clicking a range chip switches the preset', () => {
    const spec = create();
    spec.detectChanges();
    const todayChip = spec.queryAll('.filter-chip').find((b) => b.textContent?.trim().toLowerCase().startsWith('today'));
    expect(todayChip).toBeTruthy();
    todayChip!.click();
    spec.detectChanges();
    expect(spec.inject(FilterService).range()).toBe('today');
  });

  it('clicking "Custom…" reveals the date inputs', () => {
    const spec = create();
    spec.detectChanges();
    expect(spec.queryAll('input[type="date"]').length).toBe(0);
    const custom = spec.queryAll('.filter-chip').find((b) => b.textContent?.toLowerCase().includes('custom'));
    custom!.click();
    spec.detectChanges();
    expect(spec.queryAll('input[type="date"]').length).toBe(2);
  });

  it('toggling a model chip updates the service', () => {
    const spec = create();
    spec.detectChanges();
    const filter = spec.inject(FilterService);
    const firstModel = filter.allModels()[0];
    const chip = spec.queryAll('.filter-chip').find((b) => b.textContent?.trim() === firstModel);
    chip!.click();
    spec.detectChanges();
    expect(filter.models().has(firstModel)).toBe(false);
  });

  it('Clear button is hidden when no active filters and visible when any', () => {
    const spec = create();
    spec.detectChanges();
    expect(spec.query('button[data-clear]')).toBeFalsy(); // selector TBD: check actual markup
    spec.inject(FilterService).setSearch('foo');
    spec.detectChanges();
    // Replace selector with whatever the component uses for the clear control:
    expect(spec.query('[data-clear], button[aria-label="Clear filters"]')).toBeTruthy();
  });
});
```

Note: the last assertion's selector is illustrative. Read `filter-bar.component.ts` for the actual clear-control markup and update before running. If the chip text is rendered differently (e.g. icon-only), query by `data-active` or button index rather than text.

- [ ] **Step 2: Run, fix selectors against real DOM**

```bash
cd web && npm test -- filter-bar
```

- [ ] **Step 3: Commit**

```bash
git add web/src/app/shared/filter-bar.component.spec.ts
git commit -m "test(web): cover FilterBarComponent chip and date interactions"
```

---

## Task 10: GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write workflow**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  rust:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    defaults:
      run:
        working-directory: src-tauri
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - name: Install Linux Tauri deps
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
      - run: cargo test --workspace --all-features

  web:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: web
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'
          cache-dependency-path: web/package-lock.json
      - run: npm ci
      - run: npm run build
      - run: npm test
```

- [ ] **Step 2: Push branch and open a draft PR to trigger CI**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add cargo test + npm test matrix workflow"
git push -u origin tests/phase-1-harness
gh pr create --draft --title "Phase 1: test harness + filter tests + CI" --body "Establishes Rust and web test harnesses; covers filter logic on both sides; wires CI on Linux/macOS/Windows for Rust and Linux for web."
```

- [ ] **Step 3: Watch the run; fix until all jobs green**

```bash
gh pr checks
```

Common failures:
- Linux Tauri deps missing → already in workflow; if a new one shows up, add it.
- Vitest fails because of Chart.js canvas requirement — Phase 1 has no chart-component tests, so this likely won't surface until Phase 3, but if it does add `jest-canvas-mock` or the `canvas` npm package.

---

## Task 11: Final pass

- [ ] **Step 1: Mark the PR ready for review**

```bash
gh pr ready
```

- [ ] **Step 2: Verify locally one more time**

```bash
cd src-tauri && cargo test
cd ../web && npm test
```

Both green → request review or merge per the team's normal flow.

---

## Done When

- `cargo test` runs the `_harness_smoke` and `filter_query` suites green.
- `npm test` runs `FilterService` and `FilterBarComponent` suites green.
- GitHub Actions runs both on every PR and on `main`.
- The PR description links to this plan.
