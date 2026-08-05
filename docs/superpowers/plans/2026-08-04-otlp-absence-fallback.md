# OTLP-absence Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When live OTLP telemetry never arrives, Andon automatically ingests Claude Code transcripts from disk on a timer, so the dashboard fills itself, and each transcript-sourced session self-labels via the existing `cost_source` UI.

**Architecture:** A new background task (`jsonl::sweep::run_sweep`) spawned at startup mirrors the existing budget monitor: on a settings-driven interval it enumerates `~/.claude/projects/**/*.jsonl`, skips files whose mtime is unchanged since the last tick (an in-memory gate), and calls the existing idempotent `jsonl::ingest_one` on the rest. Two new settings fields (interval + on/off) drive it, exposed through a `PUT /api/settings/sweep` route and a small settings card. No schema change, no OTLP-absence detector — the reconciler's per-session `coverage_for` already skips OTLP-covered sessions, and the existing `cost_source` derivation already surfaces provenance in the UI.

**Tech Stack:** Rust (Tauri 2, axum, rusqlite/r2d2, tokio), Angular 21 (standalone, signals, OnPush), Tailwind, Vitest.

## Global Constraints

- US English everywhere (color, behavior, organize).
- Rust: no `unwrap()`/`expect()` outside `main.rs` setup or `#[cfg(test)]`. `anyhow::Result` at boundaries.
- Rust: `#[tracing::instrument]` on public async fns in `jsonl/`.
- Rust: new `serde` settings fields need **custom** defaults (`#[serde(default = "...")]`) — bare `#[serde(default)]` yields `0`/`false`, which is off/never and silently breaks "on by default".
- Rust: never hold an `rusqlite` connection across `.await`; all DB writes go through the `Ingestor`.
- Angular: standalone components only, `signal()`/`computed()`, `inject()`, `ChangeDetectionStrategy.OnPush`, `@if`/`@for`. Tailwind utilities first.
- Conventional Commits (no emojis): `type(scope): subject`.
- TDD: failing test first, then implementation.
- Do **not** run `cargo fmt` (repo is intentionally not fmt-clean).
- Rust tests: from `src-tauri/`: `cargo test --features test-support <name>`.
- Web tests: from `web/`: `npm test`. Vitest does **not** type-check — after any web change also run `npx tsc -p tsconfig.app.json --noEmit` and require it to pass.

## File Structure

- `src-tauri/src/settings.rs` — add `SweepSettings` + store accessors (Task 1).
- `src-tauri/src/jsonl/sweep.rs` — **new**: `Sweeper` mtime gate, `run_once`, `run_sweep` loop, `next_delay` (Tasks 2–3).
- `src-tauri/src/jsonl/mod.rs` — register `pub mod sweep;` (Task 2).
- `src-tauri/src/lib.rs` — spawn `run_sweep` in setup (Task 4).
- `src-tauri/src/api/routes.rs` — `PUT /api/settings/sweep` handler + route (Task 5).
- `web/src/app/core/api.service.ts` — `SweepSettings` type, extend `AppSettings`, `saveSweep()` (Task 6).
- `web/src/app/features/settings/sweep-card.component.ts` — **new** settings card (Task 7).
- `web/src/app/features/settings/settings.component.ts` — register the card (Task 7).
- `web/src/app/features/sessions/session-detail.component.ts` — optional prominent banner (Task 8, optional).

---

### Task 1: Sweep settings model + persistence

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Test: `src-tauri/src/settings.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: existing `AppSettings`, `SettingsStore`, `write_atomic`, `BudgetSettings` (the pattern to mirror).
- Produces:
  - `pub struct SweepSettings { pub interval_minutes: u32, pub enabled: bool }` (Clone, Serialize, Deserialize, PartialEq, Debug)
  - `impl Default for SweepSettings` → `{ interval_minutes: 5, enabled: true }`
  - `SettingsStore::sweep(&self) -> SweepSettings`
  - `SettingsStore::save_sweep(&self, new: SweepSettings) -> anyhow::Result<SweepSettings>`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn sweep_defaults_on_when_field_absent() {
    // AppSettings JSON that predates the sweep field must default to on/5.
    let json = r#"{"version":1,"forwarder":{"enabled":false,"endpoint":""}}"#;
    let parsed: AppSettings = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.sweep.enabled, true);
    assert_eq!(parsed.sweep.interval_minutes, 5);
}

#[test]
fn save_sweep_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.json");
    let store = SettingsStore::load(path.clone()).unwrap();
    let saved = store
        .save_sweep(SweepSettings { interval_minutes: 10, enabled: false })
        .unwrap();
    assert_eq!(saved.interval_minutes, 10);
    assert_eq!(saved.enabled, false);
    // Reload from disk proves persistence.
    let reloaded = SettingsStore::load(path).unwrap();
    assert_eq!(reloaded.sweep().interval_minutes, 10);
    assert_eq!(reloaded.sweep().enabled, false);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support sweep_defaults_on_when_field_absent save_sweep_round_trips`
Expected: FAIL — no field `sweep` on `AppSettings`, no method `sweep`/`save_sweep`.

- [ ] **Step 3: Write minimal implementation**

Add the struct + custom serde defaults (note: field uses the struct default so existing files get on/5):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SweepSettings {
    #[serde(default = "default_sweep_interval")]
    pub interval_minutes: u32,
    #[serde(default = "default_sweep_enabled")]
    pub enabled: bool,
}

fn default_sweep_interval() -> u32 { 5 }
fn default_sweep_enabled() -> bool { true }

impl Default for SweepSettings {
    fn default() -> Self {
        Self { interval_minutes: default_sweep_interval(), enabled: default_sweep_enabled() }
    }
}
```

Add the field to `AppSettings` (mirror the `budget` field):

```rust
    #[serde(default)]
    pub sweep: SweepSettings,
```

Add accessors to `SettingsStore` (mirror `budget` / `save_budget`):

```rust
pub fn sweep(&self) -> SweepSettings {
    self.inner.read().expect("settings lock").sweep.clone()
}

pub fn save_sweep(&self, new: SweepSettings) -> anyhow::Result<SweepSettings> {
    let mut w = self.inner.write().expect("settings lock");
    w.sweep = new.clone();
    let serialized = serde_json::to_string_pretty(&*w)?;
    write_atomic(&self.path, &serialized)?;
    Ok(new)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --features test-support sweep_defaults_on_when_field_absent save_sweep_round_trips`
Expected: PASS (both).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): add sweep interval + enabled (on by default)"
```

---

### Task 2: Sweeper mtime gate (pure)

**Files:**
- Create: `src-tauri/src/jsonl/sweep.rs`
- Modify: `src-tauri/src/jsonl/mod.rs` (add `pub mod sweep;`)
- Test: `src-tauri/src/jsonl/sweep.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing yet (pure).
- Produces:
  - `pub struct Sweeper` with `pub fn new() -> Sweeper`
  - `pub fn select_changed(&mut self, entries: Vec<(std::path::PathBuf, std::time::SystemTime)>) -> Vec<std::path::PathBuf>` — returns the paths whose mtime is new or changed vs the last call, and records the new mtimes.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn t(secs: u64) -> std::time::SystemTime { UNIX_EPOCH + Duration::from_secs(secs) }

    #[test]
    fn first_call_returns_all_then_unchanged_returns_none() {
        let mut s = Sweeper::new();
        let a = PathBuf::from("a.jsonl");
        let b = PathBuf::from("b.jsonl");
        let entries = vec![(a.clone(), t(100)), (b.clone(), t(100))];
        let first = s.select_changed(entries.clone());
        assert_eq!(first.len(), 2);
        let second = s.select_changed(entries);
        assert!(second.is_empty());
    }

    #[test]
    fn changed_mtime_and_new_path_are_selected() {
        let mut s = Sweeper::new();
        let a = PathBuf::from("a.jsonl");
        s.select_changed(vec![(a.clone(), t(100))]);
        // a's mtime advanced; c is brand new; a unchanged-b is gone.
        let c = PathBuf::from("c.jsonl");
        let out = s.select_changed(vec![(a.clone(), t(200)), (c.clone(), t(50))]);
        assert!(out.contains(&a));
        assert!(out.contains(&c));
        assert_eq!(out.len(), 2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support jsonl::sweep`
Expected: FAIL — module/`Sweeper` does not exist.

- [ ] **Step 3: Write minimal implementation**

Create `src-tauri/src/jsonl/sweep.rs`:

```rust
//! Periodic transcript sweep: re-ingest changed JSONL files when live OTLP is
//! absent. Dedup is handled at the SQL layer by `ingest_one`; this module only
//! avoids re-parsing files whose mtime has not changed since the last tick.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Tracks the last-seen mtime of each transcript so unchanged files are skipped.
/// In-memory only: on restart the map is empty, so the first tick re-ingests
/// everything — safe because `ingest_one` is idempotent.
pub struct Sweeper {
    last_seen: HashMap<PathBuf, SystemTime>,
}

impl Sweeper {
    pub fn new() -> Self {
        Self { last_seen: HashMap::new() }
    }

    /// Return the paths whose mtime is new or newer than last recorded, updating
    /// the record for every path passed in.
    pub fn select_changed(&mut self, entries: Vec<(PathBuf, SystemTime)>) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for (path, mtime) in entries {
            let is_new = match self.last_seen.get(&path) {
                Some(prev) => mtime > *prev,
                None => true,
            };
            if is_new {
                changed.push(path.clone());
            }
            self.last_seen.insert(path, mtime);
        }
        changed
    }
}

impl Default for Sweeper {
    fn default() -> Self { Self::new() }
}
```

Register the module in `src-tauri/src/jsonl/mod.rs` (alongside the other `pub mod` lines):

```rust
pub mod sweep;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --features test-support jsonl::sweep`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/jsonl/sweep.rs src-tauri/src/jsonl/mod.rs
git commit -m "feat(jsonl): add mtime gate for the transcript sweep"
```

---

### Task 3: Sweep execution + loop

**Files:**
- Modify: `src-tauri/src/jsonl/sweep.rs`
- Test: `src-tauri/src/jsonl/sweep.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `crate::jsonl::walker::enumerate`, `crate::jsonl::ingest_one`, `crate::otlp::ingestor::Ingestor`, `crate::otlp::IngestionControl`, `crate::settings::{SettingsStore, SweepSettings}`, `crate::db::DbPool`, `crate::diagnostics::Diagnostics`.
- Produces:
  - `pub async fn run_once(pool: &Arc<DbPool>, ingestor: &Ingestor, claude_home: &Path, sweeper: &mut Sweeper) -> anyhow::Result<usize>` — returns count of files ingested this tick.
  - `pub fn next_delay(cfg: &SweepSettings) -> Duration`
  - `pub async fn run_sweep(pool: Arc<DbPool>, settings: Arc<SettingsStore>, control: IngestionControl, diagnostics: Diagnostics)`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn next_delay_honours_enabled_and_interval() {
    use crate::settings::SweepSettings;
    let off = SweepSettings { interval_minutes: 5, enabled: false };
    assert_eq!(next_delay(&off), std::time::Duration::from_secs(60));
    let on = SweepSettings { interval_minutes: 5, enabled: true };
    assert_eq!(next_delay(&on), std::time::Duration::from_secs(300));
    // interval 0 must not busy-loop: clamp to 1 minute.
    let zero = SweepSettings { interval_minutes: 0, enabled: true };
    assert_eq!(next_delay(&zero), std::time::Duration::from_secs(60));
}

#[tokio::test]
async fn run_once_on_empty_home_ingests_nothing() {
    use std::sync::Arc;
    let tmp = tempfile::tempdir().unwrap();
    // No projects/ dir at all -> enumerate returns empty.
    let pool = Arc::new(crate::db::init(&tmp.path().join("t.db")).unwrap());
    let control = crate::otlp::IngestionControl::new();
    let diag = crate::diagnostics::Diagnostics::new();
    let ing = crate::otlp::ingestor::Ingestor::new(pool.clone(), control, diag);
    let mut sweeper = Sweeper::new();
    let n = run_once(&pool, &ing, tmp.path(), &mut sweeper).await.unwrap();
    assert_eq!(n, 0);
}
```

(If `Diagnostics::new()` takes arguments, match the constructor the scouts use in `budget/monitor.rs` setup; adjust this one line accordingly.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support jsonl::sweep`
Expected: FAIL — `run_once` / `next_delay` not defined.

- [ ] **Step 3: Write minimal implementation**

Append to `src-tauri/src/jsonl/sweep.rs` (add imports at top of file as needed):

```rust
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::db::DbPool;
use crate::diagnostics::Diagnostics;
use crate::otlp::ingestor::Ingestor;
use crate::otlp::IngestionControl;
use crate::settings::{SettingsStore, SweepSettings};

/// One sweep pass: enumerate, gate by mtime, ingest changed files. Per-file
/// failures are logged and skipped, never fatal.
#[tracing::instrument(skip(pool, ingestor, sweeper))]
pub async fn run_once(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    claude_home: &Path,
    sweeper: &mut Sweeper,
) -> anyhow::Result<usize> {
    let paths = crate::jsonl::walker::enumerate(claude_home);
    let entries: Vec<(PathBuf, SystemTime)> = paths
        .into_iter()
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, mtime))
        })
        .collect();
    let changed = sweeper.select_changed(entries);
    let mut ingested = 0usize;
    for path in &changed {
        match crate::jsonl::ingest_one(pool, ingestor, path).await {
            Ok(_) => ingested += 1,
            Err(e) => tracing::warn!(error = ?e, path = %path.display(), "sweep ingest_one failed"),
        }
    }
    Ok(ingested)
}

/// Delay before the next tick. `enabled=false` polls every 60s so a settings
/// toggle-on takes effect within a minute; `interval_minutes` is clamped to >=1
/// to prevent a busy loop.
pub fn next_delay(cfg: &SweepSettings) -> Duration {
    if !cfg.enabled {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(cfg.interval_minutes.max(1) as u64 * 60)
    }
}

/// Background loop. Reads settings fresh each tick so interval/toggle changes
/// take effect without a restart; skips work while ingestion is paused. The
/// first tick fires immediately (startup catch-up).
pub async fn run_sweep(
    pool: Arc<DbPool>,
    settings: Arc<SettingsStore>,
    control: IngestionControl,
    diagnostics: Diagnostics,
) {
    let claude_home = match dirs::home_dir() {
        Some(h) => h.join(".claude"),
        None => {
            tracing::warn!("no home directory; transcript sweep disabled");
            return;
        }
    };
    let ingestor = Ingestor::new(pool.clone(), control.clone(), diagnostics);
    let mut sweeper = Sweeper::new();
    loop {
        let cfg = settings.sweep();
        if cfg.enabled && !control.is_paused() {
            match run_once(&pool, &ingestor, &claude_home, &mut sweeper).await {
                Ok(n) if n > 0 => tracing::info!(files = n, "transcript sweep ingested changed files"),
                Ok(_) => tracing::debug!("transcript sweep: nothing changed"),
                Err(e) => tracing::warn!(error = ?e, "transcript sweep tick failed; will retry"),
            }
        }
        tokio::time::sleep(next_delay(&cfg)).await;
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --features test-support jsonl::sweep`
Expected: PASS (all sweep tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/jsonl/sweep.rs
git commit -m "feat(jsonl): sweep execution loop with pause + interval control"
```

---

### Task 4: Spawn the sweep at startup

**Files:**
- Modify: `src-tauri/src/lib.rs` (setup block, near the OTLP/monitor spawns ~lines 178–220)

**Interfaces:**
- Consumes: `jsonl::sweep::run_sweep`, the existing `pool`, `settings_store`, `control`, `diagnostics` handles already constructed in setup.
- Produces: nothing (startup glue).

- [ ] **Step 1: Add the spawn**

Mirror the OTLP spawn. Place after the budget-monitor spawn:

```rust
// Transcript sweep — auto-ingest JSONL when live OTLP is absent.
let sweep_pool = pool.clone();
let sweep_settings = settings_store.clone();
let sweep_control = control.clone();
let sweep_diag = diagnostics.clone();
tauri::async_runtime::spawn(async move {
    jsonl::sweep::run_sweep(sweep_pool, sweep_settings, sweep_control, sweep_diag).await;
});
```

(If `pool`/`settings_store`/`control`/`diagnostics` were already moved by an earlier spawn, clone them *before* that spawn, matching the existing `let x_for_y = x.clone();` idiom.)

- [ ] **Step 2: Build to verify it compiles**

Run: `cd src-tauri && cargo build`
Expected: builds clean (no move/borrow errors).

- [ ] **Step 3: Verify nothing regressed**

Run: `cd src-tauri && cargo test --features test-support`
Expected: full suite PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(app): spawn transcript sweep task at startup"
```

---

### Task 5: Settings API — `PUT /api/settings/sweep`

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (route registration ~line 64; new handler + payload near `put_budget` ~line 806)
- Test: `src-tauri/src/api/routes.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `ApiState`, `SettingsStore::save_sweep`, `crate::settings::SweepSettings`.
- Produces: `SweepPayload { interval_minutes: u32, enabled: bool }`, handler `put_sweep`, route `PUT /api/settings/sweep`. `GET /api/settings` already returns `sweep` (it serializes the whole `AppSettings`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn sweep_payload_clamps_zero_interval() {
    // A payload of 0 would busy-loop the sweeper; the handler must floor to 1.
    let p = SweepPayload { interval_minutes: 0, enabled: true };
    let s = p.sanitized();
    assert_eq!(s.interval_minutes, 1);
    assert_eq!(s.enabled, true);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support sweep_payload_clamps_zero_interval`
Expected: FAIL — `SweepPayload` / `sanitized` not defined.

- [ ] **Step 3: Write minimal implementation**

Add near `put_budget`:

```rust
#[derive(Deserialize)]
struct SweepPayload {
    interval_minutes: u32,
    enabled: bool,
}

impl SweepPayload {
    fn sanitized(&self) -> crate::settings::SweepSettings {
        crate::settings::SweepSettings {
            interval_minutes: self.interval_minutes.max(1),
            enabled: self.enabled,
        }
    }
}

#[tracing::instrument(skip(state))]
async fn put_sweep(
    State(state): State<ApiState>,
    Json(p): Json<SweepPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let saved = state.settings.save_sweep(p.sanitized())?;
    Ok(Json(serde_json::to_value(saved).unwrap_or_else(|_| json!({}))))
}
```

Register the route next to the other settings routes:

```rust
        .route("/api/settings/sweep", put(put_sweep))
```

(Ensure `put` is in the axum `routing` import list; `put_budget` already uses it.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --features test-support sweep_payload_clamps_zero_interval`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): PUT /api/settings/sweep with interval clamp"
```

---

### Task 6: Frontend settings types + `saveSweep`

**Files:**
- Modify: `web/src/app/core/api.service.ts`
- Test: `web/src/app/core/api.service.spec.ts` (create if absent, else extend)

**Interfaces:**
- Consumes: existing `HttpClient` wrapper, `AppSettings`.
- Produces: `export interface SweepSettings { interval_minutes: number; enabled: boolean }`, `AppSettings.sweep: SweepSettings`, `saveSweep(s: SweepSettings): Observable<SweepSettings>` → `PUT /api/settings/sweep`.

- [ ] **Step 1: Write the failing test**

```typescript
import { TestBed } from '@angular/core/testing';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideHttpClient } from '@angular/common/http';
import { ApiService } from './api.service';

describe('ApiService.saveSweep', () => {
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

  it('PUTs sweep settings to /api/settings/sweep', () => {
    const body = { interval_minutes: 10, enabled: false };
    api.saveSweep(body).subscribe((r) => expect(r.interval_minutes).toBe(10));
    const req = http.expectOne('/api/settings/sweep');
    expect(req.request.method).toBe('PUT');
    expect(req.request.body).toEqual(body);
    req.flush(body);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npm test -- api.service`
Expected: FAIL — `saveSweep` not a function.

- [ ] **Step 3: Write minimal implementation**

Add the interface and extend `AppSettings`:

```typescript
export interface SweepSettings {
  interval_minutes: number;
  enabled: boolean;
}

export interface AppSettings {
  version: number;
  forwarder: ForwarderSettings;
  budget: BudgetSettings;
  sweep: SweepSettings;
}
```

Add the method (mirror `saveBudget`):

```typescript
saveSweep(s: SweepSettings): Observable<SweepSettings> {
  return this.http.put<SweepSettings>('/api/settings/sweep', s);
}
```

- [ ] **Step 4: Run test + type-check**

Run: `cd web && npm test -- api.service`
Expected: PASS.
Run: `cd web && npx tsc -p tsconfig.app.json --noEmit`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/core/api.service.ts web/src/app/core/api.service.spec.ts
git commit -m "feat(web): api types + saveSweep for sweep settings"
```

---

### Task 7: Settings UI — sweep card

**Files:**
- Create: `web/src/app/features/settings/sweep-card.component.ts`
- Modify: `web/src/app/features/settings/settings.component.ts` (import + place `<app-sweep-card>`)
- Test: `web/src/app/features/settings/sweep-card.component.spec.ts`

**Interfaces:**
- Consumes: `ApiService.getSettings()`, `ApiService.saveSweep()`, `SweepSettings`.
- Produces: `<app-sweep-card>` standalone component. Follows `budget-card` (numeric input + save) and `forwarder-card` (checkbox toggle) patterns.

- [ ] **Step 1: Write the failing test**

```typescript
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { SweepCardComponent } from './sweep-card.component';
import { ApiService } from '../../core/api.service';

describe('SweepCardComponent', () => {
  it('saves the current interval and toggle via api.saveSweep', () => {
    const api = {
      getSettings: () => of({ version: 1, forwarder: {}, budget: { monthly_usd: 0 }, sweep: { interval_minutes: 5, enabled: true } }),
      saveSweep: vi.fn(() => of({ interval_minutes: 15, enabled: false })),
    };
    TestBed.configureTestingModule({
      imports: [SweepCardComponent],
      providers: [{ provide: ApiService, useValue: api }],
    });
    const fixture = TestBed.createComponent(SweepCardComponent);
    const cmp = fixture.componentInstance;
    fixture.detectChanges(); // ngOnInit loads settings
    cmp.intervalMinutes.set(15);
    cmp.enabled.set(false);
    cmp.save();
    expect(api.saveSweep).toHaveBeenCalledWith({ interval_minutes: 15, enabled: false });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npm test -- sweep-card`
Expected: FAIL — component does not exist.

- [ ] **Step 3: Write minimal implementation**

Create `web/src/app/features/settings/sweep-card.component.ts` (mirror `budget-card`'s structure and classes):

```typescript
import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-sweep-card',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="rounded border border-border bg-panel p-3">
      <div class="text-xs text-muted mb-2">Transcript sweep</div>
      <p class="text-[11px] text-muted mb-2">
        When live telemetry is not received, ingest Claude Code transcripts from disk automatically.
      </p>
      <label class="flex items-center gap-2 mb-2 text-[12px]">
        <input type="checkbox" [checked]="enabled()" (change)="onToggle($event)" />
        Enabled
      </label>
      <div class="flex items-center gap-2">
        <span class="text-[12px] text-muted">Every</span>
        <input class="w-24 bg-bg border border-border rounded px-2 py-1 text-[12px] font-mono"
               type="number" min="1" max="1440" step="1"
               [value]="intervalMinutes()" (input)="onInterval($event)" [disabled]="!enabled()" />
        <span class="text-[12px] text-muted">minutes</span>
        <button class="filter-chip" [disabled]="!dirty()" (click)="save()">save</button>
        <span class="text-[11px]" [class.text-ok]="ok()" [class.text-warn]="!ok()">{{ msg() }}</span>
      </div>
    </div>
  `,
})
export class SweepCardComponent implements OnInit {
  private api = inject(ApiService);
  intervalMinutes = signal(5);
  enabled = signal(true);
  dirty = signal(false);
  msg = signal('');
  ok = signal(true);

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.intervalMinutes.set(s.sweep?.interval_minutes ?? 5);
      this.enabled.set(s.sweep?.enabled ?? true);
      this.dirty.set(false);
    });
  }
  onToggle(e: Event) { this.enabled.set((e.target as HTMLInputElement).checked); this.dirty.set(true); }
  onInterval(e: Event) { this.intervalMinutes.set(Number((e.target as HTMLInputElement).value)); this.dirty.set(true); }
  save() {
    this.api.saveSweep({ interval_minutes: Number(this.intervalMinutes()), enabled: this.enabled() }).subscribe({
      next: () => { this.msg.set('saved'); this.ok.set(true); this.dirty.set(false); },
      error: (e) => { this.msg.set(`error: ${e?.error?.error ?? 'failed'}`); this.ok.set(false); },
    });
  }
}
```

Register it in `settings.component.ts`: add `SweepCardComponent` to `imports` and place `<app-sweep-card />` next to the budget card in the template.

- [ ] **Step 4: Run test + type-check**

Run: `cd web && npm test -- sweep-card`
Expected: PASS.
Run: `cd web && npx tsc -p tsconfig.app.json --noEmit`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/settings/sweep-card.component.ts web/src/app/features/settings/sweep-card.component.spec.ts web/src/app/features/settings/settings.component.ts
git commit -m "feat(web): settings card for transcript sweep"
```

---

### Task 8 (OPTIONAL): Prominent provenance banner on session detail

Skip unless you want the detail view to state provenance more loudly than the existing amber dot+label already does. The sessions list already shows a per-row source dot and a transcript banner; this only makes the detail view consistent and explicit.

**Files:**
- Modify: `web/src/app/features/sessions/session-detail.component.ts` (template, after the title/breadcrumb ~line 44)
- Test: `web/src/app/features/sessions/session-detail.component.spec.ts`

**Interfaces:**
- Consumes: existing `d.session.cost_source` (`CostSource`).
- Produces: a banner shown only when `cost_source === 'jsonl'`.

- [ ] **Step 1: Write the failing test**

```typescript
it('shows the transcript banner only when cost_source is jsonl', () => {
  // Render with a jsonl session and assert the banner text is present;
  // render with an otlp session and assert it is absent.
  // (Wire the component's session signal to a fake SessionDetail in each case.)
  expect(true).toBe(false); // replace with real DOM assertions per the repo's spec harness
});
```

Replace the placeholder assertion with real DOM checks modeled on the existing `session-detail.component.spec.ts` setup before implementing. Do not commit the `expect(true).toBe(false)` line.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npm test -- session-detail`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Insert into the template after the title block:

```html
@if (d.session.cost_source === 'jsonl') {
  <div class="rounded border border-amber-700/40 bg-amber-950/30 px-3 py-2 text-xs mb-3">
    This session's data was reconstructed from local transcripts — no live telemetry was received.
  </div>
}
```

- [ ] **Step 4: Run test + type-check**

Run: `cd web && npm test -- session-detail`
Expected: PASS.
Run: `cd web && npx tsc -p tsconfig.app.json --noEmit`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/sessions/session-detail.component.ts web/src/app/features/sessions/session-detail.component.spec.ts
git commit -m "feat(web): explicit transcript-provenance banner on session detail"
```

---

## Self-Review

**Spec coverage:**
- Reconciling sweep (startup + periodic, mtime-gated, pause-aware, lenient) → Tasks 2, 3, 4. ✓
- No schema change → confirmed; nothing in the plan touches migrations. ✓
- Provenance banner gated on `source='jsonl'` → already surfaced by existing UI; explicit banner is Task 8 (optional). ✓
- Settings: interval (default 5) + on/off (on by default) → Tasks 1, 5, 6, 7. ✓
- Privacy: reuses `ingest_one`, adds no new field mappings → Tasks 2–3 call `ingest_one` unchanged. ✓
- Out-of-scope items (detector, otlp_absent column, live tail, cause detection) → none appear in any task. ✓

**Placeholder scan:** The only intentional placeholder is Task 8's failing-test stub, explicitly flagged to be replaced and not committed. All other steps carry real code. ✓

**Type consistency:** `SweepSettings { interval_minutes: u32/number, enabled: bool/boolean }` is identical across settings.rs, routes.rs, api.service.ts, and the card. `save_sweep`/`saveSweep`, `run_once`/`run_sweep`/`next_delay`, `Sweeper::select_changed` names match every reference. Route path `/api/settings/sweep` matches between Task 5 and Task 6. ✓

## Assumptions the implementer must confirm in-flight (cheap, non-blocking)

- `Diagnostics::new()` constructor shape (Task 3 test) — match how `budget/monitor` / setup builds it; adjust the one line if it differs.
- The exact clone ordering of `pool`/`settings_store`/`control`/`diagnostics` in `lib.rs` setup (Task 4) — clone before any prior spawn that moves them.
- `put` is already imported from `axum::routing` (Task 5) — `put_budget` implies yes; add it if not.
- The web spec harness (Vitest + TestBed) matches Tasks 6–8's imports — mirror the existing `settings.component.spec.ts`.
