# Test Harness Phase 2 — Full Rust Coverage

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cover the Rust backend end-to-end: OTLP ingestor (per metric handler), every API route, report builders, migrations, repo/git inference, settings round-trips, and integration/diagnostics smoke tests.

**Architecture:** Build on the `tests/common/` fixture from Phase 1. Each module gets its own integration test file under `src-tauri/tests/`. API tests instantiate the axum router with the test pool and assert JSON shape + aggregate math. Ingestor tests feed crafted `ExportMetricsServiceRequest` payloads through the public ingestor entry point and assert on rows. Snapshot DTO shapes with `insta` (low-friction; reviewable diffs).

**Tech Stack:** Phase 1 harness + `insta` for JSON snapshots, `tower::ServiceExt` for invoking the axum router in-process.

**Branch:** `tests/phase-2-rust-coverage` off `main` (rebase on Phase 1 merge if not yet landed).

**Prereq:** Phase 1 merged or available as the parent commit; `tests/common/mod.rs` exists.

---

## File Structure

**Modified:**
- `src-tauri/Cargo.toml` — add `insta = "1"` and `tower = { version = "0.5", features = ["util"] }` (latter for `ServiceExt::oneshot`) under `[dev-dependencies]`.
- `src-tauri/src/api/mod.rs` — expose a `pub fn build_router(pool: Arc<DbPool>, ...) -> axum::Router` constructor that tests can call without going through main. If one already exists, reuse it.

**New tests (each one a file under `src-tauri/tests/`):**
- `migrations.rs` — idempotency + introspected schema shape.
- `ingestor_metrics.rs` — one test per known metric + unknown → metrics_raw + missing-session-id non-crash.
- `ingestor_logs.rs` — log ingestion path.
- `ingestor_transport.rs` — gRPC and HTTP transports produce identical rows from the same payload; 127.0.0.1 bind.
- `api_overview.rs` — overview endpoints (5 routes).
- `api_sessions.rs` — sessions + detail (3 routes).
- `api_files.rs` — files endpoints (2 routes).
- `api_reports.rs` — reports / tape (6 routes).
- `api_settings.rs` — settings (2 routes).
- `api_git.rs` — git / repo (3 routes).
- `api_diagnostics.rs` — diagnostics (3 routes).
- `reports_model.rs` — aggregation math in `reports/`.
- `repo_inference.rs` — git tempdir + inference assertions.
- `settings_roundtrip.rs` — read/write/defaults/corrupt-fallback.
- `integration_smoke.rs` — empty-DB no-panic.
- `diagnostics_smoke.rs` — empty-DB no-panic.

---

## Open Questions to Resolve During Execution

- **Mocking strategy:** spec recommends hand-rolled fakes over `mockall`. Stick with that. Only fake what's truly external (the OS clock, the filesystem when settings touch it). Most "fakes" are just real DB rows seeded via `seed_session`.
- **Snapshot scope:** use `insta` for DTO JSON shape on every endpoint. Don't snapshot numeric aggregate values — assert those with `assert_eq!` so a wrong number fails loudly. Use snapshots for "is the field set still correct?"
- **`current_month_bounds` and "now":** several tests will straddle clock boundaries (today/week/month). Either inject a clock (clean) or assert windows in relative terms (good enough). Prefer the latter unless a test gets flaky.

---

## Task Sequencing Notes

Group related route tests (overview, sessions, files, reports, settings, git, diagnostics) into separate files so they can land as separate commits and parallelize cleanly under subagent-driven execution.

Each test file follows this template:

```rust
mod common;

use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use andon_lib::api::build_router; // adapt name to actual export

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    };
    (status, v)
}
```

If `build_router` requires more args (ingestion control, diagnostics, settings store), Task 1 below builds a `test_router(pool)` helper in `tests/common/mod.rs` that defaults them.

---

## Task 1: Branch, deps, and test router helper

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tests/common/mod.rs`

- [ ] **Step 1: Branch**

```bash
git checkout main && git pull
git checkout -b tests/phase-2-rust-coverage
```

- [ ] **Step 2: Add dev-deps**

In `src-tauri/Cargo.toml` under `[dev-dependencies]`:

```toml
insta = { version = "1", features = ["json"] }
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

- [ ] **Step 3: Add `test_router(pool)` and `test_ingestor(pool)` to `common/mod.rs`**

Append to `src-tauri/tests/common/mod.rs`:

```rust
use andon_lib::otlp::{IngestionControl, Ingestor};
use andon_lib::diagnostics::Diagnostics;

pub fn test_ingestor(pool: &Arc<DbPool>) -> Ingestor {
    Ingestor::new(Arc::clone(pool), IngestionControl::default(), Diagnostics::default())
}

pub fn test_router(pool: &Arc<DbPool>) -> axum::Router {
    // Wire up whatever the real build_router signature requires.
    andon_lib::api::build_router(Arc::clone(pool), IngestionControl::default(), Diagnostics::default())
}
```

Adapt to actual signatures. If anything isn't `Default`, build a minimal version inline.

- [ ] **Step 4: Smoke check**

```bash
cd src-tauri && cargo test --test _harness_smoke
```

Should still pass. Commit:

```bash
git add src-tauri/
git commit -m "test: add test_router + test_ingestor helpers and insta dev-dep"
```

---

## Task 2: Migrations idempotency + schema shape

**Files:** Create `src-tauri/tests/migrations.rs`

- [ ] **Step 1: Test**

```rust
mod common;

#[test]
fn migrations_are_idempotent() {
    let (pool, _g) = common::fixture_pool();
    // fixture_pool already runs migrations once; running again must not error.
    andon_lib::db::run_migrations(&pool).expect("second migration run");
}

#[test]
fn schema_matches_documented_tables() {
    let (pool, _g) = common::fixture_pool();
    let conn = pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap();
    let names: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
    for expected in [
        "active_time", "cost_entries", "file_changes", "git_activity",
        "metrics_raw", "sessions", "token_usage", "tool_decisions",
    ] {
        assert!(names.iter().any(|n| n == expected), "missing table {expected}; have {names:?}");
    }
}
```

- [ ] **Step 2: Run + commit**

```bash
cd src-tauri && cargo test --test migrations
git add . && git commit -m "test: migrations idempotency and schema shape"
```

---

## Task 3: Ingestor — one test per known metric

**Files:** Create `src-tauri/tests/ingestor_metrics.rs`

- [ ] **Step 1: Write tests per metric**

One `#[test]` per metric name from CLAUDE.md's "Metrics to capture" table:

- `claude_code.session.count` → row in `sessions`
- `claude_code.lines_of_code.count` → row in `file_changes`
- `claude_code.pull_request.count` → row in `git_activity` with activity='pull_request'
- `claude_code.commit.count` → `git_activity` with activity='commit'
- `claude_code.cost.usage` → `cost_entries`
- `claude_code.token.usage` → `token_usage` with correct `token_type`
- `claude_code.code_edit_tool.decision` → `tool_decisions`
- `claude_code.active_time.total` → `active_time`

Template per metric:

```rust
mod common;

#[test]
fn cost_usage_metric_inserts_cost_entry() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "s1")],
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-opus-4-5-20251001")],
        2.50,
    );
    ingestor.ingest_metrics_v2(payload, "test").expect("ingest");

    let conn = pool.get().unwrap();
    let (model, cost): (String, f64) = conn
        .query_row("SELECT model, cost_usd FROM cost_entries WHERE session_id = 's1'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(model, "claude-opus-4-5-20251001");
    assert!((cost - 2.50).abs() < 1e-9);
}
```

Repeat for each metric, varying point attributes (`token_type`, `language`, `decision`, etc.) to match what the handler reads.

- [ ] **Step 2: Unknown-metric → metrics_raw**

```rust
#[test]
fn unknown_metric_lands_in_metrics_raw() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);
    let payload = common::sample_sum_metric(
        vec![common::kv("session.id", "s1")],
        "claude_code.something.new",
        vec![],
        1.0,
    );
    ingestor.ingest_metrics_v2(payload, "test").unwrap();
    let count: i64 = pool.get().unwrap()
        .query_row("SELECT COUNT(*) FROM metrics_raw WHERE metric_name = 'claude_code.something.new'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 3: Missing-session-id does not panic**

```rust
#[test]
fn missing_session_id_does_not_panic() {
    let (pool, _g) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);
    let payload = common::sample_sum_metric(
        vec![],                  // no resource attrs at all
        "claude_code.cost.usage",
        vec![common::kv("model", "claude-opus-4-5-20251001")],
        1.0,
    );
    // Per CLAUDE.md: "If session.id is missing on a metric, store it but flag it"
    ingestor.ingest_metrics_v2(payload, "test").expect("must not error");
}
```

- [ ] **Step 4: Run + commit**

```bash
cd src-tauri && cargo test --test ingestor_metrics
git add . && git commit -m "test(ingestor): cover every known metric handler + unknown fallback"
```

---

## Task 4: Ingestor — logs

**Files:** Create `src-tauri/tests/ingestor_logs.rs`

- [ ] **Step 1: Add `sample_export_logs(...)` to `common/mod.rs`** with the same shape as `sample_sum_metric` but returning `Vec<ResourceLogs>`. Read the existing `ingestor.rs::ingest_logs_v2` to learn what attributes it cares about and craft a minimum payload that exercises each branch.

- [ ] **Step 2: Test that a sample log payload ingests without error and lands in the documented destination tables** (whichever the implementation writes to — read `ingest_logs_v2` for the list).

- [ ] **Step 3: Test the paused-control path** — set `IngestionControl::pause()`, send a payload, assert zero rows written.

- [ ] **Step 4: Commit**

```bash
git add . && git commit -m "test(ingestor): cover log ingestion path and pause flag"
```

---

## Task 5: Transport equivalence (gRPC vs HTTP)

**Files:** Create `src-tauri/tests/ingestor_transport.rs`

- [ ] **Step 1: Smoke each transport against a sample payload, assert identical row counts**

Run the gRPC service handler directly (it's a tonic trait impl — call it like a normal method) and the HTTP handler directly (axum handler is just a function — call it with a synthesized `Request<Bytes>`). Both go through the same `Ingestor::ingest_metrics_v2`.

- [ ] **Step 2: Verify `127.0.0.1` bind** — read the bind code; if it lives in a pure function, call it; otherwise assert the constant string in source via `include_str!` test. The point is to fail loudly if someone changes it to `0.0.0.0`.

- [ ] **Step 3: Verify both return `Ok` even when the ingestor errors** — inject a poisoned pool (or just use a closed pool) and confirm both transport handlers still return successful OTLP responses while logging the error. May require a thin trait abstraction on the ingestor. If introducing it costs more than the test buys, skip and document in the commit.

- [ ] **Step 4: Commit**

```bash
git add . && git commit -m "test(otlp): gRPC and HTTP transports share ingestor, never propagate errors"
```

---

## Task 6: API — overview endpoints (5 routes)

**Files:** Create `src-tauri/tests/api_overview.rs`

- [ ] **Step 1: For each of these, seed a known DB, hit the route, assert shape + aggregate math:**

- `GET /api/overview/today` — seed 2 sessions today + 1 yesterday, assert today's cost/sessions/accept rate.
- `GET /api/overview/cost-by-day` — seed 5 days of cost with 2 models, assert grouping.
- `GET /api/overview/tokens-by-day` — seed input + output + cacheRead, assert split.
- `GET /api/overview/accept-by-language` — seed accepts/rejects/aborts across 3 languages, assert ratio math (rounded to 4 decimals).
- `GET /api/overview/active-time/today` — seed user + cli rows, assert sum split.

Pattern:

```rust
mod common;
use insta::assert_json_snapshot;
// ... shared `get_json` helper (see Task Sequencing template) ...

#[tokio::test]
async fn overview_today_aggregates_only_today_rows() {
    let (pool, _g) = common::fixture_pool();
    common::seed_session(&pool, &common::SeedOpts {
        session_id: "today-1".into(),
        cost_usd: 1.0,
        decisions: vec![("accept", "rust"), ("reject", "rust")],
        ..Default::default()
    });
    let yesterday_ms = chrono::Utc::now().timestamp_millis() - 86_400_000;
    common::seed_session(&pool, &common::SeedOpts {
        session_id: "yest-1".into(),
        started_at_ms: Some(yesterday_ms),
        cost_usd: 9.99,
        ..Default::default()
    });

    let (status, body) = get_json(common::test_router(&pool), "/api/overview/today").await;
    assert_eq!(status, 200);
    assert_eq!(body["cost_usd"].as_f64().unwrap(), 1.0);
    assert_eq!(body["sessions"].as_i64().unwrap(), 1);
    assert!((body["accept_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);

    // Shape snapshot — protects field names.
    assert_json_snapshot!("overview_today_shape", body, { ".cost_usd" => "[f]", ".sessions" => "[i]", ".accept_rate" => "[f]" });
}
```

- [ ] **Step 2: Run + commit**

```bash
cd src-tauri && cargo test --test api_overview
git add . && git commit -m "test(api): overview endpoints aggregate math + shape snapshots"
```

---

## Task 7: API — sessions (3 routes)

**Files:** Create `src-tauri/tests/api_sessions.rs`

- [ ] Cover: list with `?from=&to=&limit=`, detail by id, v2 sessions filter path. Assert pagination bounds, sort order, 404 for missing id.
- [ ] Run + commit `test(api): session list and detail`.

---

## Task 8: API — files (2 routes)

**Files:** Create `src-tauri/tests/api_files.rs`

- [ ] Cover: `/api/files/heatmap?days=30` and `/api/v2/files`. Seed `file_changes` + `tool_decisions` so accept-rate computation is exercised.
- [ ] Run + commit `test(api): files heatmap and v2 files`.

---

## Task 9: API — reports / tape (6 routes)

**Files:** Create `src-tauri/tests/api_reports.rs`

- [ ] Cover: `/api/v2/tape`, `/api/v2/kpis`, `/api/sessions/:id/report` GET + POST, `/api/sessions/:id/report/open`, `/api/sessions/reports/index`. For the "open" endpoint, mock or skip the actual file-open shell call; assert request validation only.
- [ ] Run + commit `test(api): reports and tape endpoints`.

---

## Task 10: API — settings, git, diagnostics

**Files:** Create three files: `api_settings.rs`, `api_git.rs`, `api_diagnostics.rs`.

- [ ] **settings (2):** `GET /api/settings`, `PUT /api/settings/forwarder` (round-trip).
- [ ] **git (3):** `/api/repo/backfill`, `/api/repos`, `/api/overview/top-repos`. Use a tempdir git repo (see Task 13).
- [ ] **diagnostics (3):** `/api/diagnostics`, `/api/diagnostics/events`, `/api/diagnostics/export`. Assert non-empty after seeding payloads through the ingestor.
- [ ] Run + commit each as its own commit.

---

## Task 11: Report builders — math

**Files:** Create `src-tauri/tests/reports_model.rs`

- [ ] Identify report builders in `src-tauri/src/reports/`. Each pure aggregator gets a unit test that feeds fixed seed data and asserts the computed output.
- [ ] If aggregators take a `&Connection`, use the test pool's connection. If they're pure on input data structs, test directly.
- [ ] Commit `test(reports): aggregation math against fixed seed`.

---

## Task 12: Repo inference + git_query

**Files:** Create `src-tauri/tests/repo_inference.rs`

- [ ] **Step 1: Build a tempdir git repo helper** in `common/mod.rs`:

```rust
pub fn init_temp_repo(commits: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args).current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "T").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "T").env("GIT_COMMITTER_EMAIL", "t@t")
            .status().unwrap();
        assert!(status.success(), "git {:?} failed", args);
    };
    run(&["init", "-q"]);
    for (i, msg) in commits.iter().enumerate() {
        std::fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", msg]);
    }
    dir
}
```

- [ ] **Step 2: Assert inference picks the right repo for a path inside the worktree**, and that `git_query` returns the right number of commits with the expected messages.

- [ ] **Step 3: Commit** `test(git): inference + recent commits against tempdir repo`.

---

## Task 13: Settings + autostart + config round-trip

**Files:** Create `src-tauri/tests/settings_roundtrip.rs`

- [ ] Round-trip `settings.rs`: write → read → values match.
- [ ] Missing-file path → returns defaults.
- [ ] Corrupt-file path (write garbage bytes) → returns defaults, doesn't panic.
- [ ] `autostart.rs`: test pure-Rust pieces only (path computation, registry key string content, plist content). Don't touch real OS state.
- [ ] `config.rs`: same pattern.
- [ ] Commit `test(settings,autostart,config): round-trip and corrupt-file fallback`.

---

## Task 14: Integration + diagnostics smoke

**Files:** Create `src-tauri/tests/integration_smoke.rs` and `src-tauri/tests/diagnostics_smoke.rs`

- [ ] One test each: build the relevant struct against an empty fixture pool, call its main entry point, assert it returns successfully and produces empty-but-well-shaped output.
- [ ] Commit `test: integration and diagnostics empty-DB smoke`.

---

## Task 15: PR + CI

- [ ] **Step 1: Push and open PR**

```bash
git push -u origin tests/phase-2-rust-coverage
gh pr create --title "Phase 2: full Rust test coverage" --body "Covers ingestor handlers, all API routes, report builders, repo/git, settings, integration, diagnostics. ~50 new test fns across ~15 files."
```

- [ ] **Step 2: Watch CI matrix (ubuntu/macos/windows). Fix until green.**

Common pitfalls on Windows: `git` may not be on PATH in CI defaults — `actions/setup-git` if needed. Line ending differences in snapshot files — configure `.gitattributes` for `*.snap` if they churn.

- [ ] **Step 3: Mark ready, request review.**

---

## Done When

- Every metric handler in `ingestor.rs` has at least one test asserting the row it produces.
- Every route in `api/routes.rs` has at least one test asserting status, shape (insta snapshot), and at least one numeric value.
- `cargo test --workspace` is green on all three OS in CI.
- Snapshot files committed under `src-tauri/tests/snapshots/`.
