# Hook Response Envelope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ad-hoc `{"ok": true, ...}` bodies from Andon's three Claude Code hook receiver endpoints with a valid hook output envelope, eliminating "Hook JSON output validation failed" errors after every Write / Edit / MultiEdit / SessionStart / SessionEnd.

**Architecture:** Introduce a `HookOutput` struct in a new `src-tauri/src/api/hook_response.rs` module that serializes to Claude Code's hook output schema (camelCase, all fields optional). The three hook handlers in `src-tauri/src/api/routes.rs` change their return type to `Json<HookOutput>`. Handlers always return `200 OK` with `HookOutput::ok()` (which serializes to `{}`), even on invalid input — invalid input is logged via `tracing::warn!` and the DB write is skipped, matching the project rule "Always return Ok to the client — never propagate ingestion errors to Claude Code."

**Tech Stack:** Rust, axum, serde, tracing. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-18-hook-response-envelope-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/src/api/hook_response.rs` | **Create** | Defines `HookOutput` (serializable model of CC's hook output envelope) and its unit tests. |
| `src-tauri/src/api/mod.rs` | **Modify** | Add `pub mod hook_response;` so `routes.rs` can import the type. |
| `src-tauri/src/api/routes.rs` | **Modify** | Change three hook handler return types from `Json<serde_json::Value>` / `Result<Json<Value>, ApiError>` to `Json<HookOutput>`. Drop the `400 BAD_REQUEST` early-return in `hook_session_context`; log `warn!` and return `HookOutput::ok()` instead. |

No DB migration. No settings.json migration. No frontend changes.

---

## Task 1: `HookOutput` struct + serialization tests (TDD)

**Files:**
- Create: `src-tauri/src/api/hook_response.rs`
- Modify: `src-tauri/src/api/mod.rs:1-2`

- [ ] **Step 1: Add the module declaration**

Open `src-tauri/src/api/mod.rs` and change the top of the file from:

```rust
pub mod dto;
pub mod routes;
```

to:

```rust
pub mod dto;
pub mod hook_response;
pub mod routes;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/api/hook_response.rs` with the tests only (no struct yet):

```rust
//! Claude Code hook output envelope.
//!
//! Claude Code's hook runtime reads the hook command's stdout and validates it
//! against its hook output schema. Returning ad-hoc shapes (e.g. `{"ok": true}`)
//! triggers "Hook JSON output validation failed". `HookOutput` models the
//! envelope so handlers always return something CC accepts.

#[cfg(test)]
mod tests {
    use super::HookOutput;

    #[test]
    fn ok_serializes_to_empty_object() {
        let s = serde_json::to_string(&HookOutput::ok()).unwrap();
        assert_eq!(s, "{}");
    }

    #[test]
    fn none_fields_are_omitted() {
        let out = HookOutput {
            system_message: Some("hi".into()),
            ..Default::default()
        };
        let s = serde_json::to_string(&out).unwrap();
        assert_eq!(s, r#"{"systemMessage":"hi"}"#);
    }

    #[test]
    fn field_names_are_camel_case() {
        let out = HookOutput {
            r#continue: Some(false),
            suppress_output: Some(true),
            system_message: Some("paused".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&out).unwrap();
        assert!(v.get("continue").is_some(), "continue field missing");
        assert!(v.get("suppressOutput").is_some(), "suppressOutput field missing");
        assert!(v.get("systemMessage").is_some(), "systemMessage field missing");
        assert!(v.get("suppress_output").is_none(), "snake_case leaked");
        assert!(v.get("system_message").is_none(), "snake_case leaked");
    }
}
```

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo test -p andon --lib api::hook_response`

Expected: compile error — `HookOutput` not found.

- [ ] **Step 4: Implement `HookOutput`**

Add the struct above the `#[cfg(test)]` block in `src-tauri/src/api/hook_response.rs`:

```rust
use serde::Serialize;

/// Models Claude Code's hook output JSON envelope.
///
/// Every field is optional; `HookOutput::ok()` serializes to `{}` which is a
/// valid no-op hook output. Populating `system_message` surfaces a message in
/// the CC transcript; `r#continue: Some(false)` blocks the tool call (only
/// meaningful for PreToolUse — included for future use).
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#continue: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

impl HookOutput {
    /// Empty, valid hook output. Serializes to `{}`.
    pub fn ok() -> Self {
        Self::default()
    }
}
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p andon --lib api::hook_response`

Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api/hook_response.rs src-tauri/src/api/mod.rs
git commit -m "feat(api): add HookOutput envelope for Claude Code hook responses"
```

---

## Task 2: `hook_tool_use` returns `HookOutput`

**Files:**
- Modify: `src-tauri/src/api/routes.rs:1986-2103`

- [ ] **Step 1: Add the import**

In `src-tauri/src/api/routes.rs` find the existing `use super::{ApiState, dto::*};` line (around line 15) and add `hook_response::HookOutput` to a new line below it:

```rust
use super::{ApiState, dto::*};
use super::hook_response::HookOutput;
```

- [ ] **Step 2: Change the handler signature and tail**

In `src-tauri/src/api/routes.rs`, locate `async fn hook_tool_use` (around line 1986).

Change the signature from:

```rust
async fn hook_tool_use(
    State(state): State<ApiState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
```

to:

```rust
async fn hook_tool_use(
    State(state): State<ApiState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<HookOutput> {
```

Then change the **two** return statements in the body:

a) The DB-unavailable branch (around line 2064):

```rust
let conn = match state.pool.get() {
    Ok(c) => c,
    Err(_) => return Json(json!({"ok": false, "error": "db unavailable"})),
};
```

becomes:

```rust
let conn = match state.pool.get() {
    Ok(c) => c,
    Err(_) => {
        tracing::warn!("hook_tool_use: db pool unavailable");
        return Json(HookOutput::ok());
    }
};
```

b) The final happy-path return (around line 2093):

```rust
    Json(json!({
        "ok": true,
        "tool": tool,
        "file_path": file_path,
        "added": added,
        "removed": removed,
        "decision": decision,
        "wrote_file_change": wrote_file,
        "wrote_decision": wrote_decision,
    }))
}
```

becomes:

```rust
    let _ = (wrote_file, wrote_decision); // diagnostic flags retained for logging above
    Json(HookOutput::ok())
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p andon`

Expected: clean build. If `wrote_file` / `wrote_decision` produce unused-variable warnings, that's fine — the `let _ = (...)` line silences them.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "fix(api): hook_tool_use returns valid Claude Code hook output envelope"
```

---

## Task 3: `hook_session_end` returns `HookOutput`

**Files:**
- Modify: `src-tauri/src/api/routes.rs:843-940`

- [ ] **Step 1: Change the signature and remove the `ApiError` early-return**

In `src-tauri/src/api/routes.rs`, locate `async fn hook_session_end` (around line 843).

Change the signature from:

```rust
async fn hook_session_end(
    State(state): State<ApiState>,
    Json(p): Json<SessionEndPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sid = p.session_id.unwrap_or_default();
    if sid.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "session_id required".into(),
        });
    }
```

to:

```rust
async fn hook_session_end(
    State(state): State<ApiState>,
    Json(p): Json<SessionEndPayload>,
) -> Json<HookOutput> {
    let sid = p.session_id.unwrap_or_default();
    if sid.is_empty() {
        tracing::warn!("hook_session_end: missing session_id; skipping persist");
        return Json(HookOutput::ok());
    }
```

- [ ] **Step 2: Change the final return**

The function currently ends (around line 939) with:

```rust
    Ok(Json(json!({ "ok": true, "reason": p.reason })))
}
```

Change it to:

```rust
    let _ = p.reason; // diagnostic field; intentionally dropped from wire
    Json(HookOutput::ok())
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p andon`

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "fix(api): hook_session_end returns valid hook output, always 200 OK"
```

---

## Task 4: `hook_session_context` returns `HookOutput`

**Files:**
- Modify: `src-tauri/src/api/routes.rs:942-1020`

- [ ] **Step 1: Change the signature and replace the `400` branch**

In `src-tauri/src/api/routes.rs`, locate `async fn hook_session_context` (around line 942).

Change the signature and the empty-`sid` branch from:

```rust
async fn hook_session_context(
    State(state): State<ApiState>,
    Json(p): Json<crate::api::dto::SessionContextPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sid = p.session_id.trim().to_string();
    if sid.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "session_id required".into(),
        });
    }
```

to:

```rust
async fn hook_session_context(
    State(state): State<ApiState>,
    Json(p): Json<crate::api::dto::SessionContextPayload>,
) -> Json<HookOutput> {
    let sid = p.session_id.trim().to_string();
    if sid.is_empty() {
        tracing::warn!("hook_session_context: missing session_id; skipping persist");
        return Json(HookOutput::ok());
    }
```

- [ ] **Step 2: Change the final return**

The function currently ends (around line 1019) with:

```rust
    Ok(Json(json!({ "ok": true })))
}
```

Change it to:

```rust
    Json(HookOutput::ok())
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p andon`

Expected: clean build. If `StatusCode` becomes unused at the top of the file, leave it — other handlers still use it (verify with `cargo build` showing no `unused_imports` warning for it).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "fix(api): hook_session_context returns valid hook output, always 200 OK"
```

---

## Task 5: Full test suite + manual verification

**Files:** none modified in this task.

- [ ] **Step 1: Run the full Rust test suite**

Run: `cargo test -p andon`

Expected: all tests pass, including the three new `api::hook_response::tests` cases. Any pre-existing failures are out of scope for this plan — note them but do not fix here.

- [ ] **Step 2: Build the release binary**

Run: `cargo build -p andon --release`

Expected: clean build, no warnings introduced by this change.

- [ ] **Step 3: Manual smoke test**

1. Stop any running `andon.exe`.
2. Launch the freshly built release binary: `./src-tauri/target/release/andon.exe`.
3. In a separate shell, simulate a `PostToolUse` hook POST:

   ```bash
   curl -s -i -X POST http://127.0.0.1:8765/api/hooks/tool-use \
     -H "Content-Type: application/json" \
     --data '{"session_id":"smoke-test","tool_name":"Write","tool_input":{"file_path":"x.txt","content":"hi\n"}}'
   ```

   Expected response body: `{}` (literally two characters). Status: `200 OK`.

4. Repeat for the other two endpoints:

   ```bash
   curl -s -i -X POST http://127.0.0.1:8765/api/hooks/session-end \
     -H "Content-Type: application/json" \
     --data '{"session_id":"smoke-test","reason":"clear"}'

   curl -s -i -X POST http://127.0.0.1:8765/api/session/context \
     -H "Content-Type: application/json" \
     --data '{"session_id":"smoke-test","cwd":"/tmp"}'
   ```

   Expected: each returns `{}` with `200 OK`.

5. Send an intentionally invalid payload and confirm it still returns `{}` `200`:

   ```bash
   curl -s -i -X POST http://127.0.0.1:8765/api/hooks/session-end \
     -H "Content-Type: application/json" \
     --data '{}'
   ```

   Expected: `200 OK`, body `{}`. Check `~/.andon/log.txt` (Windows: `%USERPROFILE%/.andon/log.txt`) — expect a `WARN` line `hook_session_end: missing session_id; skipping persist`.

6. Open a fresh Claude Code session in any repo, run any `Write`/`Edit` tool call, then end the session. Confirm **no** "Hook JSON output validation failed" line appears in the Claude Code transcript.

- [ ] **Step 4: Commit a no-op marker only if needed**

This task introduces no code changes. Skip the commit step unless `cargo build` produced an unexpected lint that needed a fix — in which case commit it with:

```bash
git add -A
git commit -m "chore: silence post-refactor lint"
```

---

## Out of scope (do NOT do in this plan)

- Version bump to `0.4.2` and release tagging — handled by the manual release process (see memory `release_process.md`) after this branch is merged.
- API integration tests for the hook handlers — covered by `2026-05-18-test-harness-phase-{1,2,3}.md`.
- Populating `system_message` / `r#continue` from real Andon state (e.g., "ingest paused"). The struct supports it; no caller needs to use it yet.
- Touching any non-hook endpoint.
