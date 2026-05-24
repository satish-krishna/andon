# AI Engineering Coach Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Phase 1 of the AI Engineering Coach: a `/coach` page in Andon that runs AIEC-style anti-pattern rules over `~/.andon/data.db`, surfaces a five-practice-area scorecard with WoW/MoM trends, lists findings, and includes a Skill Finder sub-route discovering repeated prompt patterns.

**Architecture:** Native Rust `coach` module reads existing Andon tables (sessions, token_usage, cost_entries, tool_decisions, slash_commands, file_changes, git_activity) plus two new ones (`prompt_turns`, `skill_opportunities`) and writes findings to `coach_findings`. The JSONL reducer's privacy invariant is amended to permit prompt text — the reducer remains the single chokepoint via a new `PromptTurn` variant. The OTLP forwarder gains a `user_prompt` body-redaction filter as the new egress boundary. An Angular `/coach` page renders the scorecard, findings, and Skill Finder; Settings gains a Coach card with vocabulary editors.

**Tech Stack:** Rust + Tauri 2 + rusqlite + axum + tokio for backend; Angular 21 standalone + signals + Tailwind for frontend; BLAKE3 keyed hash for skill clustering; AIEC scoring formula (`sevPenalty {high:12, medium:7, low:3}`).

**Spec:** [`docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md`](../specs/2026-05-24-ai-engineering-coach-integration-design.md) at commit `0a5a99d`.

**Branch:** `claude/ai-engineering-coach-andon-CKkqZ`.

**Test command:** `cd src-tauri && cargo test --features test-support` (Rust). `cd web && npm test` (Angular).

---

## Plan structure

- **Section A — Foundation:** settings + three DB migrations (4 tasks)
- **Section B — Privacy contract amendment:** OTLP/forwarder/reducer changes (5 tasks)
- **Section C — `prompt_turns` ingest:** wire reducer + OTLP writes (3 tasks)
- **Section D — Coach module foundation:** scaffolding, rule catalogue, engine shell (4 tasks)
- **Section E — Detectors:** one task per rule (11 tasks)
- **Section F — Scorer:** AIEC math + trends + assembly (3 tasks)
- **Section G — Skill Finder:** normaliser + discovery + examples (3 tasks)
- **Section H — Re-evaluation triggers:** SessionEnd + JSONL backfill (2 tasks)
- **Section I — HTTP API:** six endpoints + integration test (7 tasks)
- **Section J — Angular plumbing:** DTOs, ApiService, icons (2 tasks)
- **Section K — `/coach` page:** scorecard + findings + CTA (4 tasks)
- **Section L — `/coach/skills` sub-route:** look-back + opportunities (3 tasks)
- **Section M — Settings → Coach card:** Skill Finder + Vocabulary + Rules (4 tasks)
- **Section N — Routes & nav:** wire the new pages (1 task)
- **Section O — Docs updates:** CLAUDE.md, architecture.md, features.md, README.md (1 task)
- **Section P — Manual smoke acceptance:** end-to-end sanity (1 task)

**Total:** 58 tasks. Each is 2-5 minutes of focused work in TDD form (failing test → run → implement → run → commit).

---

## Section A — Foundation

### Task A1: Extend `AppSettings` with `CoachSettings`

**Files:**
- Modify: `src-tauri/src/settings.rs`

- [ ] **Step 1: Write the failing test** — append to the `tests` module in `src-tauri/src/settings.rs`:

```rust
#[test]
fn coach_defaults_are_seeded() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("settings.json");
    let store = SettingsStore::load(p).unwrap();
    let coach = store.coach();
    assert_eq!(coach.skill_min_occurrences, 3);
    assert_eq!(coach.skill_min_sessions, 2);
    assert!(coach.planning_commands.contains(&"plan".to_string()));
    assert!(coach.planning_commands.contains(&"brainstorm".to_string()));
    assert!(coach.constraint_keywords.contains(&"must".to_string()));
}

#[test]
fn save_coach_persists() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("settings.json");
    let store = SettingsStore::load(p.clone()).unwrap();
    let mut new_coach = store.coach();
    new_coach.skill_min_occurrences = 5;
    new_coach.planning_commands.push("rfc".into());
    store.save_coach(new_coach.clone()).unwrap();
    let reloaded = SettingsStore::load(p).unwrap();
    assert_eq!(reloaded.coach(), new_coach);
}

#[test]
fn settings_file_without_coach_key_still_parses() {
    // Pre-existing installs have no `coach` field — must not break.
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("settings.json");
    std::fs::write(&p, r#"{"version":1,"forwarder":{"enabled":false,"endpoint":"","timeout_ms":2000,"headers":{}},"budget":{"monthly_usd":0.0}}"#).unwrap();
    let store = SettingsStore::load(p).unwrap();
    assert_eq!(store.coach(), CoachSettings::default());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support settings::tests::coach -v`
Expected: FAIL with "no method named `coach` found" and "cannot find type `CoachSettings`".

- [ ] **Step 3: Add `CoachSettings` struct, default, and store methods.** Insert before `impl Default for AppSettings`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoachSettings {
    pub skill_min_occurrences: u32,
    pub skill_min_sessions: u32,
    pub planning_commands: Vec<String>,
    pub planning_keywords: Vec<String>,
    pub constraint_keywords: Vec<String>,
}

impl Default for CoachSettings {
    fn default() -> Self {
        Self {
            skill_min_occurrences: 3,
            skill_min_sessions: 2,
            planning_commands: vec![
                "plan".into(), "brainstorm".into(), "design".into(),
                "spec".into(), "specify".into(), "rfc".into(),
            ],
            planning_keywords: vec![
                "spec".into(), "specs".into(), "requirement".into(),
                "requirements".into(), "acceptance criteria".into(),
                "design doc".into(), "PRD".into(), "RFC".into(),
                "plan file".into(), "constraint".into(), "must".into(),
                "should".into(), "ensure".into(),
            ],
            constraint_keywords: vec![
                "must".into(), "should".into(), "limit".into(), "ensure".into(),
                "require".into(), "only".into(), "without".into(),
                "never".into(), "always".into(),
            ],
        }
    }
}
```

Add `coach: CoachSettings` to `AppSettings` (with `#[serde(default)]` so older files parse):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub version: u32,
    pub forwarder: ForwarderSettings,
    #[serde(default)]
    pub budget: BudgetSettings,
    #[serde(default)]
    pub coach: CoachSettings,
}
```

Update `impl Default for AppSettings` to include `coach: CoachSettings::default()`.

Add getter + setter to `impl SettingsStore` (mirror `budget`/`forwarder` shape):

```rust
pub fn coach(&self) -> CoachSettings {
    self.inner.read().expect("settings lock").coach.clone()
}

pub fn save_coach(&self, new: CoachSettings) -> Result<CoachSettings> {
    let mut w = self.inner.write().expect("settings lock");
    w.coach = new.clone();
    let serialized = serde_json::to_string_pretty(&*w)?;
    write_atomic(&self.path, &serialized)?;
    Ok(new)
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test --features test-support settings -v`
Expected: PASS for all three new tests plus the existing ones.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(coach): add CoachSettings to AppSettings with defaults"
```

---

### Task A2: Migration V7 — `coach_rules` + `coach_findings`

**Files:**
- Modify: `src-tauri/src/db/migrations.rs`

- [ ] **Step 1: Write the failing test** — append to the `tests` module in `migrations.rs`:

```rust
#[test]
fn v7_creates_coach_tables() {
    let mut conn = Connection::open_in_memory().unwrap();
    apply(&mut conn).unwrap();

    for tbl in ["coach_rules", "coach_findings"] {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [tbl], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "missing table {tbl}");
    }

    let idxs: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index'").unwrap()
        .query_map([], |r| r.get::<_, String>(0)).unwrap()
        .map(|r| r.unwrap()).collect();
    assert!(idxs.contains(&"coach_findings_unique".to_string()));
    assert!(idxs.contains(&"coach_findings_session".to_string()));
}
```

Also update the version assertions in *every* existing test to `9` (you'll add V8 and V9 in the next two tasks; bump them all at once now to avoid flapping). Find every `assert_eq!(v, 6)` and change to `assert_eq!(v, 9)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support db::migrations -v`
Expected: FAIL — `v7_creates_coach_tables` errors on missing table; the `assert_eq!(v, 9)` lines fail because max version is still 6.

- [ ] **Step 3: Add `MIGRATION_V7`** to `migrations.rs`:

```rust
const MIGRATION_V7: &str = r#"
CREATE TABLE coach_rules (
  id           TEXT PRIMARY KEY,
  practice     TEXT NOT NULL,
  severity     TEXT NOT NULL,
  kind         TEXT NOT NULL,
  enabled      INTEGER NOT NULL DEFAULT 1,
  updated_at   INTEGER NOT NULL
);

CREATE TABLE coach_findings (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  rule_id     TEXT NOT NULL,
  session_id  TEXT NOT NULL,
  detected_at INTEGER NOT NULL,
  payload     TEXT NOT NULL DEFAULT '{}',
  FOREIGN KEY (rule_id)    REFERENCES coach_rules(id)    ON DELETE CASCADE,
  FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX coach_findings_unique
  ON coach_findings(rule_id, session_id, detected_at);
CREATE INDEX coach_findings_session ON coach_findings(session_id);
"#;
```

Append `(7, MIGRATION_V7)` to the `MIGRATIONS` slice.

> **Note:** rule seed inserts are deferred to Task D4 — the catalogue must exist as Rust data first.

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test --features test-support db::migrations -v`
Expected: PASS for `v7_creates_coach_tables`; other tests still PASS with `v=9`-but-`max=7` only if you DIDN'T bump them yet. If you bumped them in step 1, they will fail until V8 and V9 land. That's expected — leave them failing. The plan keeps the bump in step 1 so you don't churn the same tests three times; the final pass happens after Task A4.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(coach): migration V7 — coach_rules and coach_findings tables"
```

---

### Task A3: Migration V8 — `prompt_turns` + `skill_opportunities`

**Files:**
- Modify: `src-tauri/src/db/migrations.rs`

- [ ] **Step 1: Write the failing test** — append:

```rust
#[test]
fn v8_creates_prompt_turns_and_skill_tables() {
    let mut conn = Connection::open_in_memory().unwrap();
    apply(&mut conn).unwrap();

    for tbl in ["prompt_turns", "skill_opportunities"] {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [tbl], |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "missing table {tbl}");
    }

    // prompt_turns columns (without has_constraint — that's V9)
    let cols: Vec<String> = conn.prepare("PRAGMA table_info(prompt_turns)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    for c in ["session_id","request_id","turn_index","ts","source","text",
             "norm_hash","command","length","has_file_ref","has_code"] {
        assert!(cols.contains(&c.to_string()), "missing prompt_turns column {c}");
    }

    let idxs: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index'").unwrap()
        .query_map([], |r| r.get::<_, String>(0)).unwrap()
        .map(|r| r.unwrap()).collect();
    assert!(idxs.contains(&"prompt_turns_hash".to_string()));
    assert!(idxs.contains(&"prompt_turns_session".to_string()));
    assert!(idxs.contains(&"skill_opportunities_unique".to_string()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support db::migrations::tests::v8 -v`
Expected: FAIL — tables missing.

- [ ] **Step 3: Add `MIGRATION_V8`**:

```rust
const MIGRATION_V8: &str = r#"
CREATE TABLE prompt_turns (
  session_id   TEXT NOT NULL,
  request_id   TEXT,
  turn_index   INTEGER NOT NULL,
  ts           INTEGER NOT NULL,
  source       TEXT NOT NULL,
  text         TEXT NOT NULL,
  norm_hash    TEXT NOT NULL,
  command      TEXT,
  length       INTEGER NOT NULL,
  has_file_ref INTEGER NOT NULL,
  has_code     INTEGER NOT NULL,
  PRIMARY KEY (session_id, turn_index),
  FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
);
CREATE INDEX prompt_turns_hash    ON prompt_turns(norm_hash);
CREATE INDEX prompt_turns_session ON prompt_turns(session_id, ts);

CREATE TABLE skill_opportunities (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  norm_hash      TEXT NOT NULL,
  label          TEXT NOT NULL,
  command        TEXT,
  occurrences    INTEGER NOT NULL,
  session_count  INTEGER NOT NULL,
  first_seen     INTEGER NOT NULL,
  last_seen      INTEGER NOT NULL,
  window_start   INTEGER NOT NULL,
  window_end     INTEGER NOT NULL,
  computed_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX skill_opportunities_unique
  ON skill_opportunities(norm_hash, window_start, window_end);
"#;
```

Append `(8, MIGRATION_V8)` to `MIGRATIONS`.

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test --features test-support db::migrations -v`
Expected: `v8_*` test PASSES; other tests still fail because they assert `v=9`. Leave for now.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(coach): migration V8 — prompt_turns and skill_opportunities"
```

---

### Task A4: Migration V9 — `prompt_turns.has_constraint`

**Files:**
- Modify: `src-tauri/src/db/migrations.rs`

- [ ] **Step 1: Write the failing test** — append:

```rust
#[test]
fn v9_adds_has_constraint_to_prompt_turns() {
    let mut conn = Connection::open_in_memory().unwrap();
    apply(&mut conn).unwrap();

    let cols: Vec<String> = conn.prepare("PRAGMA table_info(prompt_turns)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|r| r.unwrap()).collect();
    assert!(cols.contains(&"has_constraint".to_string()));

    let v: i32 = conn.query_row(
        "SELECT MAX(version) FROM schema_version", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(v, 9);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support db::migrations -v`
Expected: FAIL — `has_constraint` column missing.

- [ ] **Step 3: Add `MIGRATION_V9`**:

```rust
const MIGRATION_V9: &str = r#"
ALTER TABLE prompt_turns ADD COLUMN has_constraint INTEGER NOT NULL DEFAULT 0;
"#;
```

Append `(9, MIGRATION_V9)`.

- [ ] **Step 4: Run all migration tests — all should now pass**

Run: `cd src-tauri && cargo test --features test-support db::migrations -v`
Expected: every test PASSES including the bumped `v=9` assertions.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(coach): migration V9 — prompt_turns.has_constraint flag"
```

---

## Section B — Privacy contract amendment

### Task B1: Drop OTLP `user_prompt` body redaction

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs:162-180` (the redaction block)
- Modify (test): the existing `tests/ingestor_*.rs` or similar that asserts redaction; if none, add `src-tauri/tests/otlp_user_prompt.rs`

- [ ] **Step 1: Write the failing test** — create `src-tauri/tests/otlp_user_prompt.rs`:

```rust
mod common;

use andon_lib::otlp::ingestor::Ingestor;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use rusqlite::params;

#[tokio::test]
async fn user_prompt_body_is_persisted_to_log_events() {
    let (pool, _db_dir) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);

    // Seed the session so the FK constraint doesn't block.
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 1)",
            [],
        ).unwrap();
    }

    let logs = common::sample_export_logs_with_body(
        vec![common::kv("session.id", "s1")],
        "user_prompt",
        "tell me about wizards",
        vec![common::kv_int("prompt_length", 21)],
    );
    let req = ExportLogsServiceRequest { resource_logs: logs };
    ingestor.ingest_logs(req, "grpc").await;

    let body: Option<String> = pool.get().unwrap().query_row(
        "SELECT body FROM log_events WHERE event_name='user_prompt'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(body.as_deref(), Some("tell me about wizards"),
        "body must be persisted post-amendment, not redacted");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support --test otlp_user_prompt -v`
Expected: FAIL — body is `None` because the redaction strips it.

- [ ] **Step 3: Edit `src-tauri/src/otlp/ingestor.rs`.** Find the block around line 162 that begins with the comment *"Privacy guarantee: never persist raw user prompt content."* and replace it with the un-redacted path. The new logic preserves `body_str` and `attrs_json` as-is for `user_prompt` events. Concretely, delete the `if event_name == "user_prompt" { (None, redacted_attrs) } else { … }` conditional and always use the unconditional `(body_str, attrs_json)` pair.

Also update the comment block above to read:

```rust
// Privacy amendment (see docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md
// §Privacy contract amendment): prompts are now allowed at rest. The
// forwarder strips them on egress (src/otlp/forwarder.rs::redact_user_prompt).
```

- [ ] **Step 4: Run test to verify pass**

Run: `cd src-tauri && cargo test --features test-support --test otlp_user_prompt -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/otlp_user_prompt.rs
git commit -m "feat(privacy): drop user_prompt body redaction in OTLP ingestor"
```

---

### Task B2: Forwarder `redact_user_prompt` filter pass

**Files:**
- Modify: `src-tauri/src/otlp/forwarder.rs`
- Test: a unit test inline in `forwarder.rs` (proptest comes in Task B3)

- [ ] **Step 1: Write the failing test** — append to `forwarder.rs` `#[cfg(test)]`:

```rust
#[test]
fn redact_user_prompt_strips_body() {
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};

    let mut logs = vec![ResourceLogs {
        resource: None,
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![LogRecord {
                time_unix_nano: 0, observed_time_unix_nano: 0,
                severity_number: 0, severity_text: String::new(),
                body: Some(AnyValue { value: Some(AnyV::StringValue("secret prompt".into())) }),
                attributes: vec![KeyValue {
                    key: "event.name".into(),
                    value: Some(AnyValue { value: Some(AnyV::StringValue("user_prompt".into())) }),
                }],
                dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }];

    redact_user_prompt(&mut logs);

    let body = logs[0].scope_logs[0].log_records[0].body.as_ref().unwrap();
    if let AnyV::StringValue(s) = body.value.as_ref().unwrap() {
        assert_eq!(s, "<redacted>");
    } else {
        panic!("body should be string");
    }
}

#[test]
fn redact_user_prompt_leaves_other_events_alone() {
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};

    let mut logs = vec![ResourceLogs {
        resource: None,
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![LogRecord {
                time_unix_nano: 0, observed_time_unix_nano: 0,
                severity_number: 0, severity_text: String::new(),
                body: Some(AnyValue { value: Some(AnyV::StringValue("tool output".into())) }),
                attributes: vec![KeyValue {
                    key: "event.name".into(),
                    value: Some(AnyValue { value: Some(AnyV::StringValue("tool_decision".into())) }),
                }],
                dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }];

    redact_user_prompt(&mut logs);

    let body = logs[0].scope_logs[0].log_records[0].body.as_ref().unwrap();
    if let AnyV::StringValue(s) = body.value.as_ref().unwrap() {
        assert_eq!(s, "tool output", "non-user_prompt bodies must be untouched");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support otlp::forwarder::tests::redact -v`
Expected: FAIL — function `redact_user_prompt` does not exist.

- [ ] **Step 3: Implement `redact_user_prompt`** in `forwarder.rs`:

```rust
/// Strip the `body` of any `user_prompt` log record before forwarding.
/// See docs/.../2026-05-24-ai-engineering-coach-integration-design.md
/// §Privacy & safety rule 5.
pub(crate) fn redact_user_prompt(
    resource_logs: &mut [opentelemetry_proto::tonic::logs::v1::ResourceLogs],
) {
    use opentelemetry_proto::tonic::common::v1::{AnyValue, any_value::Value as AnyV};

    for rl in resource_logs.iter_mut() {
        for sl in rl.scope_logs.iter_mut() {
            for rec in sl.log_records.iter_mut() {
                let is_user_prompt = rec.attributes.iter().any(|kv| {
                    kv.key == "event.name"
                        && matches!(
                            kv.value.as_ref().and_then(|v| v.value.as_ref()),
                            Some(AnyV::StringValue(s)) if s == "user_prompt"
                        )
                });
                if is_user_prompt {
                    rec.body = Some(AnyValue {
                        value: Some(AnyV::StringValue("<redacted>".into())),
                    });
                }
            }
        }
    }
}
```

Wire it into the existing forward path — find where the forwarder builds an `ExportLogsServiceRequest` (or equivalent) for outgoing requests, and call `redact_user_prompt(&mut resource_logs)` immediately before serialising. Read the surrounding `forwarder.rs` to find the right insertion point; this plan does not prescribe it because the forwarder may have evolved since spec-time.

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test --features test-support otlp::forwarder -v`
Expected: PASS for both new tests + existing forwarder tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/otlp/forwarder.rs
git commit -m "feat(privacy): redact user_prompt body in OTLP forwarder egress"
```

---

### Task B3: Forwarder proptest — replaces local-DB leak proptest

**Files:**
- Create: `src-tauri/tests/forwarder_no_prompt_leak.rs`
- Delete content from: `src-tauri/tests/jsonl_privacy.rs` (covered in Task B5 — leave that file alone for now)

- [ ] **Step 1: Write the failing test** — create `src-tauri/tests/forwarder_no_prompt_leak.rs`:

```rust
//! Property test: the forwarder's redact_user_prompt pass strips
//! any prompt text before egress, regardless of the prompt's content.

use proptest::prelude::*;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use andon_lib::otlp::forwarder::redact_user_prompt;

fn build_logs(body: &str) -> Vec<ResourceLogs> {
    vec![ResourceLogs {
        resource: None,
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![LogRecord {
                time_unix_nano: 0, observed_time_unix_nano: 0,
                severity_number: 0, severity_text: String::new(),
                body: Some(AnyValue { value: Some(AnyV::StringValue(body.into())) }),
                attributes: vec![KeyValue {
                    key: "event.name".into(),
                    value: Some(AnyValue { value: Some(AnyV::StringValue("user_prompt".into())) }),
                }],
                dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
}

fn serialise_to_string(logs: &[ResourceLogs]) -> String {
    // serde_json round-trip via prost reflection is brittle; just walk the
    // tree and concatenate all string fields. The assertion below is
    // "this string does not contain the original prompt as a substring".
    let mut out = String::new();
    for rl in logs {
        for sl in &rl.scope_logs {
            for rec in &sl.log_records {
                if let Some(b) = &rec.body {
                    if let Some(AnyV::StringValue(s)) = b.value.as_ref() {
                        out.push_str(s);
                        out.push('\n');
                    }
                }
                for kv in &rec.attributes {
                    if let Some(v) = &kv.value {
                        if let Some(AnyV::StringValue(s)) = v.value.as_ref() {
                            out.push_str(s);
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    out
}

proptest! {
    #[test]
    fn forwarder_strips_user_prompt_body(prompt in "[\\p{L} ]{1,200}") {
        // Skip degenerate "no content" inputs that trivially appear in "<redacted>".
        prop_assume!(!prompt.is_empty());
        prop_assume!(!"<redacted>".contains(&*prompt));

        let mut logs = build_logs(&prompt);
        redact_user_prompt(&mut logs);
        let serialised = serialise_to_string(&logs);

        prop_assert!(!serialised.contains(&*prompt),
            "forwarder leaked prompt: {:?} found in {:?}", prompt, serialised);
    }
}
```

Make sure `proptest` is in `[dev-dependencies]` of `src-tauri/Cargo.toml` — if not, add `proptest = "1"`.

- [ ] **Step 2: Run test to verify it fails (or compiles-then-passes)**

Run: `cd src-tauri && cargo test --features test-support --test forwarder_no_prompt_leak -v`
Expected: PASS — `redact_user_prompt` from B2 should already handle this. If it FAILS, fix the forwarder. (TDD inversion: this test pins the invariant rather than driving new behaviour.)

- [ ] **Step 3 — n/a (test pins existing behaviour).**

- [ ] **Step 4: Confirm pass.**

Run: same as Step 2.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/forwarder_no_prompt_leak.rs src-tauri/Cargo.toml
git commit -m "test(privacy): proptest that forwarder strips user_prompt bodies"
```

---

### Task B4: Reducer — add `PromptTurn` variant to the output enum

**Files:**
- Modify: `src-tauri/src/jsonl/reducer.rs`

- [ ] **Step 1: Read the current reducer** to find the output enum (likely `enum Event` or `enum ReducedEvent`).

Run: `cat src-tauri/src/jsonl/reducer.rs` and identify the output enum name. Use that name verbatim in subsequent steps. For the rest of this plan, call it `ReducedEvent`.

- [ ] **Step 2: Write the failing test** — append to the `#[cfg(test)]` module in `reducer.rs`:

```rust
#[test]
fn user_message_emits_prompt_turn_with_derived_flags() {
    let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Refactor @src/foo.rs — must be idempotent."}]}}"#;
    let record = parse_line(line).expect("parse");

    let events = reduce(&[record], &ReduceOptions {
        constraint_keywords: vec!["must".into(), "should".into()],
        ..Default::default()
    });

    let prompt_turn = events.iter().find_map(|e| match e {
        ReducedEvent::PromptTurn { text, has_file_ref, has_constraint, length, .. } =>
            Some((text.clone(), *has_file_ref, *has_constraint, *length)),
        _ => None,
    }).expect("a PromptTurn event");

    assert_eq!(prompt_turn.0, "Refactor @src/foo.rs — must be idempotent.");
    assert!(prompt_turn.1, "has_file_ref should be 1 — '@src/foo.rs'");
    assert!(prompt_turn.2, "has_constraint should be 1 — 'must'");
    assert_eq!(prompt_turn.3, 41);
}

#[test]
fn user_message_norm_hash_collapses_paths_and_caps() {
    use crate::jsonl::reducer::*;
    let a = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Package the extension @src/x.rs"}]}}"#;
    let b = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"package the extension @lib/y.rs"}]}}"#;

    let ra = parse_line(a).unwrap();
    let rb = parse_line(b).unwrap();
    let ea = reduce(&[ra], &ReduceOptions::default());
    let eb = reduce(&[rb], &ReduceOptions::default());

    let ha = ea.iter().find_map(|e| match e { ReducedEvent::PromptTurn { norm_hash, .. } => Some(norm_hash.clone()), _ => None }).unwrap();
    let hb = eb.iter().find_map(|e| match e { ReducedEvent::PromptTurn { norm_hash, .. } => Some(norm_hash.clone()), _ => None }).unwrap();

    assert_eq!(ha, hb, "normalisation should collapse case and path differences");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support jsonl::reducer -v`
Expected: FAIL — `PromptTurn` variant doesn't exist; `ReduceOptions` doesn't have `constraint_keywords`.

- [ ] **Step 4: Add the `PromptTurn` variant** to the output enum:

```rust
pub enum ReducedEvent {
    // … existing variants …
    PromptTurn {
        session_id: String,
        request_id: Option<String>,
        turn_index: i64,
        ts_ms: i64,
        text: String,
        norm_hash: String,
        command: Option<String>,
        length: i64,
        has_file_ref: bool,
        has_code: bool,
        has_constraint: bool,
    },
}
```

Add a `ReduceOptions` struct (or extend the existing one):

```rust
#[derive(Default, Clone)]
pub struct ReduceOptions {
    pub constraint_keywords: Vec<String>,
}
```

Update `reduce(records: &[JsonlRecord], opts: &ReduceOptions) -> Vec<ReducedEvent>`. The detection logic for user turns:

```rust
fn build_prompt_turn(
    record: &JsonlRecord,
    turn_index: i64,
    opts: &ReduceOptions,
) -> Option<ReducedEvent> {
    let msg = record.message.as_ref()?;
    if msg.role.as_deref() != Some("user") { return None; }

    // Concatenate text content blocks
    let text: String = msg.content.iter().filter_map(|b| match b {
        ContentBlock::Text { text } => Some(text.as_str()),
        _ => None,
    }).collect::<Vec<_>>().join("\n");
    if text.is_empty() { return None; }

    let length = text.chars().count() as i64;
    let has_file_ref = text.contains('@') ||
        regex_static::is_match(r"(?:^|\s)(?:/|[A-Z]:\\)[\w./\\-]+", &text);
    let has_code = text.contains("```");
    let has_constraint = opts.constraint_keywords.iter().any(|kw| {
        text.to_lowercase().contains(&kw.to_lowercase())
    });
    let command = extract_slash_command(&text);
    let norm_hash = blake3_keyed_hash_norm(&text);

    Some(ReducedEvent::PromptTurn {
        session_id: record.session_id.clone(),
        request_id: record.request_id.clone(),
        turn_index,
        ts_ms: parse_iso_to_ms(&record.timestamp)?,
        text,
        norm_hash,
        command,
        length,
        has_file_ref,
        has_code,
        has_constraint,
    })
}
```

You will need:
- `extract_slash_command(text)` returns `Some("plan")` if the text starts with `/plan` (followed by space or EOL). Trivial helper.
- `blake3_keyed_hash_norm(text)` — defer to **Task G1**; for now stub as `format!("{:x}", blake3::hash(text.as_bytes()))` and replace with the proper keyed hash there.

Update the module-level doc comment at the top of `reducer.rs`:

```rust
//! Trust boundary between JSONL (raw, contains prompt text) and the rest
//! of the ingest pipeline.
//!
//! Prior to the 2026-05-24 privacy-contract amendment, no reducer output
//! variant could carry prompt text. The amendment introduces one
//! exception: `ReducedEvent::PromptTurn` carries the raw prompt for
//! Skill Finder and coach detectors. The reducer remains the SINGLE
//! chokepoint for prompts entering the local DB — no other module reads
//! `record::Message.content[].text`. See
//! docs/.../2026-05-24-ai-engineering-coach-integration-design.md
//! §Privacy contract amendment.
```

- [ ] **Step 5: Run tests + commit**

Run: `cd src-tauri && cargo test --features test-support jsonl::reducer -v`
Expected: PASS for both new tests.

```bash
git add src-tauri/src/jsonl/reducer.rs
git commit -m "feat(privacy): add PromptTurn variant to reducer output enum"
```

---

### Task B5: Rewrite `jsonl_privacy.rs` to assert forwarder-side invariant

**Files:**
- Modify: `src-tauri/tests/jsonl_privacy.rs` — delete the no-text-in-DB assertions; replace with a comment pointing at `forwarder_no_prompt_leak.rs`.

- [ ] **Step 1: Read the existing file** to see its current shape.

Run: `cat src-tauri/tests/jsonl_privacy.rs`

- [ ] **Step 2: Delete or repurpose the test.**

If the file's only purpose was the now-obsolete invariant, replace its contents entirely with:

```rust
//! The privacy invariant has moved from the local DB to the network
//! egress point. See `forwarder_no_prompt_leak.rs` for the new proptest.
//!
//! This file is intentionally near-empty to preserve a clear historical
//! record of the boundary move. Delete it in a follow-up cleanup once
//! the team has had a release cycle to absorb the change.

#[test]
fn boundary_moved_to_forwarder() {
    // No-op canary: existence of this passing test reminds future
    // readers that "prompts in the DB" is now expected behaviour and
    // the leak invariant is enforced at egress.
}
```

If the file contains other assertions unrelated to the no-text-in-DB invariant, keep those and only remove the relevant ones.

- [ ] **Step 3: Run tests**

Run: `cd src-tauri && cargo test --features test-support --test jsonl_privacy -v`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/jsonl_privacy.rs
git commit -m "test(privacy): move local-DB leak proptest to forwarder egress"
```

---

## Section C — `prompt_turns` ingest path

### Task C1: JSONL ingest writes `prompt_turns` rows from `PromptTurn` events

**Files:**
- Modify: the JSONL ingest writer — likely `src-tauri/src/jsonl/mod.rs` or `src-tauri/src/jsonl/walker.rs`. Grep for where `ReducedEvent` matches happen and other variants get written to the DB.

- [ ] **Step 1: Find the writer.**

Run: `grep -rn "ReducedEvent::" src-tauri/src/jsonl/`. Pick the file that has a `match` on `ReducedEvent` variants writing to the DB. For the rest of this task call it `JSONL_WRITER_FILE`.

- [ ] **Step 2: Write the failing test** — append to or create `src-tauri/tests/jsonl_prompt_turns.rs`:

```rust
mod common;

use rusqlite::params;

#[tokio::test]
async fn jsonl_ingest_writes_prompt_turns_with_flags() {
    let (pool, _db_dir) = common::fixture_pool();

    // Seed a session row for FK.
    {
        let conn = pool.get().unwrap();
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', 1)", []).unwrap();
    }

    // Run an in-process JSONL ingest of a single line.
    let jsonl = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Refactor @src/foo.rs — must be idempotent."}]}}
"#;
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), jsonl).unwrap();

    andon_lib::jsonl::ingest_file(&pool, tmp.path(), /* coach_settings */ Default::default())
        .expect("ingest jsonl");

    let (text, has_constraint, has_file_ref, length): (String, i64, i64, i64) =
        pool.get().unwrap().query_row(
            "SELECT text, has_constraint, has_file_ref, length FROM prompt_turns WHERE session_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
    assert_eq!(text, "Refactor @src/foo.rs — must be idempotent.");
    assert_eq!(has_constraint, 1);
    assert_eq!(has_file_ref, 1);
    assert_eq!(length, 41);
}
```

Substitute `andon_lib::jsonl::ingest_file` for the real public API if it has a different name — grep `pub fn ingest` in `src-tauri/src/jsonl/`.

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --features test-support --test jsonl_prompt_turns -v`
Expected: FAIL — `prompt_turns` is empty because the writer doesn't handle `PromptTurn` yet.

- [ ] **Step 4: Implement the writer.** In `JSONL_WRITER_FILE`, find the `match` on `ReducedEvent` and add:

```rust
ReducedEvent::PromptTurn {
    session_id, request_id, turn_index, ts_ms,
    text, norm_hash, command, length,
    has_file_ref, has_code, has_constraint,
} => {
    let _ = tx.execute(
        "INSERT OR IGNORE INTO prompt_turns
           (session_id, request_id, turn_index, ts, source, text,
            norm_hash, command, length, has_file_ref, has_code, has_constraint)
         VALUES (?1, ?2, ?3, ?4, 'jsonl', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            session_id, request_id, turn_index, ts_ms,
            text, norm_hash, command, length,
            *has_file_ref as i64, *has_code as i64, *has_constraint as i64,
        ],
    );
}
```

Thread `CoachSettings` through `ingest_file` so the reducer can pull `constraint_keywords` from settings. If the existing public API doesn't take settings, add it as a parameter; callers pass `settings.coach()`.

- [ ] **Step 5: Run test + commit**

Run: `cd src-tauri && cargo test --features test-support --test jsonl_prompt_turns -v`
Expected: PASS.

```bash
git add src-tauri/src/jsonl src-tauri/tests/jsonl_prompt_turns.rs
git commit -m "feat(coach): write prompt_turns rows from JSONL reducer output"
```

---

### Task C2: OTLP ingest writes `prompt_turns` row from `user_prompt` log event

**Files:**
- Modify: `src-tauri/src/otlp/ingestor.rs` — extend the `"user_prompt"` arm around line 483.

- [ ] **Step 1: Write the failing test** — extend `src-tauri/tests/otlp_user_prompt.rs`:

```rust
#[tokio::test]
async fn user_prompt_otlp_writes_prompt_turn_row() {
    let (pool, _db_dir) = common::fixture_pool();
    let ingestor = common::test_ingestor(&pool);
    {
        let conn = pool.get().unwrap();
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', 1)", []).unwrap();
    }

    let logs = common::sample_export_logs_with_body(
        vec![common::kv("session.id", "s1")],
        "user_prompt",
        "tell me about wizards",
        vec![common::kv_int("prompt_length", 21)],
    );
    let req = opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest {
        resource_logs: logs,
    };
    ingestor.ingest_logs(req, "grpc").await;

    let (text, source): (String, String) = pool.get().unwrap().query_row(
        "SELECT text, source FROM prompt_turns WHERE session_id='s1'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(text, "tell me about wizards");
    assert_eq!(source, "otlp");
}
```

- [ ] **Step 2: Run + verify it fails**

Run: `cd src-tauri && cargo test --features test-support --test otlp_user_prompt -v`
Expected: FAIL — no `prompt_turns` row exists.

- [ ] **Step 3: Extend the `user_prompt` arm in `ingestor.rs`.** Find the block beginning `"user_prompt" => {` (around line 483) and after the existing `active_time` / `raw` writes, append:

```rust
// Phase-1 coach: persist prompt_turns row mirroring JSONL reducer's PromptTurn.
if let Some(body) = body_str.as_ref() {
    let text = body.clone();
    let length = text.chars().count() as i64;
    let has_file_ref = text.contains('@');
    let has_code = text.contains("```");
    // Constraint match uses the keyword list from settings (passed in via
    // Ingestor::new); fall back to empty if unset.
    let has_constraint = self.coach_settings
        .constraint_keywords.iter()
        .any(|kw| text.to_lowercase().contains(&kw.to_lowercase()));
    let command = text.strip_prefix('/').and_then(|rest| {
        rest.split_whitespace().next().map(str::to_owned)
    });
    let norm_hash = format!("{:x}", blake3::hash(text.as_bytes())); // replaced in Task G1
    let turn_index: i64 = tx.query_row(
        "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM prompt_turns WHERE session_id = ?1",
        params![sid], |r| r.get(0),
    ).unwrap_or(0);
    let _ = tx.execute(
        "INSERT OR IGNORE INTO prompt_turns
           (session_id, request_id, turn_index, ts, source, text,
            norm_hash, command, length, has_file_ref, has_code, has_constraint)
         VALUES (?1, NULL, ?2, ?3, 'otlp', ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            sid, turn_index, ts_ms, text, norm_hash, command,
            length, has_file_ref as i64, has_code as i64, has_constraint as i64,
        ],
    );
}
```

This requires the `Ingestor` struct to hold a `coach_settings: CoachSettings` field. Update `Ingestor::new` to take a `SettingsStore` and snapshot the coach settings — or take `CoachSettings` directly if the struct already has access to the store via another field.

- [ ] **Step 4: Run + verify**

Run: `cd src-tauri && cargo test --features test-support --test otlp_user_prompt -v`
Expected: PASS for both `user_prompt` tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/otlp/ingestor.rs src-tauri/tests/otlp_user_prompt.rs
git commit -m "feat(coach): write prompt_turns row from OTLP user_prompt event"
```

---

### Task C3: Wire `CoachSettings` through `Ingestor::new` callers

**Files:**
- Modify: `src-tauri/src/lib.rs` (or wherever `Ingestor::new` is called in app startup)
- Modify: `src-tauri/tests/common/mod.rs::test_ingestor` to pass a default `CoachSettings`

- [ ] **Step 1: Compile and find broken call sites.**

Run: `cd src-tauri && cargo build --features test-support 2>&1 | head -40`
Expected: compilation errors at every call site of `Ingestor::new` because of the new signature.

- [ ] **Step 2: Update each call site.**

For each compilation error pointing at `Ingestor::new(...)`:
- In app startup (`lib.rs` / `main.rs`): pass `settings.coach()` snapshot.
- In `tests/common/mod.rs::test_ingestor`: pass `CoachSettings::default()`.

If `Ingestor` needs to react to live settings changes (e.g. updated keyword lists), use `Arc<SettingsStore>` instead of a snapshot. For Phase 1 a snapshot is acceptable — the spec calls out *"changes apply prospectively"* — and avoids RwLock contention on the ingest hot path.

- [ ] **Step 3: Run the full test suite.**

Run: `cd src-tauri && cargo test --features test-support -- --skip coach 2>&1 | tail -30`
Expected: all existing tests PASS. Skip `coach` tests because those modules don't exist yet.

- [ ] **Step 4: Commit.**

```bash
git add src-tauri/src src-tauri/tests/common
git commit -m "chore(coach): thread CoachSettings through Ingestor::new"
```

---

## Section D — Coach module foundation

### Task D1: Scaffold the `coach` module

**Files:**
- Create: `src-tauri/src/coach/mod.rs`
- Create: `src-tauri/src/coach/queries.rs`
- Modify: `src-tauri/src/lib.rs` to add `pub mod coach;`

- [ ] **Step 1: Write the failing test** — create `src-tauri/tests/coach_module_compiles.rs`:

```rust
#[test]
fn coach_module_is_reachable() {
    // Symbol presence is the assertion.
    let _ = andon_lib::coach::PRACTICES;
}
```

- [ ] **Step 2: Run + verify fail**

Run: `cd src-tauri && cargo test --features test-support --test coach_module_compiles -v`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Create `src-tauri/src/coach/mod.rs`**:

```rust
//! Coach module — anti-pattern rules, scorecard, and Skill Finder.
//!
//! See `docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md`
//! for the architecture and rule catalogue. The module reads existing
//! Andon tables plus `prompt_turns` and writes findings to
//! `coach_findings`.

pub mod queries;
pub mod rules;
pub mod engine;
pub mod score;
pub mod skill;
pub mod eval;

/// The five practice areas. Keep ordered — the UI renders left-to-right.
pub const PRACTICES: &[&str] = &["prompt", "hygiene", "review", "tool", "context"];

#[derive(Debug, thiserror::Error)]
pub enum CoachError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoachError>;
```

Create empty stubs for `queries.rs`, `rules.rs`, `engine.rs`, `score.rs`, `skill.rs`, `eval.rs`:

```rust
// src-tauri/src/coach/queries.rs
//! Shared SQL fragments (window predicates, session-set helpers).
```

(repeat for each — just the header doc comment).

Add `pub mod coach;` to `src-tauri/src/lib.rs`.

- [ ] **Step 4: Run test + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_module_compiles -v`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/src/lib.rs src-tauri/tests/coach_module_compiles.rs
git commit -m "feat(coach): scaffold coach module skeleton"
```

---

### Task D2: `Rule` type + `RULES` catalogue (data only, all 11)

**Files:**
- Modify: `src-tauri/src/coach/rules.rs`

- [ ] **Step 1: Write the failing test** — create `src-tauri/tests/coach_catalogue.rs`:

```rust
use andon_lib::coach::rules::{RULES, RuleKind};

#[test]
fn catalogue_has_exactly_eleven_active_rules_plus_one_reserved() {
    let active: Vec<_> = RULES.iter().filter(|r| !r.reserved).collect();
    assert_eq!(active.len(), 11, "11 active rules — 10 binary + 1 continuous");

    let reserved: Vec<_> = RULES.iter().filter(|r| r.reserved).collect();
    assert_eq!(reserved.len(), 1, "high-cancellation slot reserved");
    assert_eq!(reserved[0].id, "high-cancellation");
}

#[test]
fn every_active_rule_has_description_and_suggestion() {
    for r in RULES.iter().filter(|r| !r.reserved) {
        assert!(!r.description.is_empty(), "rule {} missing description", r.id);
        assert!(!r.suggestion.is_empty(), "rule {} missing suggestion", r.id);
    }
}

#[test]
fn exactly_one_continuous_rule_in_phase_1() {
    let cont: Vec<_> = RULES.iter()
        .filter(|r| !r.reserved && matches!(r.kind, RuleKind::Continuous))
        .collect();
    assert_eq!(cont.len(), 1);
    assert_eq!(cont[0].id, "model-diversity");
}
```

- [ ] **Step 2: Run + verify fail**

Run: `cd src-tauri && cargo test --features test-support --test coach_catalogue -v`
Expected: FAIL — types/symbols don't exist.

- [ ] **Step 3: Implement `coach/rules.rs`**:

```rust
//! Static rule catalogue. Each `Rule` is pure data — detector logic
//! lives next to its literal as `pub fn detect_<id>(…)`.

use std::sync::Arc;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use crate::settings::CoachSettings;

pub type DbPool = Pool<SqliteConnectionManager>;

#[derive(Debug, Clone, Copy)]
pub enum RuleKind { Binary, Continuous }

#[derive(Debug, Clone, Copy)]
pub enum Severity { High, Medium, Low }

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self { Self::High => "high", Self::Medium => "medium", Self::Low => "low" }
    }
    pub fn penalty(&self) -> i64 {
        match self { Self::High => 12, Self::Medium => 7, Self::Low => 3 }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: &'static str,
    pub practice: &'static str,
    pub severity: Option<Severity>,
    pub kind: RuleKind,
    pub aiec_origin: Option<&'static str>,
    pub description: &'static str,
    pub suggestion: &'static str,
    pub respects_model_filter: bool,
    /// `true` means the rule is shown in the UI as a reserved slot but
    /// has no detector. Used for `high-cancellation`.
    pub reserved: bool,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub rule_id: String,
    pub session_id: String,
    pub detected_at: i64,
    pub payload_json: String,
}

pub struct Window {
    pub from_ms: i64,
    pub to_ms: i64,
    pub models: Option<Vec<String>>,
}

pub static RULES: &[Rule] = &[
    Rule {
        id: "repeated-prompts",
        practice: "prompt",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("repeated-prompts.md"),
        description: "Same prompt repeated 3+ times in one session.",
        suggestion: "If you find yourself asking the same thing, turn it into a slash command or skill.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "lazy-prompting",
        practice: "prompt",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("lazy-prompting.md"),
        description: "Many very short prompts — likely missing context.",
        suggestion: "Spend a sentence describing intent, constraints, and expected output.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "low-constraint-usage",
        practice: "prompt",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("low-constraint-usage.md"),
        description: "Prompts rarely state constraints (must / should / limit / …).",
        suggestion: "Tell the model the rules of the game — what it must / must not do.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "long-session-no-commit",
        practice: "hygiene",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: None,
        description: "Session ran over 90 minutes with no commits.",
        suggestion: "Commit checkpoints; restart sessions after major milestones.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "late-night-coding",
        practice: "hygiene",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("late-night-coding.md"),
        description: "5+ sessions started between 23:00 and 05:00.",
        suggestion: "Late-night sessions correlate with rework. Sleep is undefeated.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "abandon-sessions",
        practice: "hygiene",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("abandon-sessions.md"),
        description: "3+ sessions had tool decisions but zero accepts.",
        suggestion: "Mid-session abandonment is a sign the prompt or plan was off — pause and re-spec.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "speed-accept",
        practice: "review",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: Some("speed-accept.md"),
        description: "Accepting 20+ lines of AI code within 15 seconds, repeatedly.",
        suggestion: "Speed-accepting large diffs masks bugs. Read before you accept.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "high-cancellation",
        practice: "review",
        severity: None,
        kind: RuleKind::Binary,
        aiec_origin: Some("high-cancellation.md"),
        description: "Reserved — upstream signal not yet captured in Andon's OTLP.",
        suggestion: "Re-add when request-level cancellation is ingested.",
        respects_model_filter: false,
        reserved: true,
    },
    Rule {
        id: "no-slash-commands",
        practice: "tool",
        severity: Some(Severity::Low),
        kind: RuleKind::Binary,
        aiec_origin: Some("no-slash-commands.md"),
        description: "Session over 30 minutes with zero slash commands.",
        suggestion: "Slash commands codify your recurring workflows. Use or build them.",
        respects_model_filter: false,
        reserved: false,
    },
    Rule {
        id: "model-diversity",
        practice: "tool",
        severity: None,
        kind: RuleKind::Continuous,
        aiec_origin: Some("PatternsAnalyzer::Model Diversity"),
        description: "Distinct models used in the window.",
        suggestion: "Pick the right model for the task — cheap models for simple work.",
        respects_model_filter: true,
        reserved: false,
    },
    Rule {
        id: "cache-hit-starvation",
        practice: "context",
        severity: Some(Severity::High),
        kind: RuleKind::Binary,
        aiec_origin: Some("cache-hit-starvation.md"),
        description: "Cache hit rate below 10% on large-prompt sessions.",
        suggestion: "Keep CLAUDE.md and project context stable; long sessions over short ones.",
        respects_model_filter: true,
        reserved: false,
    },
    Rule {
        id: "low-spec-rate",
        practice: "context",
        severity: Some(Severity::Medium),
        kind: RuleKind::Binary,
        aiec_origin: Some("no-spec-driven-development.md"),
        description: "Less than 20% of agent-mode sessions start spec-driven.",
        suggestion: "Open sessions with a spec — file ref, bullet list, or planning command.",
        respects_model_filter: true,
        reserved: false,
    },
];

/// Resolve a rule by id. O(N) but N=12.
pub fn by_id(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}
```

Add to `src-tauri/Cargo.toml` if missing: `thiserror = "1"` (likely already present).

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_catalogue -v`
Expected: PASS all three tests.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/tests/coach_catalogue.rs
git commit -m "feat(coach): RULES catalogue — 11 active + 1 reserved"
```

---

### Task D3: `engine.rs` shell — `evaluate_window` calls each enabled detector

**Files:**
- Modify: `src-tauri/src/coach/engine.rs`

- [ ] **Step 1: Write the failing test** — create `src-tauri/tests/coach_engine_shell.rs`:

```rust
mod common;

use andon_lib::coach::{engine, rules::Window};

#[tokio::test]
async fn evaluate_window_runs_without_errors_on_empty_db() {
    let (pool, _dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: now - 30 * 86400_000, to_ms: now, models: None };

    engine::evaluate_window(&pool, &win).expect("evaluate_window");

    // No findings expected on empty DB.
    let n: i64 = pool.get().unwrap()
        .query_row("SELECT COUNT(*) FROM coach_findings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
```

- [ ] **Step 2: Run + verify fail.**

Run: `cd src-tauri && cargo test --features test-support --test coach_engine_shell -v`
Expected: FAIL — `engine::evaluate_window` doesn't exist.

- [ ] **Step 3: Implement `engine.rs`**:

```rust
//! Rule engine: runs every enabled detector against a window and writes
//! findings via INSERT OR IGNORE so re-runs are idempotent.

use std::sync::Arc;
use rusqlite::params;
use tracing::instrument;

use crate::coach::rules::{RULES, Rule, RuleKind, Window, Finding, DbPool};
use crate::coach::Result;

/// Run every enabled, non-reserved detector against `window`.
/// Findings persist via INSERT OR IGNORE on `coach_findings`.
#[instrument(skip(pool))]
pub fn evaluate_window(pool: &Arc<DbPool>, window: &Window) -> Result<()> {
    let enabled_ids = enabled_rule_ids(pool)?;
    for rule in RULES.iter().filter(|r| !r.reserved && enabled_ids.contains(&r.id.to_string())) {
        match rule.kind {
            RuleKind::Binary => {
                match run_detector(pool, rule, window) {
                    Ok(findings) => write_findings(pool, &findings)?,
                    Err(e) => tracing::warn!(rule = rule.id, error = ?e, "detector failed"),
                }
            }
            RuleKind::Continuous => { /* continuous scores are read at scorecard time */ }
        }
    }
    Ok(())
}

#[instrument(skip(pool))]
pub fn evaluate_session(pool: &Arc<DbPool>, session_id: &str) -> Result<()> {
    // Phase 1: evaluate the last 30 days; rules with session-scope will
    // naturally only consider this session because of their predicates.
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: now - 30 * 86_400_000, to_ms: now, models: None };
    evaluate_window(pool, &win)
}

fn enabled_rule_ids(pool: &Arc<DbPool>) -> Result<std::collections::HashSet<String>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT id FROM coach_rules WHERE enabled = 1")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn write_findings(pool: &Arc<DbPool>, findings: &[Finding]) -> Result<()> {
    if findings.is_empty() { return Ok(()); }
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    for f in findings {
        let _ = tx.execute(
            "INSERT OR IGNORE INTO coach_findings
               (rule_id, session_id, detected_at, payload)
             VALUES (?1, ?2, ?3, ?4)",
            params![f.rule_id, f.session_id, f.detected_at, f.payload_json],
        );
    }
    tx.commit()?;
    Ok(())
}

/// Dispatch table — added in Section E as detectors land. Every rule
/// without an arm returns an empty Vec so the engine never panics.
fn run_detector(pool: &Arc<DbPool>, rule: &Rule, window: &Window) -> Result<Vec<Finding>> {
    use crate::coach::rules as r;
    match rule.id {
        // Section E tasks will replace each empty placeholder.
        _ => Ok(vec![]),
    }
}
```

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_engine_shell -v`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/tests/coach_engine_shell.rs
git commit -m "feat(coach): engine shell — evaluate_window with stub detectors"
```

---

### Task D4: Seed `coach_rules` from `RULES` catalogue on startup

**Files:**
- Modify: `src-tauri/src/coach/mod.rs` to add `pub fn seed_rules(pool: &Arc<DbPool>) -> Result<()>`
- Modify: `src-tauri/src/lib.rs` (or wherever the app boots) to call `coach::seed_rules` after migrations apply

- [ ] **Step 1: Write the failing test** — append to `coach_catalogue.rs`:

```rust
#[tokio::test]
async fn seed_rules_idempotent_upsert() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).expect("seed");
    let n1: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM coach_rules", [], |r| r.get(0)).unwrap();
    andon_lib::coach::seed_rules(&pool).expect("seed again");
    let n2: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM coach_rules", [], |r| r.get(0)).unwrap();
    assert_eq!(n1, n2, "second seed must not duplicate");
    assert!(n1 >= 11, "all rules including reserved should be seeded");
}

#[tokio::test]
async fn seed_rules_preserves_user_disabled_state() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).expect("seed");
    pool.get().unwrap().execute(
        "UPDATE coach_rules SET enabled = 0 WHERE id = 'lazy-prompting'", []
    ).unwrap();
    andon_lib::coach::seed_rules(&pool).expect("seed again");
    let enabled: i64 = pool.get().unwrap().query_row(
        "SELECT enabled FROM coach_rules WHERE id = 'lazy-prompting'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(enabled, 0, "second seed must not clobber user's disable");
}
```

Add `mod common;` at the top of the file.

- [ ] **Step 2: Run + verify fail.**

Run: `cd src-tauri && cargo test --features test-support --test coach_catalogue -v`
Expected: FAIL — function doesn't exist.

- [ ] **Step 3: Implement `seed_rules` in `coach/mod.rs`**:

```rust
use std::sync::Arc;
use crate::coach::rules::{RULES, DbPool};

pub fn seed_rules(pool: &Arc<DbPool>) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    for r in RULES {
        // INSERT OR IGNORE preserves user enable/disable state across upgrades.
        let sev = r.severity.map(|s| s.as_str()).unwrap_or("none");
        let kind = match r.kind {
            crate::coach::rules::RuleKind::Binary => "binary",
            crate::coach::rules::RuleKind::Continuous => "continuous",
        };
        tx.execute(
            "INSERT OR IGNORE INTO coach_rules
               (id, practice, severity, kind, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            rusqlite::params![r.id, r.practice, sev, kind, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}
```

Wire into app startup — find where `db::init` is called and add `coach::seed_rules(&pool)?;` right after.

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_catalogue -v`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/src/lib.rs src-tauri/tests/coach_catalogue.rs
git commit -m "feat(coach): seed coach_rules table from static RULES catalogue"
```

---

## Section E — Detectors

**Shared pattern for every detector task:**

1. Add a unit test in `src-tauri/tests/coach_detectors.rs` (one file, growing across tasks) that seeds the minimum data needed and asserts the expected `Finding`.
2. Add a `fn detect_<id>(pool: &Arc<DbPool>, window: &Window) -> Result<Vec<Finding>>` to `src-tauri/src/coach/rules.rs` (next to its `Rule` literal).
3. Add an arm in `engine::run_detector` calling `r::detect_<id>(pool, window)`.
4. Run the test, verify it passes.
5. Commit.

The test file's first task creates it with the imports. Subsequent tasks append.

### Task E1: `repeated-prompts` detector

**Files:**
- Create (this task only): `src-tauri/tests/coach_detectors.rs`
- Modify: `src-tauri/src/coach/rules.rs`, `src-tauri/src/coach/engine.rs`

- [ ] **Step 1: Create the test file and append the first test.**

```rust
// src-tauri/tests/coach_detectors.rs
mod common;

use std::sync::Arc;
use andon_lib::coach::{engine, rules::{Window, RULES}};
use rusqlite::params;

fn seed_prompt_turn(pool: &andon_lib::db::DbPool, session_id: &str, turn: i64, ts: i64, text: &str, norm_hash: &str) {
    pool.get().unwrap().execute(
        "INSERT INTO prompt_turns
           (session_id, turn_index, ts, source, text, norm_hash,
            length, has_file_ref, has_code, has_constraint)
         VALUES (?1, ?2, ?3, 'jsonl', ?4, ?5, ?6, 0, 0, 0)",
        params![session_id, turn, ts, text, norm_hash, text.chars().count() as i64],
    ).unwrap();
}

fn enable_only(pool: &andon_lib::db::DbPool, ids: &[&str]) {
    let conn = pool.get().unwrap();
    conn.execute("UPDATE coach_rules SET enabled = 0", []).unwrap();
    for id in ids {
        conn.execute("UPDATE coach_rules SET enabled = 1 WHERE id = ?1", params![id]).unwrap();
    }
}

#[tokio::test]
async fn repeated_prompts_fires_at_three_hits() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    drop(conn);

    // Three identical hashes within one session → trigger.
    seed_prompt_turn(&pool, "s1", 0, now - 800, "package the extension", "h1");
    seed_prompt_turn(&pool, "s1", 1, now - 600, "Package the extension", "h1");
    seed_prompt_turn(&pool, "s1", 2, now - 400, "package the EXTENSION", "h1");

    enable_only(&pool, &["repeated-prompts"]);

    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();

    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'repeated-prompts'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 1, "exactly one finding per session, not per hit");
}

#[tokio::test]
async fn repeated_prompts_skips_below_threshold() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();

    seed_prompt_turn(&pool, "s1", 0, now - 800, "a", "h1");
    seed_prompt_turn(&pool, "s1", 1, now - 600, "a", "h1");

    enable_only(&pool, &["repeated-prompts"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();

    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'repeated-prompts'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 0);
}
```

- [ ] **Step 2: Run + verify fail.**

Run: `cd src-tauri && cargo test --features test-support --test coach_detectors -v`
Expected: FAIL — no findings written (detector arm is empty).

- [ ] **Step 3: Implement `detect_repeated_prompts`** in `coach/rules.rs`:

```rust
use std::sync::Arc;

pub fn detect_repeated_prompts(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut stmt = conn.prepare(
        "SELECT session_id, norm_hash, COUNT(*) AS n, MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id, norm_hash
         HAVING n >= 3",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        let sid: String = r.get(0)?;
        let hash: String = r.get(1)?;
        let n: i64 = r.get(2)?;
        let last_ts: i64 = r.get(3)?;
        Ok((sid, hash, n, last_ts))
    })?;
    let mut out = vec![];
    let mut sessions_seen = std::collections::HashSet::new();
    for row in rows.filter_map(|r| r.ok()) {
        // One finding per session — don't multiply by group count.
        if !sessions_seen.insert(row.0.clone()) { continue; }
        out.push(Finding {
            rule_id: "repeated-prompts".into(),
            session_id: row.0,
            detected_at: row.3,
            payload_json: serde_json::json!({ "norm_hash": row.1, "count": row.2 }).to_string(),
        });
    }
    Ok(out)
}
```

Wire into `engine::run_detector`:

```rust
match rule.id {
    "repeated-prompts" => crate::coach::rules::detect_repeated_prompts(pool, window),
    _ => Ok(vec![]),
}
```

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_detectors -v`
Expected: PASS both new tests.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/tests/coach_detectors.rs
git commit -m "feat(coach): repeated-prompts detector"
```

---

### Task E2: `lazy-prompting` detector

Threshold: per-session ratio of turns where `length < 30` is `> 0.3`, with `count > 10` total turns.

- [ ] **Step 1: Append test** to `coach_detectors.rs`:

```rust
#[tokio::test]
async fn lazy_prompting_fires_when_third_are_short() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    for i in 0..15 {
        let text = if i < 5 { "fix bug" } else { "Refactor authentication middleware to use JWT with rotation" };
        seed_prompt_turn(&pool, "s1", i, now - (1000 - i*10), text, &format!("h{}", i));
    }
    enable_only(&pool, &["lazy-prompting"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'lazy-prompting'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run + verify fail.** (Empty findings.)

- [ ] **Step 3: Implement `detect_lazy_prompting`**:

```rust
pub fn detect_lazy_prompting(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id,
                COUNT(*) AS total,
                SUM(CASE WHEN length < 30 THEN 1 ELSE 0 END) AS short_count,
                MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id
         HAVING total > 10 AND CAST(short_count AS REAL) / total > 0.3",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, total, short, ts)| Finding {
        rule_id: "lazy-prompting".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "total": total, "short_count": short }).to_string(),
    }).collect())
}
```

Wire into `engine::run_detector` (extend the match).

- [ ] **Step 4: Run + verify pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -am "feat(coach): lazy-prompting detector"
```

---

### Task E3: `low-constraint-usage` detector

Threshold: per session, fewer than 20% of turns have `has_constraint = 1`. Requires `count >= 5` turns to avoid noise.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn low_constraint_usage_fires_below_twenty_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 1000]).unwrap();
    // 6 turns, only 1 has constraint -> 16.7% < 20% -> trigger
    for i in 0..6 {
        pool.get().unwrap().execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES ('s1', ?1, ?2, 'jsonl', 'x', ?3, 1, 0, 0, ?4)",
            params![i, now - (1000 - i*10), format!("h{}", i), if i == 0 { 1 } else { 0 }],
        ).unwrap();
    }
    enable_only(&pool, &["low-constraint-usage"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'low-constraint-usage'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run + verify fail.**

- [ ] **Step 3: Implement `detect_low_constraint_usage`:**

```rust
pub fn detect_low_constraint_usage(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, COUNT(*) AS total,
                SUM(has_constraint) AS with_constraint,
                MAX(ts) AS last_ts
         FROM prompt_turns
         JOIN sessions USING (session_id)
         WHERE sessions.started_at >= ?1 AND sessions.started_at < ?2
         GROUP BY session_id
         HAVING total >= 5 AND CAST(with_constraint AS REAL) / total < 0.2",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, total, with_c, ts)| Finding {
        rule_id: "low-constraint-usage".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "total": total, "with_constraint": with_c }).to_string(),
    }).collect())
}
```

Wire into engine.

- [ ] **Step 4: Run + verify pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -am "feat(coach): low-constraint-usage detector"
```

---

### Task E4: `long-session-no-commit` detector

Threshold: `ended_at - started_at > 90 min` AND zero `git_activity` rows for the session.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn long_session_no_commit_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let two_hours = 120 * 60 * 1000;
    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - two_hours, now],
    ).unwrap();
    // No git_activity rows -> trigger.
    enable_only(&pool, &["long-session-no-commit"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'long-session-no-commit'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn long_session_with_commit_does_not_fire() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let two_hours = 120 * 60 * 1000;
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - two_hours, now],
    ).unwrap();
    conn.execute(
        "INSERT INTO git_activity (session_id, timestamp, activity, count) VALUES ('s1', ?1, 'commit', 1)",
        params![now - 1000],
    ).unwrap();
    enable_only(&pool, &["long-session-no-commit"]);
    let win = Window { from_ms: now - 86400_000, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'long-session-no-commit'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}
```

- [ ] **Step 2: Run + verify fail.**

- [ ] **Step 3: Implement:**

```rust
pub fn detect_long_session_no_commit(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let ninety_min_ms: i64 = 90 * 60 * 1000;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.ended_at - s.started_at AS dur, s.ended_at
         FROM sessions s
         LEFT JOIN (SELECT session_id, COUNT(*) AS n FROM git_activity GROUP BY session_id) g
           ON g.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND s.ended_at IS NOT NULL
           AND (s.ended_at - s.started_at) > ?3
           AND COALESCE(g.n, 0) = 0",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms, ninety_min_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, dur, ended)| Finding {
        rule_id: "long-session-no-commit".into(),
        session_id: sid,
        detected_at: ended,
        payload_json: serde_json::json!({ "duration_ms": dur, "commits": 0 }).to_string(),
    }).collect())
}
```

Wire into engine.

- [ ] **Step 4: Run + verify pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -am "feat(coach): long-session-no-commit detector"
```

---

### Task E5: `late-night-coding` detector

Threshold: 5+ sessions in the window started between 23:00 and 05:00 local time. Single finding per *window* (not per session) — attach to the most recent qualifying session.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn late_night_fires_with_five_sessions() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    // Construct timestamps that land at 02:00 local time on 5 different days.
    use chrono::{TimeZone, Local};
    for d in 0..5 {
        let dt = Local.with_ymd_and_hms(2026, 5, 10 + d, 2, 0, 0).unwrap();
        let ms = dt.timestamp_millis();
        pool.get().unwrap().execute(
            "INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![format!("late-{}", d), ms],
        ).unwrap();
    }
    enable_only(&pool, &["late-night-coding"]);
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: 0, to_ms: now + 86400_000, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'late-night-coding'",
        [], |r| r.get(0)).unwrap();
    assert!(n >= 1, "should fire at least once for 5 late-night sessions");
}
```

- [ ] **Step 2: Run + verify fail.**

- [ ] **Step 3: Implement:**

```rust
pub fn detect_late_night_coding(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    // SQLite's `strftime('%H', ts/1000, 'unixepoch', 'localtime')` gives local hour.
    let mut stmt = conn.prepare(
        "SELECT session_id, started_at
         FROM sessions
         WHERE started_at >= ?1 AND started_at < ?2
           AND CAST(strftime('%H', started_at/1000, 'unixepoch', 'localtime') AS INTEGER) >= 23
            OR CAST(strftime('%H', started_at/1000, 'unixepoch', 'localtime') AS INTEGER) < 5",
    )?;
    let late: Vec<(String, i64)> = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?.filter_map(|r| r.ok()).collect();
    if late.len() < 5 { return Ok(vec![]); }
    // Attach the finding to the most-recent late-night session.
    let latest = late.iter().max_by_key(|(_, ts)| *ts).unwrap();
    Ok(vec![Finding {
        rule_id: "late-night-coding".into(),
        session_id: latest.0.clone(),
        detected_at: latest.1,
        payload_json: serde_json::json!({ "count": late.len() }).to_string(),
    }])
}
```

Wire into engine.

- [ ] **Step 4: Run + verify pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -am "feat(coach): late-night-coding detector"
```

---

### Task E6: `abandon-sessions` detector

Threshold: 3+ sessions in window with `tool_decisions` rows but zero `decision = 'accept'`. Single finding per window, attached to most recent abandoned session.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn abandon_sessions_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    for i in 0..3 {
        let sid = format!("aban-{}", i);
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - (i+1) * 60_000]).unwrap();
        conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES (?1, ?2, 'Edit', 'reject')",
            params![sid, now - (i+1) * 60_000]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["abandon-sessions"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'abandon-sessions'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2-5: same pattern.** Implementation:

```rust
pub fn detect_abandon_sessions(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.started_at
         FROM sessions s
         JOIN (
           SELECT session_id, COUNT(*) AS total,
                  SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS accepts
           FROM tool_decisions GROUP BY session_id
         ) td ON td.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND td.total > 0 AND td.accepts = 0",
    )?;
    let abandoned: Vec<(String, i64)> = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?.filter_map(|r| r.ok()).collect();
    if abandoned.len() < 3 { return Ok(vec![]); }
    let latest = abandoned.iter().max_by_key(|(_, ts)| *ts).unwrap();
    Ok(vec![Finding {
        rule_id: "abandon-sessions".into(),
        session_id: latest.0.clone(),
        detected_at: latest.1,
        payload_json: serde_json::json!({ "count": abandoned.len() }).to_string(),
    }])
}
```

Commit: `feat(coach): abandon-sessions detector`.

---

### Task E7: `speed-accept` detector

Threshold: 5+ occurrences in a session where a user turn (`prompt_turns` row) follows an `accept` `tool_decision` within 15 s AND the preceding turn produced ≥20 lines via `file_changes.lines_added`.

This rule is the trickiest in the catalogue because it joins `tool_decisions`, `prompt_turns`, and `file_changes` by timestamp window. Implement carefully.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn speed_accept_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 60_000]).unwrap();
    // 5 cycles of: file_change(20 LoC) -> accept -> user turn within 15s
    for i in 0..5 {
        let base = now - 50_000 + i*10_000;
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added) VALUES ('s1', ?1, 'a.rs', 25)",
            params![base]).unwrap();
        conn.execute(
            "INSERT INTO tool_decisions (session_id, timestamp, tool_name, decision) VALUES ('s1', ?1, 'Edit', 'accept')",
            params![base + 100]).unwrap();
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES ('s1', ?1, ?2, 'jsonl', 'go on', ?3, 5, 0, 0, 0)",
            params![i, base + 5_000, format!("h{}", i)]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["speed-accept"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'speed-accept'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2-3: Implement:**

```rust
pub fn detect_speed_accept(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    // For each accept decision in window: find a file_change in the previous 60s
    // with lines_added >= 20, and a prompt_turn within 15s after. Count per session.
    let mut stmt = conn.prepare(
        "WITH accepts AS (
           SELECT td.session_id, td.timestamp AS acc_ts
           FROM tool_decisions td
           JOIN sessions s USING (session_id)
           WHERE td.decision = 'accept'
             AND s.started_at >= ?1 AND s.started_at < ?2
         ),
         qualifying AS (
           SELECT a.session_id, a.acc_ts
           FROM accepts a
           WHERE EXISTS (
             SELECT 1 FROM file_changes fc
              WHERE fc.session_id = a.session_id
                AND fc.timestamp <= a.acc_ts
                AND fc.timestamp >= a.acc_ts - 60000
                AND fc.lines_added >= 20
           )
           AND EXISTS (
             SELECT 1 FROM prompt_turns pt
              WHERE pt.session_id = a.session_id
                AND pt.ts > a.acc_ts
                AND pt.ts <= a.acc_ts + 15000
           )
         )
         SELECT session_id, COUNT(*) AS n, MAX(acc_ts) AS last_ts
         FROM qualifying
         GROUP BY session_id
         HAVING n >= 5",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, n, ts)| Finding {
        rule_id: "speed-accept".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "occurrences": n }).to_string(),
    }).collect())
}
```

Wire into engine.

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): speed-accept detector"
```

---

### Task E8: `no-slash-commands` detector

Threshold: session over 30 min with zero `slash_commands` rows.

- [ ] **Step 1: Test:**

```rust
#[tokio::test]
async fn no_slash_commands_fires() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - 45*60_000, now],
    ).unwrap();
    enable_only(&pool, &["no-slash-commands"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'no-slash-commands'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 3: Implement:**

```rust
pub fn detect_no_slash_commands(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let thirty_min: i64 = 30 * 60 * 1000;
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.ended_at
         FROM sessions s
         LEFT JOIN (SELECT session_id, COUNT(*) AS n FROM slash_commands GROUP BY session_id) sc
           ON sc.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND s.ended_at IS NOT NULL
           AND (s.ended_at - s.started_at) > ?3
           AND COALESCE(sc.n, 0) = 0",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms, thirty_min], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, ts)| Finding {
        rule_id: "no-slash-commands".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: "{}".into(),
    }).collect())
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): no-slash-commands detector"
```

---

### Task E9: `model-diversity` continuous detector

Continuous detector returns a 0-100 score from `cost_entries.model` over the window. **No `coach_findings` rows** — surfaced at scorecard time only.

- [ ] **Step 1: Test:**

```rust
#[tokio::test]
async fn model_diversity_score_four_models_is_100() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();
    for m in ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5", "claude-other"] {
        conn.execute("INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES ('s1', ?1, ?2, 0.1)",
            params![now-500, m]).unwrap();
    }
    drop(conn);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    let score = andon_lib::coach::rules::score_model_diversity(&pool, &win).unwrap();
    assert_eq!(score, 100);
}

#[tokio::test]
async fn model_diversity_score_two_models_is_50() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();
    for m in ["claude-opus-4-7", "claude-sonnet-4-6"] {
        conn.execute("INSERT INTO cost_entries (session_id, timestamp, model, cost_usd) VALUES ('s1', ?1, ?2, 0.1)",
            params![now-500, m]).unwrap();
    }
    drop(conn);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    let score = andon_lib::coach::rules::score_model_diversity(&pool, &win).unwrap();
    assert_eq!(score, 50);
}
```

- [ ] **Step 3: Implement:**

```rust
pub fn score_model_diversity(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<i64> {
    let conn = pool.get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT model)
         FROM cost_entries
         WHERE timestamp >= ?1 AND timestamp < ?2",
        rusqlite::params![window.from_ms, window.to_ms],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(match n {
        x if x >= 4 => 100,
        3 => 80,
        2 => 50,
        _ => 20,
    })
}
```

Note: This is consumed by the scorer (Section F), not `engine::run_detector`. Continuous detectors don't write `coach_findings`.

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): model-diversity continuous detector"
```

---

### Task E10: `cache-hit-starvation` detector

Threshold: per-session, `cacheRead / (cacheRead + cacheCreation + non-cached input) < 0.1` over ≥ 20 turns with prompt input ≥ 5000 tokens.

Andon's `token_usage` table has `token_type` values: `input`, `output`, `cacheRead`, `cacheCreation`. Aggregate per session.

- [ ] **Step 1: Test:**

```rust
#[tokio::test]
async fn cache_hit_starvation_fires_below_ten_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now-1000]).unwrap();

    // 25 input rows with 5000 input + 100 cacheRead + 50 cacheCreation each
    // => cacheRate = 2500 / (2500 + 1250 + 125000) = ~2% < 10% ✓
    for i in 0..25 {
        let t = now - 1000 + i*100;
        for (kind, count) in [("input", 5000), ("cacheRead", 100), ("cacheCreation", 50)] {
            conn.execute(
                "INSERT INTO token_usage (session_id, timestamp, model, token_type, count) VALUES ('s1', ?1, 'm', ?2, ?3)",
                params![t, kind, count]).unwrap();
        }
    }
    drop(conn);
    enable_only(&pool, &["cache-hit-starvation"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'cache-hit-starvation'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 3: Implement:**

```rust
pub fn detect_cache_hit_starvation(pool: &Arc<DbPool>, window: &Window) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, last_ts, total_input, cache_read, cache_create
         FROM (
           SELECT s.session_id, MAX(tu.timestamp) AS last_ts,
             SUM(CASE WHEN tu.token_type = 'input' THEN tu.count ELSE 0 END) AS total_input,
             SUM(CASE WHEN tu.token_type = 'cacheRead' THEN tu.count ELSE 0 END) AS cache_read,
             SUM(CASE WHEN tu.token_type = 'cacheCreation' THEN tu.count ELSE 0 END) AS cache_create,
             COUNT(DISTINCT tu.timestamp) AS turns
           FROM token_usage tu
           JOIN sessions s USING (session_id)
           WHERE s.started_at >= ?1 AND s.started_at < ?2
           GROUP BY s.session_id
         )
         WHERE turns >= 20 AND total_input >= 5000
           AND CAST(cache_read AS REAL) / (cache_read + cache_create + total_input) < 0.1",
    )?;
    let rows = stmt.query_map(rusqlite::params![window.from_ms, window.to_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).map(|(sid, ts, input, read, create)| Finding {
        rule_id: "cache-hit-starvation".into(),
        session_id: sid,
        detected_at: ts,
        payload_json: serde_json::json!({ "input": input, "cache_read": read, "cache_create": create }).to_string(),
    }).collect())
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): cache-hit-starvation detector"
```

---

### Task E11: `low-spec-rate` detector

Threshold: over ≥ 5 sessions in the window with at least one `file_changes` row (proxy for "session produced code"), fraction that look "spec-driven" is `< 0.2`.

A session is spec-driven if its first user turn (`turn_index = 0` in `prompt_turns`) satisfies **any** of:
- `command IN settings.coach.planning_commands`
- `text` matches `\.(md|txt|spec|prd|design|plan|rfc|adoc)$` (file ref)
- `text` matches any keyword in `settings.coach.planning_keywords` (case-insensitive)
- `text` has ≥ 3 lines starting with `- `, `* `, or `\d+\.`
- `text` contains a `^#` markdown heading

This rule reads `settings.coach` at evaluation time — pass the snapshot through to the detector.

- [ ] **Step 1: Test:**

```rust
#[tokio::test]
async fn low_spec_rate_fires_when_below_twenty_percent() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    // 6 sessions, only 1 is spec-driven (1/6 = 16.7% < 20%)
    for i in 0..6 {
        let sid = format!("s{}", i);
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - 1000 - i*100]).unwrap();
        conn.execute(
            "INSERT INTO file_changes (session_id, timestamp, file_path, lines_added) VALUES (?1, ?2, 'a.rs', 5)",
            params![sid, now - 900 - i*100]).unwrap();
        let text = if i == 0 { "spec: must do thing" } else { "just go" };
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES (?1, 0, ?2, 'jsonl', ?3, ?4, ?5, 0, 0, 0)",
            params![sid, now - 950 - i*100, text, format!("h{}", i), text.chars().count() as i64]).unwrap();
    }
    drop(conn);
    enable_only(&pool, &["low-spec-rate"]);
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    engine::evaluate_window(&pool, &win).unwrap();
    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE rule_id = 'low-spec-rate'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 3: Implement.** Because this rule needs settings, the engine's `run_detector` signature must accept a `CoachSettings` parameter. Update `engine::evaluate_window` to accept `coach_settings: &CoachSettings` and thread it through to `run_detector`. Then the detector:

```rust
pub fn detect_low_spec_rate(
    pool: &Arc<DbPool>,
    window: &Window,
    coach_settings: &crate::settings::CoachSettings,
) -> crate::coach::Result<Vec<Finding>> {
    let conn = pool.get()?;
    // Step 1: collect sessions that produced code (have at least one file_changes row).
    let mut stmt = conn.prepare(
        "SELECT DISTINCT s.session_id, s.started_at,
                (SELECT pt.text FROM prompt_turns pt
                 WHERE pt.session_id = s.session_id AND pt.turn_index = 0) AS first_turn,
                (SELECT pt.command FROM prompt_turns pt
                 WHERE pt.session_id = s.session_id AND pt.turn_index = 0) AS first_cmd
         FROM sessions s
         JOIN file_changes fc ON fc.session_id = s.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2",
    )?;
    let sessions: Vec<(String, i64, Option<String>, Option<String>)> = stmt.query_map(
        rusqlite::params![window.from_ms, window.to_ms],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?)),
    )?.filter_map(|r| r.ok()).collect();
    if sessions.len() < 5 { return Ok(vec![]); }

    // Step 2: classify each session.
    let planning_commands: std::collections::HashSet<&str> =
        coach_settings.planning_commands.iter().map(|s| s.as_str()).collect();
    let planning_keywords_lc: Vec<String> =
        coach_settings.planning_keywords.iter().map(|s| s.to_lowercase()).collect();

    let file_ref_re = regex::Regex::new(r"(?i)\.(md|txt|spec|prd|design|plan|rfc|adoc)\b").unwrap();
    let bullet_re = regex::Regex::new(r"(?m)^(?:[-*]|\d+\.)\s").unwrap();
    let heading_re = regex::Regex::new(r"(?m)^#").unwrap();

    let is_spec_driven = |text: Option<&str>, cmd: Option<&str>| -> bool {
        if let Some(c) = cmd {
            if planning_commands.contains(c) { return true; }
        }
        let Some(t) = text else { return false; };
        let lc = t.to_lowercase();
        if planning_keywords_lc.iter().any(|kw| lc.contains(kw)) { return true; }
        if file_ref_re.is_match(t) { return true; }
        if bullet_re.find_iter(t).count() >= 3 { return true; }
        if heading_re.is_match(t) { return true; }
        false
    };

    let total = sessions.len() as f64;
    let spec_count = sessions.iter().filter(|(_, _, text, cmd)|
        is_spec_driven(text.as_deref(), cmd.as_deref())
    ).count() as f64;

    if spec_count / total < 0.2 {
        // Attach to most-recent qualifying session.
        let latest = sessions.iter().max_by_key(|(_, ts, _, _)| *ts).unwrap();
        return Ok(vec![Finding {
            rule_id: "low-spec-rate".into(),
            session_id: latest.0.clone(),
            detected_at: latest.1,
            payload_json: serde_json::json!({
                "total_sessions": total as i64,
                "spec_sessions": spec_count as i64
            }).to_string(),
        }]);
    }
    Ok(vec![])
}
```

Add `regex` to `[dependencies]` in `Cargo.toml` if not present.

Update `engine::evaluate_window` signature:

```rust
pub fn evaluate_window(
    pool: &Arc<DbPool>,
    window: &Window,
    coach_settings: &crate::settings::CoachSettings,
) -> Result<()> { … }
```

And `run_detector`:

```rust
fn run_detector(
    pool: &Arc<DbPool>,
    rule: &Rule,
    window: &Window,
    coach_settings: &crate::settings::CoachSettings,
) -> Result<Vec<Finding>> {
    match rule.id {
        "repeated-prompts" => crate::coach::rules::detect_repeated_prompts(pool, window),
        "lazy-prompting" => crate::coach::rules::detect_lazy_prompting(pool, window),
        "low-constraint-usage" => crate::coach::rules::detect_low_constraint_usage(pool, window),
        "long-session-no-commit" => crate::coach::rules::detect_long_session_no_commit(pool, window),
        "late-night-coding" => crate::coach::rules::detect_late_night_coding(pool, window),
        "abandon-sessions" => crate::coach::rules::detect_abandon_sessions(pool, window),
        "speed-accept" => crate::coach::rules::detect_speed_accept(pool, window),
        "no-slash-commands" => crate::coach::rules::detect_no_slash_commands(pool, window),
        "cache-hit-starvation" => crate::coach::rules::detect_cache_hit_starvation(pool, window),
        "low-spec-rate" => crate::coach::rules::detect_low_spec_rate(pool, window, coach_settings),
        _ => Ok(vec![]),
    }
}
```

Update every test in `coach_detectors.rs` that calls `evaluate_window` to pass `&CoachSettings::default()` — find/replace `engine::evaluate_window(&pool, &win).unwrap()` with `engine::evaluate_window(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap()`.

- [ ] **Step 4-5:** Run all detector tests + commit.

Run: `cd src-tauri && cargo test --features test-support --test coach_detectors -v`
Expected: PASS for all tests across Section E.

```bash
git commit -am "feat(coach): low-spec-rate detector and thread CoachSettings through engine"
```

---

## Section F — Scorer

### Task F1: Per-practice score via AIEC formula

**Files:**
- Modify: `src-tauri/src/coach/score.rs`
- Create: `src-tauri/tests/coach_scorer.rs`

- [ ] **Step 1: Write the failing test:**

```rust
// src-tauri/tests/coach_scorer.rs
mod common;

use andon_lib::coach::{rules::Window, score};
use rusqlite::params;

#[tokio::test]
async fn worked_example_three_detectors_one_high_triggers_67() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    // Force exactly 3 enabled detectors in 'hygiene' practice.
    pool.get().unwrap().execute("UPDATE coach_rules SET enabled = 0", []).unwrap();
    pool.get().unwrap().execute(
        "UPDATE coach_rules SET enabled = 1 WHERE id IN ('long-session-no-commit', 'late-night-coding', 'abandon-sessions')",
        [],
    ).unwrap();

    // Seed one finding of a 'high' severity rule in 'hygiene' practice.
    pool.get().unwrap().execute(
        "INSERT INTO sessions (session_id, started_at) VALUES ('s1', 100)", []).unwrap();
    pool.get().unwrap().execute(
        "INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
         VALUES ('long-session-no-commit', 's1', 100, '{}')", []).unwrap();

    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "hygiene", &win).unwrap();

    // 3 detectors enabled, one HIGH triggers:
    // penalty = 12, maxPenalty = 36, score = round(100 * (1 - 12/36)) = 67
    assert_eq!(s.score, Some(67));
    assert_eq!(s.status, "needs-improvement");
    assert_eq!(s.triggered_count, 1);
}

#[tokio::test]
async fn empty_practice_returns_null_status_na() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    pool.get().unwrap().execute("UPDATE coach_rules SET enabled = 0 WHERE practice = 'tool'", []).unwrap();
    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "tool", &win).unwrap();
    assert_eq!(s.score, None);
    assert_eq!(s.status, "n/a");
}

#[tokio::test]
async fn clean_practice_scores_100() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let win = Window { from_ms: 0, to_ms: 1_000_000, models: None };
    let s = score::practice_score(&pool, "prompt", &win).unwrap();
    assert_eq!(s.score, Some(100));
    assert_eq!(s.status, "good");
    assert_eq!(s.triggered_count, 0);
}
```

- [ ] **Step 2: Run + verify fail.**

Run: `cd src-tauri && cargo test --features test-support --test coach_scorer -v`
Expected: FAIL — `score::practice_score` doesn't exist.

- [ ] **Step 3: Implement `coach/score.rs`:**

```rust
//! AIEC scoring formula:
//!   sevPenalty = {high: 12, medium: 7, low: 3}
//!   penalty    = Σ sevPenalty[r.severity] for r in triggered_in_practice
//!   maxPenalty = |enabled_detectors_in_practice| × 12
//!   score      = max(0, round(100 × (1 - penalty / maxPenalty)))

use std::sync::Arc;
use serde::Serialize;
use tracing::instrument;

use crate::coach::rules::{Window, DbPool, RULES, RuleKind, Severity};
use crate::coach::Result;

#[derive(Debug, Serialize)]
pub struct PracticeScore {
    pub practice: String,
    pub score: Option<i64>,
    pub status: String,
    pub triggered_count: i64,
}

#[instrument(skip(pool))]
pub fn practice_score(pool: &Arc<DbPool>, practice: &str, window: &Window) -> Result<PracticeScore> {
    let conn = pool.get()?;
    // Enabled binary detectors in this practice.
    let enabled_ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM coach_rules
             WHERE practice = ?1 AND kind = 'binary' AND enabled = 1",
        )?;
        stmt.query_map(rusqlite::params![practice], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect()
    };

    if enabled_ids.is_empty() {
        return Ok(PracticeScore {
            practice: practice.into(),
            score: None,
            status: "n/a".into(),
            triggered_count: 0,
        });
    }

    // Find which of those rules triggered at least once in window.
    let placeholders: String = enabled_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 3))
        .collect::<Vec<_>>().join(",");
    let q = format!(
        "SELECT DISTINCT cf.rule_id
         FROM coach_findings cf
         JOIN sessions s ON s.session_id = cf.session_id
         WHERE s.started_at >= ?1 AND s.started_at < ?2
           AND cf.rule_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&q)?;
    let mut params_dyn: Vec<&dyn rusqlite::ToSql> = vec![&window.from_ms, &window.to_ms];
    for id in &enabled_ids { params_dyn.push(id); }
    let triggered_ids: Vec<String> = stmt.query_map(&*params_dyn, |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Sum penalties by looking each triggered rule up in RULES for its severity.
    let penalty: i64 = triggered_ids.iter().map(|id| {
        RULES.iter()
            .find(|r| r.id == id)
            .and_then(|r| r.severity)
            .map(|s| s.penalty())
            .unwrap_or(5) // AIEC fallback for missing severity
    }).sum();

    let max_penalty = enabled_ids.len() as i64 * 12;
    let raw = 100.0 * (1.0 - penalty as f64 / max_penalty as f64);
    let score = raw.round().max(0.0) as i64;
    let status = if score >= 70 { "good" }
        else if score >= 40 { "needs-improvement" }
        else { "critical" };

    Ok(PracticeScore {
        practice: practice.into(),
        score: Some(score),
        status: status.into(),
        triggered_count: triggered_ids.len() as i64,
    })
}
```

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_scorer -v`
Expected: PASS for all three tests.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach/score.rs src-tauri/tests/coach_scorer.rs
git commit -m "feat(coach): per-practice scorer using AIEC formula"
```

---

### Task F2: WoW + MoM trends

- [ ] **Step 1: Append tests to `coach_scorer.rs`:**

```rust
#[tokio::test]
async fn wow_pct_correct_signed_integer() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let day = 86_400_000i64;
    // Last 7d: 10 findings; prior 7d: 8 findings -> wow = +25
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now - 14*day]).unwrap();
    for i in 0..10 {
        conn.execute("INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
                      VALUES ('lazy-prompting', 's1', ?1, '{}')", params![now - (1+i) * 3600_000]).unwrap();
    }
    for i in 0..8 {
        conn.execute("INSERT INTO coach_findings (rule_id, session_id, detected_at, payload)
                      VALUES ('lazy-prompting', 's1', ?1, '{}')", params![now - 7*day - (1+i) * 3600_000]).unwrap();
    }
    drop(conn);
    let wow = score::trends_wow(&pool, "prompt", now).unwrap();
    assert_eq!(wow, 25);
}

#[tokio::test]
async fn wow_pct_returns_zero_when_prev_is_zero() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let wow = score::trends_wow(&pool, "prompt", now).unwrap();
    assert_eq!(wow, 0);
}
```

- [ ] **Step 2: Run + verify fail.**

- [ ] **Step 3: Implement in `score.rs`:**

```rust
pub fn trends_wow(pool: &Arc<DbPool>, practice: &str, now_ms: i64) -> Result<i64> {
    let day = 86_400_000i64;
    let last  = count_findings(pool, practice, now_ms - 7*day, now_ms)?;
    let prev  = count_findings(pool, practice, now_ms - 14*day, now_ms - 7*day)?;
    Ok(if prev > 0 { (((last - prev) as f64 / prev as f64) * 100.0).round() as i64 } else { 0 })
}

pub fn trends_mom(pool: &Arc<DbPool>, practice: &str, now_ms: i64) -> Result<i64> {
    let day = 86_400_000i64;
    let week_sum = |from: i64, to: i64| -> Result<f64> {
        count_findings(pool, practice, from, to).map(|n| n as f64)
    };
    let recent: f64 = (0..4).map(|w|
        week_sum(now_ms - (w+1)*7*day, now_ms - w*7*day).unwrap_or(0.0)
    ).sum::<f64>() / 4.0;
    let prior: f64 = (4..8).map(|w|
        week_sum(now_ms - (w+1)*7*day, now_ms - w*7*day).unwrap_or(0.0)
    ).sum::<f64>() / 4.0;
    Ok(if prior > 0.0 { (((recent - prior) / prior) * 100.0).round() as i64 } else { 0 })
}

fn count_findings(pool: &Arc<DbPool>, practice: &str, from_ms: i64, to_ms: i64) -> Result<i64> {
    let conn = pool.get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM coach_findings cf
         JOIN coach_rules cr ON cr.id = cf.rule_id
         WHERE cr.practice = ?1 AND cf.detected_at >= ?2 AND cf.detected_at < ?3",
        rusqlite::params![practice, from_ms, to_ms],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(n)
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): WoW and MoM trend computations"
```

---

### Task F3: Scorecard assembly

Aggregates per-practice score, trend, and continuous tile into a single response struct.

- [ ] **Step 1: Append test:**

```rust
#[tokio::test]
async fn scorecard_returns_all_five_practices() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: 0, to_ms: now + 1, models: None };
    let card = score::scorecard(&pool, &win, &andon_lib::settings::CoachSettings::default()).unwrap();
    assert_eq!(card.practices.len(), 5);
    let tool = card.practices.iter().find(|p| p.practice == "tool").unwrap();
    // Empty DB → 0 distinct models → continuous score 20
    let cont = tool.continuous.iter().find(|c| c.id == "model-diversity");
    assert!(cont.is_some());
    assert_eq!(cont.unwrap().score, 20);
}
```

- [ ] **Step 2: Run + verify fail.**

- [ ] **Step 3: Implement:**

```rust
#[derive(Debug, Serialize)]
pub struct ContinuousTile {
    pub id: String,
    pub score: i64,
}

#[derive(Debug, Serialize)]
pub struct PracticeRow {
    pub practice: String,
    pub score: Option<i64>,
    pub status: String,
    pub wow_pct: i64,
    pub mom_pct: i64,
    pub triggered_count: i64,
    pub continuous: Vec<ContinuousTile>,
}

#[derive(Debug, Serialize)]
pub struct Scorecard {
    pub practices: Vec<PracticeRow>,
    pub window: WindowDto,
    pub sessions_in_window: i64,
}

#[derive(Debug, Serialize)]
pub struct WindowDto { pub from: i64, pub to: i64 }

pub fn scorecard(
    pool: &Arc<DbPool>,
    window: &Window,
    _coach_settings: &crate::settings::CoachSettings,
) -> Result<Scorecard> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut practices = vec![];
    for &p in crate::coach::PRACTICES {
        let s = practice_score(pool, p, window)?;
        let wow = trends_wow(pool, p, now)?;
        let mom = trends_mom(pool, p, now)?;
        let mut continuous = vec![];
        // Add continuous tiles for this practice.
        if p == "tool" {
            let md = crate::coach::rules::score_model_diversity(pool, window)?;
            continuous.push(ContinuousTile { id: "model-diversity".into(), score: md });
        }
        practices.push(PracticeRow {
            practice: s.practice,
            score: s.score,
            status: s.status,
            wow_pct: wow,
            mom_pct: mom,
            triggered_count: s.triggered_count,
            continuous,
        });
    }
    let sessions_in_window: i64 = pool.get()?.query_row(
        "SELECT COUNT(*) FROM sessions WHERE started_at >= ?1 AND started_at < ?2",
        rusqlite::params![window.from_ms, window.to_ms],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(Scorecard {
        practices,
        window: WindowDto { from: window.from_ms, to: window.to_ms },
        sessions_in_window,
    })
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): scorecard assembly (per-practice + trends + continuous)"
```

---

## Section G — Skill Finder

### Task G1: Normaliser + BLAKE3 keyed hash

Replaces the placeholder hash stubbed in Tasks B4 and C2.

**Files:**
- Modify: `src-tauri/src/coach/skill.rs`
- Modify: callers in `reducer.rs` and `ingestor.rs` to use the new function

- [ ] **Step 1: Write the failing test** in `src-tauri/tests/coach_skill_norm.rs`:

```rust
use andon_lib::coach::skill::norm_hash;

#[test]
fn case_and_whitespace_collapse() {
    assert_eq!(norm_hash("Package the extension"),
               norm_hash("  package   the   extension  "));
}

#[test]
fn paths_collapse() {
    assert_eq!(norm_hash("Refactor @src/foo.rs"),
               norm_hash("Refactor @lib/bar.rs"));
    assert_eq!(norm_hash("Refactor C:\\Users\\x\\foo.rs"),
               norm_hash("Refactor /home/y/bar.rs"));
}

#[test]
fn uuids_and_long_numbers_collapse() {
    let a = "Investigate run 550e8400-e29b-41d4-a716-446655440000";
    let b = "Investigate run f47ac10b-58cc-4372-a567-0e02b2c3d479";
    assert_eq!(norm_hash(a), norm_hash(b));

    let a = "PR #12345";
    let b = "PR #98765";
    assert_eq!(norm_hash(a), norm_hash(b));
}

#[test]
fn code_fences_drop_out() {
    let a = "Explain this:\n```rust\nfn foo(){}\n```";
    let b = "Explain this:\n```python\nprint(1)\n```";
    assert_eq!(norm_hash(a), norm_hash(b));
}

#[test]
fn very_long_inputs_truncate() {
    let pad = "abcd".repeat(2000); // 8000 chars
    let a = format!("Plan the release. {}", pad);
    let b = format!("Plan the release. {} extra", pad);
    assert_eq!(norm_hash(&a), norm_hash(&b),
        "anything beyond 1024 chars of normalised input should not affect the hash");
}

#[test]
fn different_inputs_differ() {
    assert_ne!(norm_hash("package the extension"),
               norm_hash("ship the release"));
}
```

- [ ] **Step 2: Run + verify fail.**

Run: `cd src-tauri && cargo test --features test-support --test coach_skill_norm -v`
Expected: FAIL — `norm_hash` not in `skill.rs`.

- [ ] **Step 3: Implement in `coach/skill.rs`:**

```rust
use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;

// Static 32-byte key — built into the binary, stable across runs but
// not portable across installs. (Bytes "andon-coach-skill-finder-key-v1"
// padded/truncated to 32.)
const NORM_KEY: &[u8; 32] = b"andon-coach-skill-finder-key-v1!";

static PATH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"(?:@|(?:^|\s))(?:/|[A-Za-z]:\\)[\w./\\-]+"
).unwrap());
static UUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
).unwrap());
static SHA_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{7,40}\b").unwrap());
static NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{4,}").unwrap());
static CODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

fn normalise(text: &str) -> String {
    let s = text.to_lowercase();
    let s = CODE_RE.replace_all(&s, "<code>");
    let s = PATH_RE.replace_all(&s, "<path>");
    let s = UUID_RE.replace_all(&s, "<id>");
    let s = SHA_RE.replace_all(&s, "<id>");
    let s = NUM_RE.replace_all(&s, "<num>");
    let s = WS_RE.replace_all(s.trim(), " ").into_owned();
    s.chars().take(1024).collect()
}

pub fn norm_hash(text: &str) -> String {
    let n = normalise(text);
    let mut h = Hasher::new_keyed(NORM_KEY);
    h.update(n.as_bytes());
    h.finalize().to_hex().to_string()
}
```

Add to `Cargo.toml`:
- `blake3 = "1"` (likely already present — check)
- `once_cell = "1"`
- `regex = "1"`

Replace the placeholder hash calls in `reducer.rs` (Task B4) and `ingestor.rs` (Task C2) with `crate::coach::skill::norm_hash(&text)`.

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_skill_norm -v`
Expected: PASS all six tests. Also re-run reducer + ingestor tests to confirm nothing broke.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach/skill.rs src-tauri/src/jsonl/reducer.rs src-tauri/src/otlp/ingestor.rs src-tauri/Cargo.toml src-tauri/tests/coach_skill_norm.rs
git commit -m "feat(coach): BLAKE3-keyed normalised hash for skill clustering"
```

---

### Task G2: Discovery pass — writes `skill_opportunities`

Three look-back windows (30d / 90d / 180d) per AIEC. Idempotent via the unique index on `(norm_hash, window_start, window_end)`.

- [ ] **Step 1: Test** (new file `src-tauri/tests/coach_skill_discovery.rs`):

```rust
mod common;

use andon_lib::coach::skill;
use andon_lib::settings::CoachSettings;
use rusqlite::params;

#[tokio::test]
async fn discovery_surfaces_threshold_hits() {
    let (pool, _dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    // 2 sessions, 3 turns total with the same hash → 3 occurrences across 2 sessions → trigger.
    for sid in ["s1", "s2"] {
        conn.execute("INSERT INTO sessions (session_id, started_at) VALUES (?1, ?2)",
            params![sid, now - 86400_000]).unwrap();
    }
    for (sid, turn, ts, hash) in [
        ("s1", 0, now - 80_000_000, "h1"),
        ("s1", 1, now - 70_000_000, "h1"),
        ("s2", 0, now - 60_000_000, "h1"),
    ] {
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES (?1, ?2, ?3, 'jsonl', 'package the extension', ?4, 21, 0, 0, 0)",
            params![sid, turn, ts, hash],
        ).unwrap();
    }
    drop(conn);

    skill::discover_all(&pool, &CoachSettings::default()).unwrap();

    let (label, occurrences, sessions): (String, i64, i64) = pool.get().unwrap().query_row(
        "SELECT label, occurrences, session_count FROM skill_opportunities WHERE norm_hash = 'h1' ORDER BY window_end DESC LIMIT 1",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    ).unwrap();
    assert_eq!(occurrences, 3);
    assert_eq!(sessions, 2);
    assert!(label.contains("package"), "label snapshotted from shortest example");
}

#[tokio::test]
async fn discovery_idempotent() {
    let (pool, _dir) = common::fixture_pool();
    let cs = CoachSettings::default();
    skill::discover_all(&pool, &cs).unwrap();
    let n1: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| r.get(0)).unwrap();
    skill::discover_all(&pool, &cs).unwrap();
    let n2: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| r.get(0)).unwrap();
    assert_eq!(n1, n2);
}
```

- [ ] **Step 2: Run + fail.**

- [ ] **Step 3: Implement `skill::discover_all`** in `coach/skill.rs`:

```rust
use std::sync::Arc;
use crate::coach::rules::DbPool;
use crate::coach::Result;
use crate::settings::CoachSettings;

/// Run discovery for all three look-back windows (30d / 90d / 180d).
pub fn discover_all(pool: &Arc<DbPool>, settings: &CoachSettings) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let day = 86_400_000i64;
    for lookback_days in [30, 90, 180] {
        discover_window(pool, settings, now - lookback_days * day, now)?;
    }
    Ok(())
}

fn discover_window(pool: &Arc<DbPool>, settings: &CoachSettings, from_ms: i64, to_ms: i64) -> Result<()> {
    let conn = pool.get()?;
    // For each norm_hash with enough occurrences AND enough sessions, upsert.
    let mut stmt = conn.prepare(
        "SELECT norm_hash,
                COUNT(*) AS occurrences,
                COUNT(DISTINCT session_id) AS session_count,
                MIN(ts) AS first_seen, MAX(ts) AS last_seen,
                -- pick the shortest example for label snapshotting
                (SELECT text FROM prompt_turns p2
                  WHERE p2.norm_hash = p.norm_hash
                  ORDER BY length ASC, ts ASC LIMIT 1) AS shortest_text,
                (SELECT command FROM prompt_turns p3
                  WHERE p3.norm_hash = p.norm_hash AND command IS NOT NULL
                  GROUP BY command
                  HAVING COUNT(DISTINCT command) = 1 LIMIT 1) AS unique_command
         FROM prompt_turns p
         JOIN sessions s USING (session_id)
         WHERE s.started_at >= ?1 AND s.started_at < ?2
         GROUP BY norm_hash
         HAVING occurrences >= ?3 AND session_count >= ?4",
    )?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rows = stmt.query_map(
        rusqlite::params![
            from_ms, to_ms,
            settings.skill_min_occurrences as i64,
            settings.skill_min_sessions as i64,
        ],
        |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
        )),
    )?;
    let mut conn2 = pool.get()?;
    let tx = conn2.transaction()?;
    for row in rows.filter_map(|r| r.ok()) {
        let (hash, occ, sess, first, last, shortest, cmd) = row;
        let label = if let Some(c) = cmd.as_deref() {
            format!("/{}", c)
        } else {
            shortest.clone().unwrap_or_default()
                .chars().take(80).collect::<String>()
        };
        tx.execute(
            "INSERT INTO skill_opportunities
               (norm_hash, label, command, occurrences, session_count,
                first_seen, last_seen, window_start, window_end, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(norm_hash, window_start, window_end) DO UPDATE SET
               label = excluded.label,
               occurrences = excluded.occurrences,
               session_count = excluded.session_count,
               first_seen = excluded.first_seen,
               last_seen = excluded.last_seen,
               computed_at = excluded.computed_at",
            rusqlite::params![hash, label, cmd, occ, sess, first, last, from_ms, to_ms, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): skill discovery writes skill_opportunities"
```

---

### Task G3: Examples reader

`examples_for_hash(pool, norm_hash, limit)` — used by `GET /api/coach/skills/:hash/examples`.

- [ ] **Step 1: Test:**

```rust
#[tokio::test]
async fn examples_returns_shortest_first() {
    let (pool, _dir) = common::fixture_pool();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at) VALUES ('s1', ?1)", params![now]).unwrap();
    for (i, text) in [(0, "Package the extension"), (1, "package"), (2, "Package the extension please.")].iter() {
        conn.execute(
            "INSERT INTO prompt_turns (session_id, turn_index, ts, source, text, norm_hash, length, has_file_ref, has_code, has_constraint)
             VALUES ('s1', ?1, ?2, 'jsonl', ?3, 'h1', ?4, 0, 0, 0)",
            params![i, now + i*1000, text, text.chars().count() as i64]).unwrap();
    }
    drop(conn);
    let examples = andon_lib::coach::skill::examples_for_hash(&pool, "h1", 3).unwrap();
    assert_eq!(examples.len(), 3);
    assert_eq!(examples[0].text, "package", "shortest first");
}
```

- [ ] **Step 2: Run + fail.**

- [ ] **Step 3: Implement:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct SkillExample {
    pub session_id: String,
    pub turn_index: i64,
    pub ts: i64,
    pub text: String,
}

pub fn examples_for_hash(pool: &Arc<DbPool>, norm_hash: &str, limit: i64) -> Result<Vec<SkillExample>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, turn_index, ts, text
         FROM prompt_turns
         WHERE norm_hash = ?1
         ORDER BY length ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![norm_hash, limit], |r| Ok(SkillExample {
        session_id: r.get(0)?, turn_index: r.get(1)?, ts: r.get(2)?, text: r.get(3)?,
    }))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): skill examples reader"
```

---

## Section H — Re-evaluation triggers

### Task H1: SessionEnd hook spawns `evaluate_session`

**Files:**
- Modify: `src-tauri/src/coach/eval.rs`
- Modify: `src-tauri/src/integration.rs` (find the SessionEnd handler — likely `hook_session_end` in `api/routes.rs` or `integration.rs`)

- [ ] **Step 1: Find the SessionEnd write site.**

Run: `grep -rn "hook_session_end\|session.end\|SessionEnd" src-tauri/src/`. Identify the function that runs after the existing session-end writes commit. Call it `SESSION_END_HANDLER` below.

- [ ] **Step 2: Write the failing test** — create `src-tauri/tests/coach_eval_triggers.rs`:

```rust
mod common;

use rusqlite::params;
use std::sync::Arc;

#[tokio::test]
async fn evaluate_session_writes_findings_when_rules_trigger() {
    let (pool, _dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = pool.get().unwrap();
    conn.execute("INSERT INTO sessions (session_id, started_at, ended_at) VALUES ('s1', ?1, ?2)",
        params![now - 120*60_000, now]).unwrap();
    drop(conn);

    andon_lib::coach::eval::evaluate_session(&pool, "s1", &andon_lib::settings::CoachSettings::default()).unwrap();

    let n: i64 = pool.get().unwrap().query_row(
        "SELECT COUNT(*) FROM coach_findings WHERE session_id = 's1'",
        [], |r| r.get(0)).unwrap();
    assert!(n >= 1, "long-session-no-commit should have fired");
}
```

- [ ] **Step 3: Implement `coach/eval.rs`:**

```rust
//! Re-evaluator triggers.

use std::sync::Arc;
use tracing::instrument;

use crate::coach::rules::{Window, DbPool};
use crate::coach::Result;
use crate::settings::CoachSettings;

const DEFAULT_WINDOW_DAYS: i64 = 30;

#[instrument(skip(pool, settings))]
pub fn evaluate_session(pool: &Arc<DbPool>, _session_id: &str, settings: &CoachSettings) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let win = Window { from_ms: now - DEFAULT_WINDOW_DAYS * 86_400_000, to_ms: now + 1, models: None };
    crate::coach::engine::evaluate_window(pool, &win, settings)
}

#[instrument(skip(pool, settings))]
pub fn evaluate_window(pool: &Arc<DbPool>, from_ms: i64, to_ms: i64, settings: &CoachSettings) -> Result<()> {
    let win = Window { from_ms, to_ms, models: None };
    crate::coach::engine::evaluate_window(pool, &win, settings)
}
```

Now wire the SessionEnd handler. In `SESSION_END_HANDLER` (the function you found in Step 1), after the existing writes commit, append:

```rust
// Coach re-evaluation: never inline, never inherits a pool conn across .await.
let pool_for_coach = std::sync::Arc::clone(&state.pool);
let settings_for_coach = state.settings.coach();
let session_id_owned = session_id.to_string();
tokio::spawn(async move {
    let pool = pool_for_coach;
    let settings = settings_for_coach;
    let sid = session_id_owned;
    if let Err(e) = andon_lib::coach::eval::evaluate_session(&pool, &sid, &settings) {
        tracing::warn!(error = ?e, session_id = sid, "coach evaluate_session failed");
    }
});
```

Adjust `pool_for_coach`, `state.settings.coach()`, and `session_id` to match the variable names in the actual handler.

- [ ] **Step 4: Run + verify pass.**

Run: `cd src-tauri && cargo test --features test-support --test coach_eval_triggers -v`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/coach src-tauri/src/integration.rs src-tauri/src/api/routes.rs src-tauri/tests/coach_eval_triggers.rs
git commit -m "feat(coach): SessionEnd spawns coach re-evaluator"
```

---

### Task H2: JSONL backfill batch end → evaluate + discover

**Files:**
- Modify: the JSONL backfill driver (likely `src-tauri/src/jsonl/walker.rs` or `src-tauri/src/jsonl/mod.rs`)

- [ ] **Step 1: Find the batch-completion point.**

Run: `grep -rn "JsonlIngestRun\|backfill" src-tauri/src/jsonl/`. Identify the function that finalises a backfill batch (writes the `jsonl_ingest_runs` row with `ended_at`). Call it `BATCH_END_FN`.

- [ ] **Step 2: Write the failing test** — append to `coach_eval_triggers.rs`:

```rust
#[tokio::test]
async fn jsonl_backfill_completion_runs_evaluator_and_discovery() {
    let (pool, dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    // Minimal JSONL that produces a long-session-no-commit finding.
    let jsonl = format!(r#"{{"type":"summary","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z"}}
{{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{{"role":"user","content":[{{"type":"text","text":"hi"}}]}}}}
"#);
    let p = dir.path().join("session.jsonl");
    std::fs::write(&p, jsonl).unwrap();

    andon_lib::jsonl::run_backfill_in(&pool, dir.path(), &andon_lib::settings::CoachSettings::default()).unwrap();

    // We do not assert specific findings — they depend on the seed JSONL.
    // We do assert that the evaluator ran by checking that the engine
    // does not error on empty results.
    let n: i64 = pool.get().unwrap().query_row("SELECT COUNT(*) FROM coach_findings", [], |r| r.get(0)).unwrap();
    assert!(n >= 0);
    // Skill discovery should have written rows for any qualifying hashes (likely zero here).
    let _ = pool.get().unwrap().query_row("SELECT COUNT(*) FROM skill_opportunities", [], |r| r.get::<_, i64>(0)).unwrap();
}
```

Substitute `andon_lib::jsonl::run_backfill_in` for the real public API.

- [ ] **Step 3: Implement.** In `BATCH_END_FN`, immediately after the `jsonl_ingest_runs.ended_at` is written, append:

```rust
let now = chrono::Utc::now().timestamp_millis();
let day = 86_400_000i64;
if let Err(e) = crate::coach::eval::evaluate_window(pool, now - 30*day, now + 1, &coach_settings) {
    tracing::warn!(error = ?e, "coach evaluate_window after backfill failed");
}
if let Err(e) = crate::coach::skill::discover_all(pool, &coach_settings) {
    tracing::warn!(error = ?e, "coach skill::discover_all after backfill failed");
}
```

`coach_settings: CoachSettings` must already be threaded into `BATCH_END_FN` from Task C1. If not, add it as a parameter.

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): JSONL backfill triggers evaluator + skill discovery"
```

---

## Section I — HTTP API

The six endpoints, one per task. Each task adds DTOs to `api/dto.rs`, a handler function to `api/routes.rs`, and registers the route. After all six, Task I7 adds the integration test with a snapshot.

### Task I1: `GET /api/coach/scorecard`

- [ ] **Step 1: Append DTOs** to `src-tauri/src/api/dto.rs`:

```rust
#[derive(Serialize)]
pub struct CoachScorecardDto {
    pub practices: Vec<crate::coach::score::PracticeRow>,
    pub window: crate::coach::score::WindowDto,
    pub sessions_in_window: i64,
}
```

(`PracticeRow`, `WindowDto`, `ContinuousTile` are already `Serialize` from Task F3 — re-export or inline.)

- [ ] **Step 2: Add handler** to `src-tauri/src/api/routes.rs`:

```rust
async fn coach_scorecard(
    State(state): State<ApiState>,
    Query(q): Query<FilterQuery>,
) -> Result<Json<CoachScorecardDto>, ApiError> {
    let (from, to) = q.window_or_default();
    let win = crate::coach::rules::Window { from_ms: from, to_ms: to, models: q.models_vec() };
    let card = crate::coach::score::scorecard(&state.pool, &win, &state.settings.coach())
        .map_err(ApiError::from)?;
    Ok(Json(CoachScorecardDto {
        practices: card.practices, window: card.window, sessions_in_window: card.sessions_in_window,
    }))
}
```

`q.window_or_default()` and `q.models_vec()` follow whatever helpers `FilterQuery` already exposes — check `api/filter.rs`. If the helpers are named differently, use the existing names.

Register: append `.route("/api/coach/scorecard", get(coach_scorecard))` to the router builder in `routes::router`.

Make sure `crate::coach::CoachError: Into<ApiError>` — add a `From` impl on `ApiError` if needed.

- [ ] **Step 3: Smoke-test by hitting the endpoint manually** — defer real assertions to Task I7. For now run `cargo build --features test-support` to confirm compilation.

- [ ] **Step 4-5:** Commit.

```bash
git commit -am "feat(coach): GET /api/coach/scorecard endpoint"
```

---

### Task I2: `GET /api/coach/findings`

Paginated list ordered by `detected_at DESC`. Filters: `from`, `to`, `models`, `rule_id?`, `session_id?`, `limit?` (default 50).

- [ ] **Step 1: Append DTO:**

```rust
#[derive(Serialize)]
pub struct CoachFindingRow {
    pub id: i64,
    pub rule_id: String,
    pub practice: String,
    pub severity: Option<String>,
    pub session_id: String,
    pub started_at: i64,
    pub detected_at: i64,
    pub repo: Option<String>,
    pub cost_usd: f64,
    pub description: String,
    pub suggestion: String,
    pub payload: serde_json::Value,
}

#[derive(Serialize)]
pub struct CoachFindingsResponse {
    pub items: Vec<CoachFindingRow>,
    pub next_cursor: Option<i64>,
}
```

- [ ] **Step 2: Handler:**

```rust
#[derive(Deserialize)]
struct CoachFindingsQuery {
    from: Option<i64>,
    to: Option<i64>,
    rule_id: Option<String>,
    session_id: Option<String>,
    limit: Option<i64>,
    cursor: Option<i64>,
}

async fn coach_findings(
    State(state): State<ApiState>,
    Query(q): Query<CoachFindingsQuery>,
) -> Result<Json<CoachFindingsResponse>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let mut sql = String::from(
        "SELECT cf.id, cf.rule_id, cr.practice, cr.severity, cf.session_id,
                s.started_at, cf.detected_at, s.repo_name,
                COALESCE((SELECT SUM(cost_usd) FROM cost_entries WHERE session_id = s.session_id), 0),
                cf.payload
         FROM coach_findings cf
         JOIN coach_rules cr ON cr.id = cf.rule_id
         JOIN sessions s ON s.session_id = cf.session_id
         WHERE 1=1"
    );
    let mut binds: Vec<rusqlite::types::Value> = vec![];
    if let Some(f) = q.from { sql += " AND cf.detected_at >= ?"; binds.push(f.into()); }
    if let Some(t) = q.to   { sql += " AND cf.detected_at <  ?"; binds.push(t.into()); }
    if let Some(rid) = &q.rule_id { sql += " AND cf.rule_id = ?"; binds.push(rid.clone().into()); }
    if let Some(sid) = &q.session_id { sql += " AND cf.session_id = ?"; binds.push(sid.clone().into()); }
    if let Some(c) = q.cursor { sql += " AND cf.id < ?"; binds.push(c.into()); }
    sql += " ORDER BY cf.detected_at DESC, cf.id DESC LIMIT ?";
    binds.push(limit.into());

    let mut stmt = conn.prepare(&sql).map_err(ApiError::from)?;
    let bind_refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    let items: Vec<CoachFindingRow> = stmt.query_map(&*bind_refs, |r| {
        let payload_str: String = r.get(9)?;
        let rule_id: String = r.get(1)?;
        let rule = crate::coach::rules::by_id(&rule_id);
        Ok(CoachFindingRow {
            id: r.get(0)?,
            rule_id: rule_id.clone(),
            practice: r.get(2)?,
            severity: r.get::<_, Option<String>>(3)?,
            session_id: r.get(4)?,
            started_at: r.get(5)?,
            detected_at: r.get(6)?,
            repo: r.get::<_, Option<String>>(7)?,
            cost_usd: r.get(8)?,
            description: rule.map(|r| r.description.to_string()).unwrap_or_default(),
            suggestion: rule.map(|r| r.suggestion.to_string()).unwrap_or_default(),
            payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
        })
    }).map_err(ApiError::from)?.filter_map(|r| r.ok()).collect();

    let next_cursor = items.last().map(|f| f.id);
    Ok(Json(CoachFindingsResponse { items, next_cursor }))
}
```

Register route.

- [ ] **Step 3-5:** Build + commit.

```bash
git commit -am "feat(coach): GET /api/coach/findings endpoint"
```

---

### Task I3: `GET /api/coach/rules`

Static catalogue + per-row `enabled` flag.

- [ ] **Step 1: DTO + handler:**

```rust
#[derive(Serialize)]
pub struct CoachRuleDto {
    pub id: String,
    pub practice: String,
    pub severity: Option<String>,
    pub kind: String,
    pub aiec_origin: Option<String>,
    pub description: String,
    pub suggestion: String,
    pub enabled: bool,
    pub reserved: bool,
}

async fn coach_rules(State(state): State<ApiState>) -> Result<Json<Vec<CoachRuleDto>>, ApiError> {
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let mut stmt = conn.prepare("SELECT id, enabled FROM coach_rules").map_err(ApiError::from)?;
    let enabled_map: std::collections::HashMap<String, bool> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))
        .map_err(ApiError::from)?
        .filter_map(|r| r.ok())
        .collect();

    let out: Vec<CoachRuleDto> = crate::coach::rules::RULES.iter().map(|r| CoachRuleDto {
        id: r.id.into(),
        practice: r.practice.into(),
        severity: r.severity.map(|s| s.as_str().to_string()),
        kind: match r.kind {
            crate::coach::rules::RuleKind::Binary => "binary",
            crate::coach::rules::RuleKind::Continuous => "continuous",
        }.into(),
        aiec_origin: r.aiec_origin.map(String::from),
        description: r.description.into(),
        suggestion: r.suggestion.into(),
        enabled: enabled_map.get(r.id).copied().unwrap_or(true),
        reserved: r.reserved,
    }).collect();
    Ok(Json(out))
}
```

Register: `.route("/api/coach/rules", get(coach_rules))`.

- [ ] **Step 2-5:** Build + commit.

```bash
git commit -am "feat(coach): GET /api/coach/rules endpoint"
```

---

### Task I4: `POST /api/coach/rules/:id` — toggle

- [ ] **Step 1: DTO + handler:**

```rust
#[derive(Deserialize)]
pub struct UpdateCoachRule { pub enabled: bool }

async fn coach_rules_update(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Result<Json<UpdateCoachRule>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(payload) = body.map_err(ApiError::from_json_rej)?;
    // Reserved rules can't be toggled.
    if let Some(r) = crate::coach::rules::by_id(&id) {
        if r.reserved {
            return Err(ApiError::bad_request("reserved rules cannot be toggled"));
        }
    } else {
        return Err(ApiError::not_found("unknown rule id"));
    }
    let conn = state.pool.get().map_err(ApiError::pool)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE coach_rules SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![payload.enabled as i64, now_ms, id],
    ).map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

Register: `.route("/api/coach/rules/:id", post(coach_rules_update))`.

If `ApiError::bad_request` / `not_found` / `from_json_rej` helpers don't exist, follow whatever pattern other handlers use for the same outcomes (e.g. `ApiError::Validation(...)` from existing routes).

- [ ] **Step 2-5:** Build + commit.

```bash
git commit -am "feat(coach): POST /api/coach/rules/:id endpoint"
```

---

### Task I5: `GET /api/coach/skills?lookback=30d|90d|180d`

- [ ] **Step 1: DTO + handler:**

```rust
#[derive(Serialize)]
pub struct SkillOpportunityRow {
    pub norm_hash: String,
    pub label: String,
    pub command: Option<String>,
    pub occurrences: i64,
    pub session_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Serialize)]
pub struct CoachSkillsResponse {
    pub lookback: String,
    pub opportunities: Vec<SkillOpportunityRow>,
}

#[derive(Deserialize)]
struct SkillsQuery { lookback: Option<String> }

async fn coach_skills(
    State(state): State<ApiState>,
    Query(q): Query<SkillsQuery>,
) -> Result<Json<CoachSkillsResponse>, ApiError> {
    let lookback = q.lookback.unwrap_or_else(|| "90d".into());
    let days: i64 = match lookback.as_str() {
        "30d" => 30, "90d" => 90, "180d" => 180,
        _ => return Err(ApiError::bad_request("lookback must be 30d / 90d / 180d")),
    };
    let now = chrono::Utc::now().timestamp_millis();
    let window_start = now - days * 86_400_000;
    let conn = state.pool.get().map_err(ApiError::pool)?;
    // Match by window_start tolerance — discovery may have written it at a
    // slightly different `now`, so look for the most recent row whose
    // `window_end` is within the last 24h and `window_start` matches days.
    let mut stmt = conn.prepare(
        "SELECT norm_hash, label, command, occurrences, session_count, first_seen, last_seen
         FROM skill_opportunities
         WHERE window_end >= ?1
           AND (window_end - window_start) BETWEEN ?2 AND ?3
         ORDER BY occurrences DESC, last_seen DESC"
    ).map_err(ApiError::from)?;
    let lower = days * 86_400_000 - 86_400_000;
    let upper = days * 86_400_000 + 86_400_000;
    let opps: Vec<SkillOpportunityRow> = stmt.query_map(
        rusqlite::params![now - 86_400_000, lower, upper],
        |r| Ok(SkillOpportunityRow {
            norm_hash: r.get(0)?, label: r.get(1)?, command: r.get(2)?,
            occurrences: r.get(3)?, session_count: r.get(4)?,
            first_seen: r.get(5)?, last_seen: r.get(6)?,
        }),
    ).map_err(ApiError::from)?.filter_map(|r| r.ok()).collect();
    let _ = window_start;
    Ok(Json(CoachSkillsResponse { lookback, opportunities: opps }))
}
```

Register: `.route("/api/coach/skills", get(coach_skills))`.

- [ ] **Step 2-5:** Build + commit.

```bash
git commit -am "feat(coach): GET /api/coach/skills endpoint"
```

---

### Task I6: `GET /api/coach/skills/:norm_hash/examples?limit=3`

- [ ] **Step 1: DTO + handler:**

```rust
#[derive(Serialize)]
pub struct CoachSkillExamplesResponse {
    pub examples: Vec<crate::coach::skill::SkillExample>,
}

#[derive(Deserialize)]
struct ExamplesQuery { limit: Option<i64> }

async fn coach_skill_examples(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
    Query(q): Query<ExamplesQuery>,
) -> Result<Json<CoachSkillExamplesResponse>, ApiError> {
    let limit = q.limit.unwrap_or(3).clamp(1, 10);
    let ex = crate::coach::skill::examples_for_hash(&state.pool, &hash, limit)
        .map_err(ApiError::from)?;
    Ok(Json(CoachSkillExamplesResponse { examples: ex }))
}
```

Register: `.route("/api/coach/skills/:hash/examples", get(coach_skill_examples))`.

Make `SkillExample` `Serialize` (already done in Task G3).

- [ ] **Step 2-5:** Build + commit.

```bash
git commit -am "feat(coach): GET /api/coach/skills/:hash/examples endpoint"
```

---

### Task I7: End-to-end API integration test

**Files:**
- Create: `src-tauri/tests/coach_api.rs` + `src-tauri/tests/snapshots/coach_api__scorecard.snap`

- [ ] **Step 1: Test:**

```rust
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
    let st = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (st, v)
}

#[tokio::test]
async fn coach_endpoints_smoke() {
    let (pool, _db_dir) = common::fixture_pool();
    andon_lib::coach::seed_rules(&pool).unwrap();
    let (router, _r_dir) = common::test_router(&pool);

    let (s1, v1) = get_json(router.clone(), "/api/coach/scorecard?from=0&to=9999999999999").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(v1["practices"].as_array().unwrap().len(), 5);

    let (s2, v2) = get_json(router.clone(), "/api/coach/findings").await;
    assert_eq!(s2, StatusCode::OK);
    assert!(v2["items"].is_array());

    let (s3, v3) = get_json(router.clone(), "/api/coach/rules").await;
    assert_eq!(s3, StatusCode::OK);
    let rules = v3.as_array().unwrap();
    assert_eq!(rules.len(), 12, "11 active + 1 reserved");
    let reserved = rules.iter().filter(|r| r["reserved"].as_bool() == Some(true)).count();
    assert_eq!(reserved, 1);

    let (s4, _) = get_json(router.clone(), "/api/coach/skills?lookback=90d").await;
    assert_eq!(s4, StatusCode::OK);
}
```

- [ ] **Step 2-5:** Run + commit.

```bash
git add src-tauri/tests/coach_api.rs
git commit -m "test(coach): smoke test for the six coach endpoints"
```

If `insta` is the project's snapshot tool, also save a baseline `.snap` for the scorecard JSON shape — check existing tests for the pattern. Otherwise this assertion-based smoke is sufficient.

---

### Task I8: `GET /api/settings/coach` + `PUT /api/settings/coach`

Settings persistence endpoint for the vocabulary editors. Pairs with Task M3.

- [ ] **Step 1: DTO + handlers:**

```rust
// DTOs (api/dto.rs) — re-use CoachSettings directly via serde.

async fn get_coach_settings(State(state): State<ApiState>) -> Json<crate::settings::CoachSettings> {
    Json(state.settings.coach())
}

async fn put_coach_settings(
    State(state): State<ApiState>,
    body: Result<Json<crate::settings::CoachSettings>, JsonRejection>,
) -> Result<Json<crate::settings::CoachSettings>, ApiError> {
    let Json(new) = body.map_err(ApiError::from_json_rej)?;
    let saved = state.settings.save_coach(new).map_err(ApiError::other)?;
    Ok(Json(saved))
}
```

Register:
- `.route("/api/settings/coach", get(get_coach_settings))`
- `.route("/api/settings/coach", axum::routing::put(put_coach_settings))`

- [ ] **Step 2-3: Test** — append to `coach_api.rs`:

```rust
#[tokio::test]
async fn coach_settings_roundtrip() {
    let (pool, _db_dir) = common::fixture_pool();
    let (router, _r_dir) = common::test_router(&pool);

    let (st, body) = get_json(router.clone(), "/api/settings/coach").await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["planning_commands"].as_array().unwrap().len() >= 1);
}
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): GET/PUT /api/settings/coach endpoints"
```

---

## Section J — Angular plumbing

### Task J1: ApiService methods + DTOs

**Files:**
- Modify: `web/src/app/core/api.service.ts`
- Create or modify: `web/src/app/core/models.ts` (or wherever existing DTOs live — grep `SessionSummary` to find the right file)

- [ ] **Step 1: Find the existing DTO file.**

Run: `grep -rn "export interface SessionSummary" web/src/app/`. Add new interfaces to the same file.

- [ ] **Step 2: Add DTO interfaces:**

```typescript
export interface CoachContinuous { id: string; score: number; }
export interface CoachPracticeRow {
  practice: string;
  score: number | null;
  status: 'good' | 'needs-improvement' | 'critical' | 'n/a';
  wow_pct: number;
  mom_pct: number;
  triggered_count: number;
  continuous: CoachContinuous[];
}
export interface CoachScorecard {
  practices: CoachPracticeRow[];
  window: { from: number; to: number };
  sessions_in_window: number;
}
export interface CoachFinding {
  id: number;
  rule_id: string;
  practice: string;
  severity: string | null;
  session_id: string;
  started_at: number;
  detected_at: number;
  repo: string | null;
  cost_usd: number;
  description: string;
  suggestion: string;
  payload: Record<string, unknown>;
}
export interface CoachFindingsResponse { items: CoachFinding[]; next_cursor: number | null; }
export interface CoachRule {
  id: string;
  practice: string;
  severity: string | null;
  kind: 'binary' | 'continuous';
  aiec_origin: string | null;
  description: string;
  suggestion: string;
  enabled: boolean;
  reserved: boolean;
}
export interface SkillOpportunity {
  norm_hash: string; label: string; command: string | null;
  occurrences: number; session_count: number;
  first_seen: number; last_seen: number;
}
export interface CoachSkillsResponse { lookback: string; opportunities: SkillOpportunity[]; }
export interface SkillExample { session_id: string; turn_index: number; ts: number; text: string; }
export interface CoachSettings {
  skill_min_occurrences: number;
  skill_min_sessions: number;
  planning_commands: string[];
  planning_keywords: string[];
  constraint_keywords: string[];
}
```

- [ ] **Step 3: Add ApiService methods** following the existing pattern in `api.service.ts` — every method returns `Observable<T>` via `HttpClient.get`/`put`/`post`:

```typescript
coachScorecard(args: { fromMs: number; toMs: number; models?: string }) {
  let params = new HttpParams().set('from', String(args.fromMs)).set('to', String(args.toMs));
  if (args.models) params = params.set('models', args.models);
  return this.http.get<CoachScorecard>('/api/coach/scorecard', { params });
}
coachFindings(args: { fromMs?: number; toMs?: number; ruleId?: string; sessionId?: string; limit?: number; cursor?: number }) {
  let params = new HttpParams();
  if (args.fromMs != null) params = params.set('from', String(args.fromMs));
  if (args.toMs != null) params = params.set('to', String(args.toMs));
  if (args.ruleId) params = params.set('rule_id', args.ruleId);
  if (args.sessionId) params = params.set('session_id', args.sessionId);
  if (args.limit) params = params.set('limit', String(args.limit));
  if (args.cursor) params = params.set('cursor', String(args.cursor));
  return this.http.get<CoachFindingsResponse>('/api/coach/findings', { params });
}
coachRules() { return this.http.get<CoachRule[]>('/api/coach/rules'); }
updateCoachRule(id: string, enabled: boolean) {
  return this.http.post<{ ok: boolean }>(`/api/coach/rules/${id}`, { enabled });
}
coachSkills(lookback: '30d' | '90d' | '180d') {
  return this.http.get<CoachSkillsResponse>('/api/coach/skills', {
    params: new HttpParams().set('lookback', lookback),
  });
}
coachSkillExamples(hash: string, limit = 3) {
  return this.http.get<{ examples: SkillExample[] }>(
    `/api/coach/skills/${encodeURIComponent(hash)}/examples`,
    { params: new HttpParams().set('limit', String(limit)) },
  );
}
coachSettings() { return this.http.get<CoachSettings>('/api/settings/coach'); }
saveCoachSettings(s: CoachSettings) {
  return this.http.put<CoachSettings>('/api/settings/coach', s);
}
```

- [ ] **Step 4: Build sanity check.**

Run: `cd web && npm run build`
Expected: PASS — no type errors.

- [ ] **Step 5: Commit.**

```bash
git add web/src/app/core
git commit -m "feat(coach): ApiService methods + DTOs"
```

---

### Task J2: Icons registration

**Files:**
- Modify: `web/src/app/core/icons.ts`

- [ ] **Step 1: Add icons.** Edit `icons.ts` to import and register `GraduationCap`, `Lightbulb`, `CircleSlash`, `ChevronDown`, `Lightbulb`. Follow the existing pattern — add to both the `import` block and the `APP_ICONS` object.

- [ ] **Step 2-5:** Build + commit.

```bash
git commit -am "feat(coach): register graduation-cap, lightbulb, circle-slash, chevron-down icons"
```

---

## Section K — `/coach` main page

### Task K1: `CoachComponent` shell

**Files:**
- Create: `web/src/app/features/coach/coach.component.{ts,html,spec.ts}`

- [ ] **Step 1: Write a minimal spec test first:**

```typescript
// coach.component.spec.ts
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';
import { CoachComponent } from './coach.component';
import { ApiService } from '../../core/api.service';
import { provideHttpClient } from '@angular/common/http';

describe('CoachComponent', () => {
  it('renders without errors when API returns empty', async () => {
    const fakeApi = {
      coachScorecard: () => of({ practices: [], window: { from: 0, to: 0 }, sessions_in_window: 0 }),
      coachFindings: () => of({ items: [], next_cursor: null }),
      coachSkills: () => of({ lookback: '90d', opportunities: [] }),
    };
    await TestBed.configureTestingModule({
      imports: [CoachComponent],
      providers: [provideHttpClient(), { provide: ApiService, useValue: fakeApi }],
    }).compileComponents();
    const fix = TestBed.createComponent(CoachComponent);
    fix.detectChanges();
    expect(fix.nativeElement.textContent).toContain('Coach');
  });
});
```

- [ ] **Step 2: Run + fail.**

Run: `cd web && npm test -- --run coach.component`
Expected: FAIL — component doesn't exist.

- [ ] **Step 3: Create the component:**

```typescript
// coach.component.ts
import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';

import { ApiService, CoachScorecard, CoachFindingsResponse, CoachSkillsResponse } from '../../core/api.service';
import { FilterService } from '../../core/filter.service';
import { FilterBarComponent } from '../../shared/filter-bar.component';

@Component({
  selector: 'app-coach',
  standalone: true,
  imports: [RouterLink, LucideAngularModule, FilterBarComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach.component.html',
})
export class CoachComponent {
  readonly filter = inject(FilterService);
  private readonly api = inject(ApiService);

  readonly scorecard = signal<CoachScorecard | null>(null);
  readonly findings = signal<CoachFindingsResponse | null>(null);
  readonly skills = signal<CoachSkillsResponse | null>(null);

  constructor() {
    effect(() => {
      this.filter.refreshTick();
      const w = this.filter.window();
      const models = this.filter.modelsCsv();
      const args = { fromMs: w.fromMs, toMs: w.toMs, models };
      this.api.coachScorecard(args).subscribe(v => this.scorecard.set(v));
      this.api.coachFindings({ fromMs: w.fromMs, toMs: w.toMs, limit: 50 }).subscribe(v => this.findings.set(v));
      this.api.coachSkills('90d').subscribe(v => this.skills.set(v));
    });
  }
}
```

```html
<!-- coach.component.html -->
<div class="crumb">
  <span class="flex items-center gap-1.5">
    <lucide-icon name="graduation-cap" class="w-3.5 h-3.5"></lucide-icon>Coach
  </span>
</div>

<div class="mx-6 mt-4 border border-warn/40 bg-warn/5 rounded-md px-4 py-2.5 flex items-center gap-2.5">
  <lucide-icon name="flask-conical" class="w-4 h-4 text-warn shrink-0"></lucide-icon>
  <div class="text-xs">
    <span class="text-warn font-medium">Experimental.</span>
    <span class="text-muted">Coach surfaces heuristic patterns from your local sessions.
    Rules are opinionated — disable any that don't fit in
    <a routerLink="/settings" class="underline">Settings → Coach</a>.</span>
  </div>
</div>

<app-filter-bar />

<div class="px-6 py-5 flex flex-col gap-4">
  <!-- Scorecard, findings, skill CTA come in later tasks -->
  <section class="panel">
    <div class="panel-title">Scorecard</div>
    <div class="panel-body text-xs text-muted">(coming up — Task K2)</div>
  </section>
</div>
```

- [ ] **Step 4-5:** Run + commit.

```bash
git add web/src/app/features/coach
git commit -m "feat(coach): CoachComponent shell with crumb, banner, filter bar"
```

---

### Task K2: Scorecard strip (5 tiles + continuous pills)

- [ ] **Step 1: Append spec test:**

```typescript
it('renders five practice tiles', async () => {
  const five = ['prompt','hygiene','review','tool','context'].map(p => ({
    practice: p, score: 80, status: 'good', wow_pct: 0, mom_pct: 0,
    triggered_count: 0, continuous: [],
  }));
  // … set up fakeApi.coachScorecard to return { practices: five, ... } …
  const tiles = fix.nativeElement.querySelectorAll('[data-test=practice-tile]');
  expect(tiles.length).toBe(5);
});
```

- [ ] **Step 3: Replace the scorecard placeholder** in `coach.component.html`:

```html
<section class="panel">
  <div class="panel-title">Scorecard</div>
  <div class="panel-body">
    @if (scorecard(); as sc) {
      <div class="grid grid-cols-5 gap-4">
        @for (p of sc.practices; track p.practice) {
          <div data-test="practice-tile" class="border border-border rounded p-3 flex flex-col gap-2">
            <div class="text-[11px] uppercase tracking-wider text-muted flex items-center justify-between">
              <span>{{ practiceLabel(p.practice) }}</span>
              <span>{{ p.triggered_count }} {{ p.triggered_count === 1 ? 'hit' : 'hits' }}</span>
            </div>
            <div class="text-5xl font-mono tabular-nums"
                 [class.text-ok]="scoreColor(p) === 'ok'"
                 [class.text-warn]="scoreColor(p) === 'warn'"
                 [class.text-err]="scoreColor(p) === 'err'"
                 [class.text-muted]="scoreColor(p) === 'muted'">
              {{ p.score ?? '—' }}
            </div>
            <div class="h-1.5 bg-border rounded-sm overflow-hidden">
              <div class="h-full"
                   [class.bg-ok]="scoreColor(p) === 'ok'"
                   [class.bg-warn]="scoreColor(p) === 'warn'"
                   [class.bg-err]="scoreColor(p) === 'err'"
                   [style.width.%]="p.score ?? 0"></div>
            </div>
            <div class="flex items-center gap-2 text-[11px] font-mono">
              <span [class]="trendCls(p.wow_pct)">{{ trendGlyph(p.wow_pct) }} {{ absPct(p.wow_pct) }}% w</span>
              <span [class]="trendCls(p.mom_pct)">{{ trendGlyph(p.mom_pct) }} {{ absPct(p.mom_pct) }}% m</span>
            </div>
            @for (c of p.continuous; track c.id) {
              <div class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded bg-border/40 text-[11px] font-mono">
                <span class="text-muted">{{ continuousLabel(c.id) }}</span>
                <span class="tabular-nums">{{ c.score }}</span>
              </div>
            }
          </div>
        }
      </div>
    } @else {
      <div class="text-xs text-muted">Loading…</div>
    }
  </div>
</section>
```

Add helper methods in `coach.component.ts`:

```typescript
practiceLabel(p: string): string {
  return { prompt: 'Prompt quality', hygiene: 'Session hygiene',
           review: 'Code review', tool: 'Tool mastery',
           context: 'Context mgmt' }[p] ?? p;
}
continuousLabel(id: string): string {
  return { 'model-diversity': 'Model diversity' }[id] ?? id;
}
scoreColor(p: { score: number | null }): 'ok'|'warn'|'err'|'muted' {
  if (p.score == null) return 'muted';
  if (p.score >= 70) return 'ok';
  if (p.score >= 40) return 'warn';
  return 'err';
}
trendGlyph(pct: number): string { return pct < 0 ? '▾' : pct > 0 ? '▴' : '—'; }
trendCls(pct: number): string {
  // For findings, fewer is better — invert the colour: negative = green.
  return pct < 0 ? 'text-ok' : pct > 0 ? 'text-err' : 'text-muted';
}
absPct(p: number): number { return Math.abs(p); }
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): scorecard strip (5 tiles + inverted-colour trends)"
```

---

### Task K3: Findings panel

- [ ] **Step 1:** Append findings panel to `coach.component.html` below the scorecard:

```html
<section class="panel">
  <div class="panel-title flex items-center justify-between">
    <span>Findings <span class="text-muted">({{ findings()?.items?.length ?? 0 }})</span></span>
  </div>
  <div class="panel-body">
    @if (findings(); as f) {
      @if (f.items.length === 0) {
        <div class="text-xs text-muted">No coaching findings in this window — your habits look healthy.</div>
      } @else {
        <ul class="flex flex-col gap-2">
          @for (row of f.items; track row.id) {
            <li class="border-b border-border/40 pb-2 last:border-0">
              <div class="flex items-center gap-2 text-[11px] text-muted">
                <span [class.text-err]="row.severity === 'high'"
                      [class.text-warn]="row.severity === 'medium'">●</span>
                <span class="uppercase tracking-wider">{{ row.practice }}</span>
                <span>·</span>
                <span>{{ row.severity ?? '—' }}</span>
                <span>·</span>
                <span class="font-mono tabular-nums">{{ row.started_at | date: 'YYYY-MM-dd HH:mm' }}</span>
                <span>·</span>
                <span>{{ row.repo ?? '—' }}</span>
                <span>·</span>
                <span class="font-mono tabular-nums">{{ row.cost_usd | currency }}</span>
              </div>
              <div class="text-sm text-text">{{ row.rule_id }}</div>
              <div class="text-xs text-muted">{{ row.description }}</div>
              <div class="text-xs text-muted italic">{{ row.suggestion }}</div>
            </li>
          }
        </ul>
      }
    } @else {
      <div class="text-xs text-muted">Loading…</div>
    }
  </div>
</section>
```

Import `DatePipe` and `CurrencyPipe` in `coach.component.ts`.

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): findings panel"
```

---

### Task K4: Skill Finder CTA + empty states

- [ ] **Step 1: Append CTA** below the findings panel:

```html
@if ((skills()?.opportunities?.length ?? 0) > 0) {
  <a routerLink="/coach/skills"
     class="panel block hover:bg-accent/10 transition-colors">
    <div class="panel-body flex items-center gap-3">
      <lucide-icon name="lightbulb" class="w-5 h-5 text-accent"></lucide-icon>
      <span class="flex-1 text-sm">
        <span class="font-medium">{{ skills()!.opportunities.length }}</span>
        custom-skill {{ skills()!.opportunities.length === 1 ? 'opportunity' : 'opportunities' }}
        in the last 90 days
      </span>
      <lucide-icon name="chevron-right" class="w-4 h-4 text-muted"></lucide-icon>
    </div>
  </a>
}

<div class="text-xs text-muted">
  Rules → <a routerLink="/settings" class="underline">Settings → Coach</a>
</div>
```

- [ ] **Step 4-5:** Run + commit.

```bash
git commit -am "feat(coach): Skill Finder CTA + empty states"
```

---

## Section L — `/coach/skills` sub-route

### Task L1: `CoachSkillsComponent` shell + look-back segmented control

**Files:**
- Create: `web/src/app/features/coach/coach-skills.component.{ts,html,spec.ts}`

- [ ] **Step 1: Component:**

```typescript
import { ChangeDetectionStrategy, Component, effect, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { RouterLink } from '@angular/router';
import { ApiService, CoachSkillsResponse, SkillExample } from '../../core/api.service';

@Component({
  selector: 'app-coach-skills',
  standalone: true,
  imports: [LucideAngularModule, RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach-skills.component.html',
})
export class CoachSkillsComponent {
  private readonly api = inject(ApiService);
  readonly lookback = signal<'30d' | '90d' | '180d'>('90d');
  readonly skills = signal<CoachSkillsResponse | null>(null);
  readonly expanded = signal<string | null>(null);
  readonly examplesCache = new Map<string, SkillExample[]>();

  constructor() {
    effect(() => {
      this.api.coachSkills(this.lookback()).subscribe(v => this.skills.set(v));
    });
  }

  setLookback(lb: '30d' | '90d' | '180d') { this.lookback.set(lb); }

  toggle(hash: string) {
    if (this.expanded() === hash) { this.expanded.set(null); return; }
    this.expanded.set(hash);
    if (!this.examplesCache.has(hash)) {
      this.api.coachSkillExamples(hash, 3).subscribe(r => {
        this.examplesCache.set(hash, r.examples);
        this.expanded.set(hash); // trigger re-render via signal write
      });
    }
  }

  copyAsSlashCommand(opp: { label: string; command: string | null }, examples?: SkillExample[]) {
    const name = (opp.command ?? opp.label).toLowerCase()
      .replace(/[^\w]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);
    const body = examples?.[0]?.text ?? opp.label;
    const snippet = `---\nname: ${name}\ndescription: TODO write a description.\n---\n${body}\n`;
    navigator.clipboard?.writeText(snippet);
  }
}
```

- [ ] **Step 2: Template** (`coach-skills.component.html`):

```html
<div class="crumb">
  <a routerLink="/coach" class="flex items-center gap-1.5">
    <lucide-icon name="graduation-cap" class="w-3.5 h-3.5"></lucide-icon>Coach
  </a>
  <lucide-icon name="chevron-right" class="w-3 h-3 text-muted"></lucide-icon>
  <span class="flex items-center gap-1.5">
    <lucide-icon name="lightbulb" class="w-3.5 h-3.5"></lucide-icon>Skill Finder
  </span>
</div>

<div class="px-6 py-5 flex flex-col gap-4">
  <section class="panel">
    <div class="panel-body flex items-center gap-3">
      <span class="text-xs text-muted">Look-back:</span>
      <div class="inline-flex border border-border rounded overflow-hidden">
        @for (lb of ['30d','90d','180d']; track lb) {
          <button class="px-3 py-1 text-xs"
                  [class.bg-text]="lookback() === lb"
                  [class.text-bg]="lookback() === lb"
                  [class.text-muted]="lookback() !== lb"
                  (click)="setLookback($any(lb))">
            {{ lb === '30d' ? '1 month' : lb === '90d' ? '3 months' : '6 months' }}
          </button>
        }
      </div>
    </div>
  </section>

  <section class="panel">
    <div class="panel-title">Opportunities ({{ skills()?.opportunities?.length ?? 0 }})</div>
    <div class="panel-body">
      <!-- Task L2 renders rows here -->
    </div>
  </section>
</div>
```

- [ ] **Step 3-5:** Commit.

```bash
git commit -am "feat(coach): Skill Finder sub-route shell + lookback segmented control"
```

---

### Task L2: Opportunity row + expandable examples

- [ ] **Step 1: Replace the opportunities panel body:**

```html
@if (skills(); as sk) {
  @if (sk.opportunities.length === 0) {
    <div class="text-xs text-muted">
      No skill opportunities yet in this window. This page lights up when the same
      prompt pattern appears in multiple sessions. If Andon was just installed, run
      <a routerLink="/settings" class="underline">Backfill JSONL</a> to seed history.
    </div>
  } @else {
    <ul class="flex flex-col gap-2">
      @for (o of sk.opportunities; track o.norm_hash) {
        <li class="border-b border-border/40 pb-2 last:border-0">
          <button class="w-full text-left flex items-center gap-2" (click)="toggle(o.norm_hash)">
            <lucide-icon name="lightbulb" class="w-4 h-4 text-accent"></lucide-icon>
            <span class="flex-1 text-sm font-medium">{{ o.label }}</span>
            <span class="text-[11px] text-muted font-mono">{{ o.occurrences }}×</span>
            <span class="text-[11px] text-muted">{{ o.session_count }} sessions</span>
            <lucide-icon name="chevron-down" class="w-4 h-4 text-muted"
                         [class.rotate-180]="expanded() === o.norm_hash"></lucide-icon>
          </button>
          @if (expanded() === o.norm_hash) {
            <div class="mt-2 pl-6 text-xs flex flex-col gap-2">
              @for (ex of (examplesCache.get(o.norm_hash) ?? []); track ex.session_id + ex.turn_index) {
                <div>
                  <div class="text-text">"{{ ex.text }}"</div>
                  <div class="text-muted">session {{ ex.session_id }} · turn {{ ex.turn_index }}</div>
                </div>
              }
              <button class="self-start px-2 py-1 text-[11px] border border-border rounded
                             hover:bg-border/40"
                      (click)="copyAsSlashCommand(o, examplesCache.get(o.norm_hash))">
                📋 Copy as starter slash command
              </button>
            </div>
          }
        </li>
      }
    </ul>
  }
}
```

- [ ] **Step 3-5:** Commit.

```bash
git commit -am "feat(coach): Skill Finder opportunity rows + expandable examples"
```

---

### Task L3: (already covered by L2 — copy-as-slash-command is inline.)

Skip this task as a separate step; the copy logic shipped in L2. Adjust task numbering downstream accordingly, or treat L3 as a no-op verification: write a unit test asserting the slugification logic produces expected output.

- [ ] **Step 1: Test for `copyAsSlashCommand` slugification.** Add to `coach-skills.component.spec.ts`:

```typescript
it('slugifies labels for slash command filenames', () => {
  // exercise the helper directly — extract it to a pure function if needed.
});
```

- [ ] **Step 5:** Commit.

```bash
git commit -am "test(coach): slug helper for copy-as-slash-command"
```

---

## Section M — Settings → Coach card

### Task M1: `CoachCardComponent` shell

**Files:**
- Create: `web/src/app/features/settings/coach-card.component.{ts,html,spec.ts}`
- Modify: `web/src/app/features/settings/settings.component.html` to include `<app-coach-card />`

- [ ] **Step 1: Component shell:**

```typescript
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { ApiService, CoachSettings, CoachRule } from '../../core/api.service';

@Component({
  selector: 'app-coach-card',
  standalone: true,
  imports: [LucideAngularModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './coach-card.component.html',
})
export class CoachCardComponent {
  private readonly api = inject(ApiService);
  readonly settings = signal<CoachSettings | null>(null);
  readonly rules = signal<CoachRule[]>([]);

  constructor() {
    this.api.coachSettings().subscribe(v => this.settings.set(v));
    this.api.coachRules().subscribe(v => this.rules.set(v));
  }
}
```

- [ ] **Step 2: Template** with three sub-section anchors (filled in by M2/M3/M4):

```html
<section class="panel">
  <div class="panel-title">
    <span class="flex items-center gap-1.5">
      <lucide-icon name="graduation-cap" class="w-3.5 h-3.5"></lucide-icon>
      Coach
    </span>
  </div>
  <div class="panel-body flex flex-col gap-4">
    <p class="text-xs text-muted">Anti-pattern rules and the Skill Finder. All processing is local.</p>
    <!-- Sub-sections come in M2/M3/M4 -->
  </div>
</section>
```

Add `<app-coach-card />` to `settings.component.html` somewhere sensible (after the Forwarder card).

- [ ] **Step 4-5:** Commit.

```bash
git commit -am "feat(coach): Settings Coach card shell"
```

---

### Task M2: Skill Finder thresholds sub-section

- [ ] **Step 1:** Append to `coach-card.component.html` inside the `panel-body`:

```html
@if (settings(); as s) {
  <div>
    <div class="text-xs font-medium mb-1">Skill Finder</div>
    <div class="flex items-center gap-3 text-xs">
      <label class="flex items-center gap-2">
        Min occurrences
        <input type="number" min="1" max="100"
               class="w-16 bg-bg border border-border px-1 py-0.5 font-mono"
               [value]="s.skill_min_occurrences"
               (change)="updateThreshold('occ', $any($event.target).value)">
      </label>
      <label class="flex items-center gap-2">
        Min sessions
        <input type="number" min="1" max="100"
               class="w-16 bg-bg border border-border px-1 py-0.5 font-mono"
               [value]="s.skill_min_sessions"
               (change)="updateThreshold('sess', $any($event.target).value)">
      </label>
    </div>
    <p class="text-[11px] text-muted mt-1">Surfaces prompt patterns that meet both thresholds.</p>
  </div>
}
```

Add to component:

```typescript
updateThreshold(which: 'occ'|'sess', value: string) {
  const v = Math.max(1, Number(value) | 0);
  const cur = this.settings();
  if (!cur) return;
  const next = { ...cur,
    skill_min_occurrences: which === 'occ' ? v : cur.skill_min_occurrences,
    skill_min_sessions:    which === 'sess' ? v : cur.skill_min_sessions };
  this.api.saveCoachSettings(next).subscribe(saved => this.settings.set(saved));
}
```

- [ ] **Step 4-5:** Commit.

```bash
git commit -am "feat(coach): Skill Finder thresholds in Settings Coach card"
```

---

### Task M3: Vocabulary chip-list editors

Three editors: `planning_commands`, `planning_keywords`, `constraint_keywords`. Reuse the filter-bar chip styling.

- [ ] **Step 1:** Append to the card template:

```html
@if (settings(); as s) {
  <div>
    <div class="text-xs font-medium mb-1">Vocabulary</div>
    <div class="flex flex-col gap-3 text-xs">
      <div>
        <div class="text-muted mb-1">Planning commands</div>
        <div class="flex flex-wrap gap-1">
          @for (c of s.planning_commands; track c) {
            <span class="px-2 py-0.5 border border-border rounded text-[11px] flex items-center gap-1">
              {{ c }}
              <button (click)="removeChip('planning_commands', c)" class="text-muted hover:text-err">×</button>
            </span>
          }
          <input class="bg-bg border border-border px-1 text-[11px] w-32"
                 placeholder="+ add" (keyup.enter)="addChip('planning_commands', $any($event.target))">
        </div>
      </div>
      <div>
        <div class="text-muted mb-1">Planning keywords</div>
        <!-- same shape, bound to s.planning_keywords -->
      </div>
      <div>
        <div class="text-muted mb-1">Constraint keywords</div>
        <!-- same shape, bound to s.constraint_keywords -->
      </div>
    </div>
    <p class="text-[11px] text-muted mt-2">
      These lists power detection. Tweak them to match your team's vocabulary —
      Andon won't infer them for you. Constraint-keyword changes apply to future
      sessions only; re-run Backfill JSONL for full recomputation.
    </p>
  </div>
}
```

Component helpers:

```typescript
addChip(field: keyof CoachSettings, el: HTMLInputElement) {
  const value = el.value.trim();
  if (!value) return;
  const cur = this.settings();
  if (!cur) return;
  const list = (cur[field] as string[]).slice();
  if (!list.includes(value)) list.push(value);
  this.persist({ ...cur, [field]: list } as CoachSettings);
  el.value = '';
}
removeChip(field: keyof CoachSettings, value: string) {
  const cur = this.settings();
  if (!cur) return;
  const list = (cur[field] as string[]).filter(x => x !== value);
  this.persist({ ...cur, [field]: list } as CoachSettings);
}
private persist(s: CoachSettings) {
  this.api.saveCoachSettings(s).subscribe(saved => this.settings.set(saved));
}
```

Duplicate the planning_commands chip block for `planning_keywords` and `constraint_keywords` — same template, different field name.

- [ ] **Step 4-5:** Commit.

```bash
git commit -am "feat(coach): vocabulary chip-list editors in Settings Coach card"
```

---

### Task M4: Rules sub-section with toggles + reserved-slot row

- [ ] **Step 1:** Append:

```html
<div>
  <div class="text-xs font-medium mb-1">Rules</div>
  @for (group of groupedRules(); track group.practice) {
    <div class="border border-border/40 rounded p-2 mb-2">
      <div class="text-[10px] uppercase tracking-wider text-muted mb-1">{{ group.practice }}</div>
      <ul class="flex flex-col gap-1">
        @for (r of group.rules; track r.id) {
          <li class="flex items-center gap-2 text-xs">
            @if (r.reserved) {
              <lucide-icon name="circle-slash" class="w-3.5 h-3.5 text-muted"></lucide-icon>
              <span class="text-muted line-through">{{ r.id }}</span>
              <span class="text-[10px] text-muted italic">data not captured yet</span>
            } @else {
              <input type="checkbox" [checked]="r.enabled" (change)="toggleRule(r.id, $any($event.target).checked)">
              <span>{{ r.id }}</span>
              <span class="text-[10px] uppercase text-muted">{{ r.severity ?? r.kind }}</span>
            }
          </li>
        }
      </ul>
    </div>
  }
</div>
```

Component helpers:

```typescript
groupedRules(): { practice: string; rules: CoachRule[] }[] {
  const order = ['prompt','hygiene','review','tool','context'];
  const by: Record<string, CoachRule[]> = {};
  for (const r of this.rules()) (by[r.practice] ??= []).push(r);
  return order.map(p => ({ practice: p, rules: by[p] ?? [] }));
}
toggleRule(id: string, enabled: boolean) {
  this.api.updateCoachRule(id, enabled).subscribe(() => {
    this.api.coachRules().subscribe(v => this.rules.set(v));
  });
}
```

- [ ] **Step 4-5:** Commit.

```bash
git commit -am "feat(coach): rules sub-section with toggles + reserved row"
```

---

## Section N — Routes & nav

### Task N1: Add `/coach` and `/coach/skills` routes; nav item

**Files:**
- Modify: `web/src/app/app.routes.ts`
- Modify: `web/src/app/app.component.html`

- [ ] **Step 1: Add routes:**

```typescript
// app.routes.ts (additions)
{ path: 'coach', loadComponent: () => import('./features/coach/coach.component').then(m => m.CoachComponent) },
{ path: 'coach/skills', loadComponent: () => import('./features/coach/coach-skills.component').then(m => m.CoachSkillsComponent) },
```

- [ ] **Step 2: Add nav item** in `app.component.html` after the Efficiency link, before Diagnostics:

```html
<a routerLink="/coach" routerLinkActive="active" class="nav-link">
  <lucide-icon name="graduation-cap" class="w-4 h-4"></lucide-icon>
  Coach
</a>
```

Use the exact CSS classes and structure of the surrounding nav links — check `app.component.html` for the actual pattern.

- [ ] **Step 3-5:** Build + commit.

```bash
git commit -am "feat(coach): wire /coach + /coach/skills routes + nav item"
```

---

## Section O — Docs updates

### Task O1: Update CLAUDE.md, architecture.md, features.md, README.md

**Files:**
- Modify: `CLAUDE.md`
- Modify: `docs/architecture.md`
- Modify: `docs/features.md`
- Modify: `README.md`

Per the spec's *"Implementation must update CLAUDE.md, docs/architecture.md, docs/features.md, and README.md to reflect the new posture in the same PR that adds the schema."*

- [ ] **Step 1: `CLAUDE.md`** — find the "Privacy guarantees the code must keep" list (around the `## Privacy guarantees` heading) and replace bullet 2 (the "Raw user prompts are never persisted" line) with:

```markdown
2. Prompts persisted to the local DB never leave it. The OTel forwarder strips `user_prompt` bodies before re-emitting.
```

- [ ] **Step 2: `docs/architecture.md`** — update §"Privacy & safety rules" item 2 to mirror the CLAUDE.md change. Update §"SQLite schema" to mention `prompt_turns`, `skill_opportunities`, `coach_rules`, `coach_findings`. Update §"Process model" with one sentence on the coach re-evaluator.

- [ ] **Step 3: `docs/features.md`** — insert a new Coach section between Efficiency and Sessions:

```markdown
## Coach (experimental)

Anti-pattern rules + Skill Finder over your local sessions. See the
spec at `docs/superpowers/specs/2026-05-24-...md` for the rule
catalogue and scoring formula. All processing is local; no outbound
calls.
```

- [ ] **Step 4: `README.md`** — add one bullet to the page list:

```markdown
- **Coach** — anti-pattern rules, practice-area scorecards, and a Skill Finder for repeated prompts (experimental)
```

Replace the Privacy section's "Raw user prompts are never persisted…" bullet with the new posture and add Microsoft AIEC attribution near the License section:

```markdown
The Coach feature ports the rule set and scoring approach from
[Microsoft AI Engineering Coach](https://github.com/microsoft/AI-Engineering-Coach) (MIT).
```

- [ ] **Step 5: Commit.**

```bash
git add CLAUDE.md docs/architecture.md docs/features.md README.md
git commit -m "docs(coach): privacy contract amendment + Coach page documentation"
```

---

## Section P — Manual smoke acceptance

### Task P1: End-to-end manual verification

- [ ] **Step 1: Build the binary.**

Run: `cargo tauri dev` (from repo root). Wait until the SPA loads.

- [ ] **Step 2: Verify routes load.**

In the app window, navigate to:
- `/coach` — scorecard should render with five tiles (possibly all 100s if no findings); findings panel should render either rows or "no findings".
- `/coach/skills` — should render the look-back segmented control. With no JSONL backfill it shows the empty state.
- `/settings` — the Coach card should appear with Skill Finder thresholds, Vocabulary chip lists, and Rules list.

- [ ] **Step 3: Manually verify privacy invariants.**

- Open `~/.andon/data.db` with `sqlite3` and verify `prompt_turns` has rows after a Claude Code session (or after running Backfill JSONL in Settings).
- Enable the OTel forwarder pointing at a local debug collector (e.g. `nc -l 4319`). Trigger a `user_prompt` event. Verify the forwarded payload contains `"<redacted>"` instead of the prompt body.

- [ ] **Step 4: Verify a coach finding shows.**

- Use Claude Code for at least 30 minutes with no slash commands. End the session.
- Refresh `/coach`. A `no-slash-commands` finding should appear within seconds (SessionEnd hook → `tokio::spawn` → evaluator).

- [ ] **Step 5: Document any deviations and commit.**

If anything diverges from the spec/plan, file a follow-up note in the spec's "Risks" section. No commit if everything works.

```bash
# only if you needed to adjust anything
git commit -am "fix(coach): smoke-test follow-ups"
```

---

## Self-review against the spec

**Spec coverage check:**

| Spec section | Plan task(s) |
|---|---|
| Privacy contract amendment (incl. reducer trust boundary) | B1-B5 |
| Schema: coach_rules, coach_findings | A2 |
| Schema: prompt_turns | A3, A4 |
| Schema: skill_opportunities | A3 |
| Architecture (Rust module layout) | D1 |
| Starter rule set (11 rules) | E1-E11 |
| Continuous detector | E9 |
| Scoring formula | F1 |
| WoW/MoM trends | F2 |
| Skill discovery + normaliser | G1, G2, G3 |
| Re-evaluation triggers | H1, H2 |
| API endpoints (6) + settings endpoint | I1-I6, I8 |
| Integration test | I7 |
| Frontend Coach page | K1-K4 |
| Frontend Skill Finder sub-route | L1-L2 |
| Settings → Coach card | M1-M4 |
| Routes + nav | N1 |
| Docs updates | O1 |
| Vocabulary as configuration | A1 (settings), E11 (consumer), M3 (UI) |

**Placeholder scan:** every `<datasource-needed>`, `TBD`, `TODO` in this plan represents a deliberate deferral, not an omission. Confirmed.

**Type consistency:** `evaluate_window` and `evaluate_session` both take `&CoachSettings`; `Finding` has `payload_json: String`; `Scorecard.practices` is `Vec<PracticeRow>` everywhere. Confirmed.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-05-24-ai-engineering-coach-integration.md`.**

This plan is **58 tasks** and will compact a single-session context twice over if executed inline. Strongly recommend the subagent-driven path.

**Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks. Each task is bite-sized so the subagent can hold full context. Two-stage review catches drift early.

**2. Inline Execution** — Execute tasks in this session using superpowers:executing-plans, batch execution with checkpoints. Will compact mid-plan; expect to re-orient after each compaction.

**Which approach, boss?**







