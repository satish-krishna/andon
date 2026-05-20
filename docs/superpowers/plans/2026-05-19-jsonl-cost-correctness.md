# JSONL Cost Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the 1.6–3× cost/token overcount on JSONL-ingested sessions by counting each Claude Code API call exactly once.

**Architecture:** Claude Code writes one JSONL `assistant` record per content block, each carrying the full `usage` and a shared `requestId`. The reducer collapses usage to one event per `requestId`; routing becomes binary (OTLP-covered session → JSONL writes no cost/tokens); a `request_id` column with a partial unique index makes a duplicate JSONL row physically impossible.

**Tech Stack:** Rust (rusqlite, axum, serde, tokio), SQLite, Angular 21 (standalone components, signals), Tailwind.

**Spec:** `docs/superpowers/specs/2026-05-19-jsonl-cost-correctness-design.md`

**Conventions:**
- Work on the existing branch `feature/jsonl-ingest` (the spec is already committed there).
- Rust commands run from `src-tauri/`. Web commands run from `web/`.
- Every commit message ends with the trailer `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` (shown in each commit step).
- Conventional Commits, no emojis. TDD: failing test first.

---

### Task 1: Migration v5 — `request_id` columns, partial unique indexes, `session_jsonl_calls`

**Files:**
- Modify: `src-tauri/src/db/migrations.rs`

- [ ] **Step 1: Write the failing test**

Add this test inside the `#[cfg(test)] mod tests` block in `src-tauri/src/db/migrations.rs`, after `v4_creates_jsonl_tables_and_extends_decisions`:

```rust
    #[test]
    fn v5_adds_request_id_and_coverage_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        for tbl in ["cost_entries", "token_usage"] {
            let cols: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({tbl})")).unwrap()
                .query_map([], |r| r.get::<_, String>(1)).unwrap()
                .map(|r| r.unwrap()).collect();
            assert!(cols.contains(&"request_id".to_string()), "{tbl} missing request_id");
        }

        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_jsonl_calls'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "session_jsonl_calls table missing");

        for idx in ["idx_cost_request", "idx_token_request"] {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [idx], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1, "missing index {idx}");
        }

        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(v, 5);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --features test-support v5_adds_request_id`
Expected: FAIL — `session_jsonl_calls table missing` (or version assert) — v5 does not exist yet.

- [ ] **Step 3: Add the migration**

In `src-tauri/src/db/migrations.rs`, add after the `MIGRATION_V4` constant:

```rust
const MIGRATION_V5: &str = r#"
-- Per-API-call identity for JSONL-derived rows. NULL on OTLP-derived rows.
ALTER TABLE cost_entries ADD COLUMN request_id TEXT;
ALTER TABLE token_usage  ADD COLUMN request_id TEXT;

-- Uniqueness enforced ONLY on JSONL rows; OTLP rows (request_id IS NULL) are unconstrained.
CREATE UNIQUE INDEX idx_cost_request
    ON cost_entries(request_id)            WHERE request_id IS NOT NULL;
CREATE UNIQUE INDEX idx_token_request
    ON token_usage(request_id, token_type) WHERE request_id IS NOT NULL;

-- Per-session transcript API-call count; powers partial-OTLP detection.
CREATE TABLE session_jsonl_calls (
    session_id  TEXT PRIMARY KEY,
    api_calls   INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
"#;
```

Then add `(5, MIGRATION_V5)` to the `MIGRATIONS` slice:

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
    (5, MIGRATION_V5),
];
```

- [ ] **Step 4: Bump the three existing version assertions**

In the same file, the tests `v3_adds_repo_columns_and_indexes`, `migrations_are_idempotent_across_runs`, and `v4_creates_jsonl_tables_and_extends_decisions` each end with `assert_eq!(v, 4);`. Change all three to `assert_eq!(v, 5);`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --features test-support migrations`
Expected: PASS — all `migrations::tests` pass, including `v5_adds_request_id_and_coverage_table`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(db): migration v5 — request_id columns and partial unique indexes" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Parse `requestId` from JSONL records

**Files:**
- Modify: `src-tauri/src/jsonl/record.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/jsonl/record.rs`:

```rust
    #[test]
    fn parses_request_id() {
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_abc","message":{"role":"assistant","model":"claude-opus-4-7"}}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.request_id.as_deref(), Some("req_abc"));
    }

    #[test]
    fn missing_request_id_is_none() {
        let r = parse_line(r#"{"type":"assistant","sessionId":"s1"}"#).unwrap();
        assert!(r.request_id.is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --features test-support parses_request_id`
Expected: FAIL — compile error: no field `request_id` on `JsonlRecord`.

- [ ] **Step 3: Add the field**

In `src-tauri/src/jsonl/record.rs`, add to the `JsonlRecord` struct, immediately after the `parent_uuid` field:

```rust
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --features test-support --lib jsonl::record`
Expected: PASS — all `record` tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/jsonl/record.rs
git commit -m "feat(jsonl): parse requestId from transcript records" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Reducer — collapse usage to one event per `requestId`

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs`
- Modify: `src-tauri/src/otlp/ingestor.rs` (match-arm compilation fix only)
- Modify: `src-tauri/tests/jsonl_ingest_writes.rs` (constructor compilation fix only)
- Modify: `src-tauri/tests/jsonl_pipeline.rs` (fixture fix)

- [ ] **Step 1: Write the failing regression test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/jsonl/reducer.rs`:

```rust
    #[test]
    fn multi_record_request_emits_usage_once() {
        // Claude Code splits one API call across multiple records, each carrying
        // the same requestId and identical usage. Usage must be counted once;
        // every tool_use block must still produce a ToolCall.
        let mut r = Reducer::new();
        let rec1 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#;
        let rec2 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.rs"}}]}}"#;
        let rec3 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t2","name":"Grep","input":{}}]}}"#;
        let mut all = vec![];
        for line in [rec1, rec2, rec3] {
            all.extend(r.reduce(&parse_line(line).unwrap()));
        }
        let tok = all.iter().filter(|e| matches!(e, DerivedEvent::TokenUsage { .. })).count();
        let cost = all.iter().filter(|e| matches!(e, DerivedEvent::CostEntry { .. })).count();
        let tools = all.iter().filter(|e| matches!(e, DerivedEvent::ToolCall { .. })).count();
        assert_eq!(tok, 1, "usage counted once per requestId");
        assert_eq!(cost, 1, "cost counted once per requestId");
        assert_eq!(tools, 2, "every tool_use block still recorded");
    }

    #[test]
    fn assistant_without_request_id_emits_no_usage() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":2000}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        assert!(!out.iter().any(|e| matches!(
            e, DerivedEvent::TokenUsage { .. } | DerivedEvent::CostEntry { .. }
        )));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --features test-support multi_record_request_emits_usage_once`
Expected: FAIL — `assert_eq!(tok, 1)` fails with `tok == 3` (usage emitted per record).

- [ ] **Step 3: Add `request_id` to the `DerivedEvent` variants**

In `src-tauri/src/jsonl/reducer.rs`, modify the `TokenUsage` and `CostEntry` variants of the `DerivedEvent` enum to add a `request_id` field:

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
    CostEntry {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        cost_usd: f64,
    },
```

- [ ] **Step 4: Add the `seen_requests` set to `Reducer`**

Replace the `Reducer` struct definition:

```rust
#[derive(Default)]
pub struct Reducer {
    first_turn_seen: bool,
    seen_requests: std::collections::HashSet<String>,
}
```

- [ ] **Step 5: Rewrite the usage block in `reduce_assistant`**

In `reduce_assistant`, replace the entire `if let Some(u) = msg.usage.as_ref() { ... }` block (which currently emits `TokenUsage` and `CostEntry`) with:

```rust
        // Claude Code writes one assistant record per content block; every record
        // of an API call carries the same requestId and the identical usage.
        // Emit token/cost exactly once per requestId, at its first-seen record.
        // Records with no requestId (synthetic / api-error) carry no priceable
        // usage and are skipped — they cannot be safely deduplicated.
        if let (Some(request_id), Some(u)) = (rec.request_id.as_deref(), msg.usage.as_ref()) {
            if self.seen_requests.insert(request_id.to_string())
                && u.input_tokens + u.output_tokens + u.cache_read + u.cache_creation > 0
            {
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
                if let Some(cost) = crate::jsonl::pricing::cost_for(
                    &model,
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_read,
                    u.cache_creation,
                ) {
                    if cost > 0.0 {
                        out.push(DerivedEvent::CostEntry {
                            session_id: sid.to_string(),
                            request_id: request_id.to_string(),
                            ts,
                            model: model.clone(),
                            cost_usd: cost,
                        });
                    }
                }
            }
        }
```

(The `for block in &msg.content { ... }` loop that follows — emitting `ToolCall` / `SubAgentCall` — is unchanged.)

- [ ] **Step 6: Fix the existing reducer test fixture**

In the same file, the test `assistant_emits_token_usage_and_cost` has a fixture line with no `requestId`. Add `"requestId":"req_emit",` to that JSON, immediately after `"sessionId":"s1",`:

```rust
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_emit","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;
```

- [ ] **Step 7: Fix the ingestor match arms (compilation only)**

In `src-tauri/src/otlp/ingestor.rs`, the `ingest_derived` function matches `E::TokenUsage { ... }` and `E::CostEntry { ... }`. Add `request_id: _,` to each pattern so they remain exhaustive. For `E::TokenUsage`:

```rust
                E::TokenUsage {
                    session_id,
                    request_id: _,
                    ts,
                    model,
                    input,
                    output,
                    cache_create,
                    cache_read,
                } => {
```

For `E::CostEntry`:

```rust
                E::CostEntry {
                    session_id,
                    request_id: _,
                    ts,
                    model,
                    cost_usd,
                } => {
```

(The arm bodies are unchanged in this task — Task 4 rewrites them.)

- [ ] **Step 8: Fix the test constructors (compilation only)**

In `src-tauri/tests/jsonl_ingest_writes.rs`, every `DerivedEvent::TokenUsage { ... }` and `DerivedEvent::CostEntry { ... }` literal (there are five, around lines 64, 109, 118, 231, 238) needs a `request_id` field. Add `request_id: "req_test".into(),` immediately after each `session_id:` line. Example for the first one:

```rust
    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(),
        request_id: "req_test".into(),
        ts: 10_000,
        model: "claude-opus-4-7".into(),
        input: 500,
        output: 0,
        cache_create: 0,
        cache_read: 0,
    }];
```

Use a distinct `request_id` value per literal within a test if two events represent different calls (e.g. `"req_a"` / `"req_b"` for the two `TokenUsage` events in `gap_fills_when_otlp_partial`, and matching ids for the paired `CostEntry` events in `gap_fills_cost_when_otlp_partial`). These tests are rewritten wholesale in Task 4; this step only restores compilation.

- [ ] **Step 9: Fix the pipeline test fixture**

In `src-tauri/tests/jsonl_pipeline.rs`, the `backfill_is_idempotent` test's assistant transcript line has no `requestId`. Add `"requestId":"req_idp",` after `"sessionId":"sIDP",`:

```rust
            r#"{"type":"assistant","sessionId":"sIDP","requestId":"req_idp","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":200}}}"#,
```

- [ ] **Step 10: Run the full test suite to verify it passes**

Run: `cargo test --features test-support`
Expected: PASS — all tests pass, including the two new reducer tests. (The window-dedup integration tests still pass; they are rewritten in Task 4.)

- [ ] **Step 11: Commit**

```bash
git add src-tauri/src/jsonl/reducer.rs src-tauri/src/otlp/ingestor.rs src-tauri/tests/jsonl_ingest_writes.rs src-tauri/tests/jsonl_pipeline.rs
git commit -m "fix(jsonl): collapse token/cost to one event per requestId" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Binary routing and structural `request_id` inserts

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs`
- Modify: `src-tauri/src/jsonl/reconciler.rs`
- Modify: `src-tauri/tests/jsonl_ingest_writes.rs`

- [ ] **Step 1: Replace the three window-dedup integration tests**

In `src-tauri/tests/jsonl_ingest_writes.rs`, delete the tests `dedups_token_usage_against_otlp_within_window`, `gap_fills_when_otlp_partial`, and `gap_fills_cost_when_otlp_partial`. Replace them with:

```rust
#[test]
fn otlp_coverage_skips_jsonl_cost_and_tokens() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let conn = pool.get().unwrap();
    // An OTLP token row exists for this session — it is OTLP-covered.
    conn.execute(
        "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) \
         VALUES ('s1', 100, 'claude-opus-4-7', 'input', 500)",
        [],
    )
    .unwrap();
    drop(conn);

    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: 9000,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 2000,
            cache_create: 0,
            cache_read: 0,
        },
        DerivedEvent::CostEntry {
            session_id: "s1".into(),
            request_id: "req_x".into(),
            ts: 9000,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.42,
        },
    ];
    let (tokens_written, cost_written) = ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    assert_eq!((tokens_written, cost_written), (0, 0));

    let conn = pool.get().unwrap();
    let tok: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='s1'", [], |r| r.get(0))
        .unwrap();
    let cost: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tok, 1, "only the original OTLP token row remains");
    assert_eq!(cost, 0, "no JSONL cost written for an OTLP-covered session");
}

#[test]
fn jsonl_only_writes_cost_and_tokens_with_request_id() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s2".into(),
            request_id: "req_y".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 500,
            cache_create: 0,
            cache_read: 0,
        },
        DerivedEvent::CostEntry {
            session_id: "s2".into(),
            request_id: "req_y".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
        },
    ];
    let (tokens_written, cost_written) = ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    assert_eq!(tokens_written, 2, "input + output rows");
    assert_eq!(cost_written, 1);

    let conn = pool.get().unwrap();
    let rid: String = conn
        .query_row("SELECT request_id FROM cost_entries WHERE session_id='s2'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rid, "req_y");
}

#[test]
fn jsonl_reingest_is_idempotent_via_request_id() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::TokenUsage {
            session_id: "s3".into(),
            request_id: "req_z".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            input: 1000,
            output: 500,
            cache_create: 0,
            cache_read: 0,
        },
        DerivedEvent::CostEntry {
            session_id: "s3".into(),
            request_id: "req_z".into(),
            ts: 100,
            model: "claude-opus-4-7".into(),
            cost_usd: 0.05,
        },
    ];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let (t2, c2) = ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    assert_eq!((t2, c2), (0, 0), "second ingest writes nothing");

    let conn = pool.get().unwrap();
    let cost: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='s3'", [], |r| r.get(0))
        .unwrap();
    let tok: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_usage WHERE session_id='s3'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost, 1);
    assert_eq!(tok, 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --features test-support otlp_coverage_skips_jsonl_cost_and_tokens`
Expected: FAIL — `assert_eq!((tokens_written, cost_written), (0, 0))` fails: the current code writes rows for `Coverage::Otlp`.

- [ ] **Step 3: Rewrite the `TokenUsage` arm of `ingest_derived`**

In `src-tauri/src/otlp/ingestor.rs`, replace the entire `E::TokenUsage { ... } => { ... }` arm body with:

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
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) {
                        for (kind, n) in [
                            ("input", *input),
                            ("output", *output),
                            ("cacheRead", *cache_read),
                            ("cacheCreation", *cache_create),
                        ] {
                            if n > 0 {
                                let affected = tx
                                    .execute(
                                        "INSERT INTO token_usage
                                           (session_id, request_id, timestamp, model, token_type, count)
                                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                                         ON CONFLICT(request_id, token_type)
                                           WHERE request_id IS NOT NULL DO NOTHING",
                                        params![session_id, request_id, ts, model, kind, n],
                                    )
                                    .unwrap_or(0);
                                tokens_written += affected as i64;
                            }
                        }
                    }
                }
```

- [ ] **Step 4: Rewrite the `CostEntry` arm of `ingest_derived`**

Replace the entire `E::CostEntry { ... } => { ... }` arm body with:

```rust
                E::CostEntry {
                    session_id,
                    request_id,
                    ts,
                    model,
                    cost_usd,
                } => {
                    if matches!(coverage, Coverage::JsonlOnly) && *cost_usd > 0.0 {
                        let affected = tx
                            .execute(
                                "INSERT INTO cost_entries
                                   (session_id, request_id, timestamp, model, cost_usd)
                                 VALUES (?1, ?2, ?3, ?4, ?5)
                                 ON CONFLICT(request_id)
                                   WHERE request_id IS NOT NULL DO NOTHING",
                                params![session_id, request_id, ts, model, cost_usd],
                            )
                            .unwrap_or(0);
                        cost_written += affected as i64;
                    }
                }
```

- [ ] **Step 5: Remove the `data_source = 'mixed'` flip from the `SessionLifecycle` arm**

In the `E::SessionLifecycle { ... }` arm, delete the `if matches!(coverage, Coverage::Otlp) { ... }` block containing the `UPDATE sessions SET data_source = 'mixed' ...` statement. Keep the `INSERT OR IGNORE INTO sessions ...` statement.

- [ ] **Step 6: Rename the local counters and fix the import**

At the top of `ingest_derived`, rename the two local counters:

```rust
        let mut tokens_written: i64 = 0;
        let mut cost_written: i64 = 0;
```

The function's final line becomes `Ok((tokens_written, cost_written))`.

Change the `use` line at the top of `ingest_derived` from:

```rust
        use crate::jsonl::reconciler::{cost_row_already_covered, token_row_already_covered, Coverage};
```

to:

```rust
        use crate::jsonl::reconciler::Coverage;
```

Delete the `const DEDUP_WINDOW_MS: i64 = 5_000;` line.

- [ ] **Step 7: Refine `coverage_for` and delete the window-dedup helpers**

In `src-tauri/src/jsonl/reconciler.rs`:

(a) Change the SQL inside `coverage_for` so only OTLP-written rows count. OTLP rows have `request_id IS NULL`; JSONL's own rows have a non-NULL `request_id`. Without this, a JSONL-only session would look OTLP-covered the second time its transcript is ingested, and re-ingest would silently drop every new turn. Replace the `query_row` call body with:

```rust
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage
             WHERE session_id = ?1 AND request_id IS NULL
             LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
```

(b) Delete the entire `pub fn token_row_already_covered(...)` and `pub fn cost_row_already_covered(...)` functions. In the `#[cfg(test)] mod tests` block, delete the tests `token_row_already_covered_within_5s_window` and `cost_row_already_covered_within_5s_window`. Keep `coverage_for`, the `Coverage` enum, and the tests `no_token_usage_is_jsonl_only` and `token_usage_present_is_otlp`.

(c) Add this test to the `mod tests` block (it uses the existing `pool()` helper):

```rust
    #[test]
    fn jsonl_token_rows_do_not_imply_otlp_coverage() {
        let p = pool();
        let c = p.get().unwrap();
        c.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('sJ', 0)",
            [],
        )
        .unwrap();
        // A JSONL-written token row carries a non-NULL request_id and must NOT
        // make the session look OTLP-covered.
        c.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('sJ', 'req_j', 0, 'claude-opus-4-7', 'input', 1)",
            [],
        )
        .unwrap();
        assert_eq!(coverage_for(&p, "sJ").unwrap(), Coverage::JsonlOnly);
    }
```

- [ ] **Step 8: Run the full test suite to verify it passes**

Run: `cargo test --features test-support`
Expected: PASS — all tests pass, including the three new routing tests.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/otlp/ingestor.rs src-tauri/src/jsonl/reconciler.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(jsonl): binary OTLP-wins routing with request_id ON CONFLICT inserts" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: End-to-end multi-record cost integration test

**Files:**
- Modify: `src-tauri/tests/jsonl_pipeline.rs`

- [ ] **Step 1: Write the test**

Add to `src-tauri/tests/jsonl_pipeline.rs`:

```rust
#[tokio::test]
async fn backfill_counts_multi_record_request_once() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    // One API call (req_M) split across three assistant records, each carrying
    // the identical usage — exactly how Claude Code writes transcripts.
    write_transcript(home.path(), "m", &[
        r#"{"type":"user","sessionId":"sM","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"text","text":"x"}]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}]}}"#,
        r#"{"type":"assistant","sessionId":"sM","requestId":"req_M","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t2","name":"Grep","input":{}}]}}"#,
    ]);

    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    let conn = pool.get().unwrap();
    // 1M input tokens of opus-4-7 = $15.00, counted exactly once despite 3 records
    // and 2 backfill runs.
    let cost: f64 = conn
        .query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id='sM'", [], |r| r.get(0))
        .unwrap();
    assert!((cost - 15.0).abs() < 1e-6, "expected $15.00, got {cost}");
    let cost_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sM'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_rows, 1, "one cost row per API call");
    let input_tokens: i64 = conn
        .query_row("SELECT COALESCE(SUM(count), 0) FROM token_usage WHERE session_id='sM' AND token_type='input'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(input_tokens, 1_000_000, "input tokens counted once");
}

#[tokio::test]
async fn reingest_of_grown_transcript_adds_new_turns() {
    // A JSONL-only session must stay JSONL-only across re-ingests: the second
    // backfill picks up the new turn (req_g2) without duplicating req_g1.
    // Guards the `coverage_for` `request_id IS NULL` refinement from Task 4.
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();

    write_transcript(home.path(), "g", &[
        r#"{"type":"user","sessionId":"sG","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
    ]);
    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    // The session resumes — the transcript file grows a second API call.
    write_transcript(home.path(), "g", &[
        r#"{"type":"user","sessionId":"sG","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
        r#"{"type":"assistant","sessionId":"sG","requestId":"req_g2","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
    ]);
    jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    let conn = pool.get().unwrap();
    let cost_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM cost_entries WHERE session_id='sG'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cost_rows, 2, "new turn ingested, old turn not duplicated");
}
```

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cargo test --features test-support backfill_counts_multi_record_request_once reingest_of_grown_transcript_adds_new_turns`
Expected: PASS — Tasks 3 and 4 make this correct end-to-end.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/jsonl_pipeline.rs
git commit -m "test(jsonl): end-to-end multi-record costing and grown-transcript re-ingest" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Record per-session API-call count; rename `IngestStats` fields

**Files:**
- Modify: `src-tauri/src/jsonl/mod.rs`
- Modify: `src-tauri/src/api/dto.rs`
- Modify: `src-tauri/tests/jsonl_pipeline.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/tests/jsonl_pipeline.rs`:

```rust
#[tokio::test]
async fn backfill_records_api_call_count() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "c", &[
        r#"{"type":"user","sessionId":"sC","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_1","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
        r#"{"type":"assistant","sessionId":"sC","requestId":"req_2","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20}}}"#,
    ]);

    let pool_arc = Arc::clone(&pool);
    jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();

    let conn = pool.get().unwrap();
    let api_calls: i64 = conn
        .query_row("SELECT api_calls FROM session_jsonl_calls WHERE session_id='sC'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(api_calls, 2, "two distinct requestIds across three records");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features test-support backfill_records_api_call_count`
Expected: FAIL — `session_jsonl_calls` has no row for `sC` (`QueryReturnedNoRows`).

- [ ] **Step 3: Add the `session_jsonl_calls` upsert helper to `mod.rs`**

In `src-tauri/src/jsonl/mod.rs`, add this function near the other helpers (e.g. after `finalise_run`):

```rust
fn record_jsonl_calls(pool: &Arc<DbPool>, session_id: &str, api_calls: i64) {
    let Ok(conn) = pool.get() else { return };
    let _ = conn.execute(
        "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET api_calls = ?2, updated_at = ?3",
        params![session_id, api_calls, now_ms()],
    );
}
```

- [ ] **Step 4: Call the helper and remove the dead log line in `ingest_one_inner`**

In `ingest_one_inner`, the `for (sid, events) in events_by_session { ... }` loop currently ends with a `match fresh_ing.ingest_derived(...)` block and `stats.sessions_added += 1;`. Inside the `Ok(...)` branch of that match, delete the gap-fill log block:

```rust
                    // Only an OTLP-covered session can be "gap-filled" — for a
                    // JSONL-only session these rows are the primary import, not gaps.
                    if matches!(cov, reconciler::Coverage::Otlp) && tokens + cost > 0 {
                        tracing::info!(
                            sid,
                            tokens_filled = tokens,
                            cost_filled = cost,
                            "JSONL gap-filled rows for OTLP-partial session"
                        );
                    }
```

Then, immediately before `stats.sessions_added += 1;`, add:

```rust
            let api_calls = events
                .iter()
                .filter_map(|e| match e {
                    reducer::DerivedEvent::TokenUsage { request_id, .. } => Some(request_id.clone()),
                    _ => None,
                })
                .collect::<std::collections::HashSet<_>>()
                .len() as i64;
            record_jsonl_calls(&pool_clone, &sid, api_calls);
```

- [ ] **Step 5: Rename the `IngestStats` fields**

In `src-tauri/src/jsonl/mod.rs`, rename the `IngestStats` fields `tokens_filled` → `tokens_written` and `cost_filled` → `cost_written` in the struct definition. Update the two accumulation sites: in `backfill` (`stats.tokens_filled += s.tokens_filled;` → `stats.tokens_written += s.tokens_written;` and likewise for cost) and in `ingest_one_inner` (`stats.tokens_filled += tokens;` → `stats.tokens_written += tokens;` and likewise for cost).

- [ ] **Step 6: Update the DTO**

In `src-tauri/src/api/dto.rs`, in `JsonlBackfillResponse` rename `tokens_filled` → `tokens_written` and `cost_filled` → `cost_written`. In the `From<crate::jsonl::IngestStats>` impl, update both mappings: `tokens_written: s.tokens_written,` and `cost_written: s.cost_written,`.

- [ ] **Step 7: Update the pipeline test references**

In `src-tauri/tests/jsonl_pipeline.rs`, the `backfill_is_idempotent` test references `s1.tokens_filled`, `s2.tokens_filled`, `s2.cost_filled`. Rename them to `s1.tokens_written`, `s2.tokens_written`, `s2.cost_written`.

- [ ] **Step 8: Run the full test suite to verify it passes**

Run: `cargo test --features test-support`
Expected: PASS — all tests pass, including `backfill_records_api_call_count`.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/jsonl/mod.rs src-tauri/src/api/dto.rs src-tauri/tests/jsonl_pipeline.rs
git commit -m "feat(jsonl): record per-session API-call count; rename IngestStats fields" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: `GET /api/jsonl/coverage-gaps` endpoint

**Files:**
- Modify: `src-tauri/src/api/dto.rs`
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/tests/api_jsonl.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/tests/api_jsonl.rs`:

```rust
#[tokio::test]
async fn coverage_gaps_flags_partial_otlp_session() {
    let (pool, _g) = common::fixture_pool();
    {
        let conn = pool.get().unwrap();
        // 'partial': transcript shows 5 API calls, OTLP recorded only 2 → flagged.
        conn.execute(
            "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at) VALUES ('partial', 5, 0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('partial', NULL, 100, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('partial', NULL, 200, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        // 'full': transcript and OTLP agree (2 == 2) → NOT flagged.
        conn.execute(
            "INSERT INTO session_jsonl_calls (session_id, api_calls, updated_at) VALUES ('full', 2, 0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('full', NULL, 100, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, request_id, timestamp, model, token_type, count) \
             VALUES ('full', NULL, 200, 'claude-opus-4-7', 'input', 1)",
            [],
        ).unwrap();
    }
    let (router, _g2) = test_router(&pool);
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/jsonl/coverage-gaps")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let gaps: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = gaps.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only the partial session is flagged");
    assert_eq!(arr[0]["session_id"], "partial");
    assert_eq!(arr[0]["jsonl_calls"], 5);
    assert_eq!(arr[0]["otlp_calls"], 2);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features test-support coverage_gaps_flags_partial_otlp_session`
Expected: FAIL — the route does not exist; the response status is `404`, not `200`.

- [ ] **Step 3: Add the `CoverageGap` DTO**

In `src-tauri/src/api/dto.rs`, add after `JsonlIngestRunEntry`:

```rust
#[derive(Debug, serde::Serialize)]
pub struct CoverageGap {
    pub session_id: String,
    pub jsonl_calls: i64,
    pub otlp_calls: i64,
}
```

- [ ] **Step 4: Add the route handler**

In `src-tauri/src/api/routes.rs`, add after the `jsonl_ingest_runs` handler:

```rust
#[tracing::instrument(skip(state))]
async fn jsonl_coverage_gaps(
    State(state): State<ApiState>,
) -> Json<Vec<crate::api::dto::CoverageGap>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        // OTLP-covered sessions (otlp_calls > 0) whose transcript shows more API
        // calls than OTLP recorded — a likely mid-session loss of OTel coverage.
        let Ok(mut stmt) = conn.prepare(
            "SELECT session_id, jsonl_calls, otlp_calls FROM (
                 SELECT sjc.session_id AS session_id,
                        sjc.api_calls  AS jsonl_calls,
                        (SELECT COUNT(DISTINCT timestamp) FROM token_usage tu
                         WHERE tu.session_id = sjc.session_id
                           AND tu.request_id IS NULL) AS otlp_calls
                 FROM session_jsonl_calls sjc
             )
             WHERE otlp_calls > 0 AND jsonl_calls > otlp_calls
             ORDER BY (jsonl_calls - otlp_calls) DESC",
        ) else {
            return vec![];
        };
        stmt.query_map([], |r| {
            Ok(crate::api::dto::CoverageGap {
                session_id: r.get(0)?,
                jsonl_calls: r.get(1)?,
                otlp_calls: r.get(2)?,
            })
        })
        .map(|i| i.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Json(rows)
}
```

- [ ] **Step 5: Register the route**

In `src-tauri/src/api/routes.rs`, in the `router` function, add after the `.route("/api/jsonl/ingest-runs", get(jsonl_ingest_runs))` line:

```rust
        .route("/api/jsonl/coverage-gaps", get(jsonl_coverage_gaps))
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --features test-support coverage_gaps_flags_partial_otlp_session`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/api/dto.rs src-tauri/src/api/routes.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): GET /api/jsonl/coverage-gaps for partial-OTLP sessions" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Web — field rename, coverage-gaps card

**Files:**
- Modify: `web/src/app/core/models.ts`
- Modify: `web/src/app/core/api.service.ts`
- Modify: `web/src/app/features/settings/settings.component.ts`
- Modify: `web/src/app/features/diagnostics/diagnostics.component.ts`
- Modify: `web/src/app/features/diagnostics/diagnostics.component.html`

- [ ] **Step 1: Update the models**

In `web/src/app/core/models.ts`, change the `JsonlBackfillResponse` interface fields `tokens_filled` / `cost_filled` to `tokens_written` / `cost_written`:

```ts
export interface JsonlBackfillResponse {
  files_processed: number; records_processed: number; records_errored: number;
  sessions_added: number; tokens_written: number; cost_written: number; duration_ms: number;
}
```

Add a new interface after it:

```ts
export interface CoverageGap {
  session_id: string; jsonl_calls: number; otlp_calls: number;
}
```

- [ ] **Step 2: Update the API service**

In `web/src/app/core/api.service.ts`, add `CoverageGap` to the model import list at the top. Add this method next to `jsonlIngestRuns`:

```ts
  jsonlCoverageGaps(): Observable<CoverageGap[]> {
    return this.http.get<CoverageGap[]>(`${BASE}/api/jsonl/coverage-gaps`);
  }
```

- [ ] **Step 3: Update the settings toast**

In `web/src/app/features/settings/settings.component.ts`, in `ingestJsonl()`, the `next` handler references `s.tokens_filled` and `s.cost_filled`. Update the `filled` line:

```ts
        const written = s.tokens_written + s.cost_written;
        const tail = written > 0 ? ` · wrote ${written} cost/token rows from JSONL` : '';
```

(The line below it uses `tail` in the toast message — leave that as is.)

- [ ] **Step 4: Add coverage-gaps fetch to the diagnostics component**

In `web/src/app/features/diagnostics/diagnostics.component.ts`:

Add `CoverageGap` to the import from `'../../core/models'`:

```ts
import { CoverageGap, JsonlErrorEntry } from '../../core/models';
```

Add a signal next to `jsonlErrors`:

```ts
  coverageGaps = signal<CoverageGap[]>([]);
```

In `refreshDiag()`, add a fetch next to the `jsonlErrors` subscription:

```ts
    this.api.jsonlCoverageGaps().subscribe((g) => this.coverageGaps.set(g));
```

- [ ] **Step 5: Add the coverage-gaps card to the diagnostics template**

In `web/src/app/features/diagnostics/diagnostics.component.html`, add this `<section>` immediately after the closing `</section>` of the "JSONL parse errors" panel (before the `} @else {` that follows):

```html
    <section class="panel">
      <div class="panel-title">Possible OTLP coverage gaps</div>
      <div class="panel-body">
        @if (coverageGaps().length === 0) {
          <div class="text-muted text-xs font-mono py-4">No coverage gaps detected.</div>
        } @else {
          <p class="text-xs text-warn font-mono mb-2">
            {{ coverageGaps().length }} session(s) where the transcript shows more API calls than OTLP recorded:
          </p>
          <ul class="text-[11px] font-mono space-y-1 max-h-64 overflow-y-auto">
            @for (g of coverageGaps(); track g.session_id) {
              <li class="border-b border-border/30 py-1">
                <span class="break-all">{{ g.session_id }}</span>
                <span class="text-muted"> — JSONL {{ g.jsonl_calls }} vs OTLP {{ g.otlp_calls }}</span>
              </li>
            }
          </ul>
        }
      </div>
    </section>
```

- [ ] **Step 6: Verify the web build and tests**

Run: `cd ../web; npm run build`
Expected: build succeeds with no type errors.

Run: `npm test`
Expected: PASS — Vitest suite passes.

- [ ] **Step 7: Commit**

```bash
git add web/src/app/core/models.ts web/src/app/core/api.service.ts web/src/app/features/settings/settings.component.ts web/src/app/features/diagnostics/diagnostics.component.ts web/src/app/features/diagnostics/diagnostics.component.html
git commit -m "feat(web): coverage-gaps card; rename backfill response fields" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: Documentation

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/superpowers/specs/2026-05-19-jsonl-gap-fill-design.md`
- Modify: `docs/superpowers/plans/2026-05-19-jsonl-gap-fill.md`

- [ ] **Step 1: Update `docs/architecture.md`**

Find the section describing OTLP-vs-JSONL routing / JSONL ingestion (search for "JSONL" and "reconcil"). Update it so it states:

- A session is either OTLP-covered or JSONL-only; routing is binary. OTLP-covered → JSONL contributes no cost or token rows.
- The reducer collapses usage to one event per Claude Code `requestId`; JSONL cost/token rows store `request_id` with a partial unique index that makes a duplicate impossible.
- Partial OTLP coverage is surfaced on the Diagnostics page via `GET /api/jsonl/coverage-gaps`, not merged.

Remove any description of the per-row 5-second-window dedup.

- [ ] **Step 2: Mark the superseded gap-fill spec**

At the very top of `docs/superpowers/specs/2026-05-19-jsonl-gap-fill-design.md`, insert:

```markdown
> **SUPERSEDED (2026-05-19)** by `docs/superpowers/specs/2026-05-19-jsonl-cost-correctness-design.md`.
> The per-row 5-second-window dedup described below could not catch the real
> double-count (see that spec's Motivation). Kept for historical context only.

```

- [ ] **Step 3: Mark the superseded gap-fill plan**

At the very top of `docs/superpowers/plans/2026-05-19-jsonl-gap-fill.md`, insert:

```markdown
> **SUPERSEDED (2026-05-19)** by `docs/superpowers/plans/2026-05-19-jsonl-cost-correctness.md`.

```

- [ ] **Step 4: Verify nothing is broken**

Run: `cd src-tauri; cargo test --features test-support`
Expected: PASS — full suite still green (docs-only changes).

- [ ] **Step 5: Commit**

```bash
git add docs/architecture.md docs/superpowers/specs/2026-05-19-jsonl-gap-fill-design.md docs/superpowers/plans/2026-05-19-jsonl-gap-fill.md
git commit -m "docs: binary JSONL routing; mark gap-fill spec superseded" -m "Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Final verification

After all tasks, from `src-tauri/`:

- [ ] Run `cargo test --features test-support` — full Rust suite passes.
- [ ] Run `cargo clippy --features test-support --all-targets` — no new warnings.
- [ ] From `web/`, run `npm run build` and `npm test` — both pass.
- [ ] Manual: delete the dev `data.db`, run `cargo tauri dev`, click Settings → "Ingest JSONL history", confirm the Overview cost figure is plausible (not inflated) and the Diagnostics "Possible OTLP coverage gaps" card renders.
