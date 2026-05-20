> **SUPERSEDED (2026-05-19)** by `docs/superpowers/plans/2026-05-19-jsonl-cost-correctness.md`.

# JSONL gap-fill for OTLP-partial sessions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the binary `Coverage::{Otlp, JsonlOnly}` reconciler with per-turn deduplication so JSONL fills exactly the `token_usage` and `cost_entries` rows OTLP missed for partially-captured sessions, without ever overwriting an OTLP row.

**Architecture:** Two new SQL-backed helpers (`token_row_already_covered`, `cost_row_already_covered`) check for a near-duplicate row within a ±5 s window. `Ingestor::ingest_derived` consults them per row instead of gating on session-level coverage, and returns counts of rows it actually wrote. Counts surface via `IngestStats` and the `/api/jsonl/backfill` response so the user-facing toast can show "filled N gap rows."

**Tech Stack:** Rust 1.95 (rusqlite, tokio, anyhow, tracing) · Angular 21 (signals) · SQLite. No new dependencies, no migration.

**Spec:** [`docs/superpowers/specs/2026-05-19-jsonl-gap-fill-design.md`](../specs/2026-05-19-jsonl-gap-fill-design.md)

**Branch:** `feature/jsonl-gap-fill` (already created and checked out; based on `feature/jsonl-ingest` until PR #9 lands)

---

## File structure

### Modify (Rust)
- `src-tauri/src/jsonl/reconciler.rs` — two new `*_already_covered` helpers + unit tests.
- `src-tauri/src/jsonl/mod.rs` — `IngestStats` gains `tokens_filled` and `cost_filled`; `ingest_one_inner` accumulates them and emits a `tracing::info!` when non-zero.
- `src-tauri/src/otlp/ingestor.rs` — `ingest_derived` returns `(i64, i64)`; `TokenUsage`/`CostEntry` arms switch to per-row dedup; `SlashCommand`/`SubAgentCall` get `WHERE NOT EXISTS` idempotency guards.
- `src-tauri/src/api/dto.rs` — `JsonlBackfillResponse` gains `tokens_filled` and `cost_filled`; `From<IngestStats>` propagates.

### Modify (tests)
- `src-tauri/tests/jsonl_ingest_writes.rs` — rename `skips_token_usage_when_otlp_covered` → `dedups_token_usage_against_otlp_within_window`; add `gap_fills_when_otlp_partial`; add `gap_fills_cost_when_otlp_partial`; add `dedups_slash_commands_on_repeat`.
- `src-tauri/tests/jsonl_pipeline.rs` — strengthen `backfill_is_idempotent` to count token/cost rows.

### Modify (Angular)
- `web/src/app/core/models.ts` — `JsonlBackfillResponse` gains the two fields.
- `web/src/app/features/settings/settings.component.ts` — toast string shows "filled K gap rows" when non-zero.

### Not changing
- v4 migration (schema stays).
- Privacy property test (`tests/jsonl_privacy.rs`).
- Routes, web components beyond the toast, README, pitch.

---

## Task 1: Reconciler dedup helpers

**Files:**
- Modify: `src-tauri/src/jsonl/reconciler.rs`.
- Test: same file (inline `#[cfg(test)]`).

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` in `src-tauri/src/jsonl/reconciler.rs`:

```rust
#[test]
fn token_row_already_covered_within_5s_window() {
    let p = pool();
    let c = p.get().unwrap();
    c.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 10000, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(c);

    let pool_ref = p.as_ref();
    // Exact-timestamp hit.
    assert!(token_row_already_covered(
        pool_ref, "s1", 10_000, "claude-opus-4-7", "input", 5_000,
    ));
    // Within window.
    assert!(token_row_already_covered(
        pool_ref, "s1", 11_000, "claude-opus-4-7", "input", 5_000,
    ));
    // Outside window.
    assert!(!token_row_already_covered(
        pool_ref, "s1", 16_000, "claude-opus-4-7", "input", 5_000,
    ));
    // Different model.
    assert!(!token_row_already_covered(
        pool_ref, "s1", 10_000, "claude-sonnet-4-6", "input", 5_000,
    ));
    // Different token_type.
    assert!(!token_row_already_covered(
        pool_ref, "s1", 10_000, "claude-opus-4-7", "output", 5_000,
    ));
    // Different session.
    assert!(!token_row_already_covered(
        pool_ref, "s2", 10_000, "claude-opus-4-7", "input", 5_000,
    ));
}

#[test]
fn cost_row_already_covered_within_5s_window() {
    let p = pool();
    let c = p.get().unwrap();
    c.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
         VALUES ('s1', 10000, 'claude-opus-4-7', 0.05)",
        [],
    )
    .unwrap();
    drop(c);

    let pool_ref = p.as_ref();
    assert!(cost_row_already_covered(
        pool_ref, "s1", 11_000, "claude-opus-4-7", 5_000,
    ));
    assert!(!cost_row_already_covered(
        pool_ref, "s1", 16_000, "claude-opus-4-7", 5_000,
    ));
    assert!(!cost_row_already_covered(
        pool_ref, "s1", 10_000, "claude-sonnet-4-6", 5_000,
    ));
}
```

- [ ] **Step 2: Run to verify they fail**

```powershell
cd src-tauri; cargo test --features test-support --lib jsonl::reconciler
```

Expected: compile error (`token_row_already_covered` not found).

- [ ] **Step 3: Implement the helpers**

In the same file, after `coverage_for`, add:

```rust
/// Returns true if `token_usage` already has a row for this
/// (session_id, model, token_type) with a timestamp within ±window_ms of `ts_ms`.
/// Used to dedup JSONL-derived rows against any OTLP-emitted rows for the same turn.
pub fn token_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    token_type: &str,
    window_ms: i64,
) -> bool {
    let Ok(conn) = pool.get() else {
        return true; // conservative: skip the write on pool failure
    };
    let lo = ts_ms - window_ms;
    let hi = ts_ms + window_ms;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage
             WHERE session_id = ?1 AND model = ?2 AND token_type = ?3
               AND timestamp BETWEEN ?4 AND ?5
             LIMIT 1",
            params![session_id, model, token_type, lo, hi],
            |r| r.get(0),
        )
        .unwrap_or(0);
    n > 0
}

/// Returns true if `cost_entries` already has a row for this
/// (session_id, model) with a timestamp within ±window_ms of `ts_ms`.
pub fn cost_row_already_covered(
    pool: &DbPool,
    session_id: &str,
    ts_ms: i64,
    model: &str,
    window_ms: i64,
) -> bool {
    let Ok(conn) = pool.get() else {
        return true;
    };
    let lo = ts_ms - window_ms;
    let hi = ts_ms + window_ms;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM cost_entries
             WHERE session_id = ?1 AND model = ?2
               AND timestamp BETWEEN ?3 AND ?4
             LIMIT 1",
            params![session_id, model, lo, hi],
            |r| r.get(0),
        )
        .unwrap_or(0);
    n > 0
}
```

The signatures take `&DbPool` (not `&Arc<DbPool>`) to match the existing `pool_clone.as_ref()` pattern in `ingest_one_inner` and to make tests less arc-heavy. `coverage_for` keeps its `&Arc<DbPool>` signature because callers in `ingest_one_inner` already use it that way.

- [ ] **Step 4: Run to verify they pass**

```powershell
cd src-tauri; cargo test --features test-support --lib jsonl::reconciler
```

Expected: 4 PASS (2 existing + 2 new).

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/reconciler.rs
git commit -m "feat(jsonl): per-row dedup helpers for token_usage and cost_entries"
```

---

## Task 2: `ingest_derived` returns counts, drop binary gate on TokenUsage

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs` (caller).
- Test: `src-tauri/tests/jsonl_ingest_writes.rs`.

- [ ] **Step 1: Replace the existing token-usage test**

In `src-tauri/tests/jsonl_ingest_writes.rs`, **delete** `skips_token_usage_when_otlp_covered` and **add** these two new tests in its place:

```rust
#[test]
fn dedups_token_usage_against_otlp_within_window() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 10000, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    // JSONL turn at the same timestamp must NOT duplicate the existing OTLP row.
    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(),
        ts: 10_000,
        model: "claude-opus-4-7".into(),
        input: 500,
        output: 0,
        cache_create: 0,
        cache_read: 0,
    }];
    let (tokens_filled, _) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id='s1' AND model='claude-opus-4-7' AND token_type='input'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "JSONL must not duplicate the OTLP row");
    assert_eq!(tokens_filled, 0);
}

#[test]
fn gap_fills_when_otlp_partial() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    // OTLP captured only the first turn at t=100ms.
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 100, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    // JSONL has two turns: the captured one + a later gap turn at t=10_000ms.
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 500,
            output: 0,
            cache_create: 0,
            cache_read: 0,
        },
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            ts: 10_000,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 2000,
            cache_create: 0,
            cache_read: 50,
        },
    ];
    let (tokens_filled, _) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();

    let conn = pool.get().unwrap();
    // Original OTLP row preserved.
    let otlp_count: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=100 AND token_type='input'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(otlp_count, 500);

    // Gap-turn input/output/cacheRead all written.
    let gap_input: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='input'",
            [], |r| r.get(0),
        )
        .unwrap();
    let gap_output: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='output'",
            [], |r| r.get(0),
        )
        .unwrap();
    let gap_cache_read: i64 = conn
        .query_row(
            "SELECT count FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='cacheRead'",
            [], |r| r.get(0),
        )
        .unwrap();
    assert_eq!(gap_input, 1000);
    assert_eq!(gap_output, 2000);
    assert_eq!(gap_cache_read, 50);

    // No cacheCreation row (count was 0, skipped).
    let cache_create_n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id='s1' AND timestamp=10000 AND token_type='cacheCreation'",
            [], |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cache_create_n, 0);

    // 3 rows filled.
    assert_eq!(tokens_filled, 3);

    // data_source flipped from 'otlp' to 'mixed' once JSONL contributed.
    let data_source: String = conn
        .query_row(
            "SELECT data_source FROM sessions WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(data_source, "mixed");
}
```

- [ ] **Step 2: Run to verify they fail**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
```

Expected: compile error — `ingest_derived` returns `Result<()>`, not `Result<(i64, i64)>`.

- [ ] **Step 3: Change `ingest_derived` signature and the TokenUsage arm**

In `src-tauri/src/otlp/ingestor.rs`, change the function signature and the `TokenUsage` arm. The `Coverage` parameter stays (still used by the `SessionLifecycle` arm) but stops gating tokens:

```rust
pub fn ingest_derived(
    &self,
    events: &[crate::jsonl::reducer::DerivedEvent],
    coverage: crate::jsonl::reconciler::Coverage,
) -> Result<(i64, i64)> {
    use crate::jsonl::reconciler::{cost_row_already_covered, token_row_already_covered, Coverage};
    use crate::jsonl::reducer::DerivedEvent as E;

    const DEDUP_WINDOW_MS: i64 = 5_000;

    if self.control.is_paused() {
        return Ok((0, 0));
    }
    let pool = self.pool.clone();
    let mut conn = self.pool.get()?;
    let tx = conn.transaction()?;
    let mut tokens_filled: i64 = 0;
    let mut cost_filled: i64 = 0;

    for ev in events {
        match ev {
            E::SessionLifecycle {
                session_id,
                started_at,
                ended_at,
                cc_version,
                cwd,
                git_branch,
            } => {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO sessions
                       (session_id, started_at, ended_at, service_version, cwd, repo_branch, data_source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'jsonl')",
                    params![session_id, started_at, ended_at, cc_version, cwd, git_branch],
                );
                if matches!(coverage, Coverage::Otlp) {
                    let _ = tx.execute(
                        "UPDATE sessions SET data_source = 'mixed'
                         WHERE session_id = ?1 AND data_source = 'otlp'",
                        params![session_id],
                    );
                }
            }
            E::TokenUsage {
                session_id,
                ts,
                model,
                input,
                output,
                cache_create,
                cache_read,
            } => {
                for (kind, n) in [
                    ("input", *input),
                    ("output", *output),
                    ("cacheRead", *cache_read),
                    ("cacheCreation", *cache_create),
                ] {
                    if n > 0
                        && !token_row_already_covered(
                            pool.as_ref(),
                            session_id,
                            *ts,
                            model,
                            kind,
                            DEDUP_WINDOW_MS,
                        )
                    {
                        if tx
                            .execute(
                                "INSERT INTO token_usage (session_id, timestamp, model, token_type, count)
                                 VALUES (?1, ?2, ?3, ?4, ?5)",
                                params![session_id, ts, model, kind, n],
                            )
                            .is_ok()
                        {
                            tokens_filled += 1;
                            // Flip data_source if this is the first JSONL row landing on an OTLP-marked session.
                            let _ = tx.execute(
                                "UPDATE sessions SET data_source = 'mixed'
                                 WHERE session_id = ?1 AND data_source = 'otlp'",
                                params![session_id],
                            );
                        }
                    }
                }
            }
            E::CostEntry {
                session_id,
                ts,
                model,
                cost_usd,
            } => {
                // Intentionally still gated on Coverage in this task — Task 3 switches
                // this arm to per-row dedup like TokenUsage. Kept as-is so this task's
                // diff is scoped to tokens only.
                if matches!(coverage, Coverage::JsonlOnly) && *cost_usd > 0.0 {
                    let _ = tx.execute(
                        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![session_id, ts, model, cost_usd],
                    );
                }
            }
            E::ToolCall {
                session_id,
                ts,
                tool_name,
                file_path,
                model,
            } => {
                if matches!(coverage, Coverage::JsonlOnly) {
                    let _ = tx.execute(
                        "INSERT INTO tool_decisions
                           (session_id, timestamp, tool_name, decision, language, file_path, source, model)
                         VALUES (?1, ?2, ?3, 'invoke', NULL, ?4, 'jsonl', ?5)",
                        params![session_id, ts, tool_name, file_path, model],
                    );
                }
            }
            E::SlashCommand {
                session_id,
                ts,
                name,
                arg_count,
            } => {
                let _ = tx.execute(
                    "INSERT INTO slash_commands (session_id, timestamp, command_name, arg_count)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![session_id, ts, name, arg_count],
                );
            }
            E::SubAgentCall {
                parent_id,
                child_id,
                subagent_type,
                started_at,
            } => {
                let _ = tx.execute(
                    "INSERT INTO subagent_calls
                       (parent_session_id, child_session_id, subagent_type, started_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![parent_id, child_id, subagent_type, started_at],
                );
            }
        }
    }
    tx.commit()?;
    Ok((tokens_filled, cost_filled))
}
```

- [ ] **Step 4: Fix the caller in `ingest_one_inner`**

In `src-tauri/src/jsonl/mod.rs`, find the line `if let Err(e) = fresh_ing.ingest_derived(&events, cov)` and change to:

```rust
match fresh_ing.ingest_derived(&events, cov) {
    Ok((tokens, _cost)) => {
        if tokens > 0 {
            tracing::info!(sid, tokens_filled = tokens, "JSONL gap-filled token rows");
        }
    }
    Err(e) => tracing::error!(sid, error = ?e, "ingest_derived failed"),
}
```

(Cost filled is still always 0 at this task — Task 3 enables it.)

- [ ] **Step 5: Run all jsonl/ingestor tests**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
```

Expected: 4 PASS (the renamed test, the new gap-fill test, plus the existing `writes_slash_and_subagent` and `writes_tool_decisions_for_jsonl_only_session`).

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/otlp/ingestor.rs src-tauri/src/jsonl/mod.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(jsonl): per-row dedup for token_usage; ingest_derived returns counts"
```

---

## Task 3: Per-row dedup for cost_entries

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs`.
- Test: `src-tauri/tests/jsonl_ingest_writes.rs`.

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/tests/jsonl_ingest_writes.rs`:

```rust
#[test]
fn gap_fills_cost_when_otlp_partial() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, data_source) VALUES ('s1', 0, 'otlp')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
         VALUES ('s1', 100, 'claude-opus-4-7', 0.01)",
        [],
    )
    .unwrap();
    drop(conn);

    let events = vec![
        // Overlap with the OTLP row — must NOT duplicate.
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.01,
        },
        // Gap — must be written.
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            ts: 10_000,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
        },
    ];
    let (_, cost_filled) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    assert_eq!(cost_filled, 1);

    let conn = pool.get().unwrap();
    let total: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((total - 0.06).abs() < 1e-9);
}
```

- [ ] **Step 2: Run to verify it fails**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes gap_fills_cost_when_otlp_partial
```

Expected: FAIL — `cost_filled` is 0 because the `Coverage::JsonlOnly` gate still blocks JSONL cost writes for OTLP-covered sessions.

- [ ] **Step 3: Replace the CostEntry arm**

In `src-tauri/src/otlp/ingestor.rs`, replace the entire `E::CostEntry { .. }` match arm (the placeholder block from Task 2) with:

```rust
E::CostEntry {
    session_id,
    ts,
    model,
    cost_usd,
} => {
    if *cost_usd > 0.0
        && !cost_row_already_covered(
            pool.as_ref(),
            session_id,
            *ts,
            model,
            DEDUP_WINDOW_MS,
        )
    {
        if tx
            .execute(
                "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, ts, model, cost_usd],
            )
            .is_ok()
        {
            cost_filled += 1;
            let _ = tx.execute(
                "UPDATE sessions SET data_source = 'mixed'
                 WHERE session_id = ?1 AND data_source = 'otlp'",
                params![session_id],
            );
        }
    }
}
```

- [ ] **Step 4: Update the caller to log cost fills**

In `src-tauri/src/jsonl/mod.rs`, update the `match` block from Task 2 to log cost too:

```rust
match fresh_ing.ingest_derived(&events, cov) {
    Ok((tokens, cost)) => {
        if tokens + cost > 0 {
            tracing::info!(
                sid,
                tokens_filled = tokens,
                cost_filled = cost,
                "JSONL gap-filled rows for OTLP-partial session"
            );
        }
    }
    Err(e) => tracing::error!(sid, error = ?e, "ingest_derived failed"),
}
```

- [ ] **Step 5: Run**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
```

Expected: 5 PASS (4 from before + new cost test).

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/otlp/ingestor.rs src-tauri/src/jsonl/mod.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(jsonl): per-row dedup for cost_entries"
```

---

## Task 4: Wire `tokens_filled` / `cost_filled` through IngestStats

**Files:**
- Modify: `src-tauri/src/jsonl/mod.rs`.
- Test: `src-tauri/tests/jsonl_pipeline.rs`.

- [ ] **Step 1: Strengthen the existing idempotency test**

In `src-tauri/tests/jsonl_pipeline.rs`, replace `backfill_is_idempotent` with:

```rust
#[tokio::test]
async fn backfill_is_idempotent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    // One turn with tokens + cost so the second run has something to dedup against.
    write_transcript(
        home.path(),
        "x",
        &[
            r#"{"type":"user","sessionId":"sIDP","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
            r#"{"type":"assistant","sessionId":"sIDP","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":200}}}"#,
        ],
    );
    let pool_arc = Arc::clone(&pool);

    let s1 = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let s2 = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    let conn = pool.get().unwrap();
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    let tokens: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    let costs: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sIDP'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1, "session row not duplicated");
    assert_eq!(tokens, 2, "input + output rows; no duplicates from second run");
    assert_eq!(costs, 1, "single cost row");
    assert!(s1.tokens_filled >= 2, "first run filled at least 2 token rows");
    assert_eq!(s2.tokens_filled, 0, "second run filled nothing");
    assert_eq!(s2.cost_filled, 0);
}
```

- [ ] **Step 2: Run to verify it fails**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_pipeline backfill_is_idempotent
```

Expected: compile error — `IngestStats` has no `tokens_filled` / `cost_filled` fields yet.

- [ ] **Step 3: Add the fields to `IngestStats`**

In `src-tauri/src/jsonl/mod.rs`, replace the `IngestStats` struct:

```rust
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IngestStats {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub tokens_filled: i64,
    pub cost_filled: i64,
    pub duration_ms: i64,
}
```

- [ ] **Step 4: Accumulate counts in `ingest_one_inner`**

Still in `src-tauri/src/jsonl/mod.rs`, update the per-session loop inside `ingest_one_inner` to capture the tuple and add it to `stats`:

```rust
for (sid, events) in events_by_session {
    let cov = reconciler::coverage_for(&pool_clone, &sid)
        .unwrap_or(reconciler::Coverage::JsonlOnly);
    match fresh_ing.ingest_derived(&events, cov) {
        Ok((tokens, cost)) => {
            stats.tokens_filled += tokens;
            stats.cost_filled += cost;
            if tokens + cost > 0 {
                tracing::info!(
                    sid,
                    tokens_filled = tokens,
                    cost_filled = cost,
                    "JSONL gap-filled rows for OTLP-partial session"
                );
            }
        }
        Err(e) => tracing::error!(sid, error = ?e, "ingest_derived failed"),
    }
    stats.sessions_added += 1;
}
```

This replaces the `match` block from Task 3.

- [ ] **Step 5: Also accumulate in `backfill`**

In the same file, the outer `backfill` function adds per-file stats but currently doesn't include the new fields. Update its accumulator block (the `match ingest_one_inner(...)` arm in the `for path in &files` loop):

```rust
match ingest_one_inner(pool, ingestor, path).await {
    Ok(s) => {
        stats.records_processed += s.records_processed;
        stats.records_errored += s.records_errored;
        stats.sessions_added += s.sessions_added;
        stats.tokens_filled += s.tokens_filled;
        stats.cost_filled += s.cost_filled;
    }
    Err(e) => {
        tracing::error!(?path, error = ?e, "jsonl ingest failed");
        stats.records_errored += 1;
    }
}
```

- [ ] **Step 6: Run**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_pipeline
```

Expected: PASS (both `backfill_processes_synthetic_session` and the strengthened `backfill_is_idempotent`).

- [ ] **Step 7: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/tests/jsonl_pipeline.rs
git commit -m "feat(jsonl): IngestStats tracks tokens_filled and cost_filled"
```

---

## Task 5: Idempotency for slash_commands and subagent_calls

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs`.
- Test: `src-tauri/tests/jsonl_ingest_writes.rs`.

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/tests/jsonl_ingest_writes.rs`:

```rust
#[test]
fn dedups_slash_commands_on_repeat() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![DerivedEvent::SlashCommand {
        session_id: "s1".into(),
        ts: 100,
        name: "review".into(),
        arg_count: 1,
    }];

    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM slash_commands WHERE session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second call must not duplicate slash_command");
}

#[test]
fn dedups_subagent_calls_on_repeat() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![DerivedEvent::SubAgentCall {
        parent_id: "s1".into(),
        child_id: None,
        subagent_type: Some("Explore".into()),
        started_at: 100,
    }];

    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let n: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM subagent_calls WHERE parent_session_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "second call must not duplicate subagent_call");
}
```

- [ ] **Step 2: Run to verify they fail**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes dedups_
```

Expected: FAIL — both end up with `n == 2`.

- [ ] **Step 3: Add `WHERE NOT EXISTS` guards**

In `src-tauri/src/otlp/ingestor.rs`, replace the two arms:

```rust
E::SlashCommand {
    session_id,
    ts,
    name,
    arg_count,
} => {
    let _ = tx.execute(
        "INSERT INTO slash_commands (session_id, timestamp, command_name, arg_count)
         SELECT ?1, ?2, ?3, ?4
         WHERE NOT EXISTS (
             SELECT 1 FROM slash_commands
             WHERE session_id = ?1 AND timestamp = ?2 AND command_name = ?3
         )",
        params![session_id, ts, name, arg_count],
    );
}
E::SubAgentCall {
    parent_id,
    child_id,
    subagent_type,
    started_at,
} => {
    let _ = tx.execute(
        "INSERT INTO subagent_calls
           (parent_session_id, child_session_id, subagent_type, started_at)
         SELECT ?1, ?2, ?3, ?4
         WHERE NOT EXISTS (
             SELECT 1 FROM subagent_calls
             WHERE parent_session_id = ?1
               AND started_at = ?4
               AND COALESCE(subagent_type, '') = COALESCE(?3, '')
         )",
        params![parent_id, child_id, subagent_type, started_at],
    );
}
```

`subagent_type` is the dedup key (not `child_session_id`) because the JSONL `Task` tool input reliably carries `subagent_type` but does not always carry `session_id`. See spec §"Ingestor changes".

- [ ] **Step 4: Run**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
```

Expected: 7 PASS (5 from Tasks 2–3 plus the two new dedup tests).

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(jsonl): WHERE NOT EXISTS guards make slash_commands and subagent_calls idempotent"
```

---

## Task 6: Surface counts through the API response DTO

**Files:**
- Modify: `src-tauri/src/api/dto.rs`.
- Verify: `src-tauri/tests/api_jsonl.rs` (no new test; existing smoke must still pass).

- [ ] **Step 1: Extend the DTO**

In `src-tauri/src/api/dto.rs`, replace `JsonlBackfillResponse` and its `From` impl:

```rust
#[derive(Debug, serde::Serialize)]
pub struct JsonlBackfillResponse {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub tokens_filled: i64,
    pub cost_filled: i64,
    pub duration_ms: i64,
}

impl From<crate::jsonl::IngestStats> for JsonlBackfillResponse {
    fn from(s: crate::jsonl::IngestStats) -> Self {
        Self {
            files_processed: s.files_processed,
            records_processed: s.records_processed,
            records_errored: s.records_errored,
            sessions_added: s.sessions_added,
            tokens_filled: s.tokens_filled,
            cost_filled: s.cost_filled,
            duration_ms: s.duration_ms,
        }
    }
}
```

- [ ] **Step 2: Run the existing API smoke**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
```

Expected: 4 PASS unchanged (`backfill_endpoint_returns_2xx_or_500` + the three others).

- [ ] **Step 3: Commit**

```powershell
git add src-tauri/src/api/dto.rs
git commit -m "feat(api): JsonlBackfillResponse surfaces tokens_filled and cost_filled"
```

---

## Task 7: Web — model + toast text

**Files:**
- Modify: `web/src/app/core/models.ts`.
- Modify: `web/src/app/features/settings/settings.component.ts`.

- [ ] **Step 1: Extend the model**

In `web/src/app/core/models.ts`, replace the `JsonlBackfillResponse` interface:

```ts
export interface JsonlBackfillResponse {
  files_processed: number;
  records_processed: number;
  records_errored: number;
  sessions_added: number;
  tokens_filled: number;
  cost_filled: number;
  duration_ms: number;
}
```

- [ ] **Step 2: Update the toast string**

In `web/src/app/features/settings/settings.component.ts`, replace the body of `next:` inside `ingestJsonl()`:

```ts
next: (s) => {
  const summary =
    `Ingested ${s.records_processed} records from ${s.files_processed} files`;
  const filled = s.tokens_filled + s.cost_filled;
  const tail = filled > 0 ? ` · filled ${filled} gap rows from JSONL` : '';
  this.jsonlToast.set(`${summary}${tail} (${s.records_errored} errors).`);
  this.jsonlBusy.set(false);
  this.api.jsonlIngestRuns().subscribe((rs) => this.jsonlLatestRun.set(rs[0] ?? null));
},
```

- [ ] **Step 3: Build + run web tests**

```powershell
cd web; npm run build; npm test
```

Expected: build succeeds; `Test Files 3 passed (3) · Tests 14 passed (14)`.

- [ ] **Step 4: Commit**

```powershell
git add web/src/app/core/models.ts web/src/app/features/settings/settings.component.ts
git commit -m "feat(web): surface JSONL gap-fill counts in Settings toast"
```

---

## Task 8: End-to-end verification + PR

- [ ] **Step 1: Full Rust suite**

```powershell
cd src-tauri; cargo test --features test-support
```

Expected: every suite green, including:
- `jsonl::reconciler` (4 tests)
- `jsonl_ingest_writes` (7 tests)
- `jsonl_pipeline` (2 tests, including the strengthened idempotency check)
- `jsonl_privacy` (2 tests × 256 cases)
- `api_jsonl` (4 tests)
- All previously-existing suites unchanged.

- [ ] **Step 2: Web build + tests**

```powershell
cd web; npm run build; npm test
```

Expected: build succeeds; 14/14 web tests pass.

- [ ] **Step 3: Local manual smoke (optional but recommended)**

Run `cargo run --bin andon` (the binary brings up the API on `:8765`). In another shell:

```powershell
cd scripts; python smoke_jsonl.py
```

Expected: `backfill: {... 'tokens_filled': N, 'cost_filled': M ...}` where N and M may be > 0 if the current `~/.claude/projects/` has sessions OTLP partially captured. `OK — session present: smoke-...`.

Optionally hit the Settings page and click "Ingest JSONL" — the toast should read `... · filled K gap rows from JSONL ...` when K > 0.

- [ ] **Step 4: Push + open PR**

```powershell
git push -u origin feature/jsonl-gap-fill
gh pr create --title "feat: JSONL gap-fill for OTLP-partial sessions (Plan C+)" --body "$(cat <<'EOF'
## Summary
- Replace the binary OTLP/JSONL reconciler with per-turn dedup. JSONL now fills any `token_usage` / `cost_entries` row a session is missing, regardless of whether OTLP captured part of that session.
- Addresses the enterprise-strips-hooks scenario: when `~/.claude/settings.json` is overwritten partway through a session, the JSONL transcript still has every turn — Andon now uses it to fill the gap.
- Settings toast surfaces "filled K gap rows from JSONL" when K > 0; structured `tracing::info!` line per session for grep-ability.
- Idempotency: repeated backfill runs write nothing the second time. Also fixes a latent Plan C bug where `slash_commands` / `subagent_calls` could duplicate.

## What's NOT in this PR
- No schema change. v4 stays.
- No change to `tool_decisions` (still JSONL-only sessions only).
- No automatic detection of partial-OTLP sessions; the existing user-triggered backfill flow is the surface.

## Test plan
- [x] `cd src-tauri; cargo test --features test-support` — all suites green including the new 4 unit + 4 integration tests.
- [x] `cd web; npm run build` succeeds.
- [x] `cd web; npm test` — 14/14 pass.
- [ ] Manual smoke: `cargo run --bin andon` + `scripts/smoke_jsonl.py`; observe `tokens_filled` / `cost_filled` in the response.
- [ ] Settings → "Ingest JSONL" shows the new toast tail when there are gap rows to fill.

Spec: `docs/superpowers/specs/2026-05-19-jsonl-gap-fill-design.md`
Builds on PR #9 (`feature/jsonl-ingest`).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review checklist (run before opening PR)

1. **Spec coverage:**
   - Dedup helpers (spec §Reconciler API) → Task 1.
   - Per-row dedup for tokens (spec §Ingestor changes) → Task 2.
   - Per-row dedup for cost (spec §Ingestor changes) → Task 3.
   - `IngestStats` fields + log line (spec §Stats and observability) → Task 4.
   - Idempotency for slash_commands and subagent_calls (spec §Ingestor changes) → Task 5.
   - DTO fields (spec §API surface) → Task 6.
   - Web model + toast (spec §UI changes) → Task 7.
   - Verification (spec §Testing) → Task 8.
2. **No placeholders:** Every step has either concrete code, a concrete command, or a concrete commit.
3. **Type consistency:** `ingest_derived` signature is `Result<(i64, i64)>` from Task 2 onwards. `IngestStats` fields are referenced by Task 4 onwards. `JsonlBackfillResponse` matches between Rust (Task 6) and TS (Task 7).
4. **No destructive operations:** No DELETE statements. The `UPDATE sessions SET data_source = 'mixed'` is the only mutation outside of inserts, and it only flips `'otlp'` → `'mixed'`, never overwriting JSONL or any other value.
