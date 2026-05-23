# Cost-Efficiency Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a new filterable **Efficiency** page that surfaces prompt-cache savings (issue #19) and per-model-family cost-efficiency (issue #20).

**Architecture:** Pure aggregation logic lives in a new, unit-tested `src-tauri/src/api/efficiency.rs` module. Two thin axum handlers in `routes.rs` run SQL and delegate to it, returning `serde` DTOs. A new standalone Angular `EfficiencyComponent` renders a 3-tile KPI strip plus a model-efficiency table, refetching on filter change. No schema change — every figure derives from existing `token_usage` / `cost_entries` rows.

**Tech Stack:** Rust (axum, rusqlite), Angular 21 (standalone components, signals), Tailwind, Vitest, `cargo test`.

**Reference spec:** `docs/superpowers/specs/2026-05-22-cost-efficiency-page-design.md`

---

## File Structure

**Rust (`src-tauri/`)**
- `src/api/efficiency.rs` — **new.** Pure, DB-free logic: `model_family`, `hit_ratio`, `cache_savings`, `aggregate_model_efficiency`, with unit tests. One responsibility: cost-efficiency math.
- `src/api/mod.rs` — **modify.** Declare `pub mod efficiency;`.
- `src/api/dto.rs` — **modify.** Add four response DTOs.
- `src/api/routes.rs` — **modify.** Two handlers + one query helper + two route registrations.
- `tests/common/mod.rs` — **modify.** Extend `SeedOpts` with cache-token fields.
- `tests/api_reports.rs` — **modify.** Two integration tests.

**Angular (`web/`)**
- `src/app/core/api.service.ts` — **modify.** Two DTO interfaces + two methods.
- `src/app/features/efficiency/efficiency.component.ts` — **new.**
- `src/app/features/efficiency/efficiency.component.html` — **new.**
- `src/app/features/efficiency/efficiency.component.spec.ts` — **new.**
- `src/app/app.routes.ts` — **modify.** Add `/efficiency` route.
- `src/app/app.component.html` — **modify.** Add nav item.

**Docs**
- `docs/features.md` — **modify.** Document the page.

**Conventions to follow:** All Rust commands run from `src-tauri/`. All Angular commands run from `web/`. No `unwrap`/`expect` in `src/` (test code may use them — the existing `tests/common/mod.rs` does). Conventional Commits, no emojis.

---

### Task 1: `model_family` classifier + new module

**Files:**
- Create: `src-tauri/src/api/efficiency.rs`
- Modify: `src-tauri/src/api/mod.rs:1`

- [ ] **Step 1: Create `efficiency.rs` with the failing test**

Create `src-tauri/src/api/efficiency.rs` with exactly this content:

```rust
//! Cost-efficiency math for the Efficiency page (cache savings + per-model
//! cost-efficiency). Pure and DB-free so it can be unit-tested in isolation.

/// Classify a full model id (e.g. `claude-opus-4-7-20260101`) into a coarse
/// family. Case-insensitive substring match; anything unrecognized is `other`.
pub fn model_family(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m.contains("opus") {
        "opus"
    } else if m.contains("sonnet") {
        "sonnet"
    } else if m.contains("haiku") {
        "haiku"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_family_classifies_known_families() {
        assert_eq!(model_family("claude-opus-4-7"), "opus");
        assert_eq!(model_family("claude-opus-4-7-20260101"), "opus");
        assert_eq!(model_family("claude-sonnet-4-6"), "sonnet");
        assert_eq!(model_family("claude-haiku-4-5"), "haiku");
    }

    #[test]
    fn model_family_is_case_insensitive() {
        assert_eq!(model_family("Claude-OPUS-X"), "opus");
    }

    #[test]
    fn model_family_unknown_is_other() {
        assert_eq!(model_family("gpt-4"), "other");
    }
}
```

Then add the module declaration to `src-tauri/src/api/mod.rs` — change line 1's block from:

```rust
pub mod dto;
pub mod filter;
```

to:

```rust
pub mod dto;
pub mod efficiency;
pub mod filter;
```

- [ ] **Step 2: Run the tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support model_family`
Expected: 3 tests pass (`model_family_classifies_known_families`, `model_family_is_case_insensitive`, `model_family_unknown_is_other`).

(There is no separate "fails" step here — the test and implementation are written together because a Rust module that declares a `#[cfg(test)]` test for a missing function will not compile. If you prefer a strict red phase, comment out the `model_family` fn body's branches, run, see the assertion fail, then restore.)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/api/efficiency.rs src-tauri/src/api/mod.rs
git commit -m "feat(api): add model_family classifier for the efficiency module"
```

---

### Task 2: Cache hit-ratio and savings math

**Files:**
- Modify: `src-tauri/src/api/efficiency.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/api/efficiency.rs`, add these tests inside the existing `mod tests` block (after `model_family_unknown_is_other`):

```rust
    #[test]
    fn hit_ratio_is_cache_read_over_prompt_tokens() {
        // 300 cache-read of (100 input + 100 create + 300 read) = 0.6
        assert!((hit_ratio(100, 100, 300) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn hit_ratio_zero_when_no_tokens() {
        assert_eq!(hit_ratio(0, 0, 0), 0.0);
    }

    #[test]
    fn cache_savings_opus_nets_gross_minus_overhead() {
        // opus: input 15, cache_read 1.50, cache_create 18.75 ($/Mtok).
        // 1M read  -> gross    = 1M * (15 - 1.50)  / 1e6 = 13.50
        // 1M create-> overhead = 1M * (18.75 - 15) / 1e6 = 3.75
        let s = cache_savings(
            [("claude-opus-4-7", 1_000_000i64, 1_000_000i64)].into_iter(),
        );
        assert!((s.gross - 13.50).abs() < 1e-9, "gross {}", s.gross);
        assert!(
            (s.creation_overhead - 3.75).abs() < 1e-9,
            "overhead {}",
            s.creation_overhead
        );
        assert!((s.net - 9.75).abs() < 1e-9, "net {}", s.net);
        assert_eq!(s.unpriced_cache_tokens, 0);
    }

    #[test]
    fn cache_savings_counts_unpriced_models() {
        let s = cache_savings([("mystery-model", 500i64, 500i64)].into_iter());
        assert_eq!(s.gross, 0.0);
        assert_eq!(s.net, 0.0);
        assert_eq!(s.unpriced_cache_tokens, 1000);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: FAIL — compile error `cannot find function hit_ratio` / `cannot find function cache_savings`.

- [ ] **Step 3: Write the implementation**

In `src-tauri/src/api/efficiency.rs`, add this **above** the `#[cfg(test)]` line:

```rust
use crate::jsonl::pricing;

/// Share of *prompt* tokens served from cache. Output is excluded — it is not
/// part of the prompt. Returns `0.0` when there are no prompt tokens.
pub fn hit_ratio(input: i64, cache_create: i64, cache_read: i64) -> f64 {
    let denom = input + cache_create + cache_read;
    if denom <= 0 {
        0.0
    } else {
        cache_read as f64 / denom as f64
    }
}

/// Prompt-cache savings, in USD.
#[derive(Debug, Clone, Copy)]
pub struct Savings {
    /// Discount won on cache reads vs. paying the input rate for them.
    pub gross: f64,
    /// Premium paid to write the cache vs. the input rate.
    pub creation_overhead: f64,
    /// `gross - creation_overhead` — the true saving.
    pub net: f64,
    /// cache-read + cache-create tokens on models absent from the price table.
    pub unpriced_cache_tokens: i64,
}

/// Compute cache savings from per-model `(model, cache_read, cache_create)`
/// token counts. Models not in the pricing table contribute nothing to the
/// dollar figures; their tokens are tallied into `unpriced_cache_tokens`.
pub fn cache_savings<'a>(rows: impl Iterator<Item = (&'a str, i64, i64)>) -> Savings {
    let mut gross = 0.0;
    let mut creation_overhead = 0.0;
    let mut unpriced = 0i64;
    for (model, cache_read, cache_create) in rows {
        match pricing::lookup(model) {
            Some(p) => {
                gross += cache_read as f64 / 1e6 * (p.input_per_mtok - p.cache_read_per_mtok);
                creation_overhead +=
                    cache_create as f64 / 1e6 * (p.cache_create_per_mtok - p.input_per_mtok);
            }
            None => unpriced += cache_read + cache_create,
        }
    }
    Savings {
        gross,
        creation_overhead,
        net: gross - creation_overhead,
        unpriced_cache_tokens: unpriced,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: PASS — all efficiency tests green (now 7 total).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/efficiency.rs
git commit -m "feat(api): add cache hit-ratio and savings math"
```

---

### Task 3: Response DTOs

**Files:**
- Modify: `src-tauri/src/api/dto.rs` (append at end of file)

- [ ] **Step 1: Add the DTOs**

Append to `src-tauri/src/api/dto.rs`:

```rust
// ---- Efficiency page DTOs ----

#[derive(Debug, serde::Serialize)]
pub struct CacheTokenTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_create: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheSavings {
    pub net: f64,
    pub gross: f64,
    pub creation_overhead: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheEfficiency {
    pub hit_ratio: f64,
    pub hit_ratio_prev: f64,
    pub tokens: CacheTokenTotals,
    pub savings: CacheSavings,
    pub net_prev: f64,
    pub unpriced_cache_tokens: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelEfficiencyRow {
    /// `opus` | `sonnet` | `haiku` | `other`.
    pub family: String,
    pub sessions: i64,
    pub total_cost_usd: f64,
    pub cost_per_session: f64,
    pub output_tokens: i64,
    pub cost_per_1k_output: f64,
}
```

- [ ] **Step 2: Verify it compiles**

Run (from `src-tauri/`): `cargo build`
Expected: builds cleanly (warnings about unused structs are acceptable — they are consumed in later tasks).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/api/dto.rs
git commit -m "feat(api): add cost-efficiency response DTOs"
```

---

### Task 4: Per-family model cost-efficiency aggregation

**Files:**
- Modify: `src-tauri/src/api/efficiency.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/api/efficiency.rs`, add these tests inside `mod tests` (after `cache_savings_counts_unpriced_models`):

```rust
    #[test]
    fn aggregate_buckets_session_by_dominant_family() {
        // s1: opus 5.0 + haiku 1.0 -> dominant opus, whole session (6.0) -> opus
        // s2: haiku 2.0           -> dominant haiku
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(), 5.0),
            ("s1".to_string(), "claude-haiku-4-5".to_string(), 1.0),
            ("s2".to_string(), "claude-haiku-4-5".to_string(), 2.0),
        ];
        let output_rows = vec![("s1".to_string(), 1000i64), ("s2".to_string(), 500i64)];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);

        assert_eq!(rows.len(), 2);
        // sorted by total cost desc -> opus first
        assert_eq!(rows[0].family, "opus");
        assert_eq!(rows[0].sessions, 1);
        assert!((rows[0].total_cost_usd - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_session - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_1k_output - 6.0).abs() < 1e-9); // 6.0/1000*1000
        assert_eq!(rows[1].family, "haiku");
        assert!((rows[1].cost_per_1k_output - 4.0).abs() < 1e-9); // 2.0/500*1000
    }

    #[test]
    fn aggregate_breaks_ties_toward_opus() {
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(), 2.0),
            ("s1".to_string(), "claude-sonnet-4-6".to_string(), 2.0),
        ];
        let rows = aggregate_model_efficiency(&cost_rows, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].family, "opus");
    }

    #[test]
    fn aggregate_handles_zero_output() {
        let cost_rows = vec![("s1".to_string(), "claude-opus-4-7".to_string(), 3.0)];
        let rows = aggregate_model_efficiency(&cost_rows, &[]);
        assert_eq!(rows[0].cost_per_1k_output, 0.0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: FAIL — compile error `cannot find function aggregate_model_efficiency`.

- [ ] **Step 3: Write the implementation**

In `src-tauri/src/api/efficiency.rs`, change the top `use` line from:

```rust
use crate::jsonl::pricing;
```

to:

```rust
use std::collections::HashMap;

use crate::api::dto::ModelEfficiencyRow;
use crate::jsonl::pricing;
```

Then add this **above** the `#[cfg(test)]` line:

```rust
/// Round a USD figure to 4 decimal places — matches the `round4` used by the
/// v2 API handlers. Kept local so this module stays free of `routes.rs`.
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// The family that spent the most in a session. Ties are broken toward the
/// fixed order opus > sonnet > haiku > other (strict `>` keeps the first).
fn dominant_family(costs: &HashMap<&'static str, f64>) -> &'static str {
    const ORDER: [&str; 4] = ["opus", "sonnet", "haiku", "other"];
    let mut best: &'static str = "other";
    let mut best_cost = f64::NEG_INFINITY;
    for fam in ORDER {
        if let Some(&c) = costs.get(fam) {
            if c > best_cost {
                best_cost = c;
                best = fam;
            }
        }
    }
    best
}

/// Aggregate per-session cost/output into per-family rows, attributing each
/// session wholly to its dominant family. `cost_rows` are
/// `(session_id, model, cost_usd)`; `output_rows` are `(session_id, output)`.
/// Rows are sorted by total cost descending.
pub fn aggregate_model_efficiency(
    cost_rows: &[(String, String, f64)],
    output_rows: &[(String, i64)],
) -> Vec<ModelEfficiencyRow> {
    // session_id -> family -> cost
    let mut per_session: HashMap<&str, HashMap<&'static str, f64>> = HashMap::new();
    for (sid, model, cost) in cost_rows {
        *per_session
            .entry(sid.as_str())
            .or_default()
            .entry(model_family(model))
            .or_insert(0.0) += *cost;
    }
    // session_id -> output tokens
    let mut output: HashMap<&str, i64> = HashMap::new();
    for (sid, toks) in output_rows {
        *output.entry(sid.as_str()).or_insert(0) += *toks;
    }
    // family -> (sessions, total_cost, output_tokens)
    let mut buckets: HashMap<&'static str, (i64, f64, i64)> = HashMap::new();
    for (sid, fam_costs) in &per_session {
        let fam = dominant_family(fam_costs);
        let total_cost: f64 = fam_costs.values().sum();
        let out = output.get(sid).copied().unwrap_or(0);
        let entry = buckets.entry(fam).or_insert((0, 0.0, 0));
        entry.0 += 1;
        entry.1 += total_cost;
        entry.2 += out;
    }
    let mut rows: Vec<ModelEfficiencyRow> = buckets
        .into_iter()
        .map(|(family, (sessions, total_cost, output_tokens))| ModelEfficiencyRow {
            family: family.to_string(),
            sessions,
            total_cost_usd: round4(total_cost),
            cost_per_session: round4(total_cost / sessions as f64),
            output_tokens,
            cost_per_1k_output: if output_tokens > 0 {
                round4(total_cost / output_tokens as f64 * 1000.0)
            } else {
                0.0
            },
        })
        .collect();
    rows.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: PASS — all 10 efficiency tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/efficiency.rs
git commit -m "feat(api): aggregate per-family model cost-efficiency"
```

---

### Task 5: Seed cache tokens in the test helper

**Files:**
- Modify: `src-tauri/tests/common/mod.rs:29-44` (the `SeedOpts` struct) and `:66-73` (the output-token block in `seed_session`)

- [ ] **Step 1: Add cache fields to `SeedOpts`**

In `src-tauri/tests/common/mod.rs`, in the `SeedOpts` struct, add two fields after `pub output_tokens: i64,`:

```rust
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
```

- [ ] **Step 2: Insert cache rows in `seed_session`**

In the same file, in `seed_session`, immediately after the `if opts.output_tokens > 0 { ... }` block, add:

```rust
    if opts.cache_read_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES (?, ?, ?, 'cacheRead', ?)",
            params![opts.session_id, started, opts.model, opts.cache_read_tokens],
        )
        .expect("insert cacheRead token_usage");
    }
    if opts.cache_create_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES (?, ?, ?, 'cacheCreation', ?)",
            params![opts.session_id, started, opts.model, opts.cache_create_tokens],
        )
        .expect("insert cacheCreation token_usage");
    }
```

- [ ] **Step 3: Verify existing tests still pass**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports`
Expected: PASS — all existing `api_reports` tests still green (the new `SeedOpts` fields default to `0` via `#[derive(Default)]`, so no caller is affected).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/common/mod.rs
git commit -m "test(api): seed cache tokens in the test helper"
```

---

### Task 6: `GET /api/v2/cache-efficiency` endpoint

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (route registration near `:32`; handler + helper after the `v2_cost_by_model` fn, which ends near `:1572`)
- Test: `src-tauri/tests/api_reports.rs` (append)

- [ ] **Step 1: Write the failing integration test**

Append to `src-tauri/tests/api_reports.rs`:

```rust
// ---------------------------------------------------------------------------
// 9. v2_cache_efficiency_returns_expected_shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_cache_efficiency_returns_expected_shape() {
    let (pool, _db_dir) = common::fixture_pool();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "cache-1".into(),
            started_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            model: "claude-opus-4-7".into(),
            input_tokens: 1_000_000,
            output_tokens: 200_000,
            cache_read_tokens: 1_000_000,
            cache_create_tokens: 1_000_000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/cache-efficiency").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["tokens"].is_object(), "tokens key missing");
    assert!(body["savings"].is_object(), "savings key missing");

    // hit_ratio = cacheRead / (input + cacheCreation + cacheRead)
    //           = 1e6 / (1e6 + 1e6 + 1e6) = 0.3333…
    let hr = body["hit_ratio"].as_f64().unwrap();
    assert!((hr - 0.3333).abs() < 0.01, "hit_ratio was {hr}");

    // opus: net = gross 13.50 - creation overhead 3.75 = 9.75
    let net = body["savings"]["net"].as_f64().unwrap();
    assert!((net - 9.75).abs() < 1e-6, "net was {net}");

    assert_eq!(body["tokens"]["cache_read"].as_i64().unwrap(), 1_000_000);
    assert_eq!(body["unpriced_cache_tokens"].as_i64().unwrap(), 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_cache_efficiency`
Expected: FAIL — the route does not exist, so the response is a 404 and `status` is not `OK`.

- [ ] **Step 3: Register the route**

In `src-tauri/src/api/routes.rs`, in the `router` fn, add a line directly after `.route("/api/v2/cost-by-model", get(v2_cost_by_model))`:

```rust
        .route("/api/v2/cost-by-model", get(v2_cost_by_model))
        .route("/api/v2/cache-efficiency", get(v2_cache_efficiency))
```

- [ ] **Step 4: Write the handler and query helper**

In `src-tauri/src/api/routes.rs`, immediately after the `v2_cost_by_model` function (it ends with the line `    Ok(Json(out))\n}` near line 1572), add:

```rust
/// Per-model cache savings over `[from, to)` with the model filter applied.
/// One grouped query; the per-model dollar math lives in `efficiency`.
fn cache_savings_for_window(
    conn: &rusqlite::Connection,
    from: i64,
    to: i64,
    q: &FilterQuery,
) -> crate::api::efficiency::Savings {
    let (m_sql, m_vals) = q.model_clause("model");
    let sql = format!(
        "SELECT model,
                COALESCE(SUM(CASE WHEN token_type='cacheRead'     THEN count END), 0),
                COALESCE(SUM(CASE WHEN token_type='cacheCreation' THEN count END), 0)
         FROM token_usage
         WHERE timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY model"
    );
    let mut p: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in m_vals {
        p.push(Box::new(v));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = p.iter().map(|b| &**b).collect();
    let mut rows: Vec<(String, i64, i64)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(mapped) = stmt.query_map(refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        }) {
            rows = mapped.flatten().collect();
        }
    }
    crate::api::efficiency::cache_savings(rows.iter().map(|(m, cr, cc)| (m.as_str(), *cr, *cc)))
}

async fn v2_cache_efficiency(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<CacheEfficiency>, ApiError> {
    let (from, to) = q.window();
    let (prev_from, prev_to) = prev_period_window(from, to);
    let conn = state.pool.get().map_err(ApiError::pool)?;

    let input = sum_tokens(&conn, from, to, "input", &q);
    let output = sum_tokens(&conn, from, to, "output", &q);
    let cache_read = sum_tokens(&conn, from, to, "cacheRead", &q);
    let cache_create = sum_tokens(&conn, from, to, "cacheCreation", &q);

    let hit_ratio = crate::api::efficiency::hit_ratio(input, cache_create, cache_read);
    let hit_ratio_prev = {
        let p_in = sum_tokens(&conn, prev_from, prev_to, "input", &q);
        let p_cc = sum_tokens(&conn, prev_from, prev_to, "cacheCreation", &q);
        let p_cr = sum_tokens(&conn, prev_from, prev_to, "cacheRead", &q);
        crate::api::efficiency::hit_ratio(p_in, p_cc, p_cr)
    };

    let savings = cache_savings_for_window(&conn, from, to, &q);
    let net_prev = cache_savings_for_window(&conn, prev_from, prev_to, &q).net;

    Ok(Json(CacheEfficiency {
        hit_ratio: round4(hit_ratio),
        hit_ratio_prev: round4(hit_ratio_prev),
        tokens: CacheTokenTotals {
            input,
            output,
            cache_read,
            cache_create,
        },
        savings: CacheSavings {
            net: round4(savings.net),
            gross: round4(savings.gross),
            creation_overhead: round4(savings.creation_overhead),
        },
        net_prev: round4(net_prev),
        unpriced_cache_tokens: savings.unpriced_cache_tokens,
    }))
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_cache_efficiency`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/tests/api_reports.rs
git commit -m "feat(api): add /api/v2/cache-efficiency endpoint (#19)"
```

---

### Task 7: `GET /api/v2/model-efficiency` endpoint

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (route registration + handler after `v2_cache_efficiency`)
- Test: `src-tauri/tests/api_reports.rs` (append)

- [ ] **Step 1: Write the failing integration test**

Append to `src-tauri/tests/api_reports.rs`:

```rust
// ---------------------------------------------------------------------------
// 10. v2_model_efficiency_buckets_by_dominant_family
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_model_efficiency_buckets_by_dominant_family() {
    let (pool, _db_dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();

    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "me-opus".into(),
            started_at_ms: Some(now),
            model: "claude-opus-4-7".into(),
            output_tokens: 1000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "me-haiku".into(),
            started_at_ms: Some(now),
            model: "claude-haiku-4-5".into(),
            output_tokens: 500,
            cost_usd: 1.0,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/model-efficiency").await;

    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("response must be an array");
    assert_eq!(rows.len(), 2, "expected one row per family, got {}", rows.len());

    // sorted by total cost desc -> opus first
    assert_eq!(rows[0]["family"], "opus");
    assert_eq!(rows[0]["sessions"].as_i64().unwrap(), 1);
    assert!((rows[0]["total_cost_usd"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    assert!((rows[0]["cost_per_session"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    // cost per 1k output = 5.0 / 1000 * 1000 = 5.0
    assert!((rows[0]["cost_per_1k_output"].as_f64().unwrap() - 5.0).abs() < 1e-6);
    assert_eq!(rows[1]["family"], "haiku");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_model_efficiency`
Expected: FAIL — route missing, response is 404.

- [ ] **Step 3: Register the route**

In `src-tauri/src/api/routes.rs`, in the `router` fn, add a line directly after the `cache-efficiency` route added in Task 6:

```rust
        .route("/api/v2/cache-efficiency", get(v2_cache_efficiency))
        .route("/api/v2/model-efficiency", get(v2_model_efficiency))
```

- [ ] **Step 4: Write the handler**

In `src-tauri/src/api/routes.rs`, immediately after the `v2_cache_efficiency` function added in Task 6, add:

```rust
async fn v2_model_efficiency(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<Vec<ModelEfficiencyRow>>, ApiError> {
    let (from, to) = q.window();
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let (m_sql, m_vals) = q.model_clause("model");

    // (session_id, model, cost) — folded into per-family-per-session costs.
    let cost_sql = format!(
        "SELECT session_id, model, SUM(cost_usd)
         FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id, model"
    );
    let mut cp: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in &m_vals {
        cp.push(Box::new(v.clone()));
    }
    let crefs: Vec<&dyn rusqlite::ToSql> = cp.iter().map(|b| &**b).collect();
    let mut cost_rows: Vec<(String, String, f64)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&cost_sql) {
        if let Ok(mapped) = stmt.query_map(crefs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2).unwrap_or(0.0),
            ))
        }) {
            cost_rows = mapped.flatten().collect();
        }
    }

    // (session_id, output tokens)
    let out_sql = format!(
        "SELECT session_id, SUM(count)
         FROM token_usage
         WHERE token_type = 'output' AND timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id"
    );
    let mut op: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from), Box::new(to)];
    for v in &m_vals {
        op.push(Box::new(v.clone()));
    }
    let orefs: Vec<&dyn rusqlite::ToSql> = op.iter().map(|b| &**b).collect();
    let mut output_rows: Vec<(String, i64)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&out_sql) {
        if let Ok(mapped) = stmt.query_map(orefs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1).unwrap_or(0)))
        }) {
            output_rows = mapped.flatten().collect();
        }
    }

    Ok(Json(crate::api::efficiency::aggregate_model_efficiency(
        &cost_rows,
        &output_rows,
    )))
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_model_efficiency`
Expected: PASS.

- [ ] **Step 6: Run the full Rust suite**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — entire suite green, no regressions.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/tests/api_reports.rs
git commit -m "feat(api): add /api/v2/model-efficiency endpoint (#20)"
```

---

### Task 8: API client methods (Angular)

**Files:**
- Modify: `web/src/app/core/api.service.ts` (DTO interfaces near `:67-82`; methods near `:212-226`)

- [ ] **Step 1: Add the DTO interfaces**

In `web/src/app/core/api.service.ts`, directly after the `V2CostByModel` interface (ends at `:70`), add:

```typescript
export interface V2CacheEfficiency {
  hit_ratio: number;
  hit_ratio_prev: number;
  tokens: { input: number; output: number; cache_read: number; cache_create: number };
  savings: { net: number; gross: number; creation_overhead: number };
  net_prev: number;
  unpriced_cache_tokens: number;
}

export interface V2ModelEfficiency {
  family: string;
  sessions: number;
  total_cost_usd: number;
  cost_per_session: number;
  output_tokens: number;
  cost_per_1k_output: number;
}
```

- [ ] **Step 2: Add the service methods**

In the same file, in the `ApiService` class, directly after the `costByModel(...)` method (ends at `:214`), add:

```typescript
  cacheEfficiency(args?: FilterArgs): Observable<V2CacheEfficiency> {
    return this.http.get<V2CacheEfficiency>(`${BASE}/api/v2/cache-efficiency`, {
      params: toParams(args),
    });
  }
  modelEfficiency(args?: FilterArgs): Observable<V2ModelEfficiency[]> {
    return this.http.get<V2ModelEfficiency[]>(`${BASE}/api/v2/model-efficiency`, {
      params: toParams(args),
    });
  }
```

- [ ] **Step 3: Verify the project still compiles and tests pass**

Run (from `web/`): `npm test`
Expected: PASS — the full Vitest suite stays green (this also confirms the new TypeScript compiles).

- [ ] **Step 4: Commit**

```bash
git add web/src/app/core/api.service.ts
git commit -m "feat(web): add cost-efficiency API client methods"
```

---

### Task 9: Efficiency page component (TDD)

**Files:**
- Create: `web/src/app/features/efficiency/efficiency.component.spec.ts`
- Create: `web/src/app/features/efficiency/efficiency.component.ts`
- Create: `web/src/app/features/efficiency/efficiency.component.html`

- [ ] **Step 1: Write the failing component spec**

Create `web/src/app/features/efficiency/efficiency.component.spec.ts`:

```typescript
// EfficiencyComponent tests using bare TestBed with a stubbed ApiService.
import { importProvidersFrom } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { Calendar, Gauge, Layers, RefreshCw, X, LucideAngularModule } from 'lucide-angular';
import { EfficiencyComponent } from './efficiency.component';
import { ApiService } from '../../core/api.service';

const CACHE = {
  hit_ratio: 0.68,
  hit_ratio_prev: 0.61,
  tokens: { input: 1000, output: 500, cache_read: 3400, cache_create: 700 },
  savings: { net: 42.18, gross: 58.9, creation_overhead: 16.72 },
  net_prev: 35.0,
  unpriced_cache_tokens: 0,
};
const MODELS = [
  {
    family: 'opus',
    sessions: 38,
    total_cost_usd: 69.92,
    cost_per_session: 1.84,
    output_tokens: 98480,
    cost_per_1k_output: 0.71,
  },
];

function setup(cache: unknown = CACHE, models: unknown = MODELS) {
  const fakeApi = {
    cacheEfficiency: () => of(cache),
    modelEfficiency: () => of(models),
  };
  TestBed.configureTestingModule({
    imports: [EfficiencyComponent],
    providers: [
      { provide: ApiService, useValue: fakeApi },
      importProvidersFrom(LucideAngularModule.pick({ Gauge, Calendar, Layers, RefreshCw, X })),
    ],
  });
  const fixture = TestBed.createComponent(EfficiencyComponent);
  fixture.detectChanges();
  // Second pass: the data effect runs during the first detectChanges and sets
  // the signals synchronously (of()); this flushes the resulting re-render.
  fixture.detectChanges();
  return { fixture };
}

describe('EfficiencyComponent', () => {
  it('renders the cache hit ratio', () => {
    const { fixture } = setup();
    expect(fixture.nativeElement.textContent).toContain('68%');
  });

  it('renders a model-efficiency row', () => {
    const { fixture } = setup();
    const text = fixture.nativeElement.textContent;
    expect(text).toContain('opus');
    expect(text).toContain('69.92');
  });

  it('shows the empty state when there are no model rows', () => {
    const { fixture } = setup(CACHE, []);
    expect(fixture.nativeElement.textContent).toContain('No data');
  });
});
```

- [ ] **Step 2: Run the spec to verify it fails**

Run (from `web/`): `npx vitest run efficiency.component`
Expected: FAIL — `Cannot find module './efficiency.component'`.

- [ ] **Step 3: Create the component class**

Create `web/src/app/features/efficiency/efficiency.component.ts`:

```typescript
import { CommonModule, DecimalPipe, PercentPipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, V2CacheEfficiency, V2ModelEfficiency } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

// Family → bar/dot color. Matches the Overview's MODEL_COLOR_TABLE palette.
const FAMILY_COLORS: Record<string, string> = {
  opus: '#facc15',
  sonnet: '#60a5fa',
  haiku: '#34d399',
  other: '#a78bfa',
};

@Component({
  selector: 'app-efficiency',
  standalone: true,
  imports: [CommonModule, DecimalPipe, PercentPipe, FilterBarComponent, LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './efficiency.component.html',
})
export class EfficiencyComponent {
  readonly filter = inject(FilterService);
  private readonly api = inject(ApiService);

  readonly cache = signal<V2CacheEfficiency | null>(null);
  readonly models = signal<V2ModelEfficiency[]>([]);

  constructor() {
    // Refetch whenever the filter window/models change or Refresh is clicked —
    // the same pattern as OverviewComponent.
    effect(() => {
      this.filter.refreshTick();
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.cacheEfficiency(args).subscribe((v) => this.cache.set(v));
      this.api.modelEfficiency(args).subscribe((v) => this.models.set(v));
    });
  }

  familyColor(f: string): string {
    return FAMILY_COLORS[f] ?? FAMILY_COLORS['other'];
  }

  fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
    return String(n);
  }

  /** Percentage-point delta between two ratios, e.g. "+7 pts". */
  ptDelta(cur: number, prev: number): string {
    const d = Math.round((cur - prev) * 100);
    return `${d >= 0 ? '+' : ''}${d} pts`;
  }

  /** Signed percent delta of a value vs its previous, e.g. "▲ 20%". */
  pctDelta(cur: number, prev: number): string {
    if (prev === 0) return '—';
    const d = (cur - prev) / prev;
    return `${d >= 0 ? '▲' : '▾'} ${Math.abs(d * 100).toFixed(0)}%`;
  }
}
```

- [ ] **Step 4: Create the component template**

Create `web/src/app/features/efficiency/efficiency.component.html`:

```html
<div class="crumb">
  <span class="flex items-center gap-1.5">
    <lucide-icon name="gauge" class="w-3.5 h-3.5"></lucide-icon>Efficiency
  </span>
</div>

<app-filter-bar />

<div class="px-6 py-5 flex flex-col gap-4">

  <!-- KPI strip -->
  <div class="grid grid-cols-3 gap-4">

    <!-- Cache hit ratio -->
    <section class="panel">
      <div class="panel-title">Cache hit ratio</div>
      <div class="panel-body">
        @if (cache(); as c) {
          <div class="text-5xl font-mono tabular-nums text-accent">{{ c.hit_ratio | percent : '1.0-0' }}</div>
          <div class="mt-3 h-1.5 bg-border rounded-sm overflow-hidden">
            <div class="h-full bg-accent" [style.width.%]="c.hit_ratio * 100"></div>
          </div>
          <div class="mt-2 text-[11px] text-muted font-mono">
            {{ ptDelta(c.hit_ratio, c.hit_ratio_prev) }} vs last period
          </div>
        } @else {
          <div class="text-5xl font-mono tabular-nums text-muted">—</div>
        }
      </div>
    </section>

    <!-- Net cache savings -->
    <section class="panel">
      <div class="panel-title">Net cache savings</div>
      <div class="panel-body">
        @if (cache(); as c) {
          <div class="text-5xl font-mono tabular-nums text-ok">${{ c.savings.net | number : '1.2-2' }}</div>
          <div class="mt-2 text-[11px] text-muted font-mono">
            gross ${{ c.savings.gross | number : '1.2-2' }}
            <span class="text-muted/70">−</span>
            premium ${{ c.savings.creation_overhead | number : '1.2-2' }}
          </div>
          <div class="mt-1 text-[11px] text-muted font-mono">
            {{ pctDelta(c.savings.net, c.net_prev) }} vs last period
          </div>
          @if (c.unpriced_cache_tokens > 0) {
            <div class="mt-1 text-[10px] text-muted/70">
              excludes {{ fmtTokens(c.unpriced_cache_tokens) }} tokens on un-priced models
            </div>
          }
        } @else {
          <div class="text-5xl font-mono tabular-nums text-muted">$ —</div>
        }
      </div>
    </section>

    <!-- Cache tokens -->
    <section class="panel">
      <div class="panel-title">Cache tokens</div>
      <div class="panel-body">
        @if (cache(); as c) {
          <div class="text-5xl font-mono tabular-nums">{{ fmtTokens(c.tokens.cache_read + c.tokens.cache_create) }}</div>
          <div class="mt-3 text-[11px] text-muted font-mono">
            {{ fmtTokens(c.tokens.cache_read) }} read · {{ fmtTokens(c.tokens.cache_create) }} create
          </div>
        } @else {
          <div class="text-5xl font-mono tabular-nums text-muted">—</div>
        }
      </div>
    </section>
  </div>

  <!-- Model cost-efficiency table -->
  <section class="panel">
    <div class="panel-title">Model cost-efficiency · {{ filter.range() === 'month' ? 'month-to-date' : 'period' }}</div>
    <div class="panel-body p-0">
      @if (models().length === 0) {
        <div class="px-4 py-6 text-muted text-xs font-mono">No data in this period</div>
      } @else {
        <table class="w-full font-mono text-xs">
          <thead>
            <tr class="text-[10px] uppercase text-muted">
              <th class="text-left px-4 py-1.5 font-normal">Family</th>
              <th class="text-right px-4 py-1.5 font-normal">Sessions</th>
              <th class="text-right px-4 py-1.5 font-normal">Cost / session</th>
              <th class="text-right px-4 py-1.5 font-normal">$ / 1k output</th>
              <th class="text-right px-4 py-1.5 font-normal pr-4">Total</th>
            </tr>
          </thead>
          <tbody>
            @for (m of models(); track m.family) {
              <tr class="border-t border-border/40">
                <td class="px-4 py-1.5">
                  <span class="inline-flex items-center gap-1.5">
                    <span class="inline-block w-2 h-2 rounded-full" [style.background-color]="familyColor(m.family)"></span>
                    {{ m.family }}
                  </span>
                </td>
                <td class="px-4 py-1.5 text-right tabular-nums">{{ m.sessions | number }}</td>
                <td class="px-4 py-1.5 text-right tabular-nums">${{ m.cost_per_session | number : '1.2-2' }}</td>
                <td class="px-4 py-1.5 text-right tabular-nums">${{ m.cost_per_1k_output | number : '1.2-2' }}</td>
                <td class="px-4 py-1.5 text-right pr-4 tabular-nums">${{ m.total_cost_usd | number : '1.2-2' }}</td>
              </tr>
            }
          </tbody>
        </table>
      }
    </div>
  </section>
</div>
```

- [ ] **Step 5: Run the spec to verify it passes**

Run (from `web/`): `npx vitest run efficiency.component`
Expected: PASS — all 3 `EfficiencyComponent` tests green.

- [ ] **Step 6: Commit**

```bash
git add web/src/app/features/efficiency/
git commit -m "feat(web): add the Efficiency page component"
```

---

### Task 10: Route and navigation

**Files:**
- Modify: `web/src/app/app.routes.ts:9` (after the `overview` route)
- Modify: `web/src/app/app.component.html:13` (after the Overview nav link)

- [ ] **Step 1: Add the route**

In `web/src/app/app.routes.ts`, directly after the `overview` route object (the one ending `.then((m) => m.OverviewComponent),` then `},`), add:

```typescript
  {
    path: 'efficiency',
    loadComponent: () =>
      import('./features/efficiency/efficiency.component').then((m) => m.EfficiencyComponent),
  },
```

- [ ] **Step 2: Add the nav item**

In `web/src/app/app.component.html`, directly after the closing `</a>` of the Overview nav link (line 14), add:

```html
      <a routerLink="/efficiency" routerLinkActive="active" class="nav-link">
        <lucide-icon name="gauge" class="w-4 h-4"></lucide-icon>
        <span>Efficiency</span>
      </a>
```

(The `Gauge` icon is already registered in `web/src/app/core/icons.ts` — no change needed there.)

- [ ] **Step 3: Verify the build succeeds**

Run (from `web/`): `npm run build`
Expected: the SPA builds with no errors; the `efficiency.component` lazy chunk appears in the output.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/app.routes.ts web/src/app/app.component.html
git commit -m "feat(web): route and nav for the Efficiency page"
```

---

### Task 11: Document the page

**Files:**
- Modify: `docs/features.md`

- [ ] **Step 1: Read the current features doc**

Open `docs/features.md` and find the section describing the Overview page (and note the heading style — e.g. `## Overview`).

- [ ] **Step 2: Add an Efficiency section**

Add a new section, placed after the Overview section, matching the file's existing heading level and tone:

```markdown
## Efficiency

A filterable page answering "am I spending tokens well?".

- **Cache hit ratio** — the share of prompt tokens (`input + cacheCreation +
  cacheRead`) served from cache, with a percentage-point delta vs. the previous
  period.
- **Net cache savings** — gross read savings minus the cache-creation premium,
  computed per model from the built-in price table. The gross figure and the
  premium are shown so the mechanic is visible. Tokens on models not in the
  price table are excluded and footnoted.
- **Model cost-efficiency** — per model family (`opus` / `sonnet` / `haiku`),
  the cost per session and cost per 1k output tokens. Each session is
  attributed wholly to the family that spent the most in it.

All figures respect the global filter bar (window + model chips).
```

- [ ] **Step 3: Commit**

```bash
git add docs/features.md
git commit -m "docs: document the Efficiency page"
```

---

## Final verification

After all tasks, run the full suites once more to confirm nothing regressed:

- [ ] Rust: from `src-tauri/`, `cargo test --features test-support` → all green.
- [ ] Angular: from `web/`, `npm test` → all green.
- [ ] Manual smoke (optional): `cargo tauri dev`, open the app, click the **Efficiency** nav item, confirm the three tiles and the model table render and react to the filter bar.

---

## Self-review notes

- **Spec coverage:** Page & nav → Tasks 9, 10. Cache endpoint (hit ratio, savings, un-priced footnote, prev-period) → Tasks 2, 6. Model endpoint (family classifier, dominant-family bucketing, per-family metrics) → Tasks 1, 4, 7. Frontend (layout B, tiles, table, filter effect) → Task 9. Edge cases (zero-guards, empty states) → covered in `efficiency.rs` (`hit_ratio`/`aggregate` guards) and the template's `@else` / "No data" branches. Tests → every backend task is TDD; Task 9 is TDD for the component. Docs → Task 11.
- **Deviation from spec:** the spec mentioned insta snapshot tests "matching the `v2_kpis` style". This plan uses explicit value assertions instead — they cover the same shape-and-value ground without the `cargo insta accept` round-trip. The spec also aspired to `#[tracing::instrument]`; the plan omits it to match the sibling `v2_kpis` / `v2_cost_by_model` handlers, which are private and uninstrumented. Neither changes behavior.
- **Type consistency:** `CacheEfficiency` / `CacheTokenTotals` / `CacheSavings` / `ModelEfficiencyRow` field names are identical across `dto.rs` (Task 3), the handlers (Tasks 6–7), and the TypeScript interfaces (Task 8). `efficiency::Savings` is internal-only and never serialized.
```
