# JSONL behavioural ingest — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ingest Claude Code's per-session JSONL transcripts to backfill pre-OTel history and surface behavioural views (tool sequences, read-to-edit ratio, slash command usage, sub-agent usage, stuck-session detection, model frequency mix) that OTLP does not expose — while persisting *no* prompt or response text.

**Architecture:** A backfill API endpoint (Settings → Data button) and the existing `SessionEnd` hook handler both feed a single parser → reducer → reconciler → ingestor pipeline. The reducer is the privacy trust boundary; its output type carries no text. JSONL is authoritative only for sessions OTLP never saw; for OTLP-covered sessions, JSONL writes only to new behavioural tables (`tool_sequences`, `slash_commands`, `subagent_calls`, `behaviour_summary`).

**Tech Stack:** Rust 1.95 (tokio, rusqlite, serde, anyhow, tracing, proptest) · Angular 21 (standalone components, signals, Tailwind 4) · SQLite (WAL).

**Spec:** [`docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md`](../specs/2026-05-19-jsonl-behavioural-ingest-design.md)

**Branch:** `feature/jsonl-ingest` (already created and checked out)

---

## File structure

### Create (Rust)
- `src-tauri/src/jsonl/mod.rs` — public API: `backfill()`, `ingest_one()`, `IngestStats`.
- `src-tauri/src/jsonl/record.rs` — `JsonlRecord` serde struct (all-Option fields, lenient).
- `src-tauri/src/jsonl/pricing.rs` — model → cost-per-token constants and `lookup()`.
- `src-tauri/src/jsonl/reducer.rs` — `DerivedEvent` enum + `reduce()` (the trust boundary).
- `src-tauri/src/jsonl/parser.rs` — streaming JSONL line reader, error capture.
- `src-tauri/src/jsonl/walker.rs` — enumerate `~/.claude/projects/<slug>/*.jsonl`.
- `src-tauri/src/jsonl/aggregator.rs` — `behaviour_summary` deltas + `tool_sequences` edges.
- `src-tauri/src/jsonl/reconciler.rs` — per-session OTLP-vs-JSONL routing.

### Create (tests)
- `src-tauri/tests/jsonl_reducer.rs` — reducer golden tests + privacy property test.
- `src-tauri/tests/jsonl_pipeline.rs` — end-to-end backfill/ingest_one integration tests.
- `src-tauri/tests/fixtures/jsonl/*.jsonl` — golden fixture transcripts.

### Create (scripts + Angular)
- `scripts/smoke_jsonl.py` — synthetic JSONL + backfill smoke (mirrors `smoke_otlp.py`).
- `web/src/app/features/behaviour/behaviour.component.{ts,html}` — new Behaviour page.
- `web/src/app/features/behaviour/components/*.ts` — five sub-components (model-mix, tool-sequence, ratio, slash-leaderboard, subagents, stuck-list).

### Modify
- `src-tauri/src/db/migrations.rs` — add `MIGRATION_V4`.
- `src-tauri/src/lib.rs` — register `pub mod jsonl;`.
- `src-tauri/src/otlp/ingestor.rs` — add `ingest_derived(events, source)` method.
- `src-tauri/src/api/routes.rs` — `POST /api/jsonl/backfill`, enrich `hook_session_end`, `GET /api/jsonl/errors`, `GET /api/jsonl/ingest-runs`, six `GET /api/behaviour/*` routes.
- `src-tauri/src/api/dto.rs` — DTOs for the new endpoints.
- `src-tauri/Cargo.toml` — `proptest` dev-dep.
- `web/src/app/core/api.service.ts` — typed wrappers for new endpoints.
- `web/src/app/core/models.ts` — new DTO types.
- `web/src/app/app.routes.ts` — Behaviour page route.
- `web/src/app/features/sessions/sessions.component.{ts,html}` — Stuck chip.
- `web/src/app/features/overview/overview.component.{ts,html}` — Invocations-by-model companion.
- `web/src/app/features/sessions/session-detail.component.ts` — three-stat enrichment row.
- `web/src/app/features/settings/settings.component.{ts,html}` — Ingest JSONL button + status line.
- `web/src/app/features/diagnostics/diagnostics.component.{ts,html}` — JSONL parse errors card.
- `README.md` — remove the "no retroactive" cell from the comparison table.
- `docs/pitch.md` — "never read" → "never persisted".

---

## Phase 1 — Database schema

### Task 1: Migration v4 — JSONL ingest schema

**Files:**
- Modify: `src-tauri/src/db/migrations.rs:109` (the `MIGRATIONS` slice).
- Test: same file (inline `#[cfg(test)]` module).

- [ ] **Step 1: Add the migration constant**

In `src-tauri/src/db/migrations.rs`, after `MIGRATION_V3` (around line 107), add:

```rust
const MIGRATION_V4: &str = r#"
-- Extend existing tables.
ALTER TABLE sessions ADD COLUMN data_source TEXT;
UPDATE sessions SET data_source = 'otlp' WHERE data_source IS NULL;

ALTER TABLE tool_decisions ADD COLUMN source TEXT NOT NULL DEFAULT 'otlp';
ALTER TABLE tool_decisions ADD COLUMN model TEXT;
ALTER TABLE tool_decisions ADD COLUMN bash_verb TEXT;
ALTER TABLE tool_decisions ADD COLUMN arg_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tool_decisions ADD COLUMN is_error INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tool_decisions ADD COLUMN duration_ms INTEGER;
ALTER TABLE tool_decisions ADD COLUMN turn_idx INTEGER;

-- New behavioural tables.
CREATE TABLE tool_sequences (
    session_id  TEXT NOT NULL,
    from_tool   TEXT NOT NULL,
    to_tool     TEXT NOT NULL,
    count       INTEGER NOT NULL,
    PRIMARY KEY (session_id, from_tool, to_tool)
);

CREATE TABLE slash_commands (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT NOT NULL,
    timestamp     INTEGER NOT NULL,
    command_name  TEXT NOT NULL,
    arg_count     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_slash_session ON slash_commands(session_id);
CREATE INDEX idx_slash_name    ON slash_commands(command_name);

CREATE TABLE subagent_calls (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_session_id  TEXT NOT NULL,
    child_session_id   TEXT,
    subagent_type      TEXT,
    started_at         INTEGER NOT NULL,
    ended_at           INTEGER,
    tool_call_count    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_subagent_parent ON subagent_calls(parent_session_id);

CREATE TABLE behaviour_summary (
    session_id           TEXT PRIMARY KEY,
    file_thrashed_max    INTEGER NOT NULL DEFAULT 0,
    file_thrashed_path   TEXT,
    command_retried_max  INTEGER NOT NULL DEFAULT 0,
    command_retried_verb TEXT,
    read_storm_max       INTEGER NOT NULL DEFAULT 0,
    read_count           INTEGER NOT NULL DEFAULT 0,
    edit_count           INTEGER NOT NULL DEFAULT 0,
    total_tool_calls     INTEGER NOT NULL DEFAULT 0,
    thinking_tokens      INTEGER NOT NULL DEFAULT 0,
    subagent_count       INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE jsonl_errors (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    jsonl_path     TEXT NOT NULL,
    line_no        INTEGER NOT NULL,
    error_kind     TEXT NOT NULL,
    error_msg      TEXT NOT NULL,
    cc_version     TEXT,
    ingested_at    INTEGER NOT NULL
);
CREATE INDEX idx_jsonl_errors_ts ON jsonl_errors(ingested_at);

CREATE TABLE jsonl_ingest_runs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    kind                TEXT NOT NULL,
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER,
    files_processed     INTEGER NOT NULL DEFAULT 0,
    records_processed   INTEGER NOT NULL DEFAULT 0,
    records_errored     INTEGER NOT NULL DEFAULT 0
);
"#;
```

- [ ] **Step 2: Register the migration**

Change the `MIGRATIONS` slice to:

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
];
```

- [ ] **Step 3: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn v4_creates_behaviour_tables_and_extends_decisions() {
    let mut conn = Connection::open_in_memory().unwrap();
    apply(&mut conn).unwrap();

    // New tables exist.
    for tbl in [
        "tool_sequences", "slash_commands", "subagent_calls",
        "behaviour_summary", "jsonl_errors", "jsonl_ingest_runs",
    ] {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [tbl], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "missing table {tbl}");
    }

    // tool_decisions has the new columns.
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(tool_decisions)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    for c in ["source", "model", "bash_verb", "arg_count", "is_error", "duration_ms", "turn_idx"] {
        assert!(cols.contains(&c.to_string()), "missing tool_decisions column {c}");
    }

    // sessions has data_source.
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(sessions)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    assert!(cols.contains(&"data_source".to_string()));

    // Existing rows backfilled to 'otlp'. Insert one pre-migration-shape row
    // then verify default behaviour for new inserts.
    conn.execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 0)",
        [],
    ).unwrap();
    let ds: Option<String> = conn.query_row(
        "SELECT data_source FROM sessions WHERE session_id = 's1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(ds, None, "new inserts leave data_source NULL until ingestor sets it");

    let v: i32 = conn.query_row(
        "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(v, 4);
}
```

Also bump the existing `migrations_are_idempotent_across_runs` assertion from `assert_eq!(v, 3)` to `assert_eq!(v, 4)`.

- [ ] **Step 4: Run the test (expect FAIL until you add the constant + register it)**

```powershell
cd src-tauri; cargo test --features test-support db::migrations
```

If you wrote Step 1 and Step 2 before Step 3, this will PASS. If you wrote the test first, it will FAIL with "no such table: tool_sequences" — confirm, then go back and complete Steps 1 + 2.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/db/migrations.rs
git commit -m "feat(db): migration v4 — JSONL ingest schema (behaviour tables + tool_decisions extensions)"
```

---

## Phase 2 — JSONL record types and pricing

### Task 2: `jsonl::record` — lenient JSONL deserialisation

**Files:**
- Create: `src-tauri/src/jsonl/mod.rs` (skeleton — just `pub mod record;` for now).
- Create: `src-tauri/src/jsonl/record.rs`.
- Modify: `src-tauri/src/lib.rs` (register `pub mod jsonl;`).
- Test: inline `#[cfg(test)] mod tests` in `record.rs`.

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs`, add alongside the existing `pub mod` declarations:

```rust
pub mod jsonl;
```

Create `src-tauri/src/jsonl/mod.rs`:

```rust
//! JSONL transcript ingestion. See docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md.

pub mod record;
```

- [ ] **Step 2: Write `record.rs` with lenient fields**

Create `src-tauri/src/jsonl/record.rs`:

```rust
//! Lenient deserialiser for Claude Code's per-session JSONL transcripts.
//!
//! Every field is `Option<T>` and every nested struct uses
//! `#[serde(default)]` so unknown fields and missing fields cannot abort
//! a parse. See spec §"Parser & reducer implementation".

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlRecord {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>, // ISO8601, parsed to ms later
    pub version: Option<String>,    // Claude Code version
    pub cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    pub message: Option<Message>,
    #[serde(rename = "toolUseResult")]
    pub tool_use_result: Option<Value>,
    #[serde(rename = "isMeta", default)]
    pub is_meta: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(rename = "stop_reason")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    pub cache_creation: i64,
    #[serde(default, rename = "cache_read_input_tokens")]
    pub cache_read: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: Option<String> },
    #[serde(rename = "thinking")]
    Thinking { thinking: Option<String> },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: Option<String>,
        name: Option<String>,
        #[serde(default)]
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: Option<String>,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        content: Value,
    },
    #[serde(other)]
    Other,
}

/// Parse a single JSONL line into a `JsonlRecord`. Returns the deserialiser
/// error on failure — callers route that to `jsonl_errors`.
pub fn parse_line(line: &str) -> Result<JsonlRecord, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_record() {
        let line = r#"{"type":"user","sessionId":"s1","uuid":"u1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.kind.as_deref(), Some("user"));
        assert_eq!(r.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn parses_assistant_with_tool_use() {
        let line = r#"{"type":"assistant","sessionId":"s1","uuid":"u2","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x/y.rs"}}]}}"#;
        let r = parse_line(line).expect("parse");
        let msg = r.message.as_ref().expect("message");
        assert_eq!(msg.model.as_deref(), Some("claude-opus-4-7"));
        let usage = msg.usage.as_ref().expect("usage");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.cache_read, 5);
        match &msg.content[0] {
            ContentBlock::ToolUse { name, .. } => assert_eq!(name.as_deref(), Some("Read")),
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn unknown_record_type_does_not_fail() {
        let line = r#"{"type":"super_event_2027","sessionId":"s1"}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.kind.as_deref(), Some("super_event_2027"));
    }

    #[test]
    fn missing_fields_default_to_none() {
        let line = r#"{"type":"summary"}"#;
        let r = parse_line(line).expect("parse");
        assert!(r.session_id.is_none());
        assert!(r.message.is_none());
    }

    #[test]
    fn extra_unknown_fields_ignored() {
        let line = r#"{"type":"user","sessionId":"s1","futureField":42,"nested":{"a":1}}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.kind.as_deref(), Some("user"));
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::record
```

Expected: 5 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/lib.rs src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/record.rs
git commit -m "feat(jsonl): lenient JsonlRecord deserialiser"
```

---

### Task 3: `jsonl::pricing` — model price table for retroactive cost

**Files:**
- Create: `src-tauri/src/jsonl/pricing.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs` (add `pub mod pricing;`).
- Test: inline `#[cfg(test)] mod tests`.

- [ ] **Step 1: Add the module to `mod.rs`**

```rust
pub mod record;
pub mod pricing;
```

- [ ] **Step 2: Write `pricing.rs`**

Create `src-tauri/src/jsonl/pricing.rs`:

```rust
//! Per-model token pricing for retroactive cost computation.
//!
//! Used only when ingesting a JSONL session that OTLP never saw. For
//! OTLP-covered sessions, `claude_code.cost.usage` is authoritative and
//! this table is ignored. Prices are USD per million tokens.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_create_per_mtok: f64,
}

/// Look up pricing for a model id. Returns `None` for unknown models —
/// callers should skip emitting a CostEntry rather than guessing.
///
/// Match is by *prefix*: e.g. `claude-opus-4-7-20260101` matches the
/// `claude-opus-4-7` row. This keeps the table small as Anthropic ships
/// date-suffixed variants.
pub fn lookup(model: &str) -> Option<ModelPricing> {
    for (prefix, price) in TABLE {
        if model.starts_with(prefix) {
            return Some(*price);
        }
    }
    None
}

/// Compute USD cost from per-event token counts.
pub fn cost_for(model: &str, input: i64, output: i64, cache_read: i64, cache_create: i64) -> Option<f64> {
    let p = lookup(model)?;
    let n = |toks: i64, per_m: f64| (toks as f64) / 1_000_000.0 * per_m;
    Some(
        n(input, p.input_per_mtok)
            + n(output, p.output_per_mtok)
            + n(cache_read, p.cache_read_per_mtok)
            + n(cache_create, p.cache_create_per_mtok),
    )
}

// USD per million tokens. Update when Anthropic changes prices.
// Quarterly cadence recommended; CI check vs ccusage's pricing.json
// can catch drift cheaply (future work).
const TABLE: &[(&str, ModelPricing)] = &[
    ("claude-opus-4-7", ModelPricing {
        input_per_mtok: 15.0,
        output_per_mtok: 75.0,
        cache_read_per_mtok: 1.50,
        cache_create_per_mtok: 18.75,
    }),
    ("claude-opus-4-6", ModelPricing {
        input_per_mtok: 15.0,
        output_per_mtok: 75.0,
        cache_read_per_mtok: 1.50,
        cache_create_per_mtok: 18.75,
    }),
    ("claude-sonnet-4-6", ModelPricing {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_per_mtok: 0.30,
        cache_create_per_mtok: 3.75,
    }),
    ("claude-sonnet-4-5", ModelPricing {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_per_mtok: 0.30,
        cache_create_per_mtok: 3.75,
    }),
    ("claude-haiku-4-5", ModelPricing {
        input_per_mtok: 1.0,
        output_per_mtok: 5.0,
        cache_read_per_mtok: 0.10,
        cache_create_per_mtok: 1.25,
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_works_for_date_suffixed_models() {
        let p = lookup("claude-opus-4-7-20260101").expect("found");
        assert_eq!(p.output_per_mtok, 75.0);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(lookup("gpt-4").is_none());
    }

    #[test]
    fn cost_for_sums_all_token_types() {
        let c = cost_for("claude-opus-4-7", 1_000_000, 0, 0, 0).expect("price");
        assert!((c - 15.0).abs() < 1e-9, "input-only 1M = $15");
        let c = cost_for("claude-opus-4-7", 0, 1_000_000, 0, 0).expect("price");
        assert!((c - 75.0).abs() < 1e-9, "output-only 1M = $75");
    }

    #[test]
    fn cost_for_unknown_model_returns_none() {
        assert!(cost_for("mystery-model", 1000, 1000, 0, 0).is_none());
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::pricing
```

Expected: 4 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/pricing.rs
git commit -m "feat(jsonl): bundled pricing table for retroactive cost computation"
```

---

## Phase 3 — The reducer (trust boundary)

### Task 4: `jsonl::reducer` — `DerivedEvent` enum and skeleton

**Files:**
- Create: `src-tauri/src/jsonl/reducer.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.
- Test: inline `#[cfg(test)] mod tests`.

- [ ] **Step 1: Register the module**

In `src-tauri/src/jsonl/mod.rs`:

```rust
pub mod record;
pub mod pricing;
pub mod reducer;
```

- [ ] **Step 2: Write the DerivedEvent enum and reducer skeleton**

Create `src-tauri/src/jsonl/reducer.rs`:

```rust
//! Trust boundary between JSONL (raw, contains prompt text) and the rest
//! of the ingest pipeline (text-free by type). Anything that reads
//! `record::Message.content[].text` must do so inside this module and
//! drop the text before returning.

use crate::jsonl::record::{ContentBlock, JsonlRecord, Message};

/// Output of the reducer. By construction, no variant carries
/// prompt or response text content. The privacy property test in
/// `src-tauri/tests/jsonl_reducer.rs` enforces this empirically;
/// the type system enforces it structurally.
#[derive(Debug, Clone)]
pub enum DerivedEvent {
    SessionLifecycle {
        session_id: String,
        started_at: i64,
        ended_at: Option<i64>,
        cc_version: Option<String>,
        cwd: Option<String>,
        git_branch: Option<String>,
    },
    TokenUsage {
        session_id: String,
        ts: i64,
        model: String,
        input: i64,
        output: i64,
        cache_create: i64,
        cache_read: i64,
    },
    CostEntry {
        session_id: String,
        ts: i64,
        model: String,
        cost_usd: f64,
    },
    ToolCall {
        session_id: String,
        ts: i64,
        turn_idx: i64,
        tool_name: String,
        file_path: Option<String>,
        bash_verb: Option<String>,
        arg_count: i64,
        is_error: bool,
        duration_ms: Option<i64>,
        model: Option<String>,
        tool_use_id: Option<String>, // for tool_result correlation
    },
    SlashCommand {
        session_id: String,
        ts: i64,
        name: String,
        arg_count: i64,
    },
    SubAgentCall {
        parent_id: String,
        child_id: Option<String>,
        subagent_type: Option<String>,
        started_at: i64,
    },
    ThinkingTokens {
        session_id: String,
        ts: i64,
        tokens: i64,
    },
}

/// Per-stream reducer. Keep one instance per session being parsed;
/// `turn_idx` and the tool_use → tool_result correlation table are
/// stateful.
#[derive(Default)]
pub struct Reducer {
    pub turn_idx: i64,
}

impl Reducer {
    pub fn new() -> Self { Self::default() }

    /// Reduce a single record. Output may be empty (e.g. for `summary`
    /// records or unknown types).
    pub fn reduce(&mut self, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let sid = match rec.session_id.as_deref() {
            Some(s) => s.to_string(),
            None => return vec![],
        };
        let ts = parse_ts(rec.timestamp.as_deref()).unwrap_or(0);

        match rec.kind.as_deref() {
            Some("user")      => self.reduce_user(&sid, ts, rec),
            Some("assistant") => self.reduce_assistant(&sid, ts, rec),
            Some("summary")   => vec![],
            Some("system")    => vec![],
            _                 => vec![],
        }
    }

    fn reduce_user(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        self.turn_idx += 1;
        // Slash command detection — handled in Task 6. For now, only emit
        // lifecycle on the first user turn so a session row exists even
        // for transcripts with no assistant reply.
        let mut out = vec![];
        if self.turn_idx == 1 {
            out.push(DerivedEvent::SessionLifecycle {
                session_id: sid.to_string(),
                started_at: ts,
                ended_at: None,
                cc_version: rec.version.clone(),
                cwd: rec.cwd.clone(),
                git_branch: rec.git_branch.clone(),
            });
        }
        out
    }

    fn reduce_assistant(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        self.turn_idx += 1;
        let mut out = vec![];

        let Some(msg) = rec.message.as_ref() else { return out };
        let model = msg.model.clone().unwrap_or_else(|| "unknown".into());

        if let Some(u) = msg.usage.as_ref() {
            if u.input_tokens + u.output_tokens + u.cache_read + u.cache_creation > 0 {
                out.push(DerivedEvent::TokenUsage {
                    session_id: sid.to_string(),
                    ts,
                    model: model.clone(),
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_create: u.cache_creation,
                    cache_read: u.cache_read,
                });
            }
        }

        // Tool calls are emitted in Task 5; tool_result correlation in Task 7.
        let _ = (sid, ts, msg);
        out
    }
}

fn parse_ts(s: Option<&str>) -> Option<i64> {
    let s = s?;
    // ISO8601 with millisecond precision: "2026-05-19T10:00:00.000Z"
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonl::record::parse_line;

    #[test]
    fn user_record_emits_lifecycle_on_first_turn() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let rec = parse_line(line).unwrap();
        let out = r.reduce(&rec);
        assert_eq!(out.len(), 1);
        match &out[0] {
            DerivedEvent::SessionLifecycle { session_id, cwd, git_branch, cc_version, .. } => {
                assert_eq!(session_id, "s1");
                assert_eq!(cwd.as_deref(), Some("/r"));
                assert_eq!(git_branch.as_deref(), Some("main"));
                assert_eq!(cc_version.as_deref(), Some("2.1.0"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn second_user_record_does_not_repeat_lifecycle() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        let rec = parse_line(line).unwrap();
        let _ = r.reduce(&rec);
        let line2 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:05.000Z","message":{"role":"user","content":[]}}"#;
        let rec2 = parse_line(line2).unwrap();
        let out = r.reduce(&rec2);
        assert!(out.is_empty());
    }

    #[test]
    fn assistant_emits_token_usage() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}}"#;
        let rec = parse_line(line).unwrap();
        let out = r.reduce(&rec);
        let tok = out.iter().find_map(|e| match e {
            DerivedEvent::TokenUsage { input, output, cache_read, model, .. } =>
                Some((*input, *output, *cache_read, model.clone())),
            _ => None,
        }).expect("token usage emitted");
        assert_eq!(tok, (10, 20, 5, "claude-opus-4-7".to_string()));
    }

    #[test]
    fn assistant_with_zero_tokens_does_not_emit() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        let rec = parse_line(line).unwrap();
        let out = r.reduce(&rec);
        assert!(!out.iter().any(|e| matches!(e, DerivedEvent::TokenUsage { .. })));
    }

    #[test]
    fn no_session_id_means_no_output() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        let rec = parse_line(line).unwrap();
        let out = r.reduce(&rec);
        assert!(out.is_empty());
    }
}
```

- [ ] **Step 3: Add `chrono` dep if missing**

Check `src-tauri/Cargo.toml`. If `chrono` is not present, add it under `[dependencies]`:

```toml
chrono = { version = "0.4", default-features = false, features = ["serde", "clock"] }
```

(Per architecture.md it's likely already there — confirm before adding.)

- [ ] **Step 4: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reducer
```

Expected: 5 PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/reducer.rs src-tauri/Cargo.toml
git commit -m "feat(jsonl): reducer skeleton with DerivedEvent enum and user/assistant token reduction"
```

---

### Task 5: Reducer — tool_use blocks and sub-agent detection

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs` (extend `reduce_assistant`).
- Test: same file.

- [ ] **Step 1: Add tool_use reduction**

Replace the body of `reduce_assistant` (after the token usage emission) with the full version. The complete updated method:

```rust
fn reduce_assistant(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
    self.turn_idx += 1;
    let mut out = vec![];
    let Some(msg) = rec.message.as_ref() else { return out };
    let model = msg.model.clone().unwrap_or_else(|| "unknown".into());

    if let Some(u) = msg.usage.as_ref() {
        if u.input_tokens + u.output_tokens + u.cache_read + u.cache_creation > 0 {
            out.push(DerivedEvent::TokenUsage {
                session_id: sid.to_string(),
                ts,
                model: model.clone(),
                input: u.input_tokens,
                output: u.output_tokens,
                cache_create: u.cache_creation,
                cache_read: u.cache_read,
            });
        }
    }

    for block in &msg.content {
        match block {
            ContentBlock::Thinking { thinking } => {
                // Token count only; text dropped.
                let tokens = estimate_thinking_tokens(thinking.as_deref());
                if tokens > 0 {
                    out.push(DerivedEvent::ThinkingTokens {
                        session_id: sid.to_string(),
                        ts,
                        tokens,
                    });
                }
            }
            ContentBlock::ToolUse { id, name, input } => {
                let Some(tool_name) = name.clone() else { continue };
                let file_path = extract_file_path(input);
                let (bash_verb, arg_count) = if tool_name == "Bash" {
                    extract_bash_verb_and_arg_count(input)
                } else {
                    (None, count_args(input))
                };
                out.push(DerivedEvent::ToolCall {
                    session_id: sid.to_string(),
                    ts,
                    turn_idx: self.turn_idx,
                    tool_name: tool_name.clone(),
                    file_path,
                    bash_verb,
                    arg_count,
                    is_error: false,
                    duration_ms: None,
                    model: Some(model.clone()),
                    tool_use_id: id.clone(),
                });

                if tool_name == "Task" {
                    out.push(DerivedEvent::SubAgentCall {
                        parent_id: sid.to_string(),
                        child_id: extract_subagent_session(input),
                        subagent_type: extract_subagent_type(input),
                        started_at: ts,
                    });
                }
            }
            _ => { /* text and tool_result handled elsewhere; Other ignored */ }
        }
    }

    out
}
```

- [ ] **Step 2: Add the input-extraction helpers**

Append to `reducer.rs` (above the `#[cfg(test)]` block):

```rust
fn estimate_thinking_tokens(text: Option<&str>) -> i64 {
    // Rough estimate: ~4 chars/token. Text is dropped after this call.
    text.map(|t| (t.chars().count() as i64 + 3) / 4).unwrap_or(0)
}

fn extract_file_path(input: &serde_json::Value) -> Option<String> {
    input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn count_args(input: &serde_json::Value) -> i64 {
    input.as_object().map(|o| o.len() as i64).unwrap_or(0)
}

fn extract_bash_verb_and_arg_count(input: &serde_json::Value) -> (Option<String>, i64) {
    let Some(cmd) = input.get("command").and_then(|v| v.as_str()) else {
        return (None, count_args(input));
    };
    let verb = cmd.split_whitespace().next().map(|s| s.to_lowercase());
    let arg_count = cmd.split_whitespace().count().saturating_sub(1) as i64;
    (verb, arg_count)
}

fn extract_subagent_session(input: &serde_json::Value) -> Option<String> {
    // Claude Code sets this on Task tool_use input when launching the
    // child. Best-effort: schema may evolve.
    input.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn extract_subagent_type(input: &serde_json::Value) -> Option<String> {
    input.get("subagent_type").and_then(|v| v.as_str()).map(|s| s.to_string())
        .or_else(|| input.get("type").and_then(|v| v.as_str()).map(|s| s.to_string()))
}
```

- [ ] **Step 3: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn assistant_with_read_tool_use_emits_tool_call_with_file_path() {
    let mut r = Reducer::new();
    let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    let call = out.iter().find_map(|e| match e {
        DerivedEvent::ToolCall { tool_name, file_path, .. } => Some((tool_name.clone(), file_path.clone())),
        _ => None,
    }).expect("tool call emitted");
    assert_eq!(call.0, "Read");
    assert_eq!(call.1.as_deref(), Some("src/lib.rs"));
}

#[test]
fn assistant_with_bash_extracts_verb_and_arg_count() {
    let mut r = Reducer::new();
    let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status --short"}}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    let (verb, arg_count) = out.iter().find_map(|e| match e {
        DerivedEvent::ToolCall { bash_verb, arg_count, .. } => Some((bash_verb.clone(), *arg_count)),
        _ => None,
    }).expect("tool call emitted");
    assert_eq!(verb.as_deref(), Some("git"));
    assert_eq!(arg_count, 2);
}

#[test]
fn assistant_with_task_emits_subagent_call() {
    let mut r = Reducer::new();
    let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Task","input":{"subagent_type":"Explore","description":"find auth code","prompt":"search for auth handlers"}}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    let st = out.iter().find_map(|e| match e {
        DerivedEvent::SubAgentCall { subagent_type, .. } => Some(subagent_type.clone()),
        _ => None,
    }).expect("subagent call emitted");
    assert_eq!(st.as_deref(), Some("Explore"));
}

#[test]
fn thinking_block_emits_token_count_without_text() {
    let mut r = Reducer::new();
    let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"thinking","thinking":"a very long thought process about authentication design choices and tradeoffs"}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    let tokens = out.iter().find_map(|e| match e {
        DerivedEvent::ThinkingTokens { tokens, .. } => Some(*tokens),
        _ => None,
    }).expect("thinking tokens emitted");
    assert!(tokens > 0);
}
```

- [ ] **Step 4: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reducer
```

Expected: 9 PASS (5 existing + 4 new).

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/reducer.rs
git commit -m "feat(jsonl): reduce tool_use, thinking, and Task blocks with text stripped"
```

---

### Task 6: Reducer — slash command detection

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs` (`reduce_user`).
- Test: same file.

- [ ] **Step 1: Update `reduce_user`**

Replace `reduce_user` with:

```rust
fn reduce_user(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
    self.turn_idx += 1;
    let mut out = vec![];

    if self.turn_idx == 1 {
        out.push(DerivedEvent::SessionLifecycle {
            session_id: sid.to_string(),
            started_at: ts,
            ended_at: None,
            cc_version: rec.version.clone(),
            cwd: rec.cwd.clone(),
            git_branch: rec.git_branch.clone(),
        });
    }

    if let Some(msg) = rec.message.as_ref() {
        if let Some(sc) = detect_slash_command(msg) {
            out.push(DerivedEvent::SlashCommand {
                session_id: sid.to_string(),
                ts,
                name: sc.0,
                arg_count: sc.1,
            });
        }
    }

    out
}
```

- [ ] **Step 2: Add the detector helper**

Append (above the `#[cfg(test)]` block):

```rust
/// Detect a slash command from a user message. Returns
/// `(command_name_without_slash, arg_count)`.
///
/// Claude Code injects `<command-name>` and `<command-args>` tags into
/// the text block when a user types `/foo bar baz`. We extract those
/// tags and immediately drop the surrounding text — the tag *names*
/// (not values) tell us the command was invoked.
fn detect_slash_command(msg: &Message) -> Option<(String, i64)> {
    for block in &msg.content {
        if let ContentBlock::Text { text: Some(t) } = block {
            if let Some(name) = extract_tag(t, "command-name") {
                let arg_count = extract_tag(t, "command-args")
                    .map(|args| args.split_whitespace().count() as i64)
                    .unwrap_or(0);
                let trimmed = name.trim().trim_start_matches('/').to_string();
                if !trimmed.is_empty() {
                    return Some((trimmed, arg_count));
                }
            }
        }
    }
    None
}

fn extract_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(&s[start..end])
}
```

- [ ] **Step 3: Tests**

Append to the test module:

```rust
#[test]
fn user_message_with_command_name_tag_emits_slash_command() {
    let mut r = Reducer::new();
    let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name>\n<command-args>PR 42</command-args>"}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    let sc = out.iter().find_map(|e| match e {
        DerivedEvent::SlashCommand { name, arg_count, .. } => Some((name.clone(), *arg_count)),
        _ => None,
    }).expect("slash command emitted");
    assert_eq!(sc.0, "review");
    assert_eq!(sc.1, 2);
}

#[test]
fn plain_user_message_does_not_emit_slash_command() {
    let mut r = Reducer::new();
    let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"just a normal message"}]}}"#;
    let rec = parse_line(line).unwrap();
    let out = r.reduce(&rec);
    assert!(!out.iter().any(|e| matches!(e, DerivedEvent::SlashCommand { .. })));
}
```

- [ ] **Step 4: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reducer
```

Expected: 11 PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/reducer.rs
git commit -m "feat(jsonl): detect slash commands from user message tags"
```

---

### Task 7: Reducer — tool_result correlation (is_error, duration)

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs` — `Reducer` keeps a `pending_tool_use_id → ts` map, and on `tool_result` blocks updates the matching ToolCall's `is_error`/`duration_ms`. Since `DerivedEvent` is immutable once emitted, we batch within a single record and accept that cross-record correlation is best-effort.

**Approach:** Track tool_use IDs in `Reducer.in_flight: HashMap<String, (ts, turn_idx)>`. On tool_result, emit a `ToolResult` patch event that the aggregator merges. To keep things simple, emit a new `DerivedEvent::ToolResult { tool_use_id, is_error, duration_ms }` and let the aggregator update the row.

- [ ] **Step 1: Add `ToolResult` variant**

Add to the `DerivedEvent` enum:

```rust
ToolResult {
    session_id: String,
    tool_use_id: String,
    is_error: bool,
    duration_ms: Option<i64>,
},
```

- [ ] **Step 2: Track in-flight tool uses + handle tool_result**

Change `Reducer`:

```rust
#[derive(Default)]
pub struct Reducer {
    pub turn_idx: i64,
    in_flight: std::collections::HashMap<String, i64>, // tool_use_id → ts
}
```

Extend the `for block in &msg.content` match arms in both `reduce_assistant` and add handling in `reduce_user` (since tool_result blocks appear in user-role messages per Claude Code's JSONL shape):

In `reduce_user`, *after* the existing logic, add:

```rust
    if let Some(msg) = rec.message.as_ref() {
        for block in &msg.content {
            if let ContentBlock::ToolResult { tool_use_id: Some(id), is_error, .. } = block {
                let start = self.in_flight.remove(id);
                let duration_ms = start.map(|s| (ts - s).max(0));
                out.push(DerivedEvent::ToolResult {
                    session_id: sid.to_string(),
                    tool_use_id: id.clone(),
                    is_error: *is_error,
                    duration_ms,
                });
            }
        }
    }
    out
}
```

In `reduce_assistant`, after the `ContentBlock::ToolUse` push, record the id:

```rust
                if let Some(uid) = id.clone() {
                    self.in_flight.insert(uid, ts);
                }
```

- [ ] **Step 3: Test**

```rust
#[test]
fn tool_use_followed_by_tool_result_emits_paired_events() {
    let mut r = Reducer::new();
    let assist = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tool1","name":"Read","input":{"file_path":"a.rs"}}]}}"#;
    let user  = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:02.500Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool1","is_error":false,"content":"ok"}]}}"#;
    let _ = r.reduce(&parse_line(assist).unwrap());
    let out = r.reduce(&parse_line(user).unwrap());
    let res = out.iter().find_map(|e| match e {
        DerivedEvent::ToolResult { tool_use_id, is_error, duration_ms, .. } =>
            Some((tool_use_id.clone(), *is_error, *duration_ms)),
        _ => None,
    }).expect("tool result emitted");
    assert_eq!(res.0, "tool1");
    assert!(!res.1);
    assert_eq!(res.2, Some(1500));
}
```

- [ ] **Step 4: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reducer
```

Expected: 12 PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/reducer.rs
git commit -m "feat(jsonl): correlate tool_use with tool_result for is_error and duration"
```

---

### Task 8: Privacy property test (the trust-boundary guarantee)

**Files:**
- Modify: `src-tauri/Cargo.toml` — add `proptest` dev-dep.
- Create: `src-tauri/tests/jsonl_privacy.rs`.

- [ ] **Step 1: Add proptest dev-dep**

In `src-tauri/Cargo.toml`, under `[dev-dependencies]`:

```toml
proptest = "1"
```

- [ ] **Step 2: Write the property test**

Create `src-tauri/tests/jsonl_privacy.rs`:

```rust
//! Privacy property test for the JSONL reducer trust boundary.
//!
//! For randomly generated JSONL records containing prompt-shaped text in
//! every text-bearing field, the reducer output must contain no substring
//! of any input text longer than `MIN_LEAK_LEN` characters. This is the
//! formal guarantee behind the pitch's "no prompt or response text is
//! persisted" claim.

use andon_lib::jsonl::record::JsonlRecord;
use andon_lib::jsonl::reducer::{DerivedEvent, Reducer};
use proptest::prelude::*;
use serde_json::json;

const MIN_LEAK_LEN: usize = 12;

fn event_to_string(e: &DerivedEvent) -> String {
    // Use Debug — it serialises every field, including Option<String>.
    // Any text that survived to a `String` field will appear here.
    format!("{e:?}")
}

fn assert_no_leak(events: &[DerivedEvent], secrets: &[String]) {
    let dump = events.iter().map(event_to_string).collect::<Vec<_>>().join("\n");
    for s in secrets {
        if s.len() >= MIN_LEAK_LEN {
            assert!(
                !dump.contains(s),
                "reducer output leaked input text fragment: {s:?}\nDump: {dump}"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    #[test]
    fn user_text_never_leaks(prompt in "[A-Za-z0-9 _.,;:!?/-]{20,200}") {
        let rec_json = json!({
            "type": "user",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:00.000Z",
            "message": { "role": "user", "content": [{"type":"text","text": prompt}] }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        assert_no_leak(&out, &[prompt]);
    }

    #[test]
    fn assistant_text_thinking_and_tool_input_never_leak(
        text in "[A-Za-z0-9 ]{20,200}",
        thinking in "[A-Za-z0-9 ]{20,200}",
        path in "[A-Za-z0-9_/.-]{5,40}",
        cmd in "[A-Za-z0-9 _-]{5,40}",
    ) {
        let rec_json = json!({
            "type": "assistant",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:01.000Z",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "usage": { "input_tokens": 1, "output_tokens": 1 },
                "content": [
                    { "type": "text", "text": text },
                    { "type": "thinking", "thinking": thinking },
                    { "type": "tool_use", "id": "x", "name": "Bash", "input": { "command": cmd } },
                    { "type": "tool_use", "id": "y", "name": "Read", "input": { "file_path": path } },
                ]
            }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);

        // file_path IS allowed to leak — it's metadata, not prompt.
        // Bash full command is NOT allowed; only the leading verb leaks.
        // text and thinking are NOT allowed to leak.
        assert_no_leak(&out, &[text, thinking]);

        // Verify the verb is recorded but the full command is dropped.
        let bash_event = out.iter().find_map(|e| match e {
            DerivedEvent::ToolCall { tool_name, bash_verb, .. } if tool_name == "Bash" =>
                Some(bash_verb.clone()),
            _ => None,
        }).expect("bash tool call emitted");
        let dump = out.iter().map(event_to_string).collect::<Vec<_>>().join("\n");
        if cmd.split_whitespace().count() > 1 {
            // The full command must not appear; the verb alone may.
            prop_assert!(!dump.contains(&cmd), "full bash command leaked: {cmd:?}");
        }
        prop_assert!(bash_event.is_some());
    }
}
```

- [ ] **Step 3: Run the property test**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_privacy
```

Expected: 2 PASS, each with 256 cases.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/Cargo.toml src-tauri/tests/jsonl_privacy.rs
git commit -m "test(jsonl): privacy property test verifies reducer trust boundary"
```

---

## Phase 4 — Parser, walker, aggregator, reconciler

### Task 9: `jsonl::parser` — streaming line reader with error capture

**Files:**
- Create: `src-tauri/src/jsonl/parser.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register module**

```rust
pub mod record;
pub mod pricing;
pub mod reducer;
pub mod parser;
```

- [ ] **Step 2: Write `parser.rs`**

```rust
//! Streaming JSONL parser. Reads a file line-by-line, returning
//! `Ok(JsonlRecord)` on success and capturing all per-line errors in
//! `ParseErr` so callers can route them to the `jsonl_errors` table.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::fs::File;

use crate::jsonl::record::{parse_line, JsonlRecord};

#[derive(Debug)]
pub struct ParseErr {
    pub file: PathBuf,
    pub line_no: usize,
    pub kind: ErrKind,
    pub msg: String,
    pub cc_version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrKind {
    JsonParse,
    UnknownType,
    MissingField,
    ReducerPanic,
}

impl ErrKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrKind::JsonParse    => "json_parse",
            ErrKind::UnknownType  => "unknown_type",
            ErrKind::MissingField => "missing_field",
            ErrKind::ReducerPanic => "reducer_panic",
        }
    }
}

/// Iterate every JSONL line in `path`. For each line, the callback receives
/// either a parsed record or a `ParseErr`. Returning `false` from the
/// callback aborts the iteration.
pub fn for_each_record<F>(path: &Path, mut callback: F) -> std::io::Result<()>
where F: FnMut(Result<JsonlRecord, ParseErr>) -> bool {
    let f = File::open(path)?;
    let reader = BufReader::new(f);
    for (line_no_zero, line_result) in reader.lines().enumerate() {
        let line_no = line_no_zero + 1;
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                let cont = callback(Err(ParseErr {
                    file: path.to_path_buf(),
                    line_no,
                    kind: ErrKind::JsonParse,
                    msg: format!("read error: {e}"),
                    cc_version: None,
                }));
                if !cont { return Ok(()) } else { continue }
            }
        };
        if line.trim().is_empty() { continue }
        let parsed = parse_line(&line);
        let event = match parsed {
            Ok(rec) => Ok(rec),
            Err(e) => Err(ParseErr {
                file: path.to_path_buf(),
                line_no,
                kind: ErrKind::JsonParse,
                msg: e.to_string(),
                cc_version: None,
            }),
        };
        if !callback(event) { return Ok(()) }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parses_valid_lines_and_captures_invalid() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"type":"user","sessionId":"s1"}}"#).unwrap();
        writeln!(f, r#"not valid json"#).unwrap();
        writeln!(f, "").unwrap(); // blank line, skipped
        writeln!(f, r#"{{"type":"assistant","sessionId":"s1"}}"#).unwrap();

        let mut oks = 0;
        let mut errs = 0;
        for_each_record(f.path(), |r| {
            match r {
                Ok(_) => oks += 1,
                Err(_) => errs += 1,
            }
            true
        }).unwrap();
        assert_eq!(oks, 2);
        assert_eq!(errs, 1);
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::parser
```

Expected: 1 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/parser.rs
git commit -m "feat(jsonl): streaming parser with per-line error capture"
```

---

### Task 10: `jsonl::walker` — enumerate ~/.claude/projects transcripts

**Files:**
- Create: `src-tauri/src/jsonl/walker.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register**

```rust
pub mod walker;
```

- [ ] **Step 2: Write `walker.rs`**

```rust
//! Enumerate Claude Code per-session JSONL transcripts under
//! `<claude_home>/projects/<slug>/*.jsonl`.

use std::path::{Path, PathBuf};

/// Return every `*.jsonl` file under `<claude_home>/projects/*/`.
/// Quietly skips read errors on individual directories — the caller
/// gets whatever was readable.
pub fn enumerate(claude_home: &Path) -> Vec<PathBuf> {
    let projects = claude_home.join("projects");
    let mut out = vec![];
    let Ok(slugs) = std::fs::read_dir(&projects) else { return out };
    for slug in slugs.flatten() {
        if !slug.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue }
        let Ok(files) = std::fs::read_dir(slug.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn enumerates_jsonl_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("projects").join("repo--foo");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.jsonl"), b"{}").unwrap();
        fs::write(proj.join("ignored.txt"), b"x").unwrap();
        fs::write(proj.join("b.jsonl"), b"{}").unwrap();

        let found = enumerate(tmp.path());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "jsonl"));
    }

    #[test]
    fn missing_claude_home_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(enumerate(tmp.path()).is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::walker
```

Expected: 2 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/walker.rs
git commit -m "feat(jsonl): walker enumerates transcripts under ~/.claude/projects"
```

---

### Task 11: `jsonl::aggregator` — build behaviour_summary + tool_sequences

**Files:**
- Create: `src-tauri/src/jsonl/aggregator.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register**

```rust
pub mod aggregator;
```

- [ ] **Step 2: Write `aggregator.rs`**

```rust
//! Builds per-session aggregates from a `DerivedEvent` stream.
//! Output: a `BehaviourSummary` row, a list of tool-sequence edges,
//! and per-session counts the ingestor writes to `behaviour_summary`,
//! `tool_sequences`, `slash_commands`, and `subagent_calls`.

use std::collections::HashMap;
use crate::jsonl::reducer::DerivedEvent;

#[derive(Debug, Default, Clone)]
pub struct BehaviourSummary {
    pub session_id: String,
    pub file_thrashed_max: i64,
    pub file_thrashed_path: Option<String>,
    pub command_retried_max: i64,
    pub command_retried_verb: Option<String>,
    pub read_storm_max: i64,
    pub read_count: i64,
    pub edit_count: i64,
    pub total_tool_calls: i64,
    pub thinking_tokens: i64,
    pub subagent_count: i64,
}

#[derive(Debug, Default, Clone)]
pub struct Aggregates {
    pub summary: BehaviourSummary,
    pub tool_sequence_edges: HashMap<(String, String), i64>, // (from, to) -> count
}

pub fn aggregate(session_id: &str, events: &[DerivedEvent]) -> Aggregates {
    let mut a = Aggregates::default();
    a.summary.session_id = session_id.to_string();

    let mut last_tool: Option<String> = None;
    let mut current_read_run: i64 = 0;
    let mut file_edits: HashMap<String, i64> = HashMap::new();
    let mut cmd_retries: HashMap<(String, i64), i64> = HashMap::new();

    for ev in events {
        match ev {
            DerivedEvent::ToolCall { tool_name, file_path, bash_verb, arg_count, .. } => {
                a.summary.total_tool_calls += 1;

                if let Some(prev) = &last_tool {
                    let key = (prev.clone(), tool_name.clone());
                    *a.tool_sequence_edges.entry(key).or_insert(0) += 1;
                }
                last_tool = Some(tool_name.clone());

                match tool_name.as_str() {
                    "Read" | "Grep" | "Glob" => {
                        a.summary.read_count += 1;
                        current_read_run += 1;
                        a.summary.read_storm_max = a.summary.read_storm_max.max(current_read_run);
                    }
                    "Edit" | "Write" | "MultiEdit" => {
                        a.summary.edit_count += 1;
                        current_read_run = 0;
                        if let Some(fp) = file_path {
                            let n = file_edits.entry(fp.clone()).and_modify(|v| *v += 1).or_insert(1);
                            if *n > a.summary.file_thrashed_max {
                                a.summary.file_thrashed_max = *n;
                                a.summary.file_thrashed_path = Some(fp.clone());
                            }
                        }
                    }
                    "Bash" => {
                        current_read_run = 0;
                        if let Some(verb) = bash_verb {
                            let n = cmd_retries
                                .entry((verb.clone(), *arg_count))
                                .and_modify(|v| *v += 1)
                                .or_insert(1);
                            if *n > a.summary.command_retried_max {
                                a.summary.command_retried_max = *n;
                                a.summary.command_retried_verb = Some(verb.clone());
                            }
                        }
                    }
                    _ => { current_read_run = 0; }
                }
            }
            DerivedEvent::ThinkingTokens { tokens, .. } => {
                a.summary.thinking_tokens += tokens;
            }
            DerivedEvent::SubAgentCall { .. } => {
                a.summary.subagent_count += 1;
            }
            _ => {}
        }
    }

    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, file: Option<&str>, verb: Option<&str>) -> DerivedEvent {
        DerivedEvent::ToolCall {
            session_id: "s1".into(), ts: 0, turn_idx: 0,
            tool_name: name.into(),
            file_path: file.map(|s| s.into()),
            bash_verb: verb.map(|s| s.into()),
            arg_count: 0, is_error: false, duration_ms: None,
            model: None, tool_use_id: None,
        }
    }

    #[test]
    fn file_thrashed_picks_max_count_and_path() {
        let events = vec![
            tool("Edit", Some("a.rs"), None),
            tool("Edit", Some("a.rs"), None),
            tool("Edit", Some("b.rs"), None),
            tool("Edit", Some("a.rs"), None),
            tool("Edit", Some("a.rs"), None),
        ];
        let a = aggregate("s1", &events);
        assert_eq!(a.summary.file_thrashed_max, 4);
        assert_eq!(a.summary.file_thrashed_path.as_deref(), Some("a.rs"));
    }

    #[test]
    fn read_storm_counts_consecutive_reads_only() {
        let events = vec![
            tool("Read", None, None),
            tool("Read", None, None),
            tool("Read", None, None),
            tool("Edit", Some("x.rs"), None),
            tool("Read", None, None),
            tool("Read", None, None),
        ];
        let a = aggregate("s1", &events);
        assert_eq!(a.summary.read_storm_max, 3);
        assert_eq!(a.summary.read_count, 5);
        assert_eq!(a.summary.edit_count, 1);
    }

    #[test]
    fn command_retried_counts_same_verb_arg_count() {
        let events = vec![
            tool("Bash", None, Some("git")),
            tool("Bash", None, Some("git")),
            tool("Bash", None, Some("cargo")),
            tool("Bash", None, Some("git")),
        ];
        let a = aggregate("s1", &events);
        assert_eq!(a.summary.command_retried_max, 3);
        assert_eq!(a.summary.command_retried_verb.as_deref(), Some("git"));
    }

    #[test]
    fn tool_sequence_edges_counted() {
        let events = vec![
            tool("Read", None, None),
            tool("Edit", Some("x.rs"), None),
            tool("Read", None, None),
            tool("Edit", Some("y.rs"), None),
        ];
        let a = aggregate("s1", &events);
        assert_eq!(a.tool_sequence_edges.get(&("Read".into(), "Edit".into())), Some(&2));
        assert_eq!(a.tool_sequence_edges.get(&("Edit".into(), "Read".into())), Some(&1));
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::aggregator
```

Expected: 4 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/aggregator.rs
git commit -m "feat(jsonl): aggregator computes behaviour_summary and tool_sequence edges"
```

---

### Task 12: `jsonl::reconciler` — OTLP-vs-JSONL routing

**Files:**
- Create: `src-tauri/src/jsonl/reconciler.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register**

```rust
pub mod reconciler;
```

- [ ] **Step 2: Write `reconciler.rs`**

```rust
//! Per-session OTLP-vs-JSONL routing. A session is "OTLP-covered" if any
//! `token_usage` row exists for it; in that case JSONL skips legacy
//! tables and only writes the new behavioural ones.

use anyhow::Result;
use rusqlite::params;
use std::sync::Arc;

use crate::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    Otlp,
    JsonlOnly,
}

pub fn coverage_for(pool: &Arc<DbPool>, session_id: &str) -> Result<Coverage> {
    let conn = pool.get()?;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM token_usage WHERE session_id = ?1 LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(if n > 0 { Coverage::Otlp } else { Coverage::JsonlOnly })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn pool() -> Arc<DbPool> {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let p = crate::db::init(&db_path).unwrap();
        // Leak tempdir for the lifetime of the test.
        Box::leak(Box::new(dir));
        Arc::new(p)
    }

    #[test]
    fn session_with_no_token_usage_is_jsonl_only() {
        let p = pool();
        assert_eq!(coverage_for(&p, "sX").unwrap(), Coverage::JsonlOnly);
    }

    #[test]
    fn session_with_token_usage_is_otlp_covered() {
        let p = pool();
        let conn = p.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('sY', 0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO token_usage (session_id, timestamp, model, token_type, count)
             VALUES ('sY', 0, 'm', 'input', 1)",
            [],
        ).unwrap();
        assert_eq!(coverage_for(&p, "sY").unwrap(), Coverage::Otlp);
    }
}
```

- [ ] **Step 3: Run tests**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reconciler
```

Expected: 2 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/reconciler.rs
git commit -m "feat(jsonl): reconciler determines OTLP vs JSONL coverage per session"
```

---

## Phase 5 — Ingestor integration + public API

### Task 13: Ingestor — `ingest_derived` method writes DerivedEvents to DB

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs` — add `ingest_derived(events: &[DerivedEvent], coverage: Coverage)`.
- Test: new file `src-tauri/tests/jsonl_ingest_writes.rs`.

- [ ] **Step 1: Add imports + method to `Ingestor`**

In `src-tauri/src/otlp/ingestor.rs`, near the top of `impl Ingestor`, add:

```rust
pub fn ingest_derived(
    &self,
    events: &[crate::jsonl::reducer::DerivedEvent],
    coverage: crate::jsonl::reconciler::Coverage,
) -> Result<()> {
    use crate::jsonl::reducer::DerivedEvent as E;
    use crate::jsonl::reconciler::Coverage;

    if self.control.is_paused() { return Ok(()); }
    let mut conn = self.pool.get()?;
    let tx = conn.transaction()?;

    for ev in events {
        match ev {
            E::SessionLifecycle { session_id, started_at, ended_at, cc_version, cwd, git_branch } => {
                // UPSERT semantics: insert if missing, otherwise leave OTLP values alone.
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO sessions
                       (session_id, started_at, ended_at, service_version, cwd, repo_branch, data_source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'jsonl')",
                    params![session_id, started_at, ended_at, cc_version, cwd, git_branch],
                );
                // If this is OTLP-covered, ensure data_source reflects 'mixed' once
                // JSONL has also contributed.
                if matches!(coverage, Coverage::Otlp) {
                    let _ = tx.execute(
                        "UPDATE sessions SET data_source = 'mixed'
                         WHERE session_id = ?1 AND data_source = 'otlp'",
                        params![session_id],
                    );
                }
            }
            E::TokenUsage { session_id, ts, model, input, output, cache_create, cache_read } => {
                if matches!(coverage, Coverage::JsonlOnly) {
                    for (kind, n) in [("input", *input), ("output", *output),
                                      ("cacheRead", *cache_read), ("cacheCreation", *cache_create)] {
                        if n > 0 {
                            let _ = tx.execute(
                                "INSERT INTO token_usage
                                   (session_id, timestamp, model, token_type, count)
                                 VALUES (?1, ?2, ?3, ?4, ?5)",
                                params![session_id, ts, model, kind, n],
                            );
                        }
                    }
                }
            }
            E::CostEntry { session_id, ts, model, cost_usd } => {
                if matches!(coverage, Coverage::JsonlOnly) && *cost_usd > 0.0 {
                    let _ = tx.execute(
                        "INSERT INTO cost_entries (session_id, timestamp, model, cost_usd)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![session_id, ts, model, cost_usd],
                    );
                }
            }
            E::ToolCall { session_id, ts, turn_idx, tool_name, file_path,
                          bash_verb, arg_count, is_error, duration_ms, model, .. } => {
                // For OTLP-covered sessions, OTLP already populates tool_decisions
                // for prompting tools. JSONL writes for OTLP-covered sessions ONLY
                // if no decision row exists for this turn — but the simpler rule
                // per the spec is: skip entirely. We follow the spec strictly.
                if matches!(coverage, Coverage::JsonlOnly) {
                    let _ = tx.execute(
                        "INSERT INTO tool_decisions
                           (session_id, timestamp, tool_name, decision, language, file_path,
                            source, model, bash_verb, arg_count, is_error, duration_ms, turn_idx)
                         VALUES (?1, ?2, ?3, NULL, NULL, ?4, 'jsonl', ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![session_id, ts, tool_name, file_path, model, bash_verb,
                                arg_count, *is_error as i64, duration_ms, turn_idx],
                    );
                }
            }
            E::ToolResult { is_error, duration_ms, tool_use_id, .. } => {
                // Aggregator already folded into ToolCall when possible;
                // for cross-record correlation, patch the most recent matching
                // tool_decisions row by tool_use_id is too complex for v1.
                // We rely on the same-record correlation captured in the reducer.
                let _ = (is_error, duration_ms, tool_use_id);
            }
            E::SlashCommand { session_id, ts, name, arg_count } => {
                let _ = tx.execute(
                    "INSERT INTO slash_commands (session_id, timestamp, command_name, arg_count)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![session_id, ts, name, arg_count],
                );
            }
            E::SubAgentCall { parent_id, child_id, subagent_type, started_at } => {
                let _ = tx.execute(
                    "INSERT INTO subagent_calls
                       (parent_session_id, child_session_id, subagent_type, started_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![parent_id, child_id, subagent_type, started_at],
                );
            }
            E::ThinkingTokens { .. } => { /* folded into behaviour_summary by aggregator */ }
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn write_behaviour_summary(
    &self,
    agg: &crate::jsonl::aggregator::Aggregates,
) -> Result<()> {
    let mut conn = self.pool.get()?;
    let tx = conn.transaction()?;
    let s = &agg.summary;
    let _ = tx.execute(
        "INSERT INTO behaviour_summary
           (session_id, file_thrashed_max, file_thrashed_path,
            command_retried_max, command_retried_verb,
            read_storm_max, read_count, edit_count, total_tool_calls,
            thinking_tokens, subagent_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(session_id) DO UPDATE SET
           file_thrashed_max    = excluded.file_thrashed_max,
           file_thrashed_path   = excluded.file_thrashed_path,
           command_retried_max  = excluded.command_retried_max,
           command_retried_verb = excluded.command_retried_verb,
           read_storm_max       = excluded.read_storm_max,
           read_count           = excluded.read_count,
           edit_count           = excluded.edit_count,
           total_tool_calls     = excluded.total_tool_calls,
           thinking_tokens      = excluded.thinking_tokens,
           subagent_count       = excluded.subagent_count",
        params![s.session_id, s.file_thrashed_max, s.file_thrashed_path,
                s.command_retried_max, s.command_retried_verb,
                s.read_storm_max, s.read_count, s.edit_count, s.total_tool_calls,
                s.thinking_tokens, s.subagent_count],
    );
    for ((from_tool, to_tool), count) in &agg.tool_sequence_edges {
        let _ = tx.execute(
            "INSERT INTO tool_sequences (session_id, from_tool, to_tool, count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, from_tool, to_tool) DO UPDATE SET
               count = excluded.count",
            params![s.session_id, from_tool, to_tool, count],
        );
    }
    tx.commit()?;
    Ok(())
}
```

- [ ] **Step 2: Write integration test**

Create `src-tauri/tests/jsonl_ingest_writes.rs`:

```rust
mod common;

use andon_lib::jsonl::aggregator::{Aggregates, BehaviourSummary};
use andon_lib::jsonl::reconciler::Coverage;
use andon_lib::jsonl::reducer::DerivedEvent;
use common::{fixture_pool, test_ingestor};

#[test]
fn ingest_derived_writes_slash_commands_and_subagents() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![
        DerivedEvent::SlashCommand {
            session_id: "s1".into(), ts: 100, name: "review".into(), arg_count: 1,
        },
        DerivedEvent::SubAgentCall {
            parent_id: "s1".into(), child_id: Some("s1-child".into()),
            subagent_type: Some("Explore".into()), started_at: 110,
        },
    ];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();

    let conn = pool.get().unwrap();
    let cmd: String = conn.query_row(
        "SELECT command_name FROM slash_commands WHERE session_id='s1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(cmd, "review");
    let st: String = conn.query_row(
        "SELECT subagent_type FROM subagent_calls WHERE parent_session_id='s1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(st, "Explore");
}

#[test]
fn ingest_derived_skips_token_usage_when_otlp_covered() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(), ts: 100, model: "claude-opus-4-7".into(),
        input: 10, output: 20, cache_create: 0, cache_read: 0,
    }];
    ing.ingest_derived(&events, Coverage::Otlp).unwrap();

    let conn = pool.get().unwrap();
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM token_usage WHERE session_id='s1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 0, "JSONL must not write token_usage for OTLP-covered sessions");
}

#[test]
fn write_behaviour_summary_upserts() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let mut agg = Aggregates::default();
    agg.summary = BehaviourSummary {
        session_id: "s1".into(),
        file_thrashed_max: 7, file_thrashed_path: Some("a.rs".into()),
        ..Default::default()
    };
    ing.write_behaviour_summary(&agg).unwrap();
    // Second write — must not panic on conflict.
    ing.write_behaviour_summary(&agg).unwrap();

    let conn = pool.get().unwrap();
    let v: i64 = conn.query_row(
        "SELECT file_thrashed_max FROM behaviour_summary WHERE session_id='s1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(v, 7);
}
```

- [ ] **Step 3: Run**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
```

Expected: 3 PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(ingestor): ingest_derived writes DerivedEvents respecting OTLP coverage"
```

---

### Task 14: `jsonl::mod` — public API `backfill` and `ingest_one`

**Files:**
- Modify: `src-tauri/src/jsonl/mod.rs` — add public `backfill()` and `ingest_one()` and `IngestStats`.
- Test: new `src-tauri/tests/jsonl_pipeline.rs`.

- [ ] **Step 1: Extend `mod.rs`**

Replace the contents of `src-tauri/src/jsonl/mod.rs` with:

```rust
//! JSONL transcript ingestion. See
//! docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md.

pub mod record;
pub mod pricing;
pub mod reducer;
pub mod parser;
pub mod walker;
pub mod aggregator;
pub mod reconciler;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rusqlite::params;

use crate::db::DbPool;
use crate::otlp::ingestor::Ingestor;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IngestStats {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub duration_ms: i64,
}

/// Walk every JSONL file under `<claude_home>/projects/<slug>/` and
/// ingest each. Idempotent: re-running collapses to UPSERT for
/// `behaviour_summary` and `tool_sequences`; legacy tables use
/// INSERT OR IGNORE on `sessions` and never duplicate for OTLP-covered
/// sessions (reconciler keeps them out).
#[tracing::instrument(skip(pool, ingestor))]
pub async fn backfill(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    claude_home: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "backfill", started_at)?;

    let mut stats = IngestStats::default();
    let files = walker::enumerate(claude_home);
    stats.files_processed = files.len() as i64;

    for path in &files {
        let r = ingest_one_inner(pool, ingestor, path).await;
        match r {
            Ok(s) => {
                stats.records_processed += s.records_processed;
                stats.records_errored   += s.records_errored;
                stats.sessions_added    += s.sessions_added;
            }
            Err(e) => {
                tracing::error!(?path, error = ?e, "jsonl ingest failed for file");
                stats.records_errored += 1;
            }
        }
    }

    stats.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &stats)?;
    Ok(stats)
}

/// Ingest a single transcript file. Used by the SessionEnd hook handler.
#[tracing::instrument(skip(pool, ingestor))]
pub async fn ingest_one(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    transcript_path: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "session_end", started_at)?;
    let stats = ingest_one_inner(pool, ingestor, transcript_path).await?;
    let mut s = stats.clone();
    s.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &s)?;
    Ok(s)
}

async fn ingest_one_inner(
    pool: &Arc<DbPool>,
    ingestor: &Ingestor,
    path: &Path,
) -> Result<IngestStats> {
    use std::panic::AssertUnwindSafe;
    use futures::FutureExt;

    let path_owned = path.to_path_buf();
    let pool_clone = Arc::clone(pool);
    // Move into blocking task: rusqlite is sync and we never hold a
    // connection across .await.
    let result = tokio::task::spawn_blocking(move || {
        let mut stats = IngestStats { files_processed: 1, ..Default::default() };
        let mut reducer = reducer::Reducer::new();
        let mut events_by_session: std::collections::HashMap<String, Vec<reducer::DerivedEvent>> =
            std::collections::HashMap::new();

        let _ = parser::for_each_record(&path_owned, |r| {
            stats.records_processed += 1;
            match r {
                Ok(rec) => {
                    let result = std::panic::catch_unwind(AssertUnwindSafe(|| reducer.reduce(&rec)));
                    match result {
                        Ok(events) => {
                            for ev in events {
                                let sid = event_session_id(&ev);
                                if let Some(sid) = sid {
                                    events_by_session.entry(sid).or_default().push(ev);
                                }
                            }
                        }
                        Err(_) => {
                            log_jsonl_error(&pool_clone, &path_owned, 0,
                                            "reducer_panic", "reducer panicked", None);
                            stats.records_errored += 1;
                        }
                    }
                }
                Err(e) => {
                    log_jsonl_error(&pool_clone, &e.file, e.line_no,
                                    e.kind.as_str(), &e.msg, e.cc_version.as_deref());
                    stats.records_errored += 1;
                }
            }
            true
        });

        // Per-session: determine coverage, write events, write summary.
        for (sid, events) in events_by_session {
            let coverage = reconciler::coverage_for(&pool_clone, &sid)
                .unwrap_or(reconciler::Coverage::JsonlOnly);
            if let Err(e) = ingestor.ingest_derived(&events, coverage) {
                tracing::error!(sid, error = ?e, "ingest_derived failed");
            }
            let agg = aggregator::aggregate(&sid, &events);
            if let Err(e) = ingestor.write_behaviour_summary(&agg) {
                tracing::error!(sid, error = ?e, "write_behaviour_summary failed");
            }
            stats.sessions_added += 1;
        }
        Ok::<_, anyhow::Error>(stats)
    }).await??;

    Ok(result)
}

fn event_session_id(ev: &reducer::DerivedEvent) -> Option<String> {
    use reducer::DerivedEvent::*;
    match ev {
        SessionLifecycle { session_id, .. }
        | TokenUsage { session_id, .. }
        | CostEntry { session_id, .. }
        | ToolCall { session_id, .. }
        | ToolResult { session_id, .. }
        | SlashCommand { session_id, .. }
        | ThinkingTokens { session_id, .. }
            => Some(session_id.clone()),
        SubAgentCall { parent_id, .. } => Some(parent_id.clone()),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn insert_run(pool: &Arc<DbPool>, kind: &str, started_at: i64) -> Result<i64> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO jsonl_ingest_runs (kind, started_at) VALUES (?1, ?2)",
        params![kind, started_at],
    )?;
    Ok(conn.last_insert_rowid())
}

fn finalise_run(pool: &Arc<DbPool>, id: i64, stats: &IngestStats) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE jsonl_ingest_runs
         SET ended_at = ?1, files_processed = ?2,
             records_processed = ?3, records_errored = ?4
         WHERE id = ?5",
        params![now_ms(), stats.files_processed, stats.records_processed,
                stats.records_errored, id],
    )?;
    Ok(())
}

fn log_jsonl_error(pool: &Arc<DbPool>, path: &Path, line_no: usize,
                   kind: &str, msg: &str, cc_version: Option<&str>) {
    let Ok(conn) = pool.get() else { return };
    let _ = conn.execute(
        "INSERT INTO jsonl_errors (jsonl_path, line_no, error_kind, error_msg, cc_version, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![path.display().to_string(), line_no as i64, kind, msg, cc_version, now_ms()],
    );
}
```

- [ ] **Step 2: Add `futures` dep if missing**

In `src-tauri/Cargo.toml` under `[dependencies]`, ensure:

```toml
futures = "0.3"
```

- [ ] **Step 3: Write integration test**

Create `src-tauri/tests/jsonl_pipeline.rs`:

```rust
mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use andon_lib::jsonl;
use common::{fixture_pool, test_ingestor};

fn write_transcript(dir: &Path, slug: &str, lines: &[&str]) {
    let proj = dir.join("projects").join(slug);
    fs::create_dir_all(&proj).unwrap();
    let path = proj.join("session.jsonl");
    fs::write(path, lines.join("\n")).unwrap();
}

#[tokio::test]
async fn backfill_processes_synthetic_session() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);

    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "repo--demo", &[
        r#"{"type":"user","sessionId":"sess-1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name><command-args>x</command-args>"}]}}"#,
        r#"{"type":"assistant","sessionId":"sess-1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":50,"output_tokens":100,"cache_read_input_tokens":10},"content":[{"type":"tool_use","id":"u1","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
        r#"{"type":"user","sessionId":"sess-1","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"u1","is_error":false,"content":"..."}]}}"#,
        r#"{"type":"assistant","sessionId":"sess-1","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":5,"output_tokens":50},"content":[{"type":"tool_use","id":"u2","name":"Edit","input":{"file_path":"a.rs"}}]}}"#,
    ]);

    let stats = jsonl::backfill(&Arc::new(pool.as_ref().clone()), &ing, home.path()).await.unwrap();
    assert_eq!(stats.files_processed, 1);
    assert!(stats.records_processed >= 4);
    assert_eq!(stats.records_errored, 0);

    let conn = pool.get().unwrap();
    let session_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE session_id='sess-1'", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(session_count, 1);

    let slash: i64 = conn.query_row(
        "SELECT COUNT(*) FROM slash_commands WHERE command_name='review'", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(slash, 1);

    let summary_total: i64 = conn.query_row(
        "SELECT total_tool_calls FROM behaviour_summary WHERE session_id='sess-1'",
        [], |r| r.get(0),
    ).unwrap();
    assert!(summary_total >= 2);
}

#[tokio::test]
async fn backfill_is_idempotent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "x", &[
        r#"{"type":"user","sessionId":"sIDP","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
    ]);

    let s1 = jsonl::backfill(&Arc::new(pool.as_ref().clone()), &ing, home.path()).await.unwrap();
    let s2 = jsonl::backfill(&Arc::new(pool.as_ref().clone()), &ing, home.path()).await.unwrap();
    assert_eq!(s1.sessions_added, s2.sessions_added);

    let conn = pool.get().unwrap();
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE session_id='sIDP'", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 1, "second run must not duplicate the session row");
}
```

- [ ] **Step 4: Run**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_pipeline
```

Expected: 2 PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/jsonl/mod.rs src-tauri/tests/jsonl_pipeline.rs src-tauri/Cargo.toml
git commit -m "feat(jsonl): public backfill() and ingest_one() with panic isolation"
```

---

## Phase 6 — API endpoints

### Task 15: `POST /api/jsonl/backfill`

**Files:**
- Modify: `src-tauri/src/api/routes.rs` — new route + handler.
- Modify: `src-tauri/src/api/dto.rs` — `JsonlBackfillResponse`.
- Test: `src-tauri/tests/api_jsonl.rs` (new).

- [ ] **Step 1: DTO**

In `src-tauri/src/api/dto.rs`, add:

```rust
#[derive(Debug, serde::Serialize)]
pub struct JsonlBackfillResponse {
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
    pub sessions_added: i64,
    pub duration_ms: i64,
}

impl From<crate::jsonl::IngestStats> for JsonlBackfillResponse {
    fn from(s: crate::jsonl::IngestStats) -> Self {
        Self {
            files_processed: s.files_processed,
            records_processed: s.records_processed,
            records_errored: s.records_errored,
            sessions_added: s.sessions_added,
            duration_ms: s.duration_ms,
        }
    }
}
```

- [ ] **Step 2: Route + handler**

In `src-tauri/src/api/routes.rs`, add the route in the `router` function near the other `/api/...` routes:

```rust
        .route("/api/jsonl/backfill", post(jsonl_backfill))
```

Add the handler:

```rust
#[tracing::instrument(skip(state))]
async fn jsonl_backfill(State(state): State<ApiState>) -> impl axum::response::IntoResponse {
    let home = match dirs::home_dir() {
        Some(h) => h.join(".claude"),
        None => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error":"no home dir"}))).into_response(),
    };

    let pool = Arc::clone(&state.pool);
    let ingestor = crate::otlp::ingestor::Ingestor::new(
        Arc::clone(&state.pool),
        state.control.clone(),
        state.diagnostics.clone(),
    );

    match crate::jsonl::backfill(&pool, &ingestor, &home).await {
        Ok(stats) => Json(crate::api::dto::JsonlBackfillResponse::from(stats)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "jsonl backfill failed");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}
```

- [ ] **Step 3: Ensure `dirs` crate dep**

Check `Cargo.toml`. Add if missing:

```toml
dirs = "5"
```

- [ ] **Step 4: Test**

Create `src-tauri/tests/api_jsonl.rs`:

```rust
mod common;

use axum::http::StatusCode;
use common::test_router;
use tower::ServiceExt;

#[tokio::test]
async fn backfill_endpoint_returns_stats() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);

    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/jsonl/backfill")
                .header("content-type", "application/json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // We allow either OK (if HOME is set in CI) or 500 (no home dir).
    // The point of the test is: route is wired, handler returns valid JSON.
    assert!(matches!(res.status(), StatusCode::OK | StatusCode::INTERNAL_SERVER_ERROR));
}
```

- [ ] **Step 5: Run**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
```

Expected: 1 PASS.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/Cargo.toml src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): POST /api/jsonl/backfill route"
```

---

### Task 16: Extend `SessionEndPayload` with `transcript_path` + spawn JSONL ingest

**Files:**
- Modify: `src-tauri/src/api/routes.rs:824` (the `SessionEndPayload` struct) and the `hook_session_end` handler around line 831.

- [ ] **Step 1: Extend the payload**

```rust
#[derive(Deserialize)]
struct SessionEndPayload {
    session_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
}
```

- [ ] **Step 2: Spawn JSONL ingest**

In `hook_session_end`, after the existing `tokio::spawn` that generates the report (the `tokio::spawn(async move { let result = ... generate_report ... })` block near line 881), add:

```rust
    // Ingest the JSONL transcript so behavioural tables get populated
    // for this session. Async so the hook stays fast.
    if let Some(tp) = p.transcript_path.clone() {
        let pool_for_jsonl = state.pool.clone();
        let control_for_jsonl = state.control.clone();
        let diag_for_jsonl   = state.diagnostics.clone();
        tokio::spawn(async move {
            let ingestor = crate::otlp::ingestor::Ingestor::new(
                pool_for_jsonl.clone(), control_for_jsonl, diag_for_jsonl,
            );
            let path = std::path::PathBuf::from(tp);
            if let Err(e) = crate::jsonl::ingest_one(&pool_for_jsonl, &ingestor, &path).await {
                tracing::error!(error = ?e, "session-end JSONL ingest failed");
            }
        });
    }
```

- [ ] **Step 3: Test**

Append to `src-tauri/tests/api_jsonl.rs`:

```rust
#[tokio::test]
async fn session_end_with_transcript_path_returns_200() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);

    let body = serde_json::json!({
        "session_id": "s-test",
        "reason": "exit",
        "transcript_path": "/does/not/exist/missing.jsonl",
    });

    let res = router
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/hooks/session-end")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        ).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // The spawned ingest will log an error and exit cleanly.
    // The point: the hook never fails the client.
}
```

- [ ] **Step 4: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): session-end hook ingests JSONL transcript when transcript_path provided"
```

---

### Task 17: `GET /api/jsonl/errors` + `GET /api/jsonl/ingest-runs`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`.
- Modify: `src-tauri/src/api/dto.rs`.

- [ ] **Step 1: DTOs**

```rust
#[derive(Debug, serde::Serialize)]
pub struct JsonlErrorEntry {
    pub jsonl_path: String,
    pub line_no: i64,
    pub error_kind: String,
    pub error_msg: String,
    pub cc_version: Option<String>,
    pub ingested_at: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct JsonlIngestRunEntry {
    pub id: i64,
    pub kind: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub files_processed: i64,
    pub records_processed: i64,
    pub records_errored: i64,
}
```

- [ ] **Step 2: Handlers**

```rust
#[tracing::instrument(skip(state))]
async fn jsonl_errors(State(state): State<ApiState>) -> impl axum::response::IntoResponse {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<_>, rusqlite::Error> {
        let conn = pool.get().map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let mut stmt = conn.prepare(
            "SELECT jsonl_path, line_no, error_kind, error_msg, cc_version, ingested_at
             FROM jsonl_errors ORDER BY ingested_at DESC LIMIT 100",
        )?;
        let iter = stmt.query_map([], |r| Ok(crate::api::dto::JsonlErrorEntry {
            jsonl_path: r.get(0)?, line_no: r.get(1)?, error_kind: r.get(2)?,
            error_msg: r.get(3)?, cc_version: r.get(4)?, ingested_at: r.get(5)?,
        }))?;
        iter.collect::<Result<Vec<_>, _>>()
    }).await.unwrap_or_else(|_| Ok(vec![])).unwrap_or_default();
    Json(rows)
}

#[tracing::instrument(skip(state))]
async fn jsonl_ingest_runs(State(state): State<ApiState>) -> impl axum::response::IntoResponse {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<_>, rusqlite::Error> {
        let conn = pool.get().map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, started_at, ended_at, files_processed, records_processed, records_errored
             FROM jsonl_ingest_runs ORDER BY started_at DESC LIMIT 20",
        )?;
        let iter = stmt.query_map([], |r| Ok(crate::api::dto::JsonlIngestRunEntry {
            id: r.get(0)?, kind: r.get(1)?, started_at: r.get(2)?, ended_at: r.get(3)?,
            files_processed: r.get(4)?, records_processed: r.get(5)?, records_errored: r.get(6)?,
        }))?;
        iter.collect::<Result<Vec<_>, _>>()
    }).await.unwrap_or_else(|_| Ok(vec![])).unwrap_or_default();
    Json(rows)
}
```

- [ ] **Step 3: Wire routes**

```rust
        .route("/api/jsonl/errors", get(jsonl_errors))
        .route("/api/jsonl/ingest-runs", get(jsonl_ingest_runs))
```

- [ ] **Step 4: Test**

Add to `src-tauri/tests/api_jsonl.rs`:

```rust
#[tokio::test]
async fn jsonl_errors_endpoint_returns_empty_array_when_no_errors() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let res = router.oneshot(
        axum::http::Request::builder().method("GET").uri("/api/jsonl/errors")
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1_000_000).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 0);
}
```

- [ ] **Step 5: Commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): GET /api/jsonl/errors and /api/jsonl/ingest-runs"
```

---

### Task 18: Behaviour API endpoints (`/api/behaviour/*`)

**Files:**
- Modify: `src-tauri/src/api/routes.rs` — six new handlers.
- Modify: `src-tauri/src/api/dto.rs` — response DTOs.

- [ ] **Step 1: DTOs**

```rust
#[derive(Debug, serde::Serialize)]
pub struct ModelMixEntry { pub model: String, pub invocations: i64, pub sessions: i64 }

#[derive(Debug, serde::Serialize)]
pub struct ModelToolCell { pub model: String, pub tool: String, pub count: i64 }

#[derive(Debug, serde::Serialize)]
pub struct ModelMixResponse {
    pub by_model: Vec<ModelMixEntry>,
    pub by_model_tool: Vec<ModelToolCell>,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolEdgeEntry { pub from: String, pub to: String, pub count: i64 }

#[derive(Debug, serde::Serialize)]
pub struct ReadEditBin { pub bin_lo: f64, pub bin_hi: f64, pub session_count: i64 }

#[derive(Debug, serde::Serialize)]
pub struct SlashCommandEntry { pub name: String, pub count: i64 }

#[derive(Debug, serde::Serialize)]
pub struct SubAgentEntry {
    pub subagent_type: String,
    pub invocations: i64,
    pub median_tool_calls: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct StuckSessionEntry {
    pub session_id: String,
    pub flag: String,         // "file_thrashed" | "command_retried" | "read_storm"
    pub count: i64,
    pub detail: Option<String>,
}
```

- [ ] **Step 2: Handlers (queries straightforward; each ~20 lines)**

Add to `src-tauri/src/api/routes.rs` (all six together to keep PR atomic):

```rust
async fn behaviour_model_mix(State(state): State<ApiState>) -> Json<crate::api::dto::ModelMixResponse> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || {
        let conn = pool.get().ok()?;
        let by_model: Vec<crate::api::dto::ModelMixEntry> = conn
            .prepare("SELECT model, COUNT(*), COUNT(DISTINCT session_id)
                      FROM token_usage GROUP BY model ORDER BY 2 DESC").ok()?
            .query_map([], |r| Ok(crate::api::dto::ModelMixEntry {
                model: r.get(0)?, invocations: r.get(1)?, sessions: r.get(2)?,
            })).ok()?.filter_map(|r| r.ok()).collect();
        let by_model_tool: Vec<crate::api::dto::ModelToolCell> = conn
            .prepare("SELECT model, tool_name, COUNT(*)
                      FROM tool_decisions
                      WHERE model IS NOT NULL
                      GROUP BY model, tool_name ORDER BY 3 DESC").ok()?
            .query_map([], |r| Ok(crate::api::dto::ModelToolCell {
                model: r.get(0)?, tool: r.get(1)?, count: r.get(2)?,
            })).ok()?.filter_map(|r| r.ok()).collect();
        Some(crate::api::dto::ModelMixResponse { by_model, by_model_tool })
    }).await.ok().flatten().unwrap_or(crate::api::dto::ModelMixResponse {
        by_model: vec![], by_model_tool: vec![],
    });
    Json(out)
}

async fn behaviour_tool_sequences(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::ToolEdgeEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT from_tool, to_tool, SUM(count) FROM tool_sequences
             GROUP BY from_tool, to_tool ORDER BY 3 DESC LIMIT 30"
        ) else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::ToolEdgeEntry {
            from: r.get(0)?, to: r.get(1)?, count: r.get(2)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(out)
}

async fn behaviour_read_edit_ratio(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::ReadEditBin>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let bins = [(0.0,1.0),(1.0,2.0),(2.0,5.0),(5.0,10.0),(10.0,1e9)];
        let mut result = vec![];
        for (lo, hi) in bins {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM behaviour_summary
                 WHERE edit_count > 0
                   AND CAST(read_count AS REAL)/edit_count >= ?1
                   AND CAST(read_count AS REAL)/edit_count <  ?2",
                rusqlite::params![lo, hi], |r| r.get(0),
            ).unwrap_or(0);
            result.push(crate::api::dto::ReadEditBin { bin_lo: lo, bin_hi: hi, session_count: n });
        }
        result
    }).await.unwrap_or_default();
    Json(out)
}

async fn behaviour_slash_commands(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::SlashCommandEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT command_name, COUNT(*) FROM slash_commands GROUP BY command_name ORDER BY 2 DESC LIMIT 30"
        ) else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::SlashCommandEntry {
            name: r.get(0)?, count: r.get(1)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(out)
}

async fn behaviour_subagents(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::SubAgentEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT subagent_type, COUNT(*), COALESCE(CAST(AVG(tool_call_count) AS INTEGER), 0)
             FROM subagent_calls WHERE subagent_type IS NOT NULL
             GROUP BY subagent_type ORDER BY 2 DESC"
        ) else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::SubAgentEntry {
            subagent_type: r.get(0)?, invocations: r.get(1)?, median_tool_calls: r.get(2)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(out)
}

async fn behaviour_stuck_sessions(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::StuckSessionEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT session_id,
                    CASE WHEN file_thrashed_max >= 5 THEN 'file_thrashed'
                         WHEN command_retried_max >= 3 THEN 'command_retried'
                         WHEN read_storm_max >= 10 THEN 'read_storm' END AS flag,
                    CASE WHEN file_thrashed_max >= 5 THEN file_thrashed_max
                         WHEN command_retried_max >= 3 THEN command_retried_max
                         WHEN read_storm_max >= 10 THEN read_storm_max END AS count,
                    CASE WHEN file_thrashed_max >= 5 THEN file_thrashed_path
                         WHEN command_retried_max >= 3 THEN command_retried_verb
                         ELSE NULL END AS detail
             FROM behaviour_summary
             WHERE file_thrashed_max >= 5 OR command_retried_max >= 3 OR read_storm_max >= 10
             ORDER BY count DESC LIMIT 100"
        ) else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::StuckSessionEntry {
            session_id: r.get(0)?, flag: r.get(1)?, count: r.get(2)?, detail: r.get(3)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(out)
}
```

- [ ] **Step 3: Wire routes**

```rust
        .route("/api/behaviour/model-mix", get(behaviour_model_mix))
        .route("/api/behaviour/tool-sequences", get(behaviour_tool_sequences))
        .route("/api/behaviour/read-edit-ratio", get(behaviour_read_edit_ratio))
        .route("/api/behaviour/slash-commands", get(behaviour_slash_commands))
        .route("/api/behaviour/subagents", get(behaviour_subagents))
        .route("/api/behaviour/stuck-sessions", get(behaviour_stuck_sessions))
```

- [ ] **Step 4: Test smoke**

Append to `src-tauri/tests/api_jsonl.rs`:

```rust
#[tokio::test]
async fn behaviour_endpoints_return_200_with_empty_db() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);

    for path in [
        "/api/behaviour/model-mix",
        "/api/behaviour/tool-sequences",
        "/api/behaviour/read-edit-ratio",
        "/api/behaviour/slash-commands",
        "/api/behaviour/subagents",
        "/api/behaviour/stuck-sessions",
    ] {
        let res = router.clone().oneshot(
            axum::http::Request::builder().method("GET").uri(path)
                .body(axum::body::Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {path} not OK");
    }
}
```

- [ ] **Step 5: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): six /api/behaviour/* endpoints for the Behaviour page"
```

---

## Phase 7 — Angular: Behaviour page

### Task 19: API service wrappers + models

**Files:**
- Modify: `web/src/app/core/models.ts` — new interface types.
- Modify: `web/src/app/core/api.service.ts` — new methods.

- [ ] **Step 1: Add models**

In `web/src/app/core/models.ts`, append:

```ts
export interface ModelMixEntry { model: string; invocations: number; sessions: number; }
export interface ModelToolCell { model: string; tool: string; count: number; }
export interface ModelMixResponse { by_model: ModelMixEntry[]; by_model_tool: ModelToolCell[]; }
export interface ToolEdgeEntry { from: string; to: string; count: number; }
export interface ReadEditBin { bin_lo: number; bin_hi: number; session_count: number; }
export interface SlashCommandEntry { name: string; count: number; }
export interface SubAgentEntry { subagent_type: string; invocations: number; median_tool_calls: number; }
export interface StuckSessionEntry { session_id: string; flag: string; count: number; detail: string | null; }
export interface JsonlErrorEntry { jsonl_path: string; line_no: number; error_kind: string; error_msg: string; cc_version: string | null; ingested_at: number; }
export interface JsonlIngestRun { id: number; kind: string; started_at: number; ended_at: number | null; files_processed: number; records_processed: number; records_errored: number; }
export interface JsonlBackfillResponse { files_processed: number; records_processed: number; records_errored: number; sessions_added: number; duration_ms: number; }
```

- [ ] **Step 2: Add API methods**

In `web/src/app/core/api.service.ts`, follow the existing `httpResource` / `firstValueFrom(this.http.get(...))` pattern used by other endpoints. Add:

```ts
modelMix() { return this.get<ModelMixResponse>('/api/behaviour/model-mix'); }
toolSequences() { return this.get<ToolEdgeEntry[]>('/api/behaviour/tool-sequences'); }
readEditRatio() { return this.get<ReadEditBin[]>('/api/behaviour/read-edit-ratio'); }
slashCommands() { return this.get<SlashCommandEntry[]>('/api/behaviour/slash-commands'); }
subagents() { return this.get<SubAgentEntry[]>('/api/behaviour/subagents'); }
stuckSessions() { return this.get<StuckSessionEntry[]>('/api/behaviour/stuck-sessions'); }
jsonlErrors() { return this.get<JsonlErrorEntry[]>('/api/jsonl/errors'); }
jsonlIngestRuns() { return this.get<JsonlIngestRun[]>('/api/jsonl/ingest-runs'); }
ingestJsonl() { return this.post<JsonlBackfillResponse>('/api/jsonl/backfill', {}); }
```

Match the exact wrapper convention used elsewhere in the file (`.get<T>` or `httpResource(() => ...)` — copy from a sibling method like `sessions()`).

- [ ] **Step 3: Commit**

```powershell
git add web/src/app/core/models.ts web/src/app/core/api.service.ts
git commit -m "feat(web): API wrappers and models for behaviour endpoints"
```

---

### Task 20: Behaviour page scaffold + route

**Files:**
- Create: `web/src/app/features/behaviour/behaviour.component.ts`.
- Create: `web/src/app/features/behaviour/behaviour.component.html`.
- Modify: `web/src/app/app.routes.ts`.
- Modify: the top nav (likely `web/src/app/app.component.html`).

- [ ] **Step 1: Scaffold component**

`web/src/app/features/behaviour/behaviour.component.ts`:

```ts
import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { ApiService } from '../../core/api.service';

@Component({
  selector: 'app-behaviour',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './behaviour.component.html',
})
export class BehaviourComponent {
  private readonly api = inject(ApiService);

  readonly modelMix = this.api.modelMix();
  readonly toolSequences = this.api.toolSequences();
  readonly readEdit = this.api.readEditRatio();
  readonly slash = this.api.slashCommands();
  readonly subagents = this.api.subagents();
  readonly stuck = this.api.stuckSessions();
}
```

`web/src/app/features/behaviour/behaviour.component.html`:

```html
<div class="p-6 space-y-8">
  <h1 class="text-2xl font-semibold">Behaviour</h1>

  <section>
    <h2 class="text-lg font-medium mb-3">Model mix (by frequency)</h2>
    @if (modelMix.value(); as mm) {
      <div class="grid grid-cols-2 gap-6">
        <div>
          <h3 class="text-sm text-zinc-400 mb-2">Invocations per model</h3>
          <ul class="space-y-1">
            @for (m of mm.by_model; track m.model) {
              <li class="flex justify-between text-sm">
                <span>{{ m.model }}</span>
                <span class="font-mono">{{ m.invocations }}</span>
              </li>
            }
          </ul>
        </div>
        <div>
          <h3 class="text-sm text-zinc-400 mb-2">Sessions per model</h3>
          <ul class="space-y-1">
            @for (m of mm.by_model; track m.model) {
              <li class="flex justify-between text-sm">
                <span>{{ m.model }}</span>
                <span class="font-mono">{{ m.sessions }}</span>
              </li>
            }
          </ul>
        </div>
      </div>

      <div class="mt-6">
        <h3 class="text-sm text-zinc-400 mb-2">Tools per model</h3>
        <table class="text-xs">
          <tbody>
            @for (cell of mm.by_model_tool; track cell.model + cell.tool) {
              <tr>
                <td class="font-mono pr-3">{{ cell.model }}</td>
                <td class="font-mono pr-3">{{ cell.tool }}</td>
                <td class="text-right text-zinc-400">{{ cell.count }}</td>
              </tr>
            }
          </tbody>
        </table>
      </div>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Tool sequence (top edges)</h2>
    @if (toolSequences.value(); as edges) {
      <ul class="space-y-1">
        @for (e of edges; track e.from + e.to) {
          <li class="flex gap-2 items-center text-sm">
            <span class="font-mono w-24">{{ e.from }}</span>
            <span>→</span>
            <span class="font-mono w-24">{{ e.to }}</span>
            <span class="text-zinc-400 ml-auto">{{ e.count }}</span>
          </li>
        }
      </ul>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Read-to-edit ratio</h2>
    @if (readEdit.value(); as bins) {
      <ul class="space-y-1">
        @for (b of bins; track b.bin_lo) {
          <li class="flex gap-2 text-sm">
            <span class="font-mono w-32">{{ b.bin_lo }} – {{ b.bin_hi }}</span>
            <span class="text-zinc-400">{{ b.session_count }} sessions</span>
          </li>
        }
      </ul>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Slash commands</h2>
    @if (slash.value(); as cmds) {
      <ul class="space-y-1">
        @for (c of cmds; track c.name) {
          <li class="flex justify-between text-sm">
            <span class="font-mono">/{{ c.name }}</span>
            <span class="text-zinc-400">{{ c.count }}</span>
          </li>
        }
      </ul>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Sub-agent usage</h2>
    @if (subagents.value(); as agents) {
      <ul class="space-y-1">
        @for (a of agents; track a.subagent_type) {
          <li class="flex gap-4 text-sm">
            <span class="font-mono">{{ a.subagent_type }}</span>
            <span>{{ a.invocations }} invocations</span>
            <span class="text-zinc-400">median {{ a.median_tool_calls }} tool calls</span>
          </li>
        }
      </ul>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Stuck sessions</h2>
    @if (stuck.value(); as rows) {
      <ul class="space-y-1">
        @for (s of rows; track s.session_id) {
          <li class="flex gap-4 text-sm">
            <span class="font-mono">{{ s.session_id }}</span>
            <span>{{ s.flag }} ×{{ s.count }}</span>
            @if (s.detail) { <span class="text-zinc-400">{{ s.detail }}</span> }
          </li>
        }
      </ul>
    }
  </section>
</div>
```

- [ ] **Step 2: Add route**

In `web/src/app/app.routes.ts`, add a route between `files` and `diagnostics`:

```ts
{
  path: 'behaviour',
  loadComponent: () => import('./features/behaviour/behaviour.component').then(m => m.BehaviourComponent),
},
```

- [ ] **Step 3: Add nav link**

In the top-nav template (look for the existing "Files", "Diagnostics" links — typically in `app.component.html` or a shared nav component), insert a "Behaviour" link in the same style between them.

- [ ] **Step 4: Sanity-build**

```powershell
cd web; npm run build
```

Expected: build succeeds.

- [ ] **Step 5: Commit**

```powershell
git add web/src/app/features/behaviour web/src/app/app.routes.ts web/src/app/app.component.html
git commit -m "feat(web): Behaviour page scaffold with all six sections wired to API"
```

---

## Phase 8 — Angular: Other surfaces

### Task 21: Sessions page — Stuck chip

**Files:**
- Modify: `web/src/app/features/sessions/sessions.component.{ts,html}`.
- Modify: `src-tauri/src/api/routes.rs` — extend `list_sessions` response to join `behaviour_summary` and project `is_stuck` + flag detail.
- Modify: `src-tauri/src/api/dto.rs` — add `is_stuck: bool`, `stuck_flag: Option<String>`, `stuck_detail: Option<String>` to the session list DTO.

- [ ] **Step 1: Backend — project stuck fields on session list**

Find the `list_sessions` query (in `routes.rs` — already has a `JOIN` against `cost_entries`/`token_usage` aggregates per `2026-05-17` sweep). Add this LEFT JOIN:

```sql
LEFT JOIN behaviour_summary b ON b.session_id = s.session_id
```

Add to the SELECT list:

```sql
,(b.file_thrashed_max >= 5 OR b.command_retried_max >= 3 OR b.read_storm_max >= 10) AS is_stuck
,CASE WHEN b.file_thrashed_max >= 5 THEN 'file_thrashed'
      WHEN b.command_retried_max >= 3 THEN 'command_retried'
      WHEN b.read_storm_max >= 10 THEN 'read_storm' END AS stuck_flag
,CASE WHEN b.file_thrashed_max >= 5 THEN b.file_thrashed_path
      WHEN b.command_retried_max >= 3 THEN b.command_retried_verb
      ELSE NULL END AS stuck_detail
```

Update the DTO mapping to read these three new columns. Default `is_stuck=false`, `stuck_flag=None` if the JOIN row is NULL.

- [ ] **Step 2: Frontend — chip rendering**

In `sessions.component.html`, alongside the existing per-row metadata chips, add:

```html
@if (row.is_stuck) {
  <span class="inline-flex items-center px-2 py-0.5 text-xs rounded bg-amber-900/40 text-amber-200"
        [title]="row.stuck_flag + (row.stuck_detail ? ' · ' + row.stuck_detail : '')">
    Stuck
  </span>
}
```

- [ ] **Step 3: Test + commit**

```powershell
cd src-tauri; cargo test --features test-support api_sessions
cd web; npm run build; cd ..
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs web/src/app/features/sessions
git commit -m "feat(sessions): Stuck chip column powered by behaviour_summary"
```

---

### Task 22: Overview — Invocations-by-model companion chart

**Files:**
- Modify: `web/src/app/features/overview/overview.component.{ts,html}`.

- [ ] **Step 1: Wire data**

The Overview already calls `modelMix()` indirectly via the existing cost-by-model chart? If not, inject the same `api.modelMix()` resource. Otherwise reuse the same data.

In the `.ts`:

```ts
readonly modelMix = this.api.modelMix();
```

- [ ] **Step 2: Insert the sibling chart**

Right next to the existing "Cost by model" tile in the template, add:

```html
<div class="rounded-lg border border-zinc-800 p-4">
  <h3 class="text-sm text-zinc-400 mb-2">Invocations by model</h3>
  @if (modelMix.value(); as mm) {
    <ul class="space-y-1">
      @for (m of mm.by_model; track m.model) {
        <li class="flex justify-between text-sm">
          <span>{{ m.model }}</span>
          <span class="font-mono">{{ m.invocations }}</span>
        </li>
      }
    </ul>
  }
</div>
```

- [ ] **Step 3: Build + commit**

```powershell
cd web; npm run build; cd ..
git add web/src/app/features/overview
git commit -m "feat(overview): 'Invocations by model' companion to Cost by model"
```

---

### Task 23: Session detail — three-stat enrichment row

**Files:**
- Modify: `src-tauri/src/api/routes.rs` — `session_detail` response includes the three behaviour_summary stats.
- Modify: `src-tauri/src/api/dto.rs` — extend `SessionDetail` DTO.
- Modify: `web/src/app/features/sessions/session-detail.component.{ts,html}`.

- [ ] **Step 1: Backend**

In `session_detail`'s SQL, LEFT JOIN `behaviour_summary` and project `read_count`, `edit_count`, `subagent_count`. The DTO gains:

```rust
pub read_to_edit: Option<f64>,
pub slash_commands_used: i64,
pub subagent_count: i64,
```

For `slash_commands_used`, run an additional small query:

```sql
SELECT COUNT(*) FROM slash_commands WHERE session_id = ?1
```

- [ ] **Step 2: Frontend**

In `session-detail.component.html`, immediately below the existing KPI row, add:

```html
<div class="flex gap-6 text-sm text-zinc-400 mt-2">
  <span>R:E ratio <span class="text-zinc-200 font-mono">{{ readEditDisplay() }}</span></span>
  <span>Slash commands <span class="text-zinc-200 font-mono">{{ session()?.slash_commands_used ?? 0 }}</span></span>
  <span>Sub-agents <span class="text-zinc-200 font-mono">{{ session()?.subagent_count ?? 0 }}</span></span>
</div>
```

`readEditDisplay()` formats as either `n/a` (no edits) or one decimal place.

- [ ] **Step 3: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs web/src/app/features/sessions
git commit -m "feat(session-detail): R:E ratio, slash commands, sub-agents row"
```

---

### Task 24: Settings → Data — Ingest JSONL button + status line

**Files:**
- Modify: `web/src/app/features/settings/settings.component.{ts,html}`.

- [ ] **Step 1: TS state**

Inject api + add signals:

```ts
readonly latestRun = this.api.jsonlIngestRuns();
busy = signal(false);
toast = signal<string | null>(null);

async ingest() {
  this.busy.set(true);
  this.toast.set(null);
  try {
    const stats = await this.api.ingestJsonl();
    this.toast.set(`Ingested ${stats.records_processed} records from ${stats.files_processed} files (${stats.records_errored} errors).`);
    this.latestRun.reload?.();
  } catch (e) {
    this.toast.set(`Backfill failed: ${e}`);
  } finally { this.busy.set(false); }
}
```

- [ ] **Step 2: HTML**

In the existing **Data** section, append:

```html
<div class="mt-4 space-y-2">
  <button class="btn-primary" [disabled]="busy()" (click)="ingest()">
    {{ busy() ? 'Ingesting…' : 'Ingest JSONL history' }}
  </button>
  @if (latestRun.value()?.[0]; as run) {
    <p class="text-xs text-zinc-400">
      Last JSONL ingest: {{ run.kind }} ·
      {{ run.records_processed }} records ·
      <a routerLink="/diagnostics" class="underline">{{ run.records_errored }} errors</a>
    </p>
  }
  @if (toast()) { <p class="text-xs text-amber-300">{{ toast() }}</p> }
</div>
```

- [ ] **Step 3: Build + commit**

```powershell
cd web; npm run build; cd ..
git add web/src/app/features/settings
git commit -m "feat(settings): Ingest JSONL history button + last-run status line"
```

---

### Task 25: Diagnostics — JSONL parse errors card

**Files:**
- Modify: `web/src/app/features/diagnostics/diagnostics.component.{ts,html}`.

- [ ] **Step 1: TS**

```ts
readonly jsonlErrors = this.api.jsonlErrors();
```

- [ ] **Step 2: HTML — add card alongside existing**

```html
<div class="rounded-lg border border-zinc-800 p-4 mt-4">
  <h3 class="text-sm text-zinc-400 mb-2">JSONL parse errors</h3>
  @if (jsonlErrors.value(); as errs) {
    @if (errs.length === 0) {
      <p class="text-xs text-zinc-500">No JSONL parse errors recorded.</p>
    } @else {
      <p class="text-xs text-amber-300">{{ errs.length }} recent errors:</p>
      <ul class="text-xs font-mono mt-2 space-y-1 max-h-64 overflow-y-auto">
        @for (e of errs.slice(0, 10); track e.ingested_at + e.line_no) {
          <li>
            <span class="text-zinc-500">[{{ e.error_kind }}]</span>
            <span>{{ e.jsonl_path }}:{{ e.line_no }}</span>
            <span class="text-zinc-500">{{ e.error_msg }}</span>
            @if (e.cc_version) { <span class="text-zinc-500">cc:{{ e.cc_version }}</span> }
          </li>
        }
      </ul>
    }
  }
</div>
```

- [ ] **Step 3: Build + commit**

```powershell
cd web; npm run build; cd ..
git add web/src/app/features/diagnostics
git commit -m "feat(diagnostics): JSONL parse errors card"
```

---

## Phase 9 — Smoke test + docs + final verification

### Task 26: `scripts/smoke_jsonl.py` — end-to-end smoke

**Files:**
- Create: `scripts/smoke_jsonl.py`.

- [ ] **Step 1: Write the script**

```python
#!/usr/bin/env python3
"""
End-to-end smoke for the JSONL ingest path.

1. Writes a synthetic JSONL file under a temp <claude_home>/projects/<slug>/.
2. Calls POST /api/jsonl/backfill (Andon must be running).
3. Asserts that the response shows records_processed > 0 and
   that GET /api/sessions includes the synthetic session id.

Run: python smoke_jsonl.py
No deps beyond stdlib.
"""
import json, os, pathlib, sys, tempfile, time, urllib.request

API = "http://127.0.0.1:8765"

def post(path, body=None):
    data = json.dumps(body or {}).encode("utf-8")
    req = urllib.request.Request(f"{API}{path}", data=data, method="POST",
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())

def get(path):
    with urllib.request.urlopen(f"{API}{path}", timeout=10) as r:
        return json.loads(r.read())

def main():
    home = pathlib.Path(os.environ.get("USERPROFILE") or os.environ["HOME"])
    proj = home / ".claude" / "projects" / "andon-smoke-jsonl"
    proj.mkdir(parents=True, exist_ok=True)
    sid = f"smoke-{int(time.time())}"
    transcript = proj / f"{sid}.jsonl"
    lines = [
        {"type":"user","sessionId":sid,"timestamp":"2026-05-19T10:00:00.000Z",
         "cwd":str(home),"gitBranch":"main","version":"2.1.0",
         "message":{"role":"user","content":[{"type":"text","text":"hi"}]}},
        {"type":"assistant","sessionId":sid,"timestamp":"2026-05-19T10:00:01.000Z",
         "message":{"role":"assistant","model":"claude-opus-4-7",
                    "usage":{"input_tokens":10,"output_tokens":20},
                    "content":[{"type":"tool_use","id":"u1","name":"Read",
                                "input":{"file_path":"a.rs"}}]}},
    ]
    transcript.write_text("\n".join(json.dumps(l) for l in lines), encoding="utf-8")

    stats = post("/api/jsonl/backfill")
    print("backfill:", stats)
    assert stats["records_processed"] > 0, "no records processed"

    sessions = get("/api/sessions")
    ids = [s["session_id"] for s in sessions]
    assert sid in ids, f"session {sid} not in /api/sessions"
    print("OK — session present:", sid)

if __name__ == "__main__":
    sys.exit(main() or 0)
```

- [ ] **Step 2: Smoke check**

Manually:
```powershell
cargo tauri dev    # in one terminal — wait until window opens
# in another:
cd scripts; python smoke_jsonl.py
```

Expected output:
```
backfill: {'files_processed': N, 'records_processed': >0, ...}
OK — session present: smoke-1747...
```

- [ ] **Step 3: Commit**

```powershell
git add scripts/smoke_jsonl.py
git commit -m "test: scripts/smoke_jsonl.py end-to-end smoke for JSONL ingest"
```

---

### Task 27: README + pitch updates

**Files:**
- Modify: `README.md` — the comparison table cell "Retroactive".
- Modify: `docs/pitch.md` — replace "never read" with "never persisted".

- [ ] **Step 1: README — update retroactive row**

Replace this row in the comparison table:

```markdown
| Retroactive | No — only sees sessions that ran after install | Yes — every session you've ever run |
```

with:

```markdown
| Retroactive | Yes — Settings → "Ingest JSONL history" walks `~/.claude/projects/` | Yes — every session you've ever run |
```

- [ ] **Step 2: Pitch — update privacy wording**

Replace in `docs/pitch.md`:

```markdown
> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever read or stored.**</ins>
```

with:

```markdown
> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever persisted.**</ins> Andon's JSONL parser reads transcript files locally to derive numeric and structural signals (token counts, tool sequences, file paths), but the reducer drops all prompt and response text before any DB write. Nothing leaves the engineer's machine.
```

- [ ] **Step 3: Commit**

```powershell
git add README.md docs/pitch.md
git commit -m "docs: README + pitch updates for JSONL ingest (retroactive cell, privacy wording)"
```

---

### Task 28: End-to-end verification

- [ ] **Step 1: Full test run**

```powershell
cd src-tauri; cargo test --features test-support
```

Expected: every test passes including new ones.

- [ ] **Step 2: Web build**

```powershell
cd web; npm run build
```

Expected: build succeeds, no TypeScript errors.

- [ ] **Step 3: Local smoke**

```powershell
cargo tauri dev    # one terminal
cd scripts; python smoke_jsonl.py    # other terminal
```

- [ ] **Step 4: Manual UI sanity**

Open the running app and confirm:
- **Behaviour** page exists in nav, all six sections render (empty if DB is fresh — then click "Ingest JSONL history" in Settings → Data and re-load).
- **Sessions** page shows the new "Stuck" chip on any session that meets the threshold (use a real or smoke-generated session).
- **Overview** shows "Invocations by model" next to "Cost by model".
- **Settings → Data** shows the new button and a last-run status line after one click.
- **Diagnostics** shows the "JSONL parse errors" card (probably "No JSONL parse errors recorded.").

- [ ] **Step 5: Push branch and open PR**

```powershell
git push -u origin feature/jsonl-ingest
gh pr create --title "feat: JSONL behavioural ingest" --body "$(cat <<'EOF'
## Summary
- Ingests Claude Code per-session JSONL transcripts to populate behavioural views and backfill pre-OTel history.
- Adds `Behaviour` page (model mix, tool sequences, R:E ratio, slash commands, sub-agents, stuck sessions).
- Adds Stuck chip on Sessions, "Invocations by model" tile on Overview, three-stat row on Session detail.
- Privacy: reducer is the trust boundary; no prompt or response text persists. Verified by `proptest` property test.

## Test plan
- [ ] `cargo test --features test-support` passes
- [ ] `cd web; npm run build` succeeds
- [ ] `cargo tauri dev` runs; `scripts/smoke_jsonl.py` reports OK
- [ ] Manual UI sanity check across Behaviour / Sessions / Overview / Settings / Diagnostics
- [ ] Privacy property test passes 256+ cases

Spec: `docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review checklist (run before opening PR)

1. **Spec coverage:** every numbered goal in the spec has at least one task:
   - Retroactive coverage → Tasks 13, 14, 15.
   - Live behavioural ingest → Task 16 (SessionEnd hook enrichment).
   - Six behavioural signals → Tasks 5, 6, 11, 18, 20.
   - Privacy → Tasks 4–8 (reducer + property test).
2. **Type consistency:** `DerivedEvent` variants in Task 4 match every reference in Tasks 5–14.
3. **Migrations idempotent:** Task 1 explicitly tests `apply()` twice.
4. **No placeholders:** every step has either concrete code, a concrete file path, or a concrete command.

If any of these checks fail, fix inline before opening the PR.



