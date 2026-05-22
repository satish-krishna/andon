# Efficiency page — subagent split — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the Efficiency page's model cost-efficiency table so its rows are keyed by `(family, role)` — `main` or `subagent` — instead of family alone, by parsing the `isSidechain` flag Claude Code already stamps on subagent JSONL records and threading it through ingest, aggregation, and the table.

**Architecture:** One schema column (`is_subagent`) added to `cost_entries` and `token_usage`. The JSONL parser reads `isSidechain`; the reducer copies it into the derived events; the JSONL write path inserts it and **upserts existing rows from `false` to `true`** so OTLP-first sessions heal when their JSONL post-session ingest runs. The pure `efficiency::aggregate_model_efficiency` aggregator gains a two-pass body — existing dominant-family-per-session logic for `main` rows, simpler per-family aggregation for `subagent` rows. The frontend table gets a Role column.

**Tech Stack:** Rust (rusqlite, axum, tokio), Angular 21 (standalone components, signals), SQLite, Vitest, `cargo test`.

**Reference spec:** `docs/superpowers/specs/2026-05-22-efficiency-subagent-split-design.md`

**Branch:** `feature/efficiency-page` (the same branch PR #28 is open on — this extends, not replaces). Continue committing there.

---

## File Structure

**Rust (`src-tauri/`)**
- `src/db/migrations.rs` — **modify.** Append `MIGRATION_V6` adding `is_subagent` to two tables; register it in `MIGRATIONS`.
- `src/jsonl/record.rs` — **modify.** Add `is_sidechain: bool` field to `JsonlRecord`.
- `src/jsonl/reducer.rs` — **modify.** Add `is_subagent: bool` to `DerivedEvent::TokenUsage` and `DerivedEvent::CostEntry`; copy `rec.is_sidechain` into each when emitting.
- `src/otlp/ingestor.rs` — **modify.** The JSONL-derived write path (around `:307` and `:336`): add `is_subagent` to the SQL column list and bound params; change `ON CONFLICT … DO NOTHING` to `ON CONFLICT … DO UPDATE SET is_subagent = 1 WHERE excluded.is_subagent = 1`. The OTLP-live write paths are left alone (they get `is_subagent = 0` via the column default).
- `src/api/efficiency.rs` — **modify.** Aggregator takes 4-tuples, runs two passes (main = dominant-family-per-session; subagent = per-family), emits rows with `role`.
- `src/api/dto.rs` — **modify.** `ModelEfficiencyRow` gains `role: String`.
- `src/api/routes.rs` — **modify.** `v2_model_efficiency` SELECTs add `is_subagent` (and the output query also adds `model`); the handler passes 4-tuples to the aggregator.
- `tests/common/mod.rs` — **modify.** `SeedOpts` gains `is_subagent: bool` (defaults to `false`); `seed_session` binds it on every insert.
- `tests/api_reports.rs` — **modify.** One new test: a session-with-main + a session-with-subagent → two rows with distinct roles.

**Angular (`web/`)**
- `src/app/core/api.service.ts` — **modify.** `V2ModelEfficiency` gains `role: 'main' | 'subagent'`.
- `src/app/features/efficiency/efficiency.component.html` — **modify.** New Role column / badge between Family and Sessions; `@for` track key includes role.
- `src/app/features/efficiency/efficiency.component.spec.ts` — **modify.** One new test asserting the subagent badge renders.

**Docs**
- `docs/features.md` — **modify.** Extend the Efficiency section's model cost-efficiency bullet to mention the role split.

**Conventions:** All Rust commands from `src-tauri/`. All Angular commands from `web/`. No `unwrap()`/`expect()` in non-test Rust `src/`. Conventional Commits, no emojis. Do NOT run `cargo fmt`. The branch is `feature/efficiency-page` — all commits land there.

---

### Task 1: Migration V6 — `is_subagent` column

**Files:**
- Modify: `src-tauri/src/db/migrations.rs:179-187`

- [ ] **Step 1: Add the migration**

In `src-tauri/src/db/migrations.rs`, directly after the `MIGRATION_V5` const (which ends with `r#" … "#;` near line 179), add a new const:

```rust
const MIGRATION_V6: &str = r#"
-- Distinguishes subagent (sidechain) JSONL records from main-agent activity.
-- Set on insert from `isSidechain`; upserted false -> true when a JSONL
-- post-session ingest re-sees an OTLP-written row as a subagent record.
ALTER TABLE cost_entries ADD COLUMN is_subagent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE token_usage  ADD COLUMN is_subagent INTEGER NOT NULL DEFAULT 0;
"#;
```

Then change the `MIGRATIONS` array to include it. Replace:

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
    (5, MIGRATION_V5),
];
```

with:

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
    (5, MIGRATION_V5),
    (6, MIGRATION_V6),
];
```

- [ ] **Step 2: Verify the migration applies**

Run (from `src-tauri/`): `cargo test --features test-support migrations`
Expected: PASS — existing migration tests still pass against V6.

- [ ] **Step 3: Verify the full suite still compiles and passes**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — no failures (the column has a default, existing INSERTs continue to work).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(db): migration v6 adds is_subagent to cost_entries and token_usage"
```

---

### Task 2: Parse `isSidechain` — `record.rs`

**Files:**
- Modify: `src-tauri/src/jsonl/record.rs:7-26` (the `JsonlRecord` struct)

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/jsonl/record.rs`, add these tests inside the existing `mod tests` block (after `missing_request_id_is_none`):

```rust
    #[test]
    fn parses_is_sidechain_true() {
        let line = r#"{"type":"assistant","sessionId":"s1","isSidechain":true}"#;
        let r = parse_line(line).expect("parse");
        assert!(r.is_sidechain);
    }

    #[test]
    fn missing_is_sidechain_defaults_to_false() {
        let line = r#"{"type":"assistant","sessionId":"s1"}"#;
        let r = parse_line(line).expect("parse");
        assert!(!r.is_sidechain);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `src-tauri/`): `cargo test --features test-support --lib record::tests::parses_is_sidechain_true`
Expected: FAIL — compile error `no field 'is_sidechain' on type 'JsonlRecord'`.

- [ ] **Step 3: Add the field**

In `src-tauri/src/jsonl/record.rs`, in the `JsonlRecord` struct, directly after the `is_meta` field (the last field, line ~25), add:

```rust
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
```

The struct should now end with:

```rust
    #[serde(rename = "isMeta", default)]
    pub is_meta: bool,
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support --lib record::tests`
Expected: PASS — all record tests pass (including the two new ones).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/jsonl/record.rs
git commit -m "feat(jsonl): parse isSidechain on JSONL records"
```

---

### Task 3: Thread `is_subagent` through the reducer

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs` — `DerivedEvent` variants `TokenUsage` and `CostEntry` (around `:21-37`); `reduce_assistant` body that constructs them (around `:124-149`).

- [ ] **Step 1: Write the failing test**

In `src-tauri/src/jsonl/reducer.rs`, add this test inside the existing `mod tests` block (after `assistant_task_tool_emits_subagent`):

```rust
    #[test]
    fn assistant_with_is_sidechain_marks_usage_as_subagent() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_a","isSidechain":true,"timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-haiku-4-5","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());

        let (token_flag, cost_flag) = out.iter().fold((None, None), |acc, e| match e {
            DerivedEvent::TokenUsage { is_subagent, .. } => (Some(*is_subagent), acc.1),
            DerivedEvent::CostEntry { is_subagent, .. } => (acc.0, Some(*is_subagent)),
            _ => acc,
        });
        assert_eq!(token_flag, Some(true), "TokenUsage missing or wrong is_subagent");
        assert_eq!(cost_flag, Some(true), "CostEntry missing or wrong is_subagent");
    }

    #[test]
    fn assistant_without_is_sidechain_marks_usage_as_main() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_b","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());

        let token_flag = out.iter().find_map(|e| match e {
            DerivedEvent::TokenUsage { is_subagent, .. } => Some(*is_subagent),
            _ => None,
        });
        assert_eq!(token_flag, Some(false), "TokenUsage should default to is_subagent=false");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `src-tauri/`): `cargo test --features test-support --lib reducer::tests`
Expected: FAIL — compile error `no field 'is_subagent' on TokenUsage` / `CostEntry`.

- [ ] **Step 3: Add the `is_subagent` field to both variants**

In `src-tauri/src/jsonl/reducer.rs`, change the `TokenUsage` variant (currently lines ~21-30) from:

```rust
    TokenUsage {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        input: i64,
        output: i64,
        cache_create: i64,
        cache_read: i64,
    },
```

to:

```rust
    TokenUsage {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        input: i64,
        output: i64,
        cache_create: i64,
        cache_read: i64,
        is_subagent: bool,
    },
```

And the `CostEntry` variant (currently lines ~31-37) from:

```rust
    CostEntry {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        cost_usd: f64,
    },
```

to:

```rust
    CostEntry {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        cost_usd: f64,
        is_subagent: bool,
    },
```

- [ ] **Step 4: Populate the field in the reducer**

In `src-tauri/src/jsonl/reducer.rs`, in `reduce_assistant`, the block that pushes `TokenUsage` (currently lines ~124-133) needs `is_subagent: rec.is_sidechain,` added. Change:

```rust
                out.push(DerivedEvent::TokenUsage {
                    session_id: sid.to_string(),
                    request_id: request_id.to_string(),
                    ts,
                    model: model.clone(),
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_create: u.cache_creation,
                    cache_read: u.cache_read,
                });
```

to:

```rust
                out.push(DerivedEvent::TokenUsage {
                    session_id: sid.to_string(),
                    request_id: request_id.to_string(),
                    ts,
                    model: model.clone(),
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_create: u.cache_creation,
                    cache_read: u.cache_read,
                    is_subagent: rec.is_sidechain,
                });
```

And similarly the `CostEntry` push (currently lines ~142-149). Change:

```rust
                        out.push(DerivedEvent::CostEntry {
                            session_id: sid.to_string(),
                            request_id: request_id.to_string(),
                            ts,
                            model: model.clone(),
                            cost_usd: cost,
                        });
```

to:

```rust
                        out.push(DerivedEvent::CostEntry {
                            session_id: sid.to_string(),
                            request_id: request_id.to_string(),
                            ts,
                            model: model.clone(),
                            cost_usd: cost,
                            is_subagent: rec.is_sidechain,
                        });
```

- [ ] **Step 5: Run the tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support --lib reducer::tests`
Expected: PASS — all reducer tests pass.

- [ ] **Step 6: Verify the full crate still compiles**

Run (from `src-tauri/`): `cargo build`
Expected: builds cleanly — except possibly warnings/errors in `otlp/ingestor.rs` where the consumer destructures these variants. **Stop and look.** If `cargo build` fails because `ingestor.rs` doesn't reference `is_subagent` in its pattern matches, that is fine for now (Task 4 fixes it). If the errors are anything else, STOP and investigate.

If `ingestor.rs` errors with "missing field is_subagent in pattern", proceed to Task 4 directly — don't try to patch `ingestor.rs` in this commit's scope. If you must commit a passing build to keep the branch buildable between tasks, add `is_subagent: _,` to the existing `E::TokenUsage { … }` and `E::CostEntry { … }` patterns in `ingestor.rs` (Task 4 replaces those patches with real bindings).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/jsonl/reducer.rs src-tauri/src/otlp/ingestor.rs
git commit -m "feat(jsonl): thread is_subagent through reducer events"
```

(Include `ingestor.rs` in the commit only if you added the `is_subagent: _,` patches in Step 6.)

---

### Task 4: JSONL write path — include `is_subagent` and upsert flip

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs:307-325` (token_usage write); `:336-353` (cost_entries write)
- Test: `src-tauri/tests/api_reports.rs` (append)

- [ ] **Step 1: Write the failing integration test**

Append to `src-tauri/tests/api_reports.rs`:

```rust
// ---------------------------------------------------------------------------
// 13. jsonl_subagent_record_upserts_is_subagent_flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jsonl_subagent_record_upserts_is_subagent_flag() {
    use andon_lib::jsonl::reducer::DerivedEvent;
    use andon_lib::jsonl::reconciler::Coverage;
    let (pool, _db_dir) = common::fixture_pool();

    // Pre-seed a cost_entries / token_usage row with is_subagent=0 by hand,
    // simulating a row OTLP wrote first.
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().expect("conn");
    conn.execute(
        "INSERT INTO cost_entries (session_id, request_id, timestamp, model, cost_usd, is_subagent)
         VALUES ('s1', 'req_x', ?, 'claude-opus-4-7', 1.0, 0)",
        rusqlite::params![now],
    ).expect("insert pre-existing cost row");
    conn.execute(
        "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count, is_subagent)
         VALUES ('s1', 'req_x', ?, 'claude-opus-4-7', 'input', 100, 0)",
        rusqlite::params![now],
    ).expect("insert pre-existing token row");
    drop(conn);

    // Now re-ingest the same request_id via the JSONL path with is_subagent=true.
    let ingestor = common::test_ingestor(&pool);
    let events = vec![
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: now,
            model: "claude-opus-4-7".into(),
            cost_usd: 1.0,
            is_subagent: true,
        },
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: now,
            model: "claude-opus-4-7".into(),
            input: 100,
            output: 0,
            cache_create: 0,
            cache_read: 0,
            is_subagent: true,
        },
    ];
    ingestor.ingest_derived("s1", &events, Coverage::JsonlOnly).expect("ingest");

    // The rows should now be is_subagent=1.
    let conn = pool.get().expect("conn");
    let cost_flag: i64 = conn.query_row(
        "SELECT is_subagent FROM cost_entries WHERE request_id = 'req_x'",
        [], |r| r.get(0),
    ).expect("query cost");
    let token_flag: i64 = conn.query_row(
        "SELECT is_subagent FROM token_usage WHERE request_id = 'req_x' AND token_type = 'input'",
        [], |r| r.get(0),
    ).expect("query token");
    assert_eq!(cost_flag, 1, "cost row should be upserted to is_subagent=1");
    assert_eq!(token_flag, 1, "token row should be upserted to is_subagent=1");
}
```

**Important:** the test references `ingest_derived` and `Coverage` — confirm by reading `src/jsonl/mod.rs` and `src/otlp/ingestor.rs` that these names are correct in the current codebase. If `ingest_derived` is private or named differently, adjust the test (or the call) to use the actual entry point the JSONL pipeline uses to write derived events. The intent is: invoke the same write path a JSONL session-end ingest would.

- [ ] **Step 2: Run the test to verify it fails**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports jsonl_subagent_record_upserts_is_subagent_flag`
Expected: FAIL — either compile error (the upsert SQL isn't in place; `is_subagent` field on `DerivedEvent` was just added in Task 3 so destructuring is the next gap) OR assertion fails because the existing `ON CONFLICT … DO NOTHING` leaves the rows at 0.

- [ ] **Step 3: Update the JSONL token_usage write**

In `src-tauri/src/otlp/ingestor.rs`, find the JSONL `TokenUsage` arm (around line 286). Currently the destructure is missing `is_subagent`; the SQL is missing the column; the conflict clause does nothing. Replace the entire arm from `E::TokenUsage { … } => { … }` (lines ~286-327) with:

```rust
                E::TokenUsage {
                    session_id,
                    request_id,
                    ts,
                    model,
                    input,
                    output,
                    cache_create,
                    cache_read,
                    is_subagent,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) {
                        for (kind, n) in [
                            ("input", *input),
                            ("output", *output),
                            ("cacheRead", *cache_read),
                            ("cacheCreation", *cache_create),
                        ] {
                            if n > 0 {
                                let affected = match tx.execute(
                                    "INSERT INTO token_usage
                                       (session_id, request_id, timestamp, model, token_type, count, is_subagent)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                                     ON CONFLICT(request_id, token_type)
                                       WHERE request_id IS NOT NULL DO UPDATE
                                       SET is_subagent = 1
                                       WHERE excluded.is_subagent = 1",
                                    params![session_id, request_id, ts, model, kind, n, *is_subagent as i64],
                                ) {
                                    Ok(rows) => rows,
                                    Err(e) => {
                                        // ON CONFLICT DO UPDATE returns Ok(1) on a matching upsert
                                        // and Ok(0) when the guarded WHERE eliminates the update.
                                        // Any Err is a genuine insert failure — log, never surface.
                                        tracing::warn!(error = ?e, session_id, "JSONL token_usage insert failed");
                                        0
                                    }
                                };
                                tokens_written += affected as i64;
                            }
                        }
                    }
                }
```

- [ ] **Step 4: Update the JSONL cost_entries write**

In the same file, find the `E::CostEntry { … }` arm (around line 328). Replace the entire arm with:

```rust
                E::CostEntry {
                    session_id,
                    request_id,
                    ts,
                    model,
                    cost_usd,
                    is_subagent,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) && *cost_usd > 0.0 {
                        let affected = match tx.execute(
                            "INSERT INTO cost_entries
                               (session_id, request_id, timestamp, model, cost_usd, is_subagent)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                             ON CONFLICT(request_id)
                               WHERE request_id IS NOT NULL DO UPDATE
                               SET is_subagent = 1
                               WHERE excluded.is_subagent = 1",
                            params![session_id, request_id, ts, model, cost_usd, *is_subagent as i64],
                        ) {
                            Ok(rows) => rows,
                            Err(e) => {
                                tracing::warn!(error = ?e, session_id, "JSONL cost_entries insert failed");
                                0
                            }
                        };
                        cost_written += affected as i64;
                    }
                }
```

- [ ] **Step 5: Run the test to verify it passes**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports jsonl_subagent_record_upserts_is_subagent_flag`
Expected: PASS.

- [ ] **Step 6: Verify the full Rust suite still passes**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — no regressions.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/api_reports.rs
git commit -m "feat(jsonl): write is_subagent on JSONL ingest and upsert false->true"
```

---

### Task 5: Seed `is_subagent` in the test helper

**Files:**
- Modify: `src-tauri/tests/common/mod.rs` — `SeedOpts` struct (`:29-44`); `seed_session` (`:47-100`)

- [ ] **Step 1: Add the field to `SeedOpts`**

In `src-tauri/tests/common/mod.rs`, in the `SeedOpts` struct, directly after `pub cache_create_tokens: i64,` (the Task 5 addition from the cost-efficiency feature), add:

```rust
    pub cache_create_tokens: i64,
    /// When true, every cost/token row seeded by this call is tagged
    /// `is_subagent = 1` — simulates a JSONL-ingested subagent session.
    pub is_subagent: bool,
```

- [ ] **Step 2: Bind it in every INSERT**

In `seed_session`, the function currently has 5 token-related INSERTs (`input`, `output`, `cacheRead`, `cacheCreation`) and one `cost_entries` INSERT. Update each to include `is_subagent`. For example, the `input` INSERT changes from:

```rust
    if opts.input_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
             VALUES (?, ?, ?, 'input', ?)",
            params![opts.session_id, started, opts.model, opts.input_tokens],
        )
        .expect("insert input token_usage");
    }
```

to:

```rust
    if opts.input_tokens > 0 {
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count, is_subagent) \
             VALUES (?, ?, ?, 'input', ?, ?)",
            params![opts.session_id, started, opts.model, opts.input_tokens, opts.is_subagent as i64],
        )
        .expect("insert input token_usage");
    }
```

Apply the same shape to all four token-type INSERTs (`output`, `cacheRead`, `cacheCreation`) — extend the column list with `is_subagent`, the placeholder list with `?`, and the params with `opts.is_subagent as i64`.

The `cost_entries` INSERT changes from:

```rust
    if opts.cost_usd != 0.0 {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) \
             VALUES (?, ?, ?, ?)",
            params![opts.session_id, started, opts.model, opts.cost_usd],
        )
        .expect("insert cost_entries");
    }
```

to:

```rust
    if opts.cost_usd != 0.0 {
        conn.execute(
            "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd, is_subagent) \
             VALUES (?, ?, ?, ?, ?)",
            params![opts.session_id, started, opts.model, opts.cost_usd, opts.is_subagent as i64],
        )
        .expect("insert cost_entries");
    }
```

The `tool_decisions` and `file_changes` INSERTs are unrelated — leave them.

- [ ] **Step 3: Verify existing tests still pass**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — `SeedOpts` derives `Default`, the new `is_subagent` defaults to `false`, all existing `..Default::default()` callers are unaffected.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/common/mod.rs
git commit -m "test(api): SeedOpts gains an is_subagent option"
```

---

### Task 6: Aggregator — two-pass main/subagent + DTO `role` field

**Files:**
- Modify: `src-tauri/src/api/efficiency.rs`
- Modify: `src-tauri/src/api/dto.rs` (`ModelEfficiencyRow`)

- [ ] **Step 1: Update the DTO**

In `src-tauri/src/api/dto.rs`, in the `ModelEfficiencyRow` struct, directly after the `family: String` field, add `role`:

```rust
#[derive(Debug, serde::Serialize)]
pub struct ModelEfficiencyRow {
    /// `opus` | `sonnet` | `haiku` | `other`.
    pub family: String,
    /// `main` (the session's main-agent half) | `subagent` (sidechain).
    pub role: String,
    pub sessions: i64,
    pub total_cost_usd: f64,
    pub cost_per_session: f64,
    pub output_tokens: i64,
    pub cost_per_1k_output: f64,
}
```

- [ ] **Step 2: Write the new failing tests**

In `src-tauri/src/api/efficiency.rs`, add these tests inside `mod tests` (after `aggregate_handles_zero_output`):

```rust
    #[test]
    fn aggregate_splits_main_and_subagent_rows() {
        // s1 main opus 5.0 with 1000 output; s1 subagent haiku 1.0 with 500 output
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(),  5.0, false),
            ("s1".to_string(), "claude-haiku-4-5".to_string(), 1.0, true),
        ];
        let output_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(),  1000i64, false),
            ("s1".to_string(), "claude-haiku-4-5".to_string(),  500i64, true),
        ];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);

        assert_eq!(rows.len(), 2);
        // sort: opus main 5.0 > haiku subagent 1.0
        assert_eq!(rows[0].family, "opus");
        assert_eq!(rows[0].role, "main");
        assert_eq!(rows[0].sessions, 1);
        assert!((rows[0].total_cost_usd - 5.0).abs() < 1e-9);
        assert!((rows[0].cost_per_1k_output - 5.0).abs() < 1e-9); // 5.0/1000*1000

        assert_eq!(rows[1].family, "haiku");
        assert_eq!(rows[1].role, "subagent");
        assert_eq!(rows[1].sessions, 1);
        assert!((rows[1].total_cost_usd - 1.0).abs() < 1e-9);
        assert!((rows[1].cost_per_1k_output - 2.0).abs() < 1e-9); // 1.0/500*1000
    }

    #[test]
    fn aggregate_subagent_pass_groups_by_actual_family() {
        // Two subagent rows in the same session, two families -> two subagent rows
        let cost_rows = vec![
            ("s1".to_string(), "claude-haiku-4-5".to_string(),  1.0, true),
            ("s1".to_string(), "claude-sonnet-4-6".to_string(), 2.0, true),
        ];
        let output_rows: Vec<(String, String, i64, bool)> = vec![];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.role == "subagent"));
        // sorted by cost desc: sonnet 2.0 > haiku 1.0
        assert_eq!(rows[0].family, "sonnet");
        assert_eq!(rows[1].family, "haiku");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: FAIL — compile errors (tuple arity mismatch on the new tests; existing tests still compile against the old signature).

- [ ] **Step 4: Update the existing aggregator tests to the new tuple shape**

In `src-tauri/src/api/efficiency.rs`, update the three existing tests to use the new 4-tuple cost shape and 4-tuple output shape. (Re-using the existing fixtures with `false` for `is_subagent` and a model string on the output tuples — all "main"-only.)

Change `aggregate_buckets_session_by_dominant_family`:

```rust
    #[test]
    fn aggregate_buckets_session_by_dominant_family() {
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(),  5.0, false),
            ("s1".to_string(), "claude-haiku-4-5".to_string(), 1.0, false),
            ("s2".to_string(), "claude-haiku-4-5".to_string(), 2.0, false),
        ];
        let output_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(), 1000i64, false),
            ("s2".to_string(), "claude-haiku-4-5".to_string(),  500i64, false),
        ];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].family, "opus");
        assert_eq!(rows[0].role, "main");
        assert_eq!(rows[0].sessions, 1);
        assert!((rows[0].total_cost_usd - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_session - 6.0).abs() < 1e-9);
        assert!((rows[0].cost_per_1k_output - 6.0).abs() < 1e-9);
        assert_eq!(rows[1].family, "haiku");
        assert!((rows[1].cost_per_1k_output - 4.0).abs() < 1e-9);
    }
```

Change `aggregate_breaks_ties_toward_opus`:

```rust
    #[test]
    fn aggregate_breaks_ties_toward_opus() {
        let cost_rows = vec![
            ("s1".to_string(), "claude-opus-4-7".to_string(),  2.0, false),
            ("s1".to_string(), "claude-sonnet-4-6".to_string(), 2.0, false),
        ];
        let output_rows: Vec<(String, String, i64, bool)> = vec![];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].family, "opus");
        assert_eq!(rows[0].role, "main");
    }
```

Change `aggregate_handles_zero_output`:

```rust
    #[test]
    fn aggregate_handles_zero_output() {
        let cost_rows = vec![("s1".to_string(), "claude-opus-4-7".to_string(), 3.0, false)];
        let output_rows: Vec<(String, String, i64, bool)> = vec![];
        let rows = aggregate_model_efficiency(&cost_rows, &output_rows);
        assert_eq!(rows[0].cost_per_1k_output, 0.0);
        assert_eq!(rows[0].role, "main");
    }
```

- [ ] **Step 5: Rewrite the aggregator**

In `src-tauri/src/api/efficiency.rs`, replace the existing `aggregate_model_efficiency` function with this new implementation (the helpers `model_family`, `dominant_family`, and `round4` already exist in the module and are unchanged):

```rust
/// Aggregate per-session cost/output into per-family rows, split by role
/// (`main` vs `subagent`). `cost_rows` and `output_rows` carry an
/// `is_subagent` flag per row.
///
/// Main rows preserve the original dominant-family-per-session attribution:
/// every main row of a session contributes to that session's main half,
/// which is wholly bucketed under the dominant family. Subagent rows are a
/// pure per-actual-family aggregation; a subagent invocation typically uses
/// one model anyway, so per-family ~= per-dominant-family. Rows are sorted
/// by `total_cost_usd` descending.
pub fn aggregate_model_efficiency(
    cost_rows:   &[(String, String, f64, bool)],
    output_rows: &[(String, String, i64, bool)],
) -> Vec<ModelEfficiencyRow> {
    let mut out: Vec<ModelEfficiencyRow> = Vec::new();

    // ---- Main pass: dominant-family per session, is_subagent=false rows only ----
    {
        let mut per_session: HashMap<&str, HashMap<&'static str, f64>> = HashMap::new();
        for (sid, model, cost, is_sub) in cost_rows {
            if *is_sub { continue; }
            *per_session
                .entry(sid.as_str())
                .or_default()
                .entry(model_family(model))
                .or_insert(0.0) += *cost;
        }
        let mut output: HashMap<&str, i64> = HashMap::new();
        for (sid, _model, toks, is_sub) in output_rows {
            if *is_sub { continue; }
            *output.entry(sid.as_str()).or_insert(0) += *toks;
        }
        let mut buckets: HashMap<&'static str, (i64, f64, i64)> = HashMap::new();
        for (sid, fam_costs) in &per_session {
            let fam = dominant_family(fam_costs);
            let total_cost: f64 = fam_costs.values().sum();
            let out_toks = output.get(sid).copied().unwrap_or(0);
            let e = buckets.entry(fam).or_insert((0, 0.0, 0));
            e.0 += 1;
            e.1 += total_cost;
            e.2 += out_toks;
        }
        for (family, (sessions, total_cost, output_tokens)) in buckets {
            out.push(ModelEfficiencyRow {
                family: family.to_string(),
                role: "main".to_string(),
                sessions,
                total_cost_usd: round4(total_cost),
                cost_per_session: round4(total_cost / sessions as f64),
                output_tokens,
                cost_per_1k_output: if output_tokens > 0 {
                    round4(total_cost / output_tokens as f64 * 1000.0)
                } else { 0.0 },
            });
        }
    }

    // ---- Subagent pass: per actual family, is_subagent=true rows only ----
    {
        // family -> (distinct_sessions, total_cost, output_tokens)
        let mut buckets: HashMap<&'static str, (std::collections::HashSet<String>, f64, i64)> =
            HashMap::new();
        for (sid, model, cost, is_sub) in cost_rows {
            if !*is_sub { continue; }
            let fam = model_family(model);
            let e = buckets.entry(fam).or_insert_with(|| {
                (std::collections::HashSet::new(), 0.0, 0)
            });
            e.0.insert(sid.clone());
            e.1 += *cost;
        }
        for (_sid, model, toks, is_sub) in output_rows {
            if !*is_sub { continue; }
            let fam = model_family(model);
            if let Some(e) = buckets.get_mut(fam) {
                e.2 += *toks;
            }
        }
        for (family, (sessions_set, total_cost, output_tokens)) in buckets {
            let sessions = sessions_set.len() as i64;
            out.push(ModelEfficiencyRow {
                family: family.to_string(),
                role: "subagent".to_string(),
                sessions,
                total_cost_usd: round4(total_cost),
                cost_per_session: if sessions > 0 {
                    round4(total_cost / sessions as f64)
                } else { 0.0 },
                output_tokens,
                cost_per_1k_output: if output_tokens > 0 {
                    round4(total_cost / output_tokens as f64 * 1000.0)
                } else { 0.0 },
            });
        }
    }

    out.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}
```

(The existing top-of-file `use std::collections::HashMap;` already covers the HashMap import; `HashSet` is referenced with its full path inline.)

- [ ] **Step 6: Run the unit tests to verify they pass**

Run (from `src-tauri/`): `cargo test --features test-support --lib efficiency`
Expected: PASS — all 5 aggregator tests (3 updated + 2 new) plus the unchanged `model_family` / `hit_ratio` / `cache_savings` tests pass. **Watch the build:** the handler `v2_model_efficiency` in `routes.rs` will not yet compile against the new signature — that is fixed in Task 7.

If the workspace fails to compile because `routes.rs` calls `aggregate_model_efficiency` with the old tuple shape, that is expected. Skip the broader `cargo test` until Task 7. The `--lib efficiency` filter compiles only the lib unit tests in `efficiency.rs`.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/api/efficiency.rs src-tauri/src/api/dto.rs
git commit -m "feat(api): aggregator splits into main and subagent rows"
```

---

### Task 7: Handler `v2_model_efficiency` selects `is_subagent` (+ integration test)

**Files:**
- Modify: `src-tauri/src/api/routes.rs` — the `v2_model_efficiency` function (the one added in the cost-efficiency feature; ends with `Ok(Json(crate::api::efficiency::aggregate_model_efficiency(&cost_rows, &output_rows)))`)
- Test: `src-tauri/tests/api_reports.rs` (append)

- [ ] **Step 1: Write the failing integration test**

Append to `src-tauri/tests/api_reports.rs`:

```rust
// ---------------------------------------------------------------------------
// 14. v2_model_efficiency_splits_main_and_subagent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_model_efficiency_splits_main_and_subagent() {
    let (pool, _db_dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();

    // A main-agent session on opus
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "split-main".into(),
            started_at_ms: Some(now),
            model: "claude-opus-4-7".into(),
            output_tokens: 1000,
            cost_usd: 5.0,
            ..Default::default()
        },
    );
    // A subagent session on haiku — same start time, separate session id
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "split-sub".into(),
            started_at_ms: Some(now),
            model: "claude-haiku-4-5".into(),
            output_tokens: 500,
            cost_usd: 1.0,
            is_subagent: true,
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let (status, body) = get_json(router, "/api/v2/model-efficiency").await;

    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array");
    assert_eq!(rows.len(), 2, "expected one main + one subagent row, got {}", rows.len());

    // sort: opus main 5.0 > haiku subagent 1.0
    assert_eq!(rows[0]["family"], "opus");
    assert_eq!(rows[0]["role"], "main");
    assert_eq!(rows[1]["family"], "haiku");
    assert_eq!(rows[1]["role"], "subagent");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_model_efficiency_splits`
Expected: FAIL — compile error in `routes.rs` (the existing `v2_model_efficiency` passes 3-tuples to the now-4-tuple aggregator), or, after fixing the build, the assertion fails because the SELECTs don't pick up `is_subagent`.

- [ ] **Step 3: Update the cost query**

In `src-tauri/src/api/routes.rs`, find the `v2_model_efficiency` handler. Locate the cost SQL block (currently selects `session_id, model, SUM(cost_usd) … GROUP BY session_id, model`). Change the SQL to add `is_subagent`:

```rust
    let cost_sql = format!(
        "SELECT session_id, model, SUM(cost_usd), is_subagent
         FROM cost_entries
         WHERE timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id, model, is_subagent"
    );
```

And change the row-mapping closure that builds `cost_rows`:

```rust
    let mut cost_rows: Vec<(String, String, f64, bool)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&cost_sql) {
        if let Ok(mapped) = stmt.query_map(crefs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2).unwrap_or(0.0),
                r.get::<_, i64>(3).unwrap_or(0) != 0,
            ))
        }) {
            cost_rows = mapped.flatten().collect();
        }
    }
```

- [ ] **Step 4: Update the output query**

In the same function, the output query currently selects `session_id, SUM(count) … GROUP BY session_id`. Change to add `model` and `is_subagent`:

```rust
    let out_sql = format!(
        "SELECT session_id, model, SUM(count), is_subagent
         FROM token_usage
         WHERE token_type = 'output' AND timestamp >= ? AND timestamp < ?{m_sql}
         GROUP BY session_id, model, is_subagent"
    );
```

And the row-mapping closure for `output_rows`:

```rust
    let mut output_rows: Vec<(String, String, i64, bool)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&out_sql) {
        if let Ok(mapped) = stmt.query_map(orefs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2).unwrap_or(0),
                r.get::<_, i64>(3).unwrap_or(0) != 0,
            ))
        }) {
            output_rows = mapped.flatten().collect();
        }
    }
```

The final `Ok(Json(crate::api::efficiency::aggregate_model_efficiency(&cost_rows, &output_rows)))` line is unchanged — the aggregator now accepts the new tuple shapes.

- [ ] **Step 5: Run the new and existing model-efficiency tests**

Run (from `src-tauri/`): `cargo test --features test-support --test api_reports v2_model_efficiency`
Expected: PASS — all three model_efficiency tests pass (`v2_model_efficiency_buckets_by_dominant_family`, `v2_model_efficiency_respects_model_filter`, `v2_model_efficiency_splits_main_and_subagent`). The two pre-existing tests still pass because their rows all carry `is_subagent = 0` (the column default) and the aggregator emits `role: "main"` for them.

- [ ] **Step 6: Run the full Rust suite**

Run (from `src-tauri/`): `cargo test --features test-support`
Expected: PASS — entire suite green, no regressions.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/tests/api_reports.rs
git commit -m "feat(api): /api/v2/model-efficiency returns main and subagent rows"
```

---

### Task 8: `role` field on the TypeScript DTO

**Files:**
- Modify: `web/src/app/core/api.service.ts` — the `V2ModelEfficiency` interface

- [ ] **Step 1: Add the field**

In `web/src/app/core/api.service.ts`, in the `V2ModelEfficiency` interface, directly after `family: string;` add:

```typescript
export interface V2ModelEfficiency {
  family: string;
  role: 'main' | 'subagent';
  sessions: number;
  total_cost_usd: number;
  cost_per_session: number;
  output_tokens: number;
  cost_per_1k_output: number;
}
```

- [ ] **Step 2: Verify TypeScript still compiles and tests pass**

Run (from `web/`): `npm test`
Expected: PASS — the 4 existing `efficiency.component.spec.ts` tests still pass. The test fixtures' model rows don't yet carry `role`, but the interface change doesn't break the tests because TypeScript object literals in tests don't structurally fail on missing optional members at runtime, and the existing template renders the same — `role` will be added to the template in Task 9.

If `npm test` fails because Vitest's strict typing rejects the literal without `role`, proceed straight to Task 9 (which updates the fixtures with `role`). You can stage the change but commit Tasks 8+9 together — note this in the Task 9 commit.

- [ ] **Step 3: Commit**

```bash
git add web/src/app/core/api.service.ts
git commit -m "feat(web): V2ModelEfficiency gains the role field"
```

---

### Task 9: Frontend Role column

**Files:**
- Modify: `web/src/app/features/efficiency/efficiency.component.html`
- Modify: `web/src/app/features/efficiency/efficiency.component.spec.ts`

- [ ] **Step 1: Update the spec test fixtures and add a subagent-row test**

In `web/src/app/features/efficiency/efficiency.component.spec.ts`, find the `MODELS` const at the top:

```typescript
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
```

Replace it with one main row and one subagent row:

```typescript
const MODELS = [
  {
    family: 'opus',
    role: 'main',
    sessions: 38,
    total_cost_usd: 69.92,
    cost_per_session: 1.84,
    output_tokens: 98480,
    cost_per_1k_output: 0.71,
  },
  {
    family: 'haiku',
    role: 'subagent',
    sessions: 12,
    total_cost_usd: 4.32,
    cost_per_session: 0.36,
    output_tokens: 21000,
    cost_per_1k_output: 0.21,
  },
];
```

Then, inside `describe('EfficiencyComponent', () => { ... })`, add this new test (after the existing un-priced footnote test):

```typescript
  it('renders a subagent role badge for subagent rows', () => {
    const { fixture } = setup();
    const text = fixture.nativeElement.textContent;
    expect(text).toContain('subagent');
    expect(text).toContain('haiku');
  });
```

- [ ] **Step 2: Run the spec to verify it fails**

Run (from `web/`): `npx vitest run efficiency.component`
Expected: FAIL — the `'renders a subagent role badge for subagent rows'` test fails (`'subagent'` not found in DOM text), and possibly the existing `'renders a model-efficiency row'` test still passes since `'opus'` and `'69.92'` are still in the fixture.

- [ ] **Step 3: Update the template**

In `web/src/app/features/efficiency/efficiency.component.html`, find the model cost-efficiency table. In the `<thead>` section, change:

```html
            <tr class="text-[10px] uppercase text-muted">
              <th class="text-left px-4 py-1.5 font-normal">Family</th>
              <th class="text-right px-4 py-1.5 font-normal">Sessions</th>
              <th class="text-right px-4 py-1.5 font-normal">Cost / session</th>
              <th class="text-right px-4 py-1.5 font-normal">$ / 1k output</th>
              <th class="text-right px-4 py-1.5 font-normal pr-4">Total</th>
            </tr>
```

to (add a `Role` `<th>` between Family and Sessions):

```html
            <tr class="text-[10px] uppercase text-muted">
              <th class="text-left px-4 py-1.5 font-normal">Family</th>
              <th class="text-left px-4 py-1.5 font-normal">Role</th>
              <th class="text-right px-4 py-1.5 font-normal">Sessions</th>
              <th class="text-right px-4 py-1.5 font-normal">Cost / session</th>
              <th class="text-right px-4 py-1.5 font-normal">$ / 1k output</th>
              <th class="text-right px-4 py-1.5 font-normal pr-4">Total</th>
            </tr>
```

In the `<tbody>` `@for` block, change the `track` expression and add a new `<td>` for the Role column. The current row:

```html
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
```

becomes:

```html
            @for (m of models(); track m.family + ':' + m.role) {
              <tr class="border-t border-border/40">
                <td class="px-4 py-1.5">
                  <span class="inline-flex items-center gap-1.5">
                    <span class="inline-block w-2 h-2 rounded-full" [style.background-color]="familyColor(m.family)"></span>
                    {{ m.family }}
                  </span>
                </td>
                <td class="px-4 py-1.5">
                  <span class="text-[10px] uppercase tracking-wider"
                        [class]="m.role === 'subagent' ? 'text-accent/80' : 'text-muted'">
                    {{ m.role }}
                  </span>
                </td>
                <td class="px-4 py-1.5 text-right tabular-nums">{{ m.sessions | number }}</td>
                <td class="px-4 py-1.5 text-right tabular-nums">${{ m.cost_per_session | number : '1.2-2' }}</td>
                <td class="px-4 py-1.5 text-right tabular-nums">${{ m.cost_per_1k_output | number : '1.2-2' }}</td>
                <td class="px-4 py-1.5 text-right pr-4 tabular-nums">${{ m.total_cost_usd | number : '1.2-2' }}</td>
              </tr>
            }
```

- [ ] **Step 4: Run the spec to verify it passes**

Run (from `web/`): `npx vitest run efficiency.component`
Expected: PASS — 5 tests pass (the 4 original plus the new subagent-badge test).

- [ ] **Step 5: Run the full Angular suite to confirm no regression**

Run (from `web/`): `npm test`
Expected: PASS — all Vitest tests across the project remain green.

- [ ] **Step 6: Commit**

```bash
git add web/src/app/features/efficiency/efficiency.component.html web/src/app/features/efficiency/efficiency.component.spec.ts
git commit -m "feat(web): show role badge in the model-efficiency table"
```

---

### Task 10: Document the role split

**Files:**
- Modify: `docs/features.md` (the Efficiency section added by the cost-efficiency feature)

- [ ] **Step 1: Read and locate**

Open `docs/features.md` and find the Efficiency section's "Model cost-efficiency" bullet (the third bullet, which today reads: "per model family (`opus` / `sonnet` / `haiku`), the cost per session and cost per 1k output tokens. Each session is attributed wholly to the family that spent the most in it.").

- [ ] **Step 2: Extend the bullet**

Replace the existing "Model cost-efficiency" bullet (the third one in the Efficiency section) with:

```markdown
- **Model cost-efficiency** — per model family (`opus` / `sonnet` / `haiku`),
  split by role: **main** rows attribute each session wholly to its
  dominant-cost family; **subagent** rows aggregate sidechain (subagent) cost
  per family across all sessions in the window. The role split is JSONL-derived
  — sessions that have not been ingested by the JSONL backfill (or session-end
  ingest) will not show subagent rows. Run **Backfill JSONL** in Settings to
  re-tag existing data.
```

- [ ] **Step 3: Commit**

```bash
git add docs/features.md
git commit -m "docs: document the main/subagent split on the Efficiency page"
```

---

## Final verification

After all tasks, run the full suites once more to confirm nothing regressed:

- [ ] Rust: from `src-tauri/`, `cargo test --features test-support` → all green.
- [ ] Angular: from `web/`, `npm test` → all green.
- [ ] Manual smoke (the PR-#28 manual checkbox is now this feature's checkbox): `cargo tauri dev`, open the Efficiency page, run the **Backfill JSONL** action in Settings, then refresh — the model cost-efficiency table should now show one or more **subagent** rows for whichever subagent families your real transcripts contain.
- [ ] Update the PR #28 description: add a short note that the feature now includes the main/subagent split (and replace any "we'll do this in a follow-up" framing if it crept in).

---

## Self-review notes

- **Spec coverage:**
  - Schema column → Task 1.
  - `isSidechain` parse → Task 2.
  - Reducer thread-through → Task 3.
  - JSONL upsert flip → Task 4.
  - Aggregator two-pass (main = dominant-family; subagent = per-family) → Task 6.
  - DTO `role` field → Task 6.
  - Handler SQL adds `is_subagent` (and `model` on the output query) → Task 7.
  - Heal-on-next-ingest backfill story → Tasks 4 (mechanism) + 10 (documentation).
  - Frontend Role column + badge → Task 9.
  - Tests at every layer → Tasks 2, 3, 4, 6, 7, 9.
  - `SeedOpts.is_subagent` option → Task 5 (used by Task 7's integration test).
- **Placeholder scan:** no TBD / TODO. Every code step shows the actual code. The one judgement-call note (Task 4 Step 1: confirm `ingest_derived` is the right entry point) is explicit and bounded — if the entry-point name has drifted, the implementer adjusts the test, not the design.
- **Type consistency:** `aggregate_model_efficiency` signature is identical everywhere it appears (Tasks 6, 7); `ModelEfficiencyRow` field set is identical in Rust DTO (Task 6) and TypeScript interface (Task 8); the `role` value space is `"main" | "subagent"` in every layer. `is_subagent` is `bool` in Rust events / aggregator inputs and `INTEGER 0/1` in SQL — converted at each boundary explicitly (`as i64` on insert, `!= 0` on read).
