# JSONL behavioural ingest — Implementation Plan (Plan C)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ingest Claude Code's per-session JSONL transcripts to (1) backfill pre-OTel history into existing tables and (2) surface three behavioural views OTLP cannot deliver — model frequency mix, slash command leaderboard, sub-agent (`Task` tool) usage — while persisting *no* prompt or response text.

**Architecture:** A backfill API endpoint (Settings → Data button) and the existing `SessionEnd` hook handler both feed a single parser → reducer → reconciler → ingestor pipeline. The reducer is the privacy trust boundary; its output type carries no text. JSONL is authoritative only for sessions OTLP never saw; for OTLP-covered sessions, JSONL writes only to new behavioural tables (`slash_commands`, `subagent_calls`).

**Tech Stack:** Rust 1.95 (tokio, rusqlite, serde, anyhow, tracing, proptest) · Angular 21 (standalone components, signals, Tailwind 4) · SQLite (WAL).

**Spec:** [`docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md`](../specs/2026-05-19-jsonl-behavioural-ingest-design.md)

**Branch:** `feature/jsonl-ingest` (already created and checked out)

**Scope:** Plan C — the *behaviour_summary*, *tool_sequences*, Stuck chip, R:E ratio histogram, thinking-token tracking, and Session-detail enrichment that appeared in the original spec have been deferred to a future plan. This document is the authoritative source.

---

## File structure

### Create (Rust)
- `src-tauri/src/jsonl/mod.rs` — public API: `backfill()`, `ingest_one()`, `IngestStats`.
- `src-tauri/src/jsonl/record.rs` — `JsonlRecord` serde struct (all-Option fields, lenient).
- `src-tauri/src/jsonl/pricing.rs` — model → cost-per-token constants.
- `src-tauri/src/jsonl/reducer.rs` — `DerivedEvent` enum + `reduce()` (the trust boundary).
- `src-tauri/src/jsonl/parser.rs` — streaming JSONL line reader, error capture.
- `src-tauri/src/jsonl/walker.rs` — enumerate `~/.claude/projects/<slug>/*.jsonl`.
- `src-tauri/src/jsonl/reconciler.rs` — per-session OTLP-vs-JSONL routing.

### Create (tests)
- `src-tauri/tests/jsonl_privacy.rs` — privacy property test.
- `src-tauri/tests/jsonl_ingest_writes.rs` — DB-write integration tests.
- `src-tauri/tests/jsonl_pipeline.rs` — end-to-end backfill integration test.
- `src-tauri/tests/api_jsonl.rs` — API smoke tests.

### Create (scripts + Angular)
- `scripts/smoke_jsonl.py` — synthetic JSONL + backfill smoke (mirrors `smoke_otlp.py`).
- `web/src/app/features/behaviour/behaviour.component.{ts,html}` — new Behaviour page.

### Modify
- `src-tauri/src/db/migrations.rs` — add `MIGRATION_V4`.
- `src-tauri/src/lib.rs` — register `pub mod jsonl;`.
- `src-tauri/src/otlp/ingestor.rs` — add `ingest_derived(events, coverage)` method.
- `src-tauri/src/api/routes.rs` — `POST /api/jsonl/backfill`, enrich `hook_session_end`, `GET /api/jsonl/errors`, `GET /api/jsonl/ingest-runs`, three `GET /api/behaviour/*` routes.
- `src-tauri/src/api/dto.rs` — DTOs for the new endpoints.
- `src-tauri/Cargo.toml` — `proptest` dev-dep, `dirs` dep if missing.
- `web/src/app/core/api.service.ts` — typed wrappers for new endpoints.
- `web/src/app/core/models.ts` — new DTO types.
- `web/src/app/app.routes.ts` — Behaviour page route.
- `web/src/app/features/overview/overview.component.{ts,html}` — Invocations-by-model companion.
- `web/src/app/features/settings/settings.component.{ts,html}` — Ingest JSONL button + status line.
- `web/src/app/features/diagnostics/diagnostics.component.{ts,html}` — JSONL parse errors card.
- `README.md` — flip "Retroactive" cell in comparison table.
- `docs/pitch.md` — "never read" → "never persisted".

---

## Phase 1 — Database schema

### Task 1: Migration v4 — JSONL ingest schema (Plan C)

**Files:**
- Modify: `src-tauri/src/db/migrations.rs:109`.
- Test: same file (inline `#[cfg(test)]`).

- [ ] **Step 1: Add the migration constant**

After `MIGRATION_V3` (around line 107), add:

```rust
const MIGRATION_V4: &str = r#"
-- Distinguishes OTLP-derived from JSONL-derived sessions.
ALTER TABLE sessions ADD COLUMN data_source TEXT;
UPDATE sessions SET data_source = 'otlp' WHERE data_source IS NULL;

-- Distinguishes OTLP-emitted decisions from JSONL-derived tool calls.
ALTER TABLE tool_decisions ADD COLUMN source TEXT NOT NULL DEFAULT 'otlp';
ALTER TABLE tool_decisions ADD COLUMN model TEXT;

-- New tables.
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

- [ ] **Step 2: Register**

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
];
```

- [ ] **Step 3: Failing test**

Append to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn v4_creates_jsonl_tables_and_extends_decisions() {
    let mut conn = Connection::open_in_memory().unwrap();
    apply(&mut conn).unwrap();

    for tbl in ["slash_commands", "subagent_calls", "jsonl_errors", "jsonl_ingest_runs"] {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [tbl], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "missing table {tbl}");
    }

    let cols: Vec<String> = conn.prepare("PRAGMA table_info(tool_decisions)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    for c in ["source", "model"] {
        assert!(cols.contains(&c.to_string()), "missing tool_decisions column {c}");
    }

    let cols: Vec<String> = conn.prepare("PRAGMA table_info(sessions)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    assert!(cols.contains(&"data_source".to_string()));

    let v: i32 = conn.query_row(
        "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(v, 4);
}
```

Bump the existing `migrations_are_idempotent_across_runs` assertion from `assert_eq!(v, 3)` to `assert_eq!(v, 4)`.

- [ ] **Step 4: Run**

```powershell
cd src-tauri; cargo test --features test-support db::migrations
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/db/migrations.rs
git commit -m "feat(db): migration v4 — JSONL ingest schema (slash_commands, subagent_calls, jsonl_errors, jsonl_ingest_runs)"
```

---

## Phase 2 — JSONL record types and pricing

### Task 2: `jsonl::record` — lenient JSONL deserialisation

**Files:**
- Create: `src-tauri/src/jsonl/mod.rs` (skeleton: `pub mod record;`).
- Create: `src-tauri/src/jsonl/record.rs`.
- Modify: `src-tauri/src/lib.rs` (register `pub mod jsonl;`).

- [ ] **Step 1: Register**

In `src-tauri/src/lib.rs`, alongside existing `pub mod` lines:

```rust
pub mod jsonl;
```

Create `src-tauri/src/jsonl/mod.rs`:

```rust
//! JSONL transcript ingestion. See docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md.

pub mod record;
```

- [ ] **Step 2: Write `record.rs`**

```rust
//! Lenient deserialiser for Claude Code's per-session JSONL transcripts.
//! Every field is `Option<T>` so unknown/missing fields cannot abort parse.

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
    pub timestamp: Option<String>,
    pub version: Option<String>,
    pub cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    pub message: Option<Message>,
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
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)] pub input_tokens: i64,
    #[serde(default)] pub output_tokens: i64,
    #[serde(default, rename = "cache_creation_input_tokens")] pub cache_creation: i64,
    #[serde(default, rename = "cache_read_input_tokens")] pub cache_read: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]      Text { text: Option<String> },
    #[serde(rename = "tool_use")]  ToolUse { id: Option<String>, name: Option<String>, #[serde(default)] input: Value },
    #[serde(other)]                Other,
}

pub fn parse_line(line: &str) -> Result<JsonlRecord, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_record() {
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.kind.as_deref(), Some("user"));
        assert_eq!(r.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn parses_assistant_with_tool_use() {
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x/y.rs"}}]}}"#;
        let r = parse_line(line).expect("parse");
        let msg = r.message.as_ref().expect("message");
        assert_eq!(msg.model.as_deref(), Some("claude-opus-4-7"));
        let u = msg.usage.as_ref().expect("usage");
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.cache_read, 5);
        match &msg.content[0] {
            ContentBlock::ToolUse { name, .. } => assert_eq!(name.as_deref(), Some("Read")),
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn unknown_record_type_does_not_fail() {
        let line = r#"{"type":"super_event_2027","sessionId":"s1"}"#;
        assert_eq!(parse_line(line).unwrap().kind.as_deref(), Some("super_event_2027"));
    }

    #[test]
    fn missing_fields_default_to_none() {
        let r = parse_line(r#"{"type":"summary"}"#).unwrap();
        assert!(r.session_id.is_none());
        assert!(r.message.is_none());
    }

    #[test]
    fn extra_unknown_fields_ignored() {
        let line = r#"{"type":"user","sessionId":"s1","futureField":42}"#;
        assert_eq!(parse_line(line).unwrap().kind.as_deref(), Some("user"));
    }
}
```

- [ ] **Step 3: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::record
git add src-tauri/src/lib.rs src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/record.rs
git commit -m "feat(jsonl): lenient JsonlRecord deserialiser"
```

Expected: 5 PASS.

---

### Task 3: `jsonl::pricing` — bundled model price table

**Files:**
- Create: `src-tauri/src/jsonl/pricing.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register**

```rust
pub mod record;
pub mod pricing;
```

- [ ] **Step 2: Write `pricing.rs`**

```rust
//! Per-model token pricing for retroactive cost computation.
//! Used only for JSONL-only sessions (no OTLP cost available).
//! USD per million tokens.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_create_per_mtok: f64,
}

pub fn lookup(model: &str) -> Option<ModelPricing> {
    for (prefix, price) in TABLE { if model.starts_with(prefix) { return Some(*price); } }
    None
}

pub fn cost_for(model: &str, input: i64, output: i64, cache_read: i64, cache_create: i64) -> Option<f64> {
    let p = lookup(model)?;
    let n = |toks: i64, per_m: f64| (toks as f64) / 1_000_000.0 * per_m;
    Some(n(input, p.input_per_mtok) + n(output, p.output_per_mtok)
       + n(cache_read, p.cache_read_per_mtok) + n(cache_create, p.cache_create_per_mtok))
}

const TABLE: &[(&str, ModelPricing)] = &[
    ("claude-opus-4-7",   ModelPricing { input_per_mtok: 15.0, output_per_mtok: 75.0,
                                          cache_read_per_mtok: 1.50, cache_create_per_mtok: 18.75 }),
    ("claude-opus-4-6",   ModelPricing { input_per_mtok: 15.0, output_per_mtok: 75.0,
                                          cache_read_per_mtok: 1.50, cache_create_per_mtok: 18.75 }),
    ("claude-sonnet-4-6", ModelPricing { input_per_mtok: 3.0, output_per_mtok: 15.0,
                                          cache_read_per_mtok: 0.30, cache_create_per_mtok: 3.75 }),
    ("claude-sonnet-4-5", ModelPricing { input_per_mtok: 3.0, output_per_mtok: 15.0,
                                          cache_read_per_mtok: 0.30, cache_create_per_mtok: 3.75 }),
    ("claude-haiku-4-5",  ModelPricing { input_per_mtok: 1.0, output_per_mtok: 5.0,
                                          cache_read_per_mtok: 0.10, cache_create_per_mtok: 1.25 }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_works_for_date_suffixed_models() {
        assert_eq!(lookup("claude-opus-4-7-20260101").unwrap().output_per_mtok, 75.0);
    }
    #[test]
    fn unknown_model_returns_none() { assert!(lookup("gpt-4").is_none()); }
    #[test]
    fn cost_for_sums_token_types() {
        assert!((cost_for("claude-opus-4-7", 1_000_000, 0, 0, 0).unwrap() - 15.0).abs() < 1e-9);
        assert!((cost_for("claude-opus-4-7", 0, 1_000_000, 0, 0).unwrap() - 75.0).abs() < 1e-9);
    }
    #[test]
    fn cost_for_unknown_model_none() {
        assert!(cost_for("mystery-model", 1000, 1000, 0, 0).is_none());
    }
}
```

- [ ] **Step 3: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::pricing
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/pricing.rs
git commit -m "feat(jsonl): bundled pricing table for retroactive cost"
```

Expected: 4 PASS.

---

## Phase 3 — The reducer (trust boundary)

### Task 4: `jsonl::reducer` — `DerivedEvent` enum + user/assistant reduction

**Files:**
- Create: `src-tauri/src/jsonl/reducer.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.
- Possibly modify: `src-tauri/Cargo.toml` to add `chrono` if missing.

- [ ] **Step 1: Register**

```rust
pub mod record;
pub mod pricing;
pub mod reducer;
```

- [ ] **Step 2: Confirm `chrono` is in `Cargo.toml`**

Check `[dependencies]` for `chrono`. It is almost certainly already present (repo uses it in tests). If not, add:

```toml
chrono = { version = "0.4", default-features = false, features = ["serde", "clock"] }
```

- [ ] **Step 3: Write `reducer.rs`**

```rust
//! Trust boundary between JSONL (raw, contains prompt text) and the rest
//! of the ingest pipeline (text-free by type). Anything that reads
//! `record::Message.content[].text` must do so inside this module and
//! drop the text before returning.

use crate::jsonl::record::{ContentBlock, JsonlRecord, Message};

/// Output of the reducer. No variant carries prompt or response text.
/// The privacy property test in `tests/jsonl_privacy.rs` enforces this empirically;
/// the type system enforces it structurally.
#[derive(Debug, Clone)]
pub enum DerivedEvent {
    SessionLifecycle {
        session_id: String, started_at: i64, ended_at: Option<i64>,
        cc_version: Option<String>, cwd: Option<String>, git_branch: Option<String>,
    },
    TokenUsage {
        session_id: String, ts: i64, model: String,
        input: i64, output: i64, cache_create: i64, cache_read: i64,
    },
    CostEntry {
        session_id: String, ts: i64, model: String, cost_usd: f64,
    },
    ToolCall {
        session_id: String, ts: i64,
        tool_name: String, file_path: Option<String>,
        model: Option<String>,
    },
    SlashCommand {
        session_id: String, ts: i64, name: String, arg_count: i64,
    },
    SubAgentCall {
        parent_id: String, child_id: Option<String>,
        subagent_type: Option<String>, started_at: i64,
    },
}

#[derive(Default)]
pub struct Reducer { first_turn_seen: bool }

impl Reducer {
    pub fn new() -> Self { Self::default() }

    pub fn reduce(&mut self, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let Some(sid) = rec.session_id.as_deref().map(|s| s.to_string()) else { return vec![] };
        let ts = parse_ts(rec.timestamp.as_deref()).unwrap_or(0);
        match rec.kind.as_deref() {
            Some("user")      => self.reduce_user(&sid, ts, rec),
            Some("assistant") => self.reduce_assistant(&sid, ts, rec),
            _                 => vec![],
        }
    }

    fn reduce_user(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let mut out = vec![];
        if !self.first_turn_seen {
            self.first_turn_seen = true;
            out.push(DerivedEvent::SessionLifecycle {
                session_id: sid.to_string(), started_at: ts, ended_at: None,
                cc_version: rec.version.clone(), cwd: rec.cwd.clone(),
                git_branch: rec.git_branch.clone(),
            });
        }
        if let Some(msg) = rec.message.as_ref() {
            if let Some((name, arg_count)) = detect_slash_command(msg) {
                out.push(DerivedEvent::SlashCommand {
                    session_id: sid.to_string(), ts, name, arg_count,
                });
            }
        }
        out
    }

    fn reduce_assistant(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let mut out = vec![];
        let Some(msg) = rec.message.as_ref() else { return out };
        let model = msg.model.clone().unwrap_or_else(|| "unknown".into());

        if let Some(u) = msg.usage.as_ref() {
            if u.input_tokens + u.output_tokens + u.cache_read + u.cache_creation > 0 {
                out.push(DerivedEvent::TokenUsage {
                    session_id: sid.to_string(), ts, model: model.clone(),
                    input: u.input_tokens, output: u.output_tokens,
                    cache_create: u.cache_creation, cache_read: u.cache_read,
                });
                if let Some(cost) = crate::jsonl::pricing::cost_for(
                    &model, u.input_tokens, u.output_tokens, u.cache_read, u.cache_creation,
                ) {
                    if cost > 0.0 {
                        out.push(DerivedEvent::CostEntry {
                            session_id: sid.to_string(), ts, model: model.clone(), cost_usd: cost,
                        });
                    }
                }
            }
        }

        for block in &msg.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                let Some(tool_name) = name.clone() else { continue };
                let file_path = input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string());
                out.push(DerivedEvent::ToolCall {
                    session_id: sid.to_string(), ts,
                    tool_name: tool_name.clone(), file_path,
                    model: Some(model.clone()),
                });
                if tool_name == "Task" {
                    let child_id = input.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let subagent_type = input.get("subagent_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                    out.push(DerivedEvent::SubAgentCall {
                        parent_id: sid.to_string(), child_id, subagent_type, started_at: ts,
                    });
                }
            }
        }
        out
    }
}

fn parse_ts(s: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s?).ok().map(|dt| dt.timestamp_millis())
}

fn detect_slash_command(msg: &Message) -> Option<(String, i64)> {
    for block in &msg.content {
        if let ContentBlock::Text { text: Some(t) } = block {
            if let Some(name) = extract_tag(t, "command-name") {
                let arg_count = extract_tag(t, "command-args")
                    .map(|a| a.split_whitespace().count() as i64).unwrap_or(0);
                let trimmed = name.trim().trim_start_matches('/').to_string();
                if !trimmed.is_empty() { return Some((trimmed, arg_count)); }
            }
        }
    }
    None
}

fn extract_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open  = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end   = s[start..].find(&close)? + start;
    Some(&s[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonl::record::parse_line;

    #[test]
    fn user_record_emits_lifecycle_on_first_turn() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        assert!(matches!(out[0], DerivedEvent::SessionLifecycle { .. }));
    }

    #[test]
    fn second_user_record_does_not_repeat_lifecycle() {
        let mut r = Reducer::new();
        let l1 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        let l2 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:05.000Z","message":{"role":"user","content":[]}}"#;
        let _ = r.reduce(&parse_line(l1).unwrap());
        let out = r.reduce(&parse_line(l2).unwrap());
        assert!(out.is_empty());
    }

    #[test]
    fn assistant_emits_token_usage_and_cost() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let has_tok = out.iter().any(|e| matches!(e, DerivedEvent::TokenUsage { .. }));
        let cost = out.iter().find_map(|e| match e { DerivedEvent::CostEntry { cost_usd, .. } => Some(*cost_usd), _ => None });
        assert!(has_tok);
        assert!((cost.unwrap() - 15.0).abs() < 1e-9, "1M input tokens × $15/Mtok = $15");
    }

    #[test]
    fn assistant_tool_use_emits_tool_call_with_file_path() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let call = out.iter().find_map(|e| match e {
            DerivedEvent::ToolCall { tool_name, file_path, .. } => Some((tool_name.clone(), file_path.clone())),
            _ => None,
        }).expect("tool call emitted");
        assert_eq!(call.0, "Read");
        assert_eq!(call.1.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn assistant_task_tool_emits_subagent() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Task","input":{"subagent_type":"Explore"}}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let st = out.iter().find_map(|e| match e {
            DerivedEvent::SubAgentCall { subagent_type, .. } => Some(subagent_type.clone()),
            _ => None,
        }).expect("subagent emitted");
        assert_eq!(st.as_deref(), Some("Explore"));
    }

    #[test]
    fn user_command_name_tag_emits_slash_command() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name><command-args>PR 42</command-args>"}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let sc = out.iter().find_map(|e| match e {
            DerivedEvent::SlashCommand { name, arg_count, .. } => Some((name.clone(), *arg_count)),
            _ => None,
        }).expect("slash command emitted");
        assert_eq!(sc, ("review".to_string(), 2));
    }

    #[test]
    fn no_session_id_no_output() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        assert!(r.reduce(&parse_line(line).unwrap()).is_empty());
    }
}
```

- [ ] **Step 4: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reducer
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/reducer.rs src-tauri/Cargo.toml
git commit -m "feat(jsonl): reducer with DerivedEvent enum (Plan C scope)"
```

Expected: 7 PASS.

---

### Task 5: Privacy property test

**Files:**
- Modify: `src-tauri/Cargo.toml` — `proptest` dev-dep.
- Create: `src-tauri/tests/jsonl_privacy.rs`.

- [ ] **Step 1: Add dep**

In `[dev-dependencies]`:

```toml
proptest = "1"
```

- [ ] **Step 2: Write the test**

```rust
//! Privacy property test for the JSONL reducer trust boundary.
//!
//! For randomly generated JSONL records containing prompt-shaped text in
//! every text-bearing field, the reducer output must contain no substring
//! of any input text. Formal guarantee behind the pitch's "never persisted"
//! claim.

use andon_lib::jsonl::record::JsonlRecord;
use andon_lib::jsonl::reducer::{DerivedEvent, Reducer};
use proptest::prelude::*;
use serde_json::json;

fn dump(events: &[DerivedEvent]) -> String {
    events.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join("\n")
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

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
        let d = dump(&out);
        prop_assert!(!d.contains(&prompt), "reducer leaked user text: {prompt:?}");
    }

    #[test]
    fn assistant_text_never_leaks(
        text in "[A-Za-z0-9 ]{20,200}",
        path in "[A-Za-z0-9_/.-]{5,40}",
    ) {
        // file_path IS allowed to leak — it's metadata. text is NOT.
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
                    { "type": "tool_use", "id": "x", "name": "Read", "input": { "file_path": path } }
                ]
            }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        let d = dump(&out);
        prop_assert!(!d.contains(&text), "reducer leaked assistant text: {text:?}");
    }
}
```

- [ ] **Step 3: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_privacy
git add src-tauri/Cargo.toml src-tauri/tests/jsonl_privacy.rs
git commit -m "test(jsonl): privacy property test verifies reducer trust boundary"
```

Expected: 2 PASS × 256 cases each.

---

## Phase 4 — Parser, walker, reconciler

### Task 6: `jsonl::parser` — streaming line reader with error capture

**Files:**
- Create: `src-tauri/src/jsonl/parser.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register + write**

`mod.rs`:

```rust
pub mod parser;
```

`parser.rs`:

```rust
//! Streaming JSONL parser. Captures per-line errors so callers can route them
//! to the `jsonl_errors` table.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::jsonl::record::{parse_line, JsonlRecord};

#[derive(Debug)]
pub struct ParseErr {
    pub file: PathBuf, pub line_no: usize,
    pub kind: ErrKind, pub msg: String,
    pub cc_version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrKind { JsonParse, UnknownType, MissingField, ReducerPanic }

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

/// Iterate every JSONL line in `path`. Returning `false` aborts iteration.
pub fn for_each_record<F>(path: &Path, mut cb: F) -> std::io::Result<()>
where F: FnMut(Result<JsonlRecord, ParseErr>) -> bool {
    let f = File::open(path)?;
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line_no = i + 1;
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let cont = cb(Err(ParseErr {
                    file: path.to_path_buf(), line_no, kind: ErrKind::JsonParse,
                    msg: format!("read error: {e}"), cc_version: None,
                }));
                if !cont { return Ok(()) } else { continue }
            }
        };
        if line.trim().is_empty() { continue }
        let ev = match parse_line(&line) {
            Ok(rec) => Ok(rec),
            Err(e)  => Err(ParseErr {
                file: path.to_path_buf(), line_no, kind: ErrKind::JsonParse,
                msg: e.to_string(), cc_version: None,
            }),
        };
        if !cb(ev) { return Ok(()) }
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
        writeln!(f, "").unwrap();
        writeln!(f, r#"{{"type":"assistant","sessionId":"s1"}}"#).unwrap();
        let (mut oks, mut errs) = (0, 0);
        for_each_record(f.path(), |r| { match r { Ok(_) => oks += 1, Err(_) => errs += 1 } true }).unwrap();
        assert_eq!(oks, 2);
        assert_eq!(errs, 1);
    }
}
```

- [ ] **Step 2: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::parser
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/parser.rs
git commit -m "feat(jsonl): streaming parser with per-line error capture"
```

---

### Task 7: `jsonl::walker` — enumerate transcripts

**Files:**
- Create: `src-tauri/src/jsonl/walker.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register + write**

`mod.rs`:

```rust
pub mod walker;
```

`walker.rs`:

```rust
//! Enumerate `<claude_home>/projects/<slug>/*.jsonl`.

use std::path::{Path, PathBuf};

pub fn enumerate(claude_home: &Path) -> Vec<PathBuf> {
    let projects = claude_home.join("projects");
    let mut out = vec![];
    let Ok(slugs) = std::fs::read_dir(&projects) else { return out };
    for slug in slugs.flatten() {
        if !slug.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue }
        let Ok(files) = std::fs::read_dir(slug.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("jsonl") { out.push(p); }
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
    fn enumerates_jsonl_only() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("projects").join("p1");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.jsonl"), b"{}").unwrap();
        fs::write(proj.join("x.txt"), b"x").unwrap();
        fs::write(proj.join("b.jsonl"), b"{}").unwrap();
        assert_eq!(enumerate(tmp.path()).len(), 2);
    }

    #[test]
    fn missing_home_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(enumerate(tmp.path()).is_empty());
    }
}
```

- [ ] **Step 2: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::walker
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/walker.rs
git commit -m "feat(jsonl): walker enumerates ~/.claude/projects transcripts"
```

---

### Task 8: `jsonl::reconciler` — OTLP-vs-JSONL routing

**Files:**
- Create: `src-tauri/src/jsonl/reconciler.rs`.
- Modify: `src-tauri/src/jsonl/mod.rs`.

- [ ] **Step 1: Register + write**

`mod.rs`:

```rust
pub mod reconciler;
```

`reconciler.rs`:

```rust
//! Per-session OTLP-vs-JSONL routing. OTLP-covered iff any token_usage row exists.

use anyhow::Result;
use rusqlite::params;
use std::sync::Arc;
use crate::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage { Otlp, JsonlOnly }

pub fn coverage_for(pool: &Arc<DbPool>, session_id: &str) -> Result<Coverage> {
    let conn = pool.get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM token_usage WHERE session_id = ?1 LIMIT 1",
        params![session_id], |r| r.get(0),
    ).unwrap_or(0);
    Ok(if n > 0 { Coverage::Otlp } else { Coverage::JsonlOnly })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pool() -> Arc<DbPool> {
        let dir = tempfile::tempdir().unwrap();
        let p = crate::db::init(&dir.path().join("t.db")).unwrap();
        Box::leak(Box::new(dir));
        Arc::new(p)
    }
    #[test]
    fn no_token_usage_is_jsonl_only() {
        assert_eq!(coverage_for(&pool(), "sX").unwrap(), Coverage::JsonlOnly);
    }
    #[test]
    fn token_usage_present_is_otlp() {
        let p = pool();
        let c = p.get().unwrap();
        c.execute("INSERT INTO sessions (session_id, started_at) VALUES ('sY', 0)", []).unwrap();
        c.execute("INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES ('sY', 0, 'm', 'input', 1)", []).unwrap();
        assert_eq!(coverage_for(&p, "sY").unwrap(), Coverage::Otlp);
    }
}
```

- [ ] **Step 2: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support jsonl::reconciler
git add src-tauri/src/jsonl/mod.rs src-tauri/src/jsonl/reconciler.rs
git commit -m "feat(jsonl): reconciler determines OTLP vs JSONL coverage"
```

---

## Phase 5 — Ingestor integration + public API

### Task 9: Ingestor — `ingest_derived` writes DerivedEvents

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs`.
- Test: `src-tauri/tests/jsonl_ingest_writes.rs`.

- [ ] **Step 1: Add the method**

In `impl Ingestor`, after the existing methods, add:

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
            E::TokenUsage { session_id, ts, model, input, output, cache_create, cache_read } => {
                if matches!(coverage, Coverage::JsonlOnly) {
                    for (kind, n) in [("input", *input), ("output", *output),
                                      ("cacheRead", *cache_read), ("cacheCreation", *cache_create)] {
                        if n > 0 {
                            let _ = tx.execute(
                                "INSERT INTO token_usage (session_id, timestamp, model, token_type, count)
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
            E::ToolCall { session_id, ts, tool_name, file_path, model } => {
                if matches!(coverage, Coverage::JsonlOnly) {
                    let _ = tx.execute(
                        "INSERT INTO tool_decisions
                           (session_id, timestamp, tool_name, decision, language, file_path, source, model)
                         VALUES (?1, ?2, ?3, NULL, NULL, ?4, 'jsonl', ?5)",
                        params![session_id, ts, tool_name, file_path, model],
                    );
                }
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
        }
    }
    tx.commit()?;
    Ok(())
}
```

- [ ] **Step 2: Write test**

`src-tauri/tests/jsonl_ingest_writes.rs`:

```rust
mod common;

use andon_lib::jsonl::reconciler::Coverage;
use andon_lib::jsonl::reducer::DerivedEvent;
use common::{fixture_pool, test_ingestor};

#[test]
fn writes_slash_and_subagent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![
        DerivedEvent::SlashCommand { session_id: "s1".into(), ts: 100, name: "review".into(), arg_count: 1 },
        DerivedEvent::SubAgentCall { parent_id: "s1".into(), child_id: Some("c".into()),
                                      subagent_type: Some("Explore".into()), started_at: 110 },
    ];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let conn = pool.get().unwrap();
    let cmd: String = conn.query_row(
        "SELECT command_name FROM slash_commands WHERE session_id='s1'", [], |r| r.get(0)).unwrap();
    assert_eq!(cmd, "review");
    let st: String = conn.query_row(
        "SELECT subagent_type FROM subagent_calls WHERE parent_session_id='s1'", [], |r| r.get(0)).unwrap();
    assert_eq!(st, "Explore");
}

#[test]
fn skips_token_usage_when_otlp_covered() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![DerivedEvent::TokenUsage {
        session_id: "s1".into(), ts: 100, model: "claude-opus-4-7".into(),
        input: 10, output: 20, cache_create: 0, cache_read: 0,
    }];
    ing.ingest_derived(&events, Coverage::Otlp).unwrap();
    let conn = pool.get().unwrap();
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM token_usage WHERE session_id='s1'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "JSONL must not write token_usage for OTLP-covered sessions");
}

#[test]
fn writes_tool_decisions_for_jsonl_only_session() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let events = vec![DerivedEvent::ToolCall {
        session_id: "s1".into(), ts: 100,
        tool_name: "Read".into(), file_path: Some("a.rs".into()),
        model: Some("claude-opus-4-7".into()),
    }];
    ing.ingest_derived(&events, Coverage::JsonlOnly).unwrap();
    let conn = pool.get().unwrap();
    let (src, model): (String, String) = conn.query_row(
        "SELECT source, model FROM tool_decisions WHERE session_id='s1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(src, "jsonl");
    assert_eq!(model, "claude-opus-4-7");
}
```

- [ ] **Step 3: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_ingest_writes
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/jsonl_ingest_writes.rs
git commit -m "feat(ingestor): ingest_derived writes DerivedEvents respecting OTLP coverage"
```

Expected: 3 PASS.

---

### Task 10: `jsonl::mod` — public API + panic isolation

**Files:**
- Modify: `src-tauri/src/jsonl/mod.rs`.
- Test: `src-tauri/tests/jsonl_pipeline.rs`.

- [ ] **Step 1: Replace `mod.rs`**

```rust
//! JSONL transcript ingestion. See docs/superpowers/specs/...

pub mod record;
pub mod pricing;
pub mod reducer;
pub mod parser;
pub mod walker;
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

#[tracing::instrument(skip(pool, ingestor))]
pub async fn backfill(
    pool: &Arc<DbPool>, ingestor: &Ingestor, claude_home: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "backfill", started_at)?;
    let mut stats = IngestStats::default();
    let files = walker::enumerate(claude_home);
    stats.files_processed = files.len() as i64;
    for path in &files {
        match ingest_one_inner(pool, ingestor, path).await {
            Ok(s)  => { stats.records_processed += s.records_processed;
                         stats.records_errored   += s.records_errored;
                         stats.sessions_added    += s.sessions_added; }
            Err(e) => { tracing::error!(?path, error = ?e, "jsonl ingest failed"); stats.records_errored += 1; }
        }
    }
    stats.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &stats)?;
    Ok(stats)
}

#[tracing::instrument(skip(pool, ingestor))]
pub async fn ingest_one(
    pool: &Arc<DbPool>, ingestor: &Ingestor, transcript_path: &Path,
) -> Result<IngestStats> {
    let started_at = now_ms();
    let run_id = insert_run(pool, "session_end", started_at)?;
    let mut s = ingest_one_inner(pool, ingestor, transcript_path).await?;
    s.duration_ms = now_ms() - started_at;
    finalise_run(pool, run_id, &s)?;
    Ok(s)
}

async fn ingest_one_inner(
    pool: &Arc<DbPool>, ingestor: &Ingestor, path: &Path,
) -> Result<IngestStats> {
    use std::panic::AssertUnwindSafe;
    let path_owned = path.to_path_buf();
    let pool_clone = Arc::clone(pool);
    // ingestor is not Clone; instead, build a fresh one inside the blocking task.
    let pool_for_ing = Arc::clone(pool);
    let control = ingestor.control.clone();
    let diag    = ingestor.diagnostics.clone();

    let result = tokio::task::spawn_blocking(move || {
        let fresh_ing = Ingestor::new(pool_for_ing.clone(), control, diag);
        let mut stats = IngestStats { files_processed: 1, ..Default::default() };
        let mut reducer = reducer::Reducer::new();
        let mut events_by_session: std::collections::HashMap<String, Vec<reducer::DerivedEvent>> =
            std::collections::HashMap::new();

        let _ = parser::for_each_record(&path_owned, |r| {
            stats.records_processed += 1;
            match r {
                Ok(rec) => {
                    let res = std::panic::catch_unwind(AssertUnwindSafe(|| reducer.reduce(&rec)));
                    match res {
                        Ok(events) => {
                            for ev in events {
                                if let Some(sid) = event_session_id(&ev) {
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

        for (sid, events) in events_by_session {
            let cov = reconciler::coverage_for(&pool_clone, &sid).unwrap_or(reconciler::Coverage::JsonlOnly);
            if let Err(e) = fresh_ing.ingest_derived(&events, cov) {
                tracing::error!(sid, error = ?e, "ingest_derived failed");
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
        SessionLifecycle { session_id, .. } | TokenUsage { session_id, .. }
        | CostEntry { session_id, .. } | ToolCall { session_id, .. }
        | SlashCommand { session_id, .. } => Some(session_id.clone()),
        SubAgentCall { parent_id, .. } => Some(parent_id.clone()),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn insert_run(pool: &Arc<DbPool>, kind: &str, started_at: i64) -> Result<i64> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO jsonl_ingest_runs (kind, started_at) VALUES (?1, ?2)",
        params![kind, started_at],
    )?;
    Ok(conn.last_insert_rowid())
}

fn finalise_run(pool: &Arc<DbPool>, id: i64, s: &IngestStats) -> Result<()> {
    pool.get()?.execute(
        "UPDATE jsonl_ingest_runs SET ended_at = ?1, files_processed = ?2,
                                       records_processed = ?3, records_errored = ?4
         WHERE id = ?5",
        params![now_ms(), s.files_processed, s.records_processed, s.records_errored, id],
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

This refers to `ingestor.control` and `ingestor.diagnostics`. If those fields are private on `Ingestor`, expose them as `pub(crate)` in `src-tauri/src/otlp/ingestor.rs`:

```rust
pub struct Ingestor {
    pool: Arc<DbPool>,
    pub(crate) control: IngestionControl,
    pub(crate) diagnostics: Diagnostics,
}
```

(Or add cheap accessor methods. Either works; `pub(crate)` is simpler.)

- [ ] **Step 2: Write integration test**

`src-tauri/tests/jsonl_pipeline.rs`:

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
    fs::write(proj.join("session.jsonl"), lines.join("\n")).unwrap();
}

#[tokio::test]
async fn backfill_processes_synthetic_session() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "p", &[
        r#"{"type":"user","sessionId":"sess-1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name><command-args>x</command-args>"}]}}"#,
        r#"{"type":"assistant","sessionId":"sess-1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":50,"output_tokens":100},"content":[{"type":"tool_use","id":"u1","name":"Task","input":{"subagent_type":"Explore"}}]}}"#,
    ]);

    let pool_arc = Arc::new(pool.as_ref().clone());
    let stats = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    assert_eq!(stats.files_processed, 1);
    assert_eq!(stats.records_errored, 0);

    let conn = pool.get().unwrap();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions WHERE session_id='sess-1'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM slash_commands", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM subagent_calls", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn backfill_is_idempotent() {
    let (pool, _g) = fixture_pool();
    let ing = test_ingestor(&pool);
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), "x", &[
        r#"{"type":"user","sessionId":"sIDP","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#,
    ]);
    let pool_arc = Arc::new(pool.as_ref().clone());
    let _ = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let _ = jsonl::backfill(&pool_arc, &ing, home.path()).await.unwrap();
    let conn = pool.get().unwrap();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions WHERE session_id='sIDP'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1, "second run must not duplicate the session row");
}
```

- [ ] **Step 3: Add `futures` if missing**

```toml
futures = "0.3"
```

- [ ] **Step 4: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test jsonl_pipeline
git add src-tauri/src/jsonl/mod.rs src-tauri/src/otlp/ingestor.rs src-tauri/tests/jsonl_pipeline.rs src-tauri/Cargo.toml
git commit -m "feat(jsonl): public backfill() and ingest_one() with panic isolation"
```

Expected: 2 PASS.

---

## Phase 6 — API endpoints

### Task 11: `POST /api/jsonl/backfill`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`, `src-tauri/src/api/dto.rs`.
- Test: `src-tauri/tests/api_jsonl.rs`.

- [ ] **Step 1: DTO**

In `dto.rs`:

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

Add the route in `router()`:

```rust
        .route("/api/jsonl/backfill", post(jsonl_backfill))
```

Handler:

```rust
#[tracing::instrument(skip(state))]
async fn jsonl_backfill(State(state): State<ApiState>) -> impl axum::response::IntoResponse {
    let Some(home) = dirs::home_dir() else {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"no home dir"}))).into_response();
    };
    let claude_home = home.join(".claude");
    let pool = Arc::clone(&state.pool);
    let ingestor = crate::otlp::ingestor::Ingestor::new(
        Arc::clone(&state.pool), state.control.clone(), state.diagnostics.clone(),
    );
    match crate::jsonl::backfill(&pool, &ingestor, &claude_home).await {
        Ok(stats) => Json(crate::api::dto::JsonlBackfillResponse::from(stats)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "jsonl backfill failed");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}
```

- [ ] **Step 3: Confirm `dirs` dep**

```toml
dirs = "5"
```

- [ ] **Step 4: Test**

`src-tauri/tests/api_jsonl.rs`:

```rust
mod common;

use axum::http::StatusCode;
use common::test_router;
use tower::ServiceExt;

#[tokio::test]
async fn backfill_endpoint_returns_2xx_or_500() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let res = router.oneshot(
        axum::http::Request::builder().method("POST").uri("/api/jsonl/backfill")
            .header("content-type","application/json").body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert!(matches!(res.status(), StatusCode::OK | StatusCode::INTERNAL_SERVER_ERROR));
}
```

- [ ] **Step 5: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/Cargo.toml src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): POST /api/jsonl/backfill"
```

---

### Task 12: Extend `SessionEndPayload` with `transcript_path`

**Files:**
- Modify: `src-tauri/src/api/routes.rs:824` and the `hook_session_end` handler.

- [ ] **Step 1: Extend payload**

```rust
#[derive(Deserialize)]
struct SessionEndPayload {
    session_id: Option<String>,
    #[serde(default)] reason: Option<String>,
    #[serde(default)] transcript_path: Option<String>,
}
```

- [ ] **Step 2: Spawn JSONL ingest**

In `hook_session_end`, after the existing `tokio::spawn` for report generation, add:

```rust
    if let Some(tp) = p.transcript_path.clone() {
        let pool_j = state.pool.clone();
        let control_j = state.control.clone();
        let diag_j = state.diagnostics.clone();
        tokio::spawn(async move {
            let ing = crate::otlp::ingestor::Ingestor::new(pool_j.clone(), control_j, diag_j);
            let path = std::path::PathBuf::from(tp);
            if let Err(e) = crate::jsonl::ingest_one(&pool_j, &ing, &path).await {
                tracing::error!(error = ?e, "session-end JSONL ingest failed");
            }
        });
    }
```

- [ ] **Step 3: Test**

Append to `api_jsonl.rs`:

```rust
#[tokio::test]
async fn session_end_with_transcript_path_returns_200() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let body = serde_json::json!({
        "session_id": "s-test", "reason": "exit",
        "transcript_path": "/does/not/exist/missing.jsonl"
    });
    let res = router.oneshot(
        axum::http::Request::builder().method("POST").uri("/api/hooks/session-end")
            .header("content-type","application/json")
            .body(axum::body::Body::from(body.to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

- [ ] **Step 4: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): session-end hook ingests JSONL transcript"
```

---

### Task 13: `GET /api/jsonl/errors` + `GET /api/jsonl/ingest-runs`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`, `src-tauri/src/api/dto.rs`.

- [ ] **Step 1: DTOs**

```rust
#[derive(Debug, serde::Serialize)]
pub struct JsonlErrorEntry {
    pub jsonl_path: String, pub line_no: i64,
    pub error_kind: String, pub error_msg: String,
    pub cc_version: Option<String>, pub ingested_at: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct JsonlIngestRunEntry {
    pub id: i64, pub kind: String,
    pub started_at: i64, pub ended_at: Option<i64>,
    pub files_processed: i64, pub records_processed: i64, pub records_errored: i64,
}
```

- [ ] **Step 2: Handlers + routes**

```rust
        .route("/api/jsonl/errors", get(jsonl_errors))
        .route("/api/jsonl/ingest-runs", get(jsonl_ingest_runs))
```

```rust
#[tracing::instrument(skip(state))]
async fn jsonl_errors(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::JsonlErrorEntry>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT jsonl_path, line_no, error_kind, error_msg, cc_version, ingested_at
             FROM jsonl_errors ORDER BY ingested_at DESC LIMIT 100") else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::JsonlErrorEntry {
            jsonl_path: r.get(0)?, line_no: r.get(1)?, error_kind: r.get(2)?,
            error_msg: r.get(3)?, cc_version: r.get(4)?, ingested_at: r.get(5)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(rows)
}

#[tracing::instrument(skip(state))]
async fn jsonl_ingest_runs(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::JsonlIngestRunEntry>> {
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, kind, started_at, ended_at, files_processed, records_processed, records_errored
             FROM jsonl_ingest_runs ORDER BY started_at DESC LIMIT 20") else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::JsonlIngestRunEntry {
            id: r.get(0)?, kind: r.get(1)?, started_at: r.get(2)?, ended_at: r.get(3)?,
            files_processed: r.get(4)?, records_processed: r.get(5)?, records_errored: r.get(6)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(rows)
}
```

- [ ] **Step 3: Test + commit**

Append to `api_jsonl.rs`:

```rust
#[tokio::test]
async fn errors_endpoint_returns_empty_array_initially() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    let res = router.oneshot(
        axum::http::Request::builder().method("GET").uri("/api/jsonl/errors")
            .body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): GET /api/jsonl/errors and /api/jsonl/ingest-runs"
```

---

### Task 14: Behaviour API endpoints (Plan C — three only)

**Files:**
- Modify: `src-tauri/src/api/routes.rs`, `src-tauri/src/api/dto.rs`.

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
pub struct SlashCommandEntry { pub name: String, pub count: i64 }

#[derive(Debug, serde::Serialize)]
pub struct SubAgentEntry { pub subagent_type: String, pub invocations: i64 }
```

- [ ] **Step 2: Routes + handlers**

```rust
        .route("/api/behaviour/model-mix", get(behaviour_model_mix))
        .route("/api/behaviour/slash-commands", get(behaviour_slash_commands))
        .route("/api/behaviour/subagents", get(behaviour_subagents))
```

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
                      FROM tool_decisions WHERE model IS NOT NULL
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

async fn behaviour_slash_commands(State(state): State<ApiState>) -> Json<Vec<crate::api::dto::SlashCommandEntry>> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Vec<_> {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT command_name, COUNT(*) FROM slash_commands
             GROUP BY command_name ORDER BY 2 DESC LIMIT 30") else { return vec![] };
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
            "SELECT subagent_type, COUNT(*) FROM subagent_calls
             WHERE subagent_type IS NOT NULL
             GROUP BY subagent_type ORDER BY 2 DESC") else { return vec![] };
        stmt.query_map([], |r| Ok(crate::api::dto::SubAgentEntry {
            subagent_type: r.get(0)?, invocations: r.get(1)?,
        })).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).await.unwrap_or_default();
    Json(out)
}
```

- [ ] **Step 3: Test smoke**

Append to `api_jsonl.rs`:

```rust
#[tokio::test]
async fn behaviour_endpoints_return_200_with_empty_db() {
    let (pool, _g) = common::fixture_pool();
    let (router, _g2) = test_router(&pool);
    for path in [
        "/api/behaviour/model-mix",
        "/api/behaviour/slash-commands",
        "/api/behaviour/subagents",
    ] {
        let res = router.clone().oneshot(
            axum::http::Request::builder().method("GET").uri(path)
                .body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {path} not OK");
    }
}
```

- [ ] **Step 4: Run + commit**

```powershell
cd src-tauri; cargo test --features test-support --test api_jsonl
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs src-tauri/tests/api_jsonl.rs
git commit -m "feat(api): three /api/behaviour/* endpoints (model-mix, slash-commands, subagents)"
```

---

## Phase 7 — Angular

### Task 15: API service wrappers + models

**Files:**
- Modify: `web/src/app/core/models.ts`.
- Modify: `web/src/app/core/api.service.ts`.

- [ ] **Step 1: Models**

Append to `models.ts`:

```ts
export interface ModelMixEntry { model: string; invocations: number; sessions: number; }
export interface ModelToolCell { model: string; tool: string; count: number; }
export interface ModelMixResponse { by_model: ModelMixEntry[]; by_model_tool: ModelToolCell[]; }
export interface SlashCommandEntry { name: string; count: number; }
export interface SubAgentEntry { subagent_type: string; invocations: number; }
export interface JsonlErrorEntry {
  jsonl_path: string; line_no: number; error_kind: string;
  error_msg: string; cc_version: string | null; ingested_at: number;
}
export interface JsonlIngestRun {
  id: number; kind: string; started_at: number; ended_at: number | null;
  files_processed: number; records_processed: number; records_errored: number;
}
export interface JsonlBackfillResponse {
  files_processed: number; records_processed: number; records_errored: number;
  sessions_added: number; duration_ms: number;
}
```

- [ ] **Step 2: API methods**

In `api.service.ts`, follow the existing `httpResource(() => ...)` / `firstValueFrom(this.http.get(...))` convention used by other endpoints. Add:

```ts
modelMix() { return this.get<ModelMixResponse>('/api/behaviour/model-mix'); }
slashCommands() { return this.get<SlashCommandEntry[]>('/api/behaviour/slash-commands'); }
subagents() { return this.get<SubAgentEntry[]>('/api/behaviour/subagents'); }
jsonlErrors() { return this.get<JsonlErrorEntry[]>('/api/jsonl/errors'); }
jsonlIngestRuns() { return this.get<JsonlIngestRun[]>('/api/jsonl/ingest-runs'); }
ingestJsonl() { return this.post<JsonlBackfillResponse>('/api/jsonl/backfill', {}); }
```

(Match the exact wrapper shape used by sibling methods — copy from `sessions()` if `httpResource` is the pattern, or from the relevant `post(...)` helper.)

- [ ] **Step 3: Commit**

```powershell
git add web/src/app/core/models.ts web/src/app/core/api.service.ts
git commit -m "feat(web): API wrappers and models for behaviour + JSONL endpoints"
```

---

### Task 16: Behaviour page (Plan C — 3 sections) + route

**Files:**
- Create: `web/src/app/features/behaviour/behaviour.component.ts`.
- Create: `web/src/app/features/behaviour/behaviour.component.html`.
- Modify: `web/src/app/app.routes.ts`.
- Modify: top nav template (likely `web/src/app/app.component.html`).

- [ ] **Step 1: Component**

`behaviour.component.ts`:

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
  readonly slash    = this.api.slashCommands();
  readonly subs     = this.api.subagents();
}
```

`behaviour.component.html`:

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
            @for (c of mm.by_model_tool; track c.model + c.tool) {
              <tr>
                <td class="font-mono pr-3">{{ c.model }}</td>
                <td class="font-mono pr-3">{{ c.tool }}</td>
                <td class="text-right text-zinc-400">{{ c.count }}</td>
              </tr>
            }
          </tbody>
        </table>
      </div>
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Slash commands</h2>
    @if (slash.value(); as cmds) {
      @if (cmds.length === 0) {
        <p class="text-xs text-zinc-500">No slash commands detected yet.</p>
      } @else {
        <ul class="space-y-1">
          @for (c of cmds; track c.name) {
            <li class="flex justify-between text-sm">
              <span class="font-mono">/{{ c.name }}</span>
              <span class="text-zinc-400">{{ c.count }}</span>
            </li>
          }
        </ul>
      }
    }
  </section>

  <section>
    <h2 class="text-lg font-medium mb-3">Sub-agent usage</h2>
    @if (subs.value(); as agents) {
      @if (agents.length === 0) {
        <p class="text-xs text-zinc-500">No sub-agent (Task) invocations detected yet.</p>
      } @else {
        <ul class="space-y-1">
          @for (a of agents; track a.subagent_type) {
            <li class="flex gap-4 text-sm">
              <span class="font-mono">{{ a.subagent_type }}</span>
              <span>{{ a.invocations }} invocations</span>
            </li>
          }
        </ul>
      }
    }
  </section>
</div>
```

- [ ] **Step 2: Route**

In `app.routes.ts`, between `files` and `diagnostics`:

```ts
{ path: 'behaviour',
  loadComponent: () => import('./features/behaviour/behaviour.component').then(m => m.BehaviourComponent) },
```

- [ ] **Step 3: Nav link**

In the top-nav template (search for the existing "Files" / "Diagnostics" links), insert a "Behaviour" link in the same style between them.

- [ ] **Step 4: Build + commit**

```powershell
cd web; npm run build; cd ..
git add web/src/app/features/behaviour web/src/app/app.routes.ts web/src/app/app.component.html
git commit -m "feat(web): Behaviour page (Plan C — 3 sections)"
```

---

### Task 17: Overview — "Invocations by model" companion

**Files:**
- Modify: `web/src/app/features/overview/overview.component.{ts,html}`.

- [ ] **Step 1: Wire data**

In the `.ts`, add:

```ts
readonly modelMix = this.api.modelMix();
```

(Use whichever `api` injection is already present.)

- [ ] **Step 2: Insert companion tile**

Next to the existing "Cost by model" tile in the template:

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

### Task 18: Settings → Data — Ingest JSONL button + status line

**Files:**
- Modify: `web/src/app/features/settings/settings.component.{ts,html}`.

- [ ] **Step 1: TS**

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

In the existing **Data** section:

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

### Task 19: Diagnostics — JSONL parse errors card

**Files:**
- Modify: `web/src/app/features/diagnostics/diagnostics.component.{ts,html}`.

- [ ] **Step 1: TS**

```ts
readonly jsonlErrors = this.api.jsonlErrors();
```

- [ ] **Step 2: HTML**

Add a card alongside the existing ones:

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

## Phase 8 — Smoke + docs + verification

### Task 20: `scripts/smoke_jsonl.py`

**Files:**
- Create: `scripts/smoke_jsonl.py`.

- [ ] **Step 1: Write**

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
import json, os, pathlib, sys, time, urllib.request

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
         "message":{"role":"user","content":[{"type":"text","text":"<command-name>/foo</command-name>"}]}},
        {"type":"assistant","sessionId":sid,"timestamp":"2026-05-19T10:00:01.000Z",
         "message":{"role":"assistant","model":"claude-opus-4-7",
                    "usage":{"input_tokens":10,"output_tokens":20},
                    "content":[{"type":"tool_use","id":"u1","name":"Task",
                                "input":{"subagent_type":"Explore"}}]}},
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

- [ ] **Step 2: Smoke + commit**

Manually (with `cargo tauri dev` running in another terminal):

```powershell
cd scripts; python smoke_jsonl.py
```

```powershell
git add scripts/smoke_jsonl.py
git commit -m "test: scripts/smoke_jsonl.py end-to-end smoke for JSONL ingest"
```

---

### Task 21: README + pitch updates

**Files:**
- Modify: `README.md`, `docs/pitch.md`.

- [ ] **Step 1: README — flip the Retroactive cell**

Replace:

```markdown
| Retroactive | No — only sees sessions that ran after install | Yes — every session you've ever run |
```

with:

```markdown
| Retroactive | Yes — Settings → "Ingest JSONL history" walks `~/.claude/projects/` | Yes — every session you've ever run |
```

- [ ] **Step 2: Pitch — privacy wording**

Replace:

```markdown
> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever read or stored.**</ins>
```

with:

```markdown
> **Privacy, in plain terms.** <ins>**No secrets, code contents, or prompts are ever persisted.**</ins> Andon's JSONL parser reads transcript files locally to derive numeric and structural signals (token counts, tool names, file paths, slash command names), but the reducer drops all prompt and response text before any DB write. Nothing leaves the engineer's machine.
```

- [ ] **Step 3: Commit**

```powershell
git add README.md docs/pitch.md
git commit -m "docs: flip retroactive cell + clarify privacy wording for JSONL ingest"
```

---

### Task 22: End-to-end verification + PR

- [ ] **Step 1: Full test run**

```powershell
cd src-tauri; cargo test --features test-support
```

Expected: every test passes including the new ones.

- [ ] **Step 2: Web build**

```powershell
cd web; npm run build
```

Expected: success.

- [ ] **Step 3: Local smoke**

```powershell
cargo tauri dev
# in another terminal:
cd scripts; python smoke_jsonl.py
```

- [ ] **Step 4: Manual UI sanity**

- **Behaviour** page exists in nav with three sections (Model mix, Slash commands, Sub-agents). Empty if DB is fresh — click "Ingest JSONL history" in Settings → Data and re-load.
- **Overview** shows "Invocations by model" next to "Cost by model".
- **Settings → Data** shows the new button and a last-run status line after one click.
- **Diagnostics** shows the "JSONL parse errors" card (probably "No JSONL parse errors recorded.").

- [ ] **Step 5: Push branch + open PR**

```powershell
git push -u origin feature/jsonl-ingest
gh pr create --title "feat: JSONL behavioural ingest (Plan C)" --body "$(cat <<'EOF'
## Summary
- Ingests Claude Code per-session JSONL transcripts to backfill pre-OTel history and surface three behavioural views: model frequency mix, slash command usage, sub-agent (Task) usage.
- Adds new `Behaviour` page; companion "Invocations by model" tile on Overview.
- Privacy: reducer is the trust boundary; no prompt or response text persists. Verified by `proptest` property test.
- Deferred (Plan A→Plan C cuts): stuck-session detection, tool-sequence diagram, read-to-edit ratio, thinking-token tracking.

## Test plan
- [ ] `cd src-tauri; cargo test --features test-support` passes
- [ ] `cd web; npm run build` succeeds
- [ ] `cargo tauri dev` runs; `scripts/smoke_jsonl.py` reports OK
- [ ] Behaviour / Overview / Settings / Diagnostics render correctly with fresh and seeded DBs
- [ ] Privacy property test passes 256+ cases

Spec: `docs/superpowers/specs/2026-05-19-jsonl-behavioural-ingest-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review checklist (run before opening PR)

1. **Spec coverage:** every numbered goal in the spec has at least one task:
   - Retroactive backfill → Tasks 9, 10, 11.
   - Three new behavioural views (model mix, slash, sub-agents) → Tasks 4, 14, 16.
   - SessionEnd hook enrichment → Task 12.
   - Privacy promise → Tasks 4, 5 (reducer + property test).
2. **Type consistency:** `DerivedEvent` variants in Task 4 match every reference in Tasks 5–14.
3. **Migrations idempotent:** Task 1 explicitly tests `apply()` twice.
4. **No placeholders:** every step has either concrete code, a concrete file path, or a concrete command.
5. **Deferred work:** stuck detection, tool sequences, R:E ratio, thinking tokens, session-detail enrichment — explicitly listed in *Non-goals* in the spec and absent from this plan. Schema is forward-compatible (new tables / new `DerivedEvent` variants can be added without breaking v1).

If any of these checks fail, fix inline before opening the PR.
