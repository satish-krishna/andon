# Sessions Totals Row + Line-Change Split — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a grand totals row to the Sessions page, a per-session Lines column, and a Code/Docs/Other split of summed line changes.

**Architecture:** The `v2_sessions` API handler is the single place that knows which sessions are "in view", so it computes everything: per-row `lines_added`/`lines_removed` and a new `totals` object (including the Code/Docs/Other split). The Angular Sessions page only renders — pure split math is extracted into a testable helper module.

**Tech Stack:** Rust (axum, rusqlite, serde, insta snapshots), Angular 21 (standalone components, signals), Vitest, Tailwind.

**Spec:** `docs/superpowers/specs/2026-05-21-sessions-totals-and-line-split-design.md`

**Branch:** `feature/sessions-totals-line-split` (already checked out).

---

## File structure

| File | Responsibility | Change |
|---|---|---|
| `src-tauri/src/api/routes.rs` | `change_kind()` classifier; `v2_sessions` per-row lines + totals | Modify |
| `src-tauri/src/api/dto.rs` | `LinePair`, `LineSplit`, `SessionTotals`; `totals` on `SessionListResponse` | Modify |
| `src-tauri/tests/api_sessions.rs` | Integration tests for the new totals | Modify |
| `src-tauri/tests/snapshots/api_sessions__v2_sessions_shape.snap` | Snapshot of the v2 response shape | Regenerate |
| `web/src/app/core/api.service.ts` | TypeScript types for the new response fields | Modify |
| `web/src/app/features/sessions/line-split.ts` | Pure helper: `LineSplit` → bar segments | Create |
| `web/src/app/features/sessions/line-split.spec.ts` | Unit tests for the helper | Create |
| `web/src/app/features/sessions/sessions.component.ts` | `loadSessions()` refactor; `totals` signal; computeds | Modify |
| `web/src/app/features/sessions/sessions.component.html` | Lines column; `colspan` 11→12; `<tfoot>` | Modify |
| `docs/features.md` | Document the totals row + Lines column | Modify |

---

## Task 1: Backend — classifier, DTOs, and `v2_sessions` totals

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (classifier near `lang_from_path` ~line 1979; `v2_sessions` ~lines 1722-1808; tests in `mod tests` ~line 2570)
- Modify: `src-tauri/src/api/dto.rs` (after `SessionListResponse`, ~line 146)
- Test: `src-tauri/tests/api_sessions.rs` (append after the last test, ~line 535)
- Regenerate: `src-tauri/tests/snapshots/api_sessions__v2_sessions_shape.snap`

All commands in this task run from the `src-tauri/` directory.

- [ ] **Step 1: Write the failing classifier unit test**

In `src-tauri/src/api/routes.rs`, inside the existing `#[cfg(test)] mod tests { ... }` block (starts ~line 2570), add this test as the last item before the closing `}`:

```rust
    #[test]
    fn change_kind_buckets_paths_three_ways() {
        // Code: named languages, including config files.
        assert_eq!(change_kind("src/main.rs"), ChangeKind::Code);
        assert_eq!(change_kind("Cargo.toml"), ChangeKind::Code);
        assert_eq!(change_kind("web/package.json"), ChangeKind::Code);
        assert_eq!(change_kind("a.b.rs"), ChangeKind::Code);
        // Docs: prose extensions, case-insensitive.
        assert_eq!(change_kind("README.md"), ChangeKind::Docs);
        assert_eq!(change_kind("notes.txt"), ChangeKind::Docs);
        assert_eq!(change_kind("docs/guide.rst"), ChangeKind::Docs);
        assert_eq!(change_kind("CHANGELOG.MD"), ChangeKind::Docs);
        // Other: nothing lang_from_path can name.
        assert_eq!(change_kind("Makefile"), ChangeKind::Other);
        assert_eq!(change_kind("data.xyz"), ChangeKind::Other);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features test-support change_kind_buckets_paths_three_ways`
Expected: FAIL — compile error, `cannot find function change_kind` / `cannot find type ChangeKind`.

- [ ] **Step 3: Add the `ChangeKind` enum and `change_kind()` function**

In `src-tauri/src/api/routes.rs`, immediately **after** the closing `}` of `fn lang_from_path` (the function ends ~line 2001), add:

```rust
/// Coarse category for a changed file, derived from its path extension.
/// Used by `v2_sessions` to split aggregate line changes three ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Code,
    Docs,
    Other,
}

/// Bucket a file path into Code / Docs / Other.
///
/// Docs = prose file extensions. Other = anything `lang_from_path` cannot
/// name. Everything else — including config files like `.toml` / `.json` —
/// is Code. Classification is case-insensitive.
fn change_kind(path: &str) -> ChangeKind {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "md" | "markdown" | "mdx" | "txt" | "rst" | "adoc" | "asciidoc" => ChangeKind::Docs,
        _ if lang_from_path(path) == "other" => ChangeKind::Other,
        _ => ChangeKind::Code,
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --features test-support change_kind_buckets_paths_three_ways`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): add change_kind() file classifier"
```

- [ ] **Step 6: Write the failing integration tests**

In `src-tauri/tests/api_sessions.rs`, append after the last test (after the closing `}` of `cost_source_reflects_request_id`, ~line 535):

```rust
// ---------------------------------------------------------------------------
// 10. v2_sessions_reports_line_totals_and_split
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_sessions_reports_line_totals_and_split() {
    let (pool, _db_dir) = common::fixture_pool();

    // Session A: a code file and a docs file.
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "lines-a".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 1.0,
            model: "claude-sonnet".into(),
            files: vec![
                common::FileChange { path: "src/lib.rs", added: 100, removed: 20 },
                common::FileChange { path: "README.md", added: 30, removed: 5 },
            ],
            ..Default::default()
        },
    );
    // Session B: a config file (-> Code) and an unclassifiable file (-> Other).
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "lines-b".into(),
            started_at_ms: Some(ms_ago(1)),
            cost_usd: 0.5,
            model: "claude-haiku".into(),
            files: vec![
                common::FileChange { path: "Cargo.toml", added: 8, removed: 2 },
                common::FileChange { path: "Makefile", added: 4, removed: 1 },
            ],
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let from = ms_ago(5);
    let to = anchor_ms() + 1000;
    let url = format!("/api/v2/sessions?from={from}&to={to}");
    let (status, body) = get_json(router, &url).await;

    assert_eq!(status, StatusCode::OK);

    // Per-row lines.
    let pick = |id: &str| -> Value {
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["session_id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(pick("lines-a")["lines_added"], 130); // 100 + 30
    assert_eq!(pick("lines-a")["lines_removed"], 25); // 20 + 5
    assert_eq!(pick("lines-b")["lines_added"], 12); // 8 + 4
    assert_eq!(pick("lines-b")["lines_removed"], 3); // 2 + 1

    // Totals block.
    let t = &body["totals"];
    assert_eq!(t["sessions"], 2);
    assert_eq!(t["lines_added"], 142); // 130 + 12
    assert_eq!(t["lines_removed"], 28); // 25 + 3

    // Code = src/lib.rs + Cargo.toml ; Docs = README.md ; Other = Makefile.
    assert_eq!(t["lines"]["code"]["added"], 108); // 100 + 8
    assert_eq!(t["lines"]["code"]["removed"], 22); // 20 + 2
    assert_eq!(t["lines"]["docs"]["added"], 30);
    assert_eq!(t["lines"]["docs"]["removed"], 5);
    assert_eq!(t["lines"]["other"]["added"], 4);
    assert_eq!(t["lines"]["other"]["removed"], 1);

    // The grand total equals the sum of the three buckets.
    let sum_added = t["lines"]["code"]["added"].as_i64().unwrap()
        + t["lines"]["docs"]["added"].as_i64().unwrap()
        + t["lines"]["other"]["added"].as_i64().unwrap();
    assert_eq!(t["lines_added"].as_i64().unwrap(), sum_added);
}

// ---------------------------------------------------------------------------
// 11. v2_sessions_totals_are_zero_without_file_changes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v2_sessions_totals_are_zero_without_file_changes() {
    let (pool, _db_dir) = common::fixture_pool();
    common::seed_session(
        &pool,
        &common::SeedOpts {
            session_id: "nofiles-1".into(),
            started_at_ms: Some(anchor_ms()),
            cost_usd: 2.0,
            model: "claude-sonnet".into(),
            ..Default::default()
        },
    );

    let (router, _router_dir) = common::test_router(&pool);
    let from = ms_ago(5);
    let to = anchor_ms() + 1000;
    let (status, body) =
        get_json(router, &format!("/api/v2/sessions?from={from}&to={to}")).await;

    assert_eq!(status, StatusCode::OK);
    let t = &body["totals"];
    assert_eq!(t["sessions"], 1);
    assert_eq!(t["lines_added"], 0);
    assert_eq!(t["lines_removed"], 0);
    assert_eq!(t["lines"]["code"]["added"], 0);
    assert_eq!(t["lines"]["other"]["removed"], 0);
    // Non-line totals still aggregate.
    assert!((t["cost_usd"].as_f64().unwrap() - 2.0).abs() < 1e-9);
}
```

- [ ] **Step 7: Run the new tests to verify they fail**

Run: `cargo test --features test-support v2_sessions_reports_line_totals_and_split v2_sessions_totals_are_zero_without_file_changes`
Expected: FAIL — `body["totals"]` is `Null`, so the `t["sessions"]` / `t["lines"]...` assertions fail (and `lines_added` on rows is missing).

- [ ] **Step 8: Add the DTOs**

In `src-tauri/src/api/dto.rs`, replace the `SessionListResponse` struct (~lines 142-146):

```rust
#[derive(serde::Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<serde_json::Value>,
    pub coverage: CoverageHint,
}
```

with:

```rust
#[derive(serde::Serialize, Default, Clone, Copy)]
pub struct LinePair {
    pub added: i64,
    pub removed: i64,
}

#[derive(serde::Serialize, Default)]
pub struct LineSplit {
    pub code: LinePair,
    pub docs: LinePair,
    pub other: LinePair,
}

#[derive(serde::Serialize)]
pub struct SessionTotals {
    pub sessions: i64,
    pub cost_usd: f64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub accepts: i64,
    pub rejects: i64,
    pub aborts: i64,
    pub decisions: i64,
    pub duration_seconds: f64,
    /// Sum of all three buckets' `added`.
    pub lines_added: i64,
    /// Sum of all three buckets' `removed`.
    pub lines_removed: i64,
    pub lines: LineSplit,
}

#[derive(serde::Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<serde_json::Value>,
    pub coverage: CoverageHint,
    pub totals: SessionTotals,
}
```

(`routes.rs` imports `dto::*`, so the new types need no import change there.)

- [ ] **Step 9: Add per-row lines to the `v2_sessions` SQL**

In `src-tauri/src/api/routes.rs`, in `v2_sessions`, find the end of the main `SELECT` (~lines 1737-1742) and replace:

```rust
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source
         FROM sessions s
```

with:

```rust
                (CASE
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id AND request_id IS NULL) THEN 'otlp'
                   WHEN EXISTS(SELECT 1 FROM cost_entries WHERE session_id = s.session_id) THEN 'jsonl'
                   ELSE NULL
                 END) AS cost_source,
                COALESCE((SELECT SUM(lines_added)   FROM file_changes WHERE session_id = s.session_id), 0) AS lines_added,
                COALESCE((SELECT SUM(lines_removed) FROM file_changes WHERE session_id = s.session_id), 0) AS lines_removed
         FROM sessions s
```

- [ ] **Step 10: Add the two new columns to the row JSON closure**

In the same function, find the end of the `query_map` closure (~lines 1787-1789) and replace:

```rust
            "cost_source":     r.get::<_, Option<String>>(21)?,
        }))
    })?;
```

with:

```rust
            "cost_source":     r.get::<_, Option<String>>(21)?,
            "lines_added":     r.get::<_, i64>(22)?,
            "lines_removed":   r.get::<_, i64>(23)?,
        }))
    })?;
```

- [ ] **Step 11: Compute the totals and return them**

In the same function, find the final return (~lines 1804-1808):

```rust
    Ok(Json(SessionListResponse {
        sessions,
        coverage: CoverageHint { total, with_repo },
    }))
}
```

Replace it with:

```rust
    // ---- Totals over exactly the rows just returned ----
    let mut t_cost = 0.0_f64;
    let mut t_tok_in = 0_i64;
    let mut t_tok_out = 0_i64;
    let mut t_accepts = 0_i64;
    let mut t_rejects = 0_i64;
    let mut t_aborts = 0_i64;
    let mut t_decisions = 0_i64;
    let mut t_duration = 0.0_f64;
    for s in &sessions {
        t_cost += s["cost_usd"].as_f64().unwrap_or(0.0);
        t_tok_in += s["tokens_input"].as_i64().unwrap_or(0);
        t_tok_out += s["tokens_output"].as_i64().unwrap_or(0);
        t_accepts += s["accepts"].as_i64().unwrap_or(0);
        t_rejects += s["rejects"].as_i64().unwrap_or(0);
        t_aborts += s["aborts"].as_i64().unwrap_or(0);
        t_decisions += s["decisions"].as_i64().unwrap_or(0);
        t_duration += s["duration_seconds"].as_f64().unwrap_or(0.0);
    }

    // Code/Docs/Other split: one query over exactly the returned session ids.
    let mut split = LineSplit::default();
    let ids: Vec<String> = sessions
        .iter()
        .filter_map(|s| s["session_id"].as_str().map(str::to_string))
        .collect();
    if !ids.is_empty() {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let fc_sql = format!(
            "SELECT file_path, lines_added, lines_removed
             FROM file_changes WHERE session_id IN ({placeholders})"
        );
        let mut fc_stmt = conn.prepare(&fc_sql)?;
        let id_refs: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let fc_rows = fc_stmt.query_map(id_refs.as_slice(), |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, i64>(1).unwrap_or(0),
                r.get::<_, i64>(2).unwrap_or(0),
            ))
        })?;
        for (path, added, removed) in fc_rows.flatten() {
            let kind = match path.as_deref() {
                Some(p) => change_kind(p),
                None => ChangeKind::Other,
            };
            let pair = match kind {
                ChangeKind::Code => &mut split.code,
                ChangeKind::Docs => &mut split.docs,
                ChangeKind::Other => &mut split.other,
            };
            pair.added += added;
            pair.removed += removed;
        }
    }

    let totals = SessionTotals {
        sessions: sessions.len() as i64,
        cost_usd: round4(t_cost),
        tokens_input: t_tok_in,
        tokens_output: t_tok_out,
        accepts: t_accepts,
        rejects: t_rejects,
        aborts: t_aborts,
        decisions: t_decisions,
        duration_seconds: t_duration,
        lines_added: split.code.added + split.docs.added + split.other.added,
        lines_removed: split.code.removed + split.docs.removed + split.other.removed,
        lines: split,
    };

    Ok(Json(SessionListResponse {
        sessions,
        coverage: CoverageHint { total, with_repo },
        totals,
    }))
}
```

- [ ] **Step 12: Run the new integration tests to verify they pass**

Run: `cargo test --features test-support v2_sessions_reports_line_totals_and_split v2_sessions_totals_are_zero_without_file_changes`
Expected: PASS (both).

- [ ] **Step 13: Run the full `api_sessions` suite — the snapshot test will fail**

Run: `cargo test --features test-support --test api_sessions`
Expected: `v2_sessions_returns_sessions_and_coverage` FAILS — the response shape changed (each session now has `lines_added`/`lines_removed`, and there is a new top-level `totals` object). insta writes `src-tauri/tests/snapshots/api_sessions__v2_sessions_shape.snap.new`.

- [ ] **Step 14: Review and accept the regenerated snapshot**

Open `src-tauri/tests/snapshots/api_sessions__v2_sessions_shape.snap.new` and confirm the only differences from the old snapshot are: (a) `lines_added: 0` and `lines_removed: 0` on each session, and (b) a new `totals` object with `lines.{code,docs,other}` all zero (that test seeds no `file_changes`).

Then accept it:

Run: `cargo insta accept`
(If `cargo insta` is not installed: `cargo install cargo-insta`, then re-run.)

- [ ] **Step 15: Run the full `api_sessions` suite to verify it passes**

Run: `cargo test --features test-support --test api_sessions`
Expected: PASS (all tests).

- [ ] **Step 16: Commit**

```bash
git add src-tauri/src/api/dto.rs src-tauri/src/api/routes.rs src-tauri/tests/api_sessions.rs src-tauri/tests/snapshots/api_sessions__v2_sessions_shape.snap
git commit -m "feat(api): v2/sessions returns per-row lines and a totals block"
```

---

## Task 2: Frontend — API response types

**Files:**
- Modify: `web/src/app/core/api.service.ts` (`V2Session` ~lines 84-107; `SessionListResponse` ~lines 114-117)

- [ ] **Step 1: Add `lines_added` / `lines_removed` to `V2Session`**

In `web/src/app/core/api.service.ts`, in the `V2Session` interface, after the `cost_source: CostSource;` line, add:

```ts
  lines_added: number;
  lines_removed: number;
```

- [ ] **Step 2: Add the totals interfaces and extend `SessionListResponse`**

In the same file, replace the `SessionListResponse` interface (~lines 114-117):

```ts
export interface SessionListResponse {
  sessions: V2Session[];
  coverage: CoverageHint;
}
```

with:

```ts
export interface LinePair {
  added: number;
  removed: number;
}

export interface LineSplit {
  code: LinePair;
  docs: LinePair;
  other: LinePair;
}

export interface SessionTotals {
  sessions: number;
  cost_usd: number;
  tokens_input: number;
  tokens_output: number;
  accepts: number;
  rejects: number;
  aborts: number;
  decisions: number;
  duration_seconds: number;
  lines_added: number;
  lines_removed: number;
  lines: LineSplit;
}

export interface SessionListResponse {
  sessions: V2Session[];
  coverage: CoverageHint;
  totals: SessionTotals;
}
```

- [ ] **Step 3: Verify the project still type-checks**

Run (from `web/`): `npm run build`
Expected: PASS — the build completes without TypeScript errors.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/core/api.service.ts
git commit -m "feat(web): add line/totals types to the sessions response"
```

---

## Task 3: Frontend — `line-split.ts` pure helper

**Files:**
- Create: `web/src/app/features/sessions/line-split.ts`
- Test: `web/src/app/features/sessions/line-split.spec.ts`

All commands in this task run from the `web/` directory.

- [ ] **Step 1: Write the failing test**

Create `web/src/app/features/sessions/line-split.spec.ts` (Vitest globals are enabled in this repo — no `import { describe, ... }` needed, matching `tape-selection.spec.ts`):

```ts
import { lineSplitSegments } from './line-split';
import { LineSplit } from '../../core/api.service';

function split(
  code: [number, number],
  docs: [number, number],
  other: [number, number],
): LineSplit {
  return {
    code: { added: code[0], removed: code[1] },
    docs: { added: docs[0], removed: docs[1] },
    other: { added: other[0], removed: other[1] },
  };
}

describe('lineSplitSegments', () => {
  it('returns code, docs, other in that order', () => {
    const segs = lineSplitSegments(split([1, 0], [2, 0], [3, 0]));
    expect(segs.map((s) => s.kind)).toEqual(['code', 'docs', 'other']);
  });

  it('computes churn as added + removed', () => {
    const segs = lineSplitSegments(split([10, 4], [0, 0], [0, 0]));
    expect(segs[0].churn).toBe(14);
    expect(segs[0].added).toBe(10);
    expect(segs[0].removed).toBe(4);
  });

  it('computes pct as each bucket share of total churn', () => {
    // total churn = (20+10) + (5+5) + 0 = 40 ; code 75%, docs 25%, other 0%
    const segs = lineSplitSegments(split([20, 10], [5, 5], [0, 0]));
    expect(segs[0].pct).toBeCloseTo(75);
    expect(segs[1].pct).toBeCloseTo(25);
    expect(segs[2].pct).toBe(0);
  });

  it('yields all-zero pct and churn when there is no churn', () => {
    const segs = lineSplitSegments(split([0, 0], [0, 0], [0, 0]));
    expect(segs.every((s) => s.pct === 0)).toBe(true);
    expect(segs.every((s) => s.churn === 0)).toBe(true);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/app/features/sessions/line-split.spec.ts`
Expected: FAIL — `Cannot find module './line-split'`.

- [ ] **Step 3: Create the helper**

Create `web/src/app/features/sessions/line-split.ts`:

```ts
import { LineSplit } from '../../core/api.service';

export type ChangeKind = 'code' | 'docs' | 'other';

export interface LineSegment {
  kind: ChangeKind;
  added: number;
  removed: number;
  /** added + removed */
  churn: number;
  /** 0-100, this bucket's share of total churn; 0 when there is no churn */
  pct: number;
}

/**
 * Break a LineSplit into three segments (code, docs, other) for the
 * totals-row bar. `pct` is each bucket's share of total churn (added +
 * removed), so the segments tile a 100%-wide bar. When there is no churn at
 * all, every `pct` is 0.
 */
export function lineSplitSegments(split: LineSplit): LineSegment[] {
  const order: ChangeKind[] = ['code', 'docs', 'other'];
  const churnOf = (p: { added: number; removed: number }) => p.added + p.removed;
  const total = churnOf(split.code) + churnOf(split.docs) + churnOf(split.other);
  return order.map((kind) => {
    const pair = split[kind];
    const churn = churnOf(pair);
    return {
      kind,
      added: pair.added,
      removed: pair.removed,
      churn,
      pct: total === 0 ? 0 : (churn / total) * 100,
    };
  });
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/app/features/sessions/line-split.spec.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/sessions/line-split.ts web/src/app/features/sessions/line-split.spec.ts
git commit -m "feat(web): add line-split helper for the totals bar"
```

---

## Task 4: Frontend — Sessions component + template

**Files:**
- Modify: `web/src/app/features/sessions/sessions.component.ts`
- Modify: `web/src/app/features/sessions/sessions.component.html`

All commands in this task run from the `web/` directory. This component has no unit test (consistent with the repo's treatment of heavy feature components); it is verified by `npm run build` plus a manual check.

- [ ] **Step 1: Update the component imports**

In `web/src/app/features/sessions/sessions.component.ts`, replace the import line (line 7):

```ts
import { ApiService, CoverageHint, V2Session, V2FileRow } from '../../core/api.service';
```

with:

```ts
import { ApiService, CoverageHint, SessionTotals, V2Session, V2FileRow } from '../../core/api.service';
import { ChangeKind, LineSegment, lineSplitSegments } from './line-split';
```

- [ ] **Step 2: Add the `totals` signal**

In the same file, after the `coverage = signal<CoverageHint | null>(null);` line (line 32), add:

```ts
  totals = signal<SessionTotals | null>(null);
```

- [ ] **Step 3: Add the totals computeds**

After the `missingRepoPct` computed block (ends ~line 38, before `searchInput = '';`), add:

```ts
  readonly totalsAcceptRate = computed(() => {
    const t = this.totals();
    if (!t) return 0;
    const d = t.accepts + t.rejects;
    return d === 0 ? 0 : t.accepts / d;
  });

  readonly totalsSegments = computed<LineSegment[]>(() => {
    const t = this.totals();
    return t ? lineSplitSegments(t.lines) : [];
  });

  readonly hasLineData = computed(() => {
    const t = this.totals();
    return !!t && t.lines_added + t.lines_removed > 0;
  });
```

- [ ] **Step 4: Replace the constructor with a `loadSessions()` extraction**

Replace the entire `constructor()` block (lines 42-58):

```ts
  constructor() {
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const repo = this.filter.reposCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models, repo, sort: this.sort(), limit: 200 };
      this.api.sessionsV2({ ...args, search: this.searchInput || undefined }).subscribe((resp) => {
        this.rows.set(resp.sessions);
        this.coverage.set(resp.coverage);
        this.loaded.set(true);
      });
      this.api.listRepos({ from: w.fromMs, to: w.toMs, limit: 20 }).subscribe((r) => {
        this.repoOptions.set(r);
      });
    });
  }
```

with:

```ts
  constructor() {
    effect(() => {
      this.filter.refreshTick(); // re-run when the Refresh button is clicked
      this.loadSessions();
      const w = this.filter.window();
      this.api.listRepos({ from: w.fromMs, to: w.toMs, limit: 20 }).subscribe((r) => {
        this.repoOptions.set(r);
      });
    });
  }

  private loadSessions(): void {
    const w = this.filter.window();
    const models = this.filter.modelsCsv();
    const repo = this.filter.reposCsv();
    this.api
      .sessionsV2({
        fromMs: w.fromMs,
        toMs: w.toMs,
        models,
        repo,
        sort: this.sort(),
        limit: 200,
        search: this.searchInput || undefined,
      })
      .subscribe((resp) => {
        this.rows.set(resp.sessions);
        this.coverage.set(resp.coverage);
        this.totals.set(resp.totals);
        this.loaded.set(true);
      });
  }
```

(`loadSessions()` reads `filter.window()`, `filter.modelsCsv()`, `filter.reposCsv()` and `sort()` — so calling it inside the `effect()` keeps all the same reactive dependencies the old inline code had.)

- [ ] **Step 5: Simplify `onSearch` and `runBackfill` to use `loadSessions()`**

Replace `onSearch` (lines 68-79):

```ts
  onSearch(v: string) {
    this.searchInput = v;
    const w = this.filter.window();
    const models = this.filter.modelsCsv();
    const repo = this.filter.reposCsv();
    this.api
      .sessionsV2({ fromMs: w.fromMs, toMs: w.toMs, models, repo, sort: this.sort(), limit: 200, search: v || undefined })
      .subscribe((resp) => {
        this.rows.set(resp.sessions);
        this.coverage.set(resp.coverage);
      });
  }
```

with:

```ts
  onSearch(v: string) {
    this.searchInput = v;
    this.loadSessions();
  }
```

Then replace `runBackfill` (lines 81-92):

```ts
  runBackfill() {
    const w = this.filter.window();
    const models = this.filter.modelsCsv();
    const repo = this.filter.reposCsv();
    this.api.backfillRepos().subscribe(() => {
      const args = { fromMs: w.fromMs, toMs: w.toMs, models, repo, sort: this.sort(), limit: 200 };
      this.api.sessionsV2({ ...args, search: this.searchInput || undefined }).subscribe((resp) => {
        this.rows.set(resp.sessions);
        this.coverage.set(resp.coverage);
      });
    });
  }
```

with:

```ts
  runBackfill() {
    this.api.backfillRepos().subscribe(() => this.loadSessions());
  }
```

- [ ] **Step 6: Add the `segmentColor` helper**

In the same file, after the `modelColor` method (ends ~line 165, before the final closing `}` of the class), add:

```ts
  segmentColor(kind: ChangeKind): string {
    switch (kind) {
      case 'code':
        return '#60a5fa';
      case 'docs':
        return '#34d399';
      default:
        return '#7b8794';
    }
  }
```

- [ ] **Step 7: Add the Lines column header**

In `web/src/app/features/sessions/sessions.component.html`, in the `<thead>` row, after the Tokens header (line 76: `<th class="px-2 py-2 font-normal text-right">Tokens</th>`), add:

```html
              <th class="px-2 py-2 font-normal text-right">Lines</th>
```

- [ ] **Step 8: Add the per-row Lines cell**

In the body row, after the Tokens `<td>` (line 121, the one with `{{ s.tokens_input | number }}`), add:

```html
                  <td class="px-2 py-2 text-right tabular-nums">
                    @if (s.lines_added + s.lines_removed === 0) {
                      <span class="text-muted">—</span>
                    } @else {
                      <span class="text-ok">+{{ s.lines_added | number }}</span>
                      <span class="text-err ml-1">−{{ s.lines_removed | number }}</span>
                    }
                  </td>
```

- [ ] **Step 9: Bump the two `colspan="11"` to `colspan="12"`**

The table now has 12 columns. There are exactly two occurrences of `colspan="11"`:
- The day-group header row (~line 84): `<tr><td colspan="11" class="px-4 pt-3 pb-1 ...">`
- The expanded-row cell (~line 143): `<td colspan="11" class="px-6 py-5">`

Change both to `colspan="12"`.

- [ ] **Step 10: Add the `<tfoot>`**

In the same file, immediately after the `</tbody>` closing tag and before `</table>` (~line 224), insert:

```html
          @if (totals(); as t) {
            @if (t.sessions > 0) {
              <tfoot class="border-t-2 border-border bg-bg/60">
                <tr class="font-mono text-xs">
                  <td colspan="6" class="px-4 py-2 text-[10px] uppercase tracking-wider text-muted">
                    Total · {{ t.sessions | number }} sessions
                  </td>
                  <td class="px-2 py-2 text-right tabular-nums">${{ t.cost_usd | number : '1.2-2' }}</td>
                  <td class="px-2 py-2 text-right tabular-nums">{{ t.tokens_input | number }}<span class="text-muted">/</span>{{ t.tokens_output | number }}</td>
                  <td class="px-2 py-2 text-right tabular-nums">
                    <span class="text-ok">+{{ t.lines_added | number }}</span>
                    <span class="text-err ml-1">−{{ t.lines_removed | number }}</span>
                  </td>
                  <td class="px-2 py-2 text-right tabular-nums">{{ t.decisions | number }}</td>
                  <td class="px-2 py-2 text-right tabular-nums text-muted">{{ fmtDuration(t.duration_seconds) }}</td>
                  <td class="px-2 py-2 text-right pr-4 tabular-nums">{{ totalsAcceptRate() * 100 | number : '1.0-0' }}%</td>
                </tr>
                <tr>
                  <td colspan="12" class="px-4 py-3">
                    @if (hasLineData()) {
                      <div class="flex items-center gap-3 font-mono">
                        <span class="text-[10px] uppercase tracking-wider text-muted">Changes</span>
                        <div class="flex h-2 flex-1 overflow-hidden rounded">
                          @for (seg of totalsSegments(); track seg.kind) {
                            @if (seg.pct > 0) {
                              <div [style.width.%]="seg.pct" [style.background-color]="segmentColor(seg.kind)"></div>
                            }
                          }
                        </div>
                        <div class="flex items-center gap-3 text-[10px]">
                          @for (seg of totalsSegments(); track seg.kind) {
                            <span class="inline-flex items-center gap-1.5">
                              <span class="inline-block w-2 h-2 rounded-full" [style.background-color]="segmentColor(seg.kind)"></span>
                              {{ seg.kind }}
                              <span class="text-ok">+{{ seg.added | number }}</span>
                              <span class="text-err">−{{ seg.removed | number }}</span>
                            </span>
                          }
                        </div>
                      </div>
                    } @else {
                      <div class="text-[10px] text-muted font-mono">
                        No file-change data in view — line counts are captured by the PostToolUse hook (Settings → Integration).
                      </div>
                    }
                  </td>
                </tr>
              </tfoot>
            }
          }
```

- [ ] **Step 11: Build and verify**

Run (from `web/`): `npm run build`
Expected: PASS — build completes without errors.

- [ ] **Step 12: Manual check**

Start the app (`cargo tauri dev` from the repo root) and open the Sessions page. Confirm:
- A **Lines** column appears after Tokens; rows with file data show `+added −removed`, rows without show `—`.
- A **totals row** sits at the bottom: `Total · N sessions` plus summed Cost / Tokens / Lines / Decisions / Duration / Accept.
- Below it, either a **segmented Code/Docs/Other bar with a legend** (when there is line data) or the muted **"No file-change data"** hint.
- With a filter that matches no sessions, no totals row renders.

- [ ] **Step 13: Commit**

```bash
git add web/src/app/features/sessions/sessions.component.ts web/src/app/features/sessions/sessions.component.html
git commit -m "feat(sessions): totals row, Lines column, and code/docs/other split"
```

---

## Task 5: Documentation

**Files:**
- Modify: `docs/features.md` (the **Sessions** section, ~lines 21-34)

- [ ] **Step 1: Document the new features**

In `docs/features.md`, in the **Sessions** section, after the bullet `- Accept rate per row is computed as ...` (line 33), add:

```markdown
- **LINES column** *(new)* — per-session lines added / removed. Shows `—` when no file-change data is available (line counts are captured by the PostToolUse hook).
- **Totals row** *(new)* — a grand-total row pinned at the bottom of the table sums cost, tokens, lines, decisions, duration, and accept rate across every session in view. Below it, a segmented bar splits the summed line changes into **Code / Docs / Other** (config files count as code; unclassifiable files as other).
```

- [ ] **Step 2: Commit**

```bash
git add docs/features.md
git commit -m "docs: document the sessions totals row and Lines column"
```

---

## Self-review notes

- **Spec coverage:** `change_kind()` → Task 1 Steps 1-5. Backend per-row lines + totals + DTOs → Task 1 Steps 6-16. Frontend types → Task 2. Split helper + tests → Task 3. Lines column, `colspan` bump, two-row `<tfoot>`, `loadSessions()` refactor, empty/edge states → Task 4. `docs/features.md` → Task 5. Rust unit + integration tests → Task 1. Helper unit tests → Task 3.
- **Test-strategy note vs spec §5:** the spec named a `sessions.component` Vitest test. There is no existing `sessions.component.spec.ts` and the repo does not unit-test heavy feature components (e.g. `OverviewComponent`). The plan instead extracts the split math into the pure, fully-tested `line-split.ts` module and verifies the template via `npm run build` + a manual check — same approach the repo took for the comparable Overview tape feature. The spec's testing *intent* (verify the split logic) is preserved.
- **Type consistency:** `LinePair` / `LineSplit` / `SessionTotals` field names match between `dto.rs` (Task 1) and `api.service.ts` (Task 2). `lineSplitSegments` / `LineSegment` / `ChangeKind` names match between `line-split.ts` (Task 3) and `sessions.component.ts` (Task 4). Row JSON keys `lines_added` / `lines_removed` (Task 1 Step 10) match the `V2Session` fields (Task 2 Step 1) and the template (Task 4 Step 8).
