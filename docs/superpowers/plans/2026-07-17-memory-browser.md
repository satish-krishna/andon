# Memory Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Andon a page that browses, edits, and deletes what Claude Code remembers about a project, with each memory traceable to the session that last wrote it.

**Architecture:** A new crate-internal `memory` module owns everything that touches `~/.claude/projects/<slug>/memory/` on disk (path guard, frontmatter parsing, file CRUD) plus the append-only `memory_provenance` ledger. A new `api::memory_routes` module exposes five endpoints and is merged into the existing router. Provenance is captured by a branch inside the **existing** `/api/hooks/tool-use` handler — no new hooks, no `settings.json` change. A new Angular feature page reads on navigation with a manual refresh.

**Tech Stack:** Rust (axum 0.7 path syntax `:param`, rusqlite, r2d2, serde, tracing, anyhow), SQLite, Angular 21 (standalone, signals, OnPush, Tailwind), Vitest.

**Spec:** `docs/superpowers/specs/2026-07-16-memory-browser-design.md`

**Branch:** `feat/memory-browser` (already checked out).

## Global Constraints

- **US English everywhere** (color, behavior, organize). Applies to code, comments, UI copy, commit messages.
- **No `unwrap()` / `expect()`** outside `main.rs` setup. Use `anyhow::Result` at boundaries, `Option` for lookups that legitimately miss.
- **Never hold an `rusqlite` connection across `.await`.**
- **`tracing::instrument`** on public async fns in `api/` and `db/`.
- **`serde` for every JSON payload** — never hand-write JSON strings.
- **Hook handlers always return `Ok`.** Ingestion failures log and are swallowed, never surfaced to the client.
- **Angular:** standalone components only, `signal()`/`computed()`, `inject()`, `ChangeDetectionStrategy.OnPush`, `@if`/`@for` (never `*ngIf`/`*ngFor`), Tailwind utilities first.
- **Conventional Commits, no emojis:** `type(scope): subject`.
- **TDD:** failing test first, then implementation, then refactor.
- **Never run `cargo fmt`** on this repo — it is not fmt-clean and has pre-existing clippy warnings. Match surrounding style by hand.
- Rust tests: `cd src-tauri; cargo test --features test-support`. Angular tests: `cd web; npm test`.

## Ground truth established during design (do not re-derive)

- Claude Code mangles a project's absolute path into a slug by replacing each of `:`, `\`, `/`, and `.` with `-`. `D:\Repos\andon` → `D--Repos-andon`; `C:\Users\psati\.kata\worktrees` → `C--Users-psati--kata-worktrees`. **This rule is used only for repo-label matching, never to locate folders** — folders are found by enumerating the disk.
- On the author's machine 11 project folders already contain a `memory/` directory, so the project switcher has real content.
- The existing `PostToolUse` hook (`src-tauri/src/integration.rs:18-21`) has matcher `Write|Edit|MultiEdit` and pipes Claude Code's raw hook JSON to `/api/hooks/tool-use`. The payload carries `session_id`, `tool_name`, and `tool_input.file_path`.
- `sessions.repo_root` exists (added in `MIGRATION_V4`).
- `ApiError` is defined at `src-tauri/src/api/routes.rs:845` with private fields and constructors `ApiError::pool` / `ApiError::not_found`; `From<rusqlite::Error>` exists.

## Deviation from the spec, deliberate

The spec's testing section lists "frontmatter parsing" under Angular. This plan parses frontmatter **once, in Rust** (`memory::store`), because `list` needs it server-side anyway and two parsers would drift. The API ships parsed fields, so the Angular tests cover rendering concerns only (empty state, origin-unknown labeling, delete confirm). No behavior in the spec changes.

## File Structure

**Create**
- `src-tauri/src/memory/mod.rs` — module wiring only.
- `src-tauri/src/memory/paths.rs` — slug rule, disk enumeration, the containment guard, and hook-path classification. No I/O beyond directory listing and canonicalization.
- `src-tauri/src/memory/store.rs` — frontmatter parsing and memory file CRUD. Depends on `paths` for every path it touches.
- `src-tauri/src/memory/provenance.rs` — append-only ledger writes and touch queries. The only module that knows the `memory_provenance` schema.
- `src-tauri/src/api/memory_routes.rs` — axum handlers + DTOs for the five endpoints.
- `web/src/app/features/memory/memory.component.ts` / `.html` — the page.
- `web/src/app/features/memory/memory.component.spec.ts` — page tests.

**Modify**
- `src-tauri/src/db/migrations.rs` — add `MIGRATION_V7` const + array entry.
- `src-tauri/src/lib.rs` — register `memory` module.
- `src-tauri/src/api/mod.rs` — declare `memory_routes`.
- `src-tauri/src/api/routes.rs` — merge the memory router; add the provenance branch to `hook_tool_use`.
- `web/src/app/core/models.ts` — memory interfaces.
- `web/src/app/core/api.service.ts` — memory methods.
- `web/src/app/app.routes.ts` — `/memory` route.
- `web/src/app/app.component.html` — nav link.

Boundaries: `paths` never reads file contents; `store` never talks to SQLite; `provenance` never touches the filesystem; `memory_routes` orchestrates the three and owns HTTP concerns only. `routes.rs` (2903 lines) gains only a router merge and the hook branch — no new endpoints are added to it.

---

### Task 1: `memory_provenance` table (MIGRATION_V7)

**Files:**
- Modify: `src-tauri/src/db/migrations.rs:180-196`
- Test: `src-tauri/src/db/migrations.rs` (existing `mod tests` at the bottom of the file)

**Interfaces:**
- Consumes: nothing.
- Produces: table `memory_provenance(id INTEGER PK AUTOINCREMENT, session_id TEXT NOT NULL, project_slug TEXT NOT NULL, memory_file TEXT NOT NULL, action TEXT NOT NULL, ts INTEGER NOT NULL)` and index `idx_memory_prov_file`.

**Critical:** the table has **no foreign key** to `sessions`. `PRAGMA foreign_keys = ON` is set in `db::init`, and the `andon-user` sentinel is not a real session ID — an FK would reject every UI edit. Provenance is also append-only and must survive rows whose session was never ingested.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` block at the bottom of `src-tauri/src/db/migrations.rs`. Match the style of the neighboring `["cwd", "repo_root", ...]` column assertion already there.

```rust
    #[test]
    fn v7_creates_memory_provenance() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        apply(&mut conn).expect("migrations apply");

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory_provenance)")
            .and_then(|mut s| {
                s.query_map([], |r| r.get::<_, String>(1))
                    .and_then(|rows| rows.collect())
            })
            .expect("table_info");
        for expected in ["session_id", "project_slug", "memory_file", "action", "ts"] {
            assert!(cols.contains(&expected.to_string()), "missing column {expected}");
        }

        let idxs: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='memory_provenance'")
            .and_then(|mut s| {
                s.query_map([], |r| r.get::<_, String>(0))
                    .and_then(|rows| rows.collect())
            })
            .expect("index list");
        assert!(idxs.contains(&"idx_memory_prov_file".to_string()));
    }

    #[test]
    fn memory_provenance_accepts_the_andon_user_sentinel() {
        // No FK to sessions: a UI edit has no session row, and foreign_keys is ON in db::init.
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("enable fks");
        apply(&mut conn).expect("migrations apply");

        conn.execute(
            "INSERT INTO memory_provenance (session_id, project_slug, memory_file, action, ts)
             VALUES ('andon-user', 'D--Repos-andon', 'user_role.md', 'edit', 1)",
            [],
        )
        .expect("sentinel insert must not be rejected by a foreign key");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support v7_creates_memory_provenance memory_provenance_accepts
```

Expected: FAIL — `no such table: memory_provenance`.

- [ ] **Step 3: Add the migration**

Insert after the `MIGRATION_V6` const (around line 187), before `const MIGRATIONS`:

```rust
const MIGRATION_V7: &str = r#"
-- Append-only ledger mapping a memory file to the session that wrote it.
-- Rows outlive their files: deleting a memory appends a 'delete' row rather
-- than removing history, which is what makes churn (a human delete followed
-- by a model re-create) observable.
-- No FK to sessions on purpose: UI edits use the 'andon-user' sentinel, which
-- is not a session id, and foreign_keys is ON.
CREATE TABLE IF NOT EXISTS memory_provenance (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT    NOT NULL,
    project_slug TEXT    NOT NULL,
    memory_file  TEXT    NOT NULL,
    action       TEXT    NOT NULL,
    ts           INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_prov_file
    ON memory_provenance(project_slug, memory_file, ts DESC);
"#;
```

Then extend the array:

```rust
const MIGRATIONS: &[(i32, &str)] = &[
    (1, MIGRATION_V1),
    (2, MIGRATION_V2),
    (3, MIGRATION_V3),
    (4, MIGRATION_V4),
    (5, MIGRATION_V5),
    (6, MIGRATION_V6),
    (7, MIGRATION_V7),
];
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support migrations
```

Expected: PASS, including the pre-existing migration tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/migrations.rs
git commit -m "feat(memory): add memory_provenance table (MIGRATION_V7)"
```

---

### Task 2: `memory::paths` — slug rule, disk enumeration, containment guard

**Files:**
- Create: `src-tauri/src/memory/mod.rs`, `src-tauri/src/memory/paths.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: inline `mod tests` in `src-tauri/src/memory/paths.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn slug_for_project(root: &str) -> String`
  - `pub fn projects_root() -> Option<PathBuf>`
  - `pub fn memory_dir(slug: &str) -> Option<PathBuf>`
  - `pub fn projects_with_memory() -> Vec<String>`
  - `pub fn resolve_memory_path(slug: &str, rel: &str) -> Option<PathBuf>`
  - `pub fn classify_memory_write(abs: &str) -> Option<(String, String)>` returning `(project_slug, relative_file)`
  - `pub fn guard_under(base: &Path, rel: &str) -> Option<PathBuf>` (the testable core of the guard)

**This task is the security boundary.** `resolve_memory_path` is what stands between a client-supplied string and `fs::remove_file`. Anything reachable from the SPA is reachable from any web page on the machine, because `127.0.0.1:8765` is not origin-restricted. Model it on `validate_transcript_path` (`src-tauri/src/api/routes.rs:896-905`): canonicalize, then assert containment, then assert extension.

Note `Path::join` **replaces** the base when the argument is absolute (`base.join("C:\\evil")` == `C:\evil`). Canonicalize-then-`starts_with` catches that, and Step 1 tests it explicitly rather than trusting it.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/memory/paths.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn slug_replaces_separators_dots_and_colons() {
        assert_eq!(slug_for_project("D:\\Repos\\andon"), "D--Repos-andon");
        assert_eq!(slug_for_project("/home/p/proj"), "-home-p-proj");
        assert_eq!(
            slug_for_project("C:\\Users\\psati\\.kata\\worktrees"),
            "C--Users-psati--kata-worktrees"
        );
    }

    fn temp_base() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("andon-memtest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create temp base");
        fs::write(base.join("ok.md"), "hello").expect("write ok.md");
        fs::canonicalize(&base).expect("canonicalize base")
    }

    #[test]
    fn guard_allows_a_plain_md_file_inside_the_base() {
        let base = temp_base();
        let got = guard_under(&base, "ok.md").expect("plain file inside base is allowed");
        assert!(got.starts_with(&base));
    }

    #[test]
    fn guard_rejects_parent_traversal() {
        let base = temp_base();
        assert!(guard_under(&base, "../../../windows/system32/drivers/etc/hosts").is_none());
        assert!(guard_under(&base, "..\\ok.md").is_none());
    }

    #[test]
    fn guard_rejects_an_absolute_path() {
        // Path::join replaces the base when the argument is absolute.
        let base = temp_base();
        assert!(guard_under(&base, "C:\\Windows\\System32\\drivers\\etc\\hosts").is_none());
        assert!(guard_under(&base, "/etc/passwd").is_none());
    }

    #[test]
    fn guard_rejects_a_non_markdown_file() {
        let base = temp_base();
        fs::write(base.join("secrets.json"), "{}").expect("write secrets.json");
        assert!(guard_under(&base, "secrets.json").is_none());
    }

    #[test]
    fn guard_rejects_a_missing_file() {
        let base = temp_base();
        assert!(guard_under(&base, "nope.md").is_none());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support memory::paths
```

Expected: FAIL to compile — `cannot find function guard_under`.

- [ ] **Step 3: Write the implementation**

Create `src-tauri/src/memory/mod.rs`:

```rust
pub mod paths;
pub mod provenance;
pub mod store;
```

> Note: `provenance` and `store` land in Tasks 3 and 4. Until then, create empty placeholder files `src-tauri/src/memory/store.rs` and `src-tauri/src/memory/provenance.rs` so the crate compiles.

Prepend to `src-tauri/src/memory/paths.rs` (above the test module):

```rust
use std::path::{Path, PathBuf};

/// Claude Code derives a project's folder name by replacing each of `:`, `\`,
/// `/`, and `.` in the absolute path with `-`. Used only to match a known repo
/// root to an on-disk slug for labeling; folders are always located by
/// enumerating the disk, never by constructing a slug.
pub fn slug_for_project(root: &str) -> String {
    root.chars()
        .map(|c| match c {
            ':' | '\\' | '/' | '.' => '-',
            other => other,
        })
        .collect()
}

pub fn projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

pub fn memory_dir(slug: &str) -> Option<PathBuf> {
    // Reject a slug that could escape the projects root.
    if slug.is_empty() || slug.contains('/') || slug.contains('\\') || slug.contains("..") {
        return None;
    }
    Some(projects_root()?.join(slug).join("memory"))
}

/// Ground truth: every project slug that actually has a memory folder on disk.
pub fn projects_with_memory() -> Vec<String> {
    let Some(root) = projects_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("memory").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    out.sort();
    out
}

/// The containment guard. Resolves `rel` against `base`, canonicalizes both,
/// and only returns a path that provably lives inside `base` and is Markdown.
/// Modeled on `api::routes::validate_transcript_path`.
pub fn guard_under(base: &Path, rel: &str) -> Option<PathBuf> {
    let base = base.canonicalize().ok()?;
    let path = base.join(rel).canonicalize().ok()?;
    let is_md = path.extension().and_then(|e| e.to_str()) == Some("md");
    (path.starts_with(&base) && is_md && path.is_file()).then_some(path)
}

/// Guarded resolution of a client-supplied memory file within a project.
pub fn resolve_memory_path(slug: &str, rel: &str) -> Option<PathBuf> {
    guard_under(&memory_dir(slug)?, rel)
}

/// Given an absolute path a hook reported, decide whether it names a memory
/// file and, if so, which project and relative file it belongs to.
pub fn classify_memory_write(abs: &str) -> Option<(String, String)> {
    let root = projects_root()?.canonicalize().ok()?;
    let path = PathBuf::from(abs).canonicalize().ok()?;
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return None;
    }
    let rest = path.strip_prefix(&root).ok()?;
    let mut comps = rest.components();
    let slug = comps.next()?.as_os_str().to_str()?.to_string();
    if comps.next()?.as_os_str() != "memory" {
        return None;
    }
    let rel: PathBuf = comps.collect();
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some((slug, rel.to_string_lossy().replace('\\', "/")))
}
```

Register the module in `src-tauri/src/lib.rs`, following the existing cfg pattern used by `diagnostics`, `git_query`, and friends. Add after the `pub mod jsonl;` line:

```rust
#[cfg(feature = "test-support")]
pub mod memory;
#[cfg(not(feature = "test-support"))]
mod memory;
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support memory::paths
```

Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/memory/ src-tauri/src/lib.rs
git commit -m "feat(memory): add path guard and project slug resolution"
```

---

### Task 3: `memory::store` — frontmatter parsing and file CRUD

**Files:**
- Create/replace: `src-tauri/src/memory/store.rs`
- Test: inline `mod tests` in the same file

**Interfaces:**
- Consumes: `memory::paths::{memory_dir, resolve_memory_path}`.
- Produces:
  - `pub struct MemoryDoc { pub file: String, pub name: Option<String>, pub description: Option<String>, pub kind: Option<String>, pub body: String, pub raw: String, pub parse_ok: bool }` (derives `Debug, Clone, PartialEq, serde::Serialize`)
  - `pub fn parse_doc(file: &str, raw: &str) -> MemoryDoc`
  - `pub fn list(slug: &str) -> Vec<MemoryDoc>`
  - `pub fn read(slug: &str, rel: &str) -> Option<String>`
  - `pub fn save(slug: &str, rel: &str, content: &str) -> anyhow::Result<()>`
  - `pub fn delete(slug: &str, rel: &str) -> anyhow::Result<()>`
  - `pub fn strip_index_line(index: &str, file: &str) -> String`

Frontmatter shape (from the user's global memory convention): `---`, then `name:`, `description:`, `metadata:` with a nested `type:`, then `---`, then the body. `kind` on `MemoryDoc` carries `metadata.type` (`type` is a Rust keyword; do not name the field `type`). A file whose frontmatter will not parse sets `parse_ok: false` and puts the **entire raw text** in `body` — a memory you cannot parse is still one you need to see and delete. `raw` always carries the complete file text in every branch; the editor in Task 9 edits whole files, and reconstructing frontmatter from parsed fields would silently drop anything the parser ignored.

`list` excludes `MEMORY.md`; the index is fetched separately by the endpoint in Task 5.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "---\nname: user-role\ndescription: who the user is\nmetadata:\n  type: user\n---\n\nThe user maintains Andon.\n";

    #[test]
    fn parses_well_formed_frontmatter() {
        let d = parse_doc("user_role.md", GOOD);
        assert!(d.parse_ok);
        assert_eq!(d.raw, GOOD, "raw must survive parsing so the editor can round-trip it");
        assert_eq!(d.file, "user_role.md");
        assert_eq!(d.name.as_deref(), Some("user-role"));
        assert_eq!(d.description.as_deref(), Some("who the user is"));
        assert_eq!(d.kind.as_deref(), Some("user"));
        assert_eq!(d.body.trim(), "The user maintains Andon.");
    }

    #[test]
    fn malformed_frontmatter_keeps_the_raw_text_visible() {
        // No closing delimiter. The file must still be viewable and deletable.
        let raw = "---\nname: broken\nthis is not yaml at all";
        let d = parse_doc("broken.md", raw);
        assert!(!d.parse_ok);
        assert_eq!(d.body, raw, "raw text must survive so the user can see it");
        assert_eq!(d.file, "broken.md");
    }

    #[test]
    fn a_file_with_no_frontmatter_is_all_body() {
        let d = parse_doc("plain.md", "just a note");
        assert!(!d.parse_ok);
        assert_eq!(d.body, "just a note");
    }

    #[test]
    fn strip_index_line_removes_only_the_matching_pointer() {
        let index = "- [Role](user_role.md) — who they are\n- [Formatting](repo-formatting-state.md) — style\n";
        let out = strip_index_line(index, "user_role.md");
        assert!(!out.contains("user_role.md"));
        assert!(out.contains("repo-formatting-state.md"), "other pointers must survive");
    }

    #[test]
    fn strip_index_line_is_a_noop_when_the_file_is_absent() {
        let index = "- [Formatting](repo-formatting-state.md) — style\n";
        assert_eq!(strip_index_line(index, "user_role.md"), index);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support memory::store
```

Expected: FAIL to compile — `cannot find function parse_doc`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/memory/store.rs`:

```rust
use anyhow::{Context, Result};
use serde::Serialize;

use super::paths::{memory_dir, resolve_memory_path};

pub const INDEX_FILE: &str = "MEMORY.md";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryDoc {
    pub file: String,
    pub name: Option<String>,
    pub description: Option<String>,
    /// `metadata.type` from the frontmatter. Named `kind` because `type` is a
    /// Rust keyword.
    pub kind: Option<String>,
    pub body: String,
    /// The complete file text, always. The editor round-trips this; rebuilding a
    /// file from the parsed fields would drop whatever the parser ignored.
    pub raw: String,
    /// False when the frontmatter could not be parsed. `body` then holds the
    /// entire raw file so an unparseable memory is still viewable.
    pub parse_ok: bool,
}

fn unparsed(file: &str, raw: &str) -> MemoryDoc {
    MemoryDoc {
        file: file.to_string(),
        name: None,
        description: None,
        kind: None,
        body: raw.to_string(),
        raw: raw.to_string(),
        parse_ok: false,
    }
}

/// Parses the `name` / `description` / `metadata.type` frontmatter block.
/// Hand-rolled rather than pulled from a YAML crate: the shape is fixed and
/// three scalar keys do not justify a dependency.
pub fn parse_doc(file: &str, raw: &str) -> MemoryDoc {
    let rest = match raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n")) {
        Some(r) => r,
        None => return unparsed(file, raw),
    };
    let Some(end) = rest.find("\n---") else {
        return unparsed(file, raw);
    };
    let (front, after) = rest.split_at(end);
    let body = after
        .trim_start_matches('\n')
        .trim_start_matches("---")
        .trim_start_matches(['\r', '\n'])
        .to_string();

    let mut name = None;
    let mut description = None;
    let mut kind = None;
    let mut in_metadata = false;
    for line in front.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("name:") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = trimmed.strip_prefix("description:") {
            description = Some(v.trim().to_string());
        } else if trimmed.starts_with("metadata:") {
            in_metadata = true;
        } else if in_metadata && indented {
            if let Some(v) = trimmed.strip_prefix("type:") {
                kind = Some(v.trim().to_string());
            }
        } else if !trimmed.is_empty() && !indented {
            in_metadata = false;
        }
    }

    if name.is_none() && description.is_none() && kind.is_none() {
        return unparsed(file, raw);
    }

    MemoryDoc { file: file.to_string(), name, description, kind, body, raw: raw.to_string(), parse_ok: true }
}

/// Every parsed memory in the project, excluding the MEMORY.md index.
/// A missing folder yields an empty list: that is the common case, not an error.
pub fn list(slug: &str) -> Vec<MemoryDoc> {
    let Some(dir) = memory_dir(slug) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut docs: Vec<MemoryDoc> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let file = path.file_name()?.to_str()?.to_string();
            if file == INDEX_FILE {
                return None;
            }
            let raw = std::fs::read_to_string(&path).ok()?;
            Some(parse_doc(&file, &raw))
        })
        .collect();
    docs.sort_by(|a, b| a.file.cmp(&b.file));
    docs
}

pub fn read(slug: &str, rel: &str) -> Option<String> {
    std::fs::read_to_string(resolve_memory_path(slug, rel)?).ok()
}

pub fn save(slug: &str, rel: &str, content: &str) -> Result<()> {
    let path = resolve_memory_path(slug, rel).context("memory path rejected by guard")?;
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
}

/// Removes the memory file and its pointer line from MEMORY.md.
/// Hard delete, no undo: memories are a few lines each and self-regenerating.
pub fn delete(slug: &str, rel: &str) -> Result<()> {
    let path = resolve_memory_path(slug, rel).context("memory path rejected by guard")?;
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;

    if let Some(index) = resolve_memory_path(slug, INDEX_FILE) {
        if let Ok(raw) = std::fs::read_to_string(&index) {
            let next = strip_index_line(&raw, rel);
            if next != raw {
                if let Err(e) = std::fs::write(&index, next) {
                    // The memory is already gone; a stale index line is cosmetic.
                    tracing::warn!(error = %e, "memory::delete: could not rewrite MEMORY.md");
                }
            }
        }
    }
    Ok(())
}

/// Drops any MEMORY.md line whose Markdown link targets `file`.
pub fn strip_index_line(index: &str, file: &str) -> String {
    let needle = format!("]({file})");
    let kept: Vec<&str> = index.lines().filter(|l| !l.contains(&needle)).collect();
    let mut out = kept.join("\n");
    if index.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support memory::store
```

Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/memory/store.rs
git commit -m "feat(memory): parse memory frontmatter and add file CRUD"
```

---

### Task 4: `memory::provenance` — the append-only ledger

**Files:**
- Create/replace: `src-tauri/src/memory/provenance.rs`
- Test: inline `mod tests` in the same file

**Interfaces:**
- Consumes: `memory_provenance` table from Task 1.
- Produces:
  - `pub const ANDON_USER: &str = "andon-user";`
  - `pub enum Action { Create, Update, Edit, Delete }` with `pub fn as_str(&self) -> &'static str` and `pub fn for_tool(tool: &str) -> Option<Action>`
  - `pub struct Touch { pub session_id: String, pub action: String, pub ts: i64 }` (derives `Debug, Clone, PartialEq, serde::Serialize`)
  - `pub fn record(conn: &rusqlite::Connection, session_id: &str, slug: &str, file: &str, action: Action, ts: i64) -> rusqlite::Result<()>`
  - `pub fn touches(conn: &rusqlite::Connection, slug: &str, file: &str) -> rusqlite::Result<Vec<Touch>>` — most recent first
  - `pub fn last_touch(conn: &rusqlite::Connection, slug: &str, file: &str) -> Option<Touch>`

**Action vocabulary, fixed here:** `Write` → `create`, `Edit`/`MultiEdit` → `update`, UI save → `edit`, UI delete → `delete`. Derived from the tool name rather than from disk state, because a `PostToolUse` hook fires *after* the write and cannot know whether the file existed beforehand. This makes `create` mean "written wholesale by the model", not "first ever touch" — an honest limit worth a code comment, and one that does not affect churn measurement, which only needs to distinguish `andon-user` rows from model rows.

**`record` never deletes.** There is no function to remove rows. Deleting a memory appends a `delete` row.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut c).expect("migrations apply");
        c
    }

    #[test]
    fn action_maps_from_the_hook_tool_name() {
        assert_eq!(Action::for_tool("Write").map(|a| a.as_str()), Some("create"));
        assert_eq!(Action::for_tool("Edit").map(|a| a.as_str()), Some("update"));
        assert_eq!(Action::for_tool("MultiEdit").map(|a| a.as_str()), Some("update"));
        assert!(Action::for_tool("Bash").is_none());
    }

    #[test]
    fn touches_returns_most_recent_first() {
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("record create");
        record(&c, "sess-b", "P", "m.md", Action::Update, 200).expect("record update");

        let got = touches(&c, "P", "m.md").expect("touches");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].session_id, "sess-b");
        assert_eq!(got[0].action, "update");
        assert_eq!(got[1].session_id, "sess-a");
    }

    #[test]
    fn last_touch_is_the_headline_origin() {
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("record create");
        record(&c, "sess-b", "P", "m.md", Action::Update, 200).expect("record update");
        assert_eq!(last_touch(&c, "P", "m.md").expect("some touch").session_id, "sess-b");
    }

    #[test]
    fn last_touch_is_none_for_a_pre_ledger_memory() {
        let c = db();
        assert!(last_touch(&c, "P", "never-seen.md").is_none());
    }

    #[test]
    fn rows_are_scoped_per_project() {
        let c = db();
        record(&c, "sess-a", "P1", "m.md", Action::Create, 100).expect("record");
        assert!(touches(&c, "P2", "m.md").expect("touches").is_empty());
    }

    #[test]
    fn deleting_a_memory_appends_history_rather_than_erasing_it() {
        // The churn signal this feature exists to collect: a human delete
        // followed by the model writing the fact back.
        let c = db();
        record(&c, "sess-a", "P", "m.md", Action::Create, 100).expect("model create");
        record(&c, ANDON_USER, "P", "m.md", Action::Delete, 200).expect("human delete");
        record(&c, "sess-b", "P", "m.md", Action::Create, 300).expect("model re-create");

        let got = touches(&c, "P", "m.md").expect("touches");
        assert_eq!(got.len(), 3, "history must survive the delete");
        assert_eq!(got[0].session_id, "sess-b");
        assert_eq!(got[1].session_id, ANDON_USER);
        assert_eq!(got[1].action, "delete");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support memory::provenance
```

Expected: FAIL to compile — `cannot find function record`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/memory/provenance.rs`:

```rust
use rusqlite::{Connection, Result, params};
use serde::Serialize;

/// Sentinel session id for touches made by a human in Andon's UI, which have
/// no session. Kept out of the sessions FK graph deliberately (MIGRATION_V7).
pub const ANDON_USER: &str = "andon-user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    Edit,
    Delete,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Update => "update",
            Action::Edit => "edit",
            Action::Delete => "delete",
        }
    }

    /// Maps a hook's `tool_name` onto an action. `Write` reads as `create`
    /// because it replaces the whole file; a PostToolUse hook fires after the
    /// write and cannot tell whether the file already existed, so `create`
    /// means "written wholesale by the model", not "first ever touch".
    pub fn for_tool(tool: &str) -> Option<Action> {
        match tool {
            "Write" => Some(Action::Create),
            "Edit" | "MultiEdit" => Some(Action::Update),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Touch {
    pub session_id: String,
    pub action: String,
    pub ts: i64,
}

/// Appends a row. The ledger is append-only by design: there is deliberately no
/// function to remove rows, because a delete followed by a model re-create is
/// the churn signal this feature exists to collect.
pub fn record(
    conn: &Connection,
    session_id: &str,
    slug: &str,
    file: &str,
    action: Action,
    ts: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_provenance (session_id, project_slug, memory_file, action, ts)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, slug, file, action.as_str(), ts],
    )?;
    Ok(())
}

/// Full touch history for a memory, most recent first.
pub fn touches(conn: &Connection, slug: &str, file: &str) -> Result<Vec<Touch>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, action, ts
           FROM memory_provenance
          WHERE project_slug = ?1 AND memory_file = ?2
          ORDER BY ts DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![slug, file], |r| {
        Ok(Touch { session_id: r.get(0)?, action: r.get(1)?, ts: r.get(2)? })
    })?;
    rows.collect()
}

/// The headline origin: what the memory says now is what the last session wrote.
pub fn last_touch(conn: &Connection, slug: &str, file: &str) -> Option<Touch> {
    touches(conn, slug, file).ok()?.into_iter().next()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support memory::provenance
```

Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/memory/provenance.rs
git commit -m "feat(memory): add append-only provenance ledger"
```

---

### Task 5: Provenance branch in the existing tool-use hook

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (inside `hook_tool_use`, which starts at line 2251)
- Test: inline in the existing `mod tests` at `src-tauri/src/api/routes.rs:2792`

**Interfaces:**
- Consumes: `memory::paths::classify_memory_write`, `memory::provenance::{record, Action}`.
- Produces: `fn record_memory_touch(conn: &rusqlite::Connection, payload: &serde_json::Value, ts: i64)` — a private helper in `routes.rs`, unit-testable without HTTP.

**No hook is installed and `settings.json` is not touched.** The `PostToolUse` hook already exists with matcher `Write|Edit|MultiEdit` and already POSTs the raw payload here. This task adds a branch.

The helper must never fail the handler: a provenance error logs and is swallowed, per the "hook handlers always return Ok" constraint.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` block at `src-tauri/src/api/routes.rs:2792`:

```rust
    #[test]
    fn record_memory_touch_ignores_an_ordinary_project_file() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut conn).expect("migrations apply");

        let payload = serde_json::json!({
            "session_id": "sess-1",
            "tool_name": "Write",
            "tool_input": { "file_path": "D:\\Repos\\andon\\src\\main.rs" }
        });
        record_memory_touch(&conn, &payload, 42);

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_provenance", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "non-memory writes must not reach the ledger");
    }

    #[test]
    fn record_memory_touch_ignores_a_tool_that_is_not_a_write() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut conn).expect("migrations apply");

        let payload = serde_json::json!({
            "session_id": "sess-1",
            "tool_name": "Bash",
            "tool_input": { "file_path": "whatever.md" }
        });
        record_memory_touch(&conn, &payload, 42);

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_provenance", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0);
    }

    #[test]
    fn record_memory_touch_ignores_a_payload_with_no_session_id() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut conn).expect("migrations apply");

        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "x.md" }
        });
        record_memory_touch(&conn, &payload, 42);

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_provenance", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0);
    }
```

> A positive-path test is intentionally absent here: `classify_memory_write` canonicalizes against the real `~/.claude/projects`, so a passing case would depend on the developer's home directory. Classification is proven in Task 2 against a temp directory; this task proves the branch rejects everything it should and never panics.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support record_memory_touch
```

Expected: FAIL to compile — `cannot find function record_memory_touch`.

- [ ] **Step 3: Write the implementation**

Add near `hook_tool_use` in `src-tauri/src/api/routes.rs`:

```rust
/// Records a memory-file write into the provenance ledger. Called from
/// `hook_tool_use`: the PostToolUse hook Andon already installs fires on every
/// Write/Edit/MultiEdit, so no new hook is needed. Failures log and are
/// swallowed — a hook handler always returns Ok.
fn record_memory_touch(conn: &rusqlite::Connection, payload: &serde_json::Value, ts: i64) {
    let Some(tool) = payload.get("tool_name").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(action) = crate::memory::provenance::Action::for_tool(tool) else {
        return;
    };
    let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
    else {
        return;
    };
    let Some(raw_path) = payload
        .get("tool_input")
        .and_then(|v| v.get("file_path"))
        .and_then(|v| v.as_str())
    else {
        return;
    };
    let Some((slug, file)) = crate::memory::paths::classify_memory_write(raw_path) else {
        return;
    };

    if let Err(e) = crate::memory::provenance::record(conn, sid, &slug, &file, action, ts) {
        tracing::warn!(error = %e, slug = %slug, file = %file, "record_memory_touch: ledger insert failed");
    }
}
```

Then call it inside `hook_tool_use`, immediately after the existing connection checkout succeeds (the `let conn = match state.pool.get() { ... }` block around line 2333). Insert directly below that block, before the existing line-count persistence:

```rust
    record_memory_touch(&conn, &payload, now);
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support record_memory_touch
```

Expected: PASS, 3 tests.

- [ ] **Step 5: Verify no hook or settings change crept in**

```bash
git diff --stat src-tauri/src/integration.rs
```

Expected: **empty output.** `integration.rs` must be untouched. If it changed, revert it — provenance rides the existing hook.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "feat(memory): record provenance from the existing tool-use hook"
```

---

### Task 6: Memory API endpoints

**Files:**
- Create: `src-tauri/src/api/memory_routes.rs`
- Modify: `src-tauri/src/api/mod.rs`, `src-tauri/src/api/routes.rs` (router only)
- Test: inline `mod tests` in `src-tauri/src/api/memory_routes.rs`

**Interfaces:**
- Consumes: `memory::{paths, store, provenance}`; `ApiState` and `ApiError` from `api::routes`.
- Produces `pub fn router() -> axum::Router<ApiState>` registering:
  - `GET /api/memory/projects` → `Vec<MemoryProject>`
  - `GET /api/memory/:slug` → `MemoryListResponse`
  - `PUT /api/memory/:slug/file` body `{ file, content }` → `204`
  - `POST /api/memory/:slug/delete` body `{ file }` → `204`
  - `GET /api/memory/:slug/provenance?file=<rel>` → `Vec<Touch>`
- DTOs (all `#[derive(Serialize)]` / `Deserialize` as needed):
  - `pub struct MemoryProject { pub slug: String, pub label: String, pub count: usize }`
  - `pub struct MemoryEntry { pub doc: MemoryDoc, pub origin: Option<Touch> }`
  - `pub struct MemoryListResponse { pub slug: String, pub index: Option<String>, pub entries: Vec<MemoryEntry> }`
  - `pub struct SaveBody { pub file: String, pub content: String }`
  - `pub struct DeleteBody { pub file: String }`
  - `pub struct ProvenanceQuery { pub file: String }`

`ApiError`'s fields are private and its constructors are private to `routes.rs`; add `pub(crate)` to `ApiError::not_found` and add a `pub(crate) fn bad_request(msg: &str) -> Self` alongside it so `memory_routes` can build errors. Do not make the struct fields public.

Delete uses `POST /api/memory/:slug/delete` rather than HTTP `DELETE` because `DELETE` with a request body is awkward in axum and the file name must not ride in the URL path (it would need double-encoding and would defeat the guard's clarity).

**Every write and delete must pass the guard.** `store::save` and `store::delete` call `resolve_memory_path` internally, so a rejected path returns an error before any I/O. The handler translates that into `400`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_a_matching_repo_root_over_the_raw_slug() {
        let roots = vec!["D:\\Repos\\andon".to_string(), "D:\\Repos\\blog".to_string()];
        assert_eq!(label_for("D--Repos-andon", &roots), "D:\\Repos\\andon");
    }

    #[test]
    fn label_falls_back_to_the_slug_when_no_repo_matches() {
        let roots = vec!["D:\\Repos\\andon".to_string()];
        assert_eq!(label_for("C--cmder", &roots), "C--cmder");
    }

    #[test]
    fn save_body_round_trips_as_json() {
        let b: SaveBody = serde_json::from_str(r#"{"file":"a.md","content":"hi"}"#)
            .expect("deserialize SaveBody");
        assert_eq!(b.file, "a.md");
        assert_eq!(b.content, "hi");
    }

    #[test]
    fn delete_body_round_trips_as_json() {
        let b: DeleteBody =
            serde_json::from_str(r#"{"file":"a.md"}"#).expect("deserialize DeleteBody");
        assert_eq!(b.file, "a.md");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test --features test-support memory_routes
```

Expected: FAIL to compile — `file not found for module memory_routes`.

- [ ] **Step 3: Write the implementation**

Create `src-tauri/src/api/memory_routes.rs`:

```rust
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};

use crate::api::ApiState;
use crate::api::routes::ApiError;
use crate::memory::{paths, provenance, store};

#[derive(Debug, Serialize)]
pub struct MemoryProject {
    pub slug: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct MemoryEntry {
    pub doc: store::MemoryDoc,
    /// Headline origin: the last session that wrote this file. `None` means the
    /// memory predates the ledger and must be labeled "origin unknown".
    pub origin: Option<provenance::Touch>,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub slug: String,
    /// Raw MEMORY.md text, if the project has one.
    pub index: Option<String>,
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SaveBody {
    pub file: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteBody {
    pub file: String,
}

#[derive(Debug, Deserialize)]
pub struct ProvenanceQuery {
    pub file: String,
}

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/memory/projects", get(memory_projects))
        .route("/api/memory/:slug", get(memory_list))
        .route("/api/memory/:slug/file", put(memory_save))
        .route("/api/memory/:slug/delete", post(memory_delete))
        .route("/api/memory/:slug/provenance", get(memory_touches))
}

/// Labels a slug with the repo root that mangles to it, falling back to the raw
/// slug. Slugs are enumerated from disk; the mangle rule is only used to match.
fn label_for(slug: &str, repo_roots: &[String]) -> String {
    repo_roots
        .iter()
        .find(|r| paths::slug_for_project(r) == slug)
        .cloned()
        .unwrap_or_else(|| slug.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tracing::instrument(skip(state))]
async fn memory_projects(State(state): State<ApiState>) -> Result<Json<Vec<MemoryProject>>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<Vec<MemoryProject>, ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT repo_root FROM sessions
              WHERE repo_root IS NOT NULL AND repo_root != ''",
        )?;
        let roots: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(paths::projects_with_memory()
            .into_iter()
            .map(|slug| {
                let count = store::list(&slug).len();
                let label = label_for(&slug, &roots);
                MemoryProject { slug, label, count }
            })
            .collect())
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))??;
    Ok(Json(out))
}

#[tracing::instrument(skip(state))]
async fn memory_list(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
) -> Result<Json<MemoryListResponse>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<MemoryListResponse, ApiError> {
        let docs = store::list(&slug);
        let index = store::read(&slug, store::INDEX_FILE);

        let conn = pool.get().map_err(ApiError::pool)?;
        let entries = docs
            .into_iter()
            .map(|doc| {
                let origin = provenance::last_touch(&conn, &slug, &doc.file);
                MemoryEntry { doc, origin }
            })
            .collect();
        drop(conn);

        Ok(MemoryListResponse { slug, index, entries })
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))??;
    Ok(Json(out))
}

#[tracing::instrument(skip(state, body))]
async fn memory_save(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<SaveBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        // store::save resolves through the containment guard and fails closed.
        store::save(&slug, &body.file, &body.content)
            .map_err(|e| ApiError::bad_request(&format!("save rejected: {e}")))?;

        let conn = pool.get().map_err(ApiError::pool)?;
        if let Err(e) = provenance::record(
            &conn,
            provenance::ANDON_USER,
            &slug,
            &body.file,
            provenance::Action::Edit,
            now_ms(),
        ) {
            tracing::warn!(error = %e, "memory_save: ledger insert failed");
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))??;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state, body))]
async fn memory_delete(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<DeleteBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        // Record before removing: the delete row is the churn signal, and it
        // must survive even if the ledger write is the thing that fails.
        let conn = pool.get().map_err(ApiError::pool)?;
        if let Err(e) = provenance::record(
            &conn,
            provenance::ANDON_USER,
            &slug,
            &body.file,
            provenance::Action::Delete,
            now_ms(),
        ) {
            tracing::warn!(error = %e, "memory_delete: ledger insert failed");
        }
        drop(conn);

        store::delete(&slug, &body.file)
            .map_err(|e| ApiError::bad_request(&format!("delete rejected: {e}")))
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))??;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state))]
async fn memory_touches(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Query(q): Query<ProvenanceQuery>,
) -> Result<Json<Vec<provenance::Touch>>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<Vec<provenance::Touch>, ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        Ok(provenance::touches(&conn, &slug, &q.file)?)
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))??;
    Ok(Json(out))
}
```

`ApiState` is declared in `src-tauri/src/api/mod.rs` (lines 25-38) and `ApiError` in `src-tauri/src/api/routes.rs:845`. Both resolve as written above; if `routes` is not already `pub mod routes;` in `api/mod.rs`, make it so rather than re-exporting `ApiError`.

In `src-tauri/src/api/mod.rs`, declare the module beside the existing ones:

```rust
pub mod memory_routes;
```

In `src-tauri/src/api/routes.rs`, make the error constructors reachable and add the missing one:

```rust
impl ApiError {
    fn pool(e: r2d2::Error) -> Self { /* unchanged */ }

    pub(crate) fn not_found(msg: &str) -> Self { /* body unchanged; visibility widened */ }

    pub(crate) fn bad_request(msg: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.to_string(),
        }
    }
}
```

Widen `fn pool` to `pub(crate) fn pool` as well, since the handlers above call it.

Then merge the router in `router()`, immediately before `.with_state(state)` (line 77):

```rust
        .merge(crate::api::memory_routes::router())
        .with_state(state)
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test --features test-support memory_routes
```

Expected: PASS, 4 tests.

- [ ] **Step 5: Verify the whole backend still builds and passes**

```bash
cd src-tauri && cargo test --features test-support
```

Expected: PASS. Pre-existing clippy warnings are fine; compilation errors are not.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api/memory_routes.rs src-tauri/src/api/mod.rs src-tauri/src/api/routes.rs
git commit -m "feat(memory): add guarded memory read, edit, and delete endpoints"
```

---

### Task 7: Angular models and API service methods

**Files:**
- Modify: `web/src/app/core/models.ts`, `web/src/app/core/api.service.ts`

**Interfaces:**
- Consumes: the endpoints from Task 6.
- Produces, in `models.ts`:
  ```ts
  export interface MemoryTouch { session_id: string; action: string; ts: number; }
  export interface MemoryDoc { file: string; name: string | null; description: string | null; kind: string | null; body: string; raw: string; parse_ok: boolean; }
  export interface MemoryEntry { doc: MemoryDoc; origin: MemoryTouch | null; }
  export interface MemoryListResponse { slug: string; index: string | null; entries: MemoryEntry[]; }
  export interface MemoryProject { slug: string; label: string; count: number; }
  ```
- Produces, on `ApiService`: `memoryProjects()`, `memoryList(slug)`, `memorySave(slug, file, content)`, `memoryDelete(slug, file)`, `memoryTouches(slug, file)`.

Field names are snake_case because they come straight off the Rust DTOs; do not rename them client-side.

- [ ] **Step 1: Add the interfaces**

Append to `web/src/app/core/models.ts`:

```ts
export interface MemoryTouch {
  session_id: string;
  action: string;
  ts: number;
}

export interface MemoryDoc {
  file: string;
  name: string | null;
  description: string | null;
  /** `metadata.type` from the memory's frontmatter. */
  kind: string | null;
  body: string;
  /** Complete file text including frontmatter. The editor round-trips this. */
  raw: string;
  parse_ok: boolean;
}

export interface MemoryEntry {
  doc: MemoryDoc;
  /** Last session to write this file; null means it predates the ledger. */
  origin: MemoryTouch | null;
}

export interface MemoryListResponse {
  slug: string;
  index: string | null;
  entries: MemoryEntry[];
}

export interface MemoryProject {
  slug: string;
  label: string;
  count: number;
}
```

- [ ] **Step 2: Add the service methods**

Add the new types to the existing import block from `./models` in `web/src/app/core/api.service.ts` (keep the list alphabetized as it already is): `MemoryDoc`, `MemoryEntry`, `MemoryListResponse`, `MemoryProject`, `MemoryTouch`.

Add these methods to the `ApiService` class, matching the surrounding style:

```ts
  memoryProjects(): Observable<MemoryProject[]> {
    return this.http.get<MemoryProject[]>(`${BASE}/api/memory/projects`);
  }

  memoryList(slug: string): Observable<MemoryListResponse> {
    return this.http.get<MemoryListResponse>(`${BASE}/api/memory/${encodeURIComponent(slug)}`);
  }

  memorySave(slug: string, file: string, content: string): Observable<void> {
    return this.http.put<void>(`${BASE}/api/memory/${encodeURIComponent(slug)}/file`, {
      file,
      content,
    });
  }

  memoryDelete(slug: string, file: string): Observable<void> {
    return this.http.post<void>(`${BASE}/api/memory/${encodeURIComponent(slug)}/delete`, { file });
  }

  memoryTouches(slug: string, file: string): Observable<MemoryTouch[]> {
    return this.http.get<MemoryTouch[]>(
      `${BASE}/api/memory/${encodeURIComponent(slug)}/provenance`,
      { params: new HttpParams().set('file', file) },
    );
  }
```

- [ ] **Step 3: Verify the SPA compiles**

```bash
cd web && npm run build
```

Expected: build succeeds.

- [ ] **Step 4: Commit**

```bash
git add web/src/app/core/models.ts web/src/app/core/api.service.ts
git commit -m "feat(memory): add memory models and api client methods"
```

---

### Task 8: Memory page — list, render, project switcher, refresh

**Files:**
- Create: `web/src/app/features/memory/memory.component.ts`, `web/src/app/features/memory/memory.component.html`, `web/src/app/features/memory/memory.component.spec.ts`
- Modify: `web/src/app/app.routes.ts`, `web/src/app/app.component.html`

**Interfaces:**
- Consumes: `ApiService.memoryProjects/memoryList` and the models from Task 7.
- Produces: `MemoryComponent` exported from `memory.component.ts`, route `/memory`.

The page renders **the `MEMORY.md` index plus each `memory/*.md`**. The index is the file Claude Code actually loads into context every session, so it is the one memory that always matters — it gets its own collapsed block at the top, rendered read-only. It is not editable or deletable in v1: it is a generated pointer list that `store::delete` already maintains, and hand-editing it would fight that.

**Read on navigation plus a manual refresh button. No polling, no watcher.** Look at `web/src/app/features/sessions/sessions.component.ts` for the established shape: `inject(ApiService)`, subscribe into `signal()`s, `OnPush`, a separate `.html` template.

Empty state is the common case, not an error: a project with no memory folder renders an explanation, not a failure.

- [ ] **Step 1: Write the failing tests**

Create `web/src/app/features/memory/memory.component.spec.ts`. Mirror the harness used by `web/src/app/features/sessions/sessions.component.spec.ts` — read it first and match its `TestBed` setup and provider style rather than inventing one.

```ts
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting, HttpTestingController } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { MemoryComponent } from './memory.component';

describe('MemoryComponent', () => {
  let fixture: ComponentFixture<MemoryComponent>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MemoryComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([])],
    }).compileComponents();
    fixture = TestBed.createComponent(MemoryComponent);
    http = TestBed.inject(HttpTestingController);
  });

  function flushProjects(count = 1) {
    fixture.detectChanges();
    http
      .expectOne('http://127.0.0.1:8765/api/memory/projects')
      .flush([{ slug: 'D--Repos-andon', label: 'D:\\Repos\\andon', count }]);
    fixture.detectChanges();
  }

  it('renders an empty state when the project has no memories', () => {
    flushProjects(0);
    http
      .expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon')
      .flush({ slug: 'D--Repos-andon', index: null, entries: [] });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('No memories');
  });

  it('labels a memory with no provenance as origin unknown', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: 'who the user is',
            kind: 'user',
            body: 'The user maintains Andon.',
            raw: 'The user maintains Andon.',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('Origin unknown');
    expect(text).toContain('user-role');
  });

  it('shows the MEMORY.md index when the project has one', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: '- [Role](user_role.md) — who they are
',
      entries: [],
    });
    fixture.detectChanges();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.textContent).toContain('MEMORY.md');
    expect(el.textContent).not.toContain('who they are');

    fixture.componentInstance.toggleIndex();
    fixture.detectChanges();
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('who they are');
  });

  it('links a memory with provenance to its last-touching session', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: 'who the user is',
            kind: 'user',
            body: 'body',
            raw: 'body',
            parse_ok: true,
          },
          origin: { session_id: 'sess-9', action: 'update', ts: 1 },
        },
      ],
    });
    fixture.detectChanges();

    const link = (fixture.nativeElement as HTMLElement).querySelector('a[href="/sessions/sess-9"]');
    expect(link).toBeTruthy();
  });

  afterEach(() => http.verify());
});
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd web && npm test
```

Expected: FAIL — cannot resolve `./memory.component`.

- [ ] **Step 3: Write the component**

Create `web/src/app/features/memory/memory.component.ts`:

```ts
import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { MemoryEntry, MemoryProject } from '../../core/models';

@Component({
  selector: 'app-memory',
  standalone: true,
  imports: [RouterLink],
  templateUrl: './memory.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MemoryComponent implements OnInit {
  private api = inject(ApiService);

  readonly projects = signal<MemoryProject[]>([]);
  readonly slug = signal<string>('');
  readonly entries = signal<MemoryEntry[]>([]);
  /** Raw MEMORY.md text — the index Claude Code loads into context each session. */
  readonly index = signal<string | null>(null);
  readonly showIndex = signal(false);
  readonly loading = signal(false);

  ngOnInit(): void {
    this.api.memoryProjects().subscribe((ps) => {
      this.projects.set(ps);
      if (ps.length > 0) {
        this.select(ps[0].slug);
      }
    });
  }

  select(slug: string): void {
    this.slug.set(slug);
    this.refresh();
  }

  toggleIndex(): void {
    this.showIndex.update((v) => !v);
  }

  onProjectChange(event: Event): void {
    this.select((event.target as HTMLSelectElement).value);
  }

  /** Reads from disk on demand. No watcher: memory changes at most once a session. */
  refresh(): void {
    const slug = this.slug();
    if (!slug) return;
    this.loading.set(true);
    this.api.memoryList(slug).subscribe({
      next: (r) => {
        this.entries.set(r.entries);
        this.index.set(r.index);
        this.loading.set(false);
      },
      error: () => {
        this.entries.set([]);
        this.index.set(null);
        this.loading.set(false);
      },
    });
  }
}
```

Create `web/src/app/features/memory/memory.component.html`:

```html
<div class="p-6 space-y-4">
  <header class="flex items-center gap-3">
    <h1 class="text-xl font-semibold">Memory</h1>
    @if (projects().length > 0) {
      <select
        class="rounded border border-neutral-700 bg-neutral-900 px-2 py-1 text-sm"
        [value]="slug()"
        (change)="onProjectChange($event)"
      >
        @for (p of projects(); track p.slug) {
          <option [value]="p.slug">{{ p.label }} ({{ p.count }})</option>
        }
      </select>
    }
    <button
      class="rounded border border-neutral-700 px-2 py-1 text-sm hover:bg-neutral-800"
      (click)="refresh()"
      [disabled]="loading()"
    >
      Refresh
    </button>
  </header>

  <p class="text-sm text-neutral-400">
    What Claude Code remembers about this project. Read live from disk on open — press Refresh
    after a session writes.
  </p>

  @if (index(); as idx) {
    <section class="rounded border border-neutral-800 p-4">
      <button class="flex items-center gap-2 text-sm font-medium" (click)="toggleIndex()">
        <span>MEMORY.md</span>
        <span class="text-xs text-neutral-500">
          {{ showIndex() ? 'hide' : 'show' }} — the index loaded into context each session
        </span>
      </button>
      @if (showIndex()) {
        <pre class="mt-2 whitespace-pre-wrap text-sm text-neutral-300">{{ idx }}</pre>
      }
    </section>
  }

  @if (entries().length === 0 && !loading()) {
    <div class="rounded border border-neutral-800 p-6 text-sm text-neutral-400">
      No memories for this project yet. Claude Code writes these itself as it learns things worth
      carrying between sessions.
    </div>
  }

  @for (e of entries(); track e.doc.file) {
    <article class="rounded border border-neutral-800 p-4 space-y-2">
      <div class="flex items-center gap-2">
        <h2 class="font-medium">{{ e.doc.name ?? e.doc.file }}</h2>
        @if (e.doc.kind) {
          <span class="rounded bg-neutral-800 px-1.5 py-0.5 text-xs text-neutral-300">
            {{ e.doc.kind }}
          </span>
        }
        @if (!e.doc.parse_ok) {
          <span class="rounded bg-amber-900 px-1.5 py-0.5 text-xs text-amber-200">unparsed</span>
        }
      </div>

      @if (e.doc.description) {
        <p class="text-sm text-neutral-400">{{ e.doc.description }}</p>
      }

      <pre class="whitespace-pre-wrap text-sm text-neutral-200">{{ e.doc.body }}</pre>

      <div class="text-xs text-neutral-500">
        @if (e.origin) {
          Last written by
          <a class="underline" [routerLink]="['/sessions', e.origin.session_id]">
            {{ e.origin.session_id }}
          </a>
        } @else {
          Origin unknown — this memory predates provenance tracking.
        }
      </div>
    </article>
  }
</div>
```

Add the route to `web/src/app/app.routes.ts`, after the `behaviour` entry:

```ts
  {
    path: 'memory',
    loadComponent: () =>
      import('./features/memory/memory.component').then((m) => m.MemoryComponent),
  },
```

Add the nav link to `web/src/app/app.component.html`, after the `behaviour` link (line 28), matching the surrounding markup exactly:

```html
      <a routerLink="/memory" routerLinkActive="active" class="nav-link">
        Memory
      </a>
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd web && npm test
```

Expected: PASS, 4 new tests.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/memory/ web/src/app/app.routes.ts web/src/app/app.component.html
git commit -m "feat(memory): add memory browser page with project switcher"
```

---

### Task 9: Edit and delete from the page

**Files:**
- Modify: `web/src/app/features/memory/memory.component.ts`, `.html`, `.spec.ts`

**Interfaces:**
- Consumes: `ApiService.memorySave/memoryDelete` from Task 7.
- Produces: on `MemoryComponent` — `editing: WritableSignal<string | null>`, `draft: WritableSignal<string>`, `startEdit(e)`, `cancelEdit()`, `saveEdit(file)`, `remove(file)`.

**Delete is permanent and must be confirmed.** No undo, no trash — that is the spec's decision. `remove()` gates on `window.confirm` and does nothing if declined.

**The editor round-trips whole files.** The draft is seeded from `doc.raw` (the complete file text, including frontmatter — added in Task 3) and `PUT /file` writes back exactly what it is given. The server never reassembles a file from parsed fields, so nothing the parser ignored can be silently dropped by an edit.

- [ ] **Step 1: Write the failing tests**

Add to `web/src/app/features/memory/memory.component.spec.ts`:

```ts
  it('does not delete when the confirm is declined', () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'b',
            raw: 'b',
            parse_ok: true,
          },
          origin: null,
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('user_role.md');
    expect(confirmSpy).toHaveBeenCalled();
    http.expectNone('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');
  });

  it('posts a delete when the confirm is accepted', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [],
    });
    fixture.detectChanges();

    fixture.componentInstance.remove('user_role.md');
    const req = http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon/delete');
    expect(req.request.body).toEqual({ file: 'user_role.md' });
    req.flush(null);
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [],
    });
  });
```

Add `import { vi } from 'vitest';` to the spec's imports.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd web && npm test
```

Expected: FAIL — `remove is not a function`.

- [ ] **Step 3: Write the implementation**

Add to `MemoryComponent` in `memory.component.ts`:

```ts
  readonly editing = signal<string | null>(null);
  readonly draft = signal('');

  startEdit(e: MemoryEntry): void {
    this.editing.set(e.doc.file);
    this.draft.set(e.doc.raw);
  }

  cancelEdit(): void {
    this.editing.set(null);
    this.draft.set('');
  }

  onDraftInput(event: Event): void {
    this.draft.set((event.target as HTMLTextAreaElement).value);
  }

  saveEdit(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    this.api.memorySave(slug, file, this.draft()).subscribe({
      next: () => {
        this.cancelEdit();
        this.refresh();
      },
    });
  }

  /** Permanent. No undo and no trash: memories are small and self-regenerating. */
  remove(file: string): void {
    const slug = this.slug();
    if (!slug) return;
    if (!window.confirm(`Delete ${file}? This cannot be undone.`)) return;
    this.api.memoryDelete(slug, file).subscribe({ next: () => this.refresh() });
  }
```

Add `MemoryEntry` to the existing `models` import if it is not already there.

In `memory.component.html`, replace the `<pre>` line with an editing-aware block:

```html
      @if (editing() === e.doc.file) {
        <textarea
          class="h-64 w-full rounded border border-neutral-700 bg-neutral-950 p-2 font-mono text-sm"
          [value]="draft()"
          (input)="onDraftInput($event)"
        ></textarea>
        <div class="flex gap-2">
          <button
            class="rounded bg-neutral-100 px-2 py-1 text-sm text-neutral-900"
            (click)="saveEdit(e.doc.file)"
          >
            Save
          </button>
          <button
            class="rounded border border-neutral-700 px-2 py-1 text-sm"
            (click)="cancelEdit()"
          >
            Cancel
          </button>
        </div>
      } @else {
        <pre class="whitespace-pre-wrap text-sm text-neutral-200">{{ e.doc.body }}</pre>
        <div class="flex gap-2">
          <button
            class="rounded border border-neutral-700 px-2 py-1 text-sm hover:bg-neutral-800"
            (click)="startEdit(e)"
          >
            Edit
          </button>
          <button
            class="rounded border border-red-900 px-2 py-1 text-sm text-red-300 hover:bg-red-950"
            (click)="remove(e.doc.file)"
          >
            Delete
          </button>
        </div>
      }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd web && npm test
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/app/features/memory/
git commit -m "feat(memory): edit and delete memories from the browser"
```

---

### Task 10: Touch history on demand

**Files:**
- Modify: `web/src/app/features/memory/memory.component.ts`, `.html`, `.spec.ts`

**Interfaces:**
- Consumes: `ApiService.memoryTouches` from Task 7.
- Produces: `history: WritableSignal<Record<string, MemoryTouch[]>>`, `openHistory: WritableSignal<string | null>`, `toggleHistory(file)`, `isAndonUser(t)`.

The headline is the last touch (Task 8). This adds the full list behind a disclosure, including the `andon-user` sentinel rows, which must render as "you, in Andon" rather than as a session link — there is no session page for a human edit.

- [ ] **Step 1: Write the failing test**

Add to `memory.component.spec.ts`:

```ts
  it('shows human edits as andon-user rather than a session link', () => {
    flushProjects();
    http.expectOne('http://127.0.0.1:8765/api/memory/D--Repos-andon').flush({
      slug: 'D--Repos-andon',
      index: null,
      entries: [
        {
          doc: {
            file: 'user_role.md',
            name: 'user-role',
            description: null,
            kind: 'user',
            body: 'b',
            raw: 'b',
            parse_ok: true,
          },
          origin: { session_id: 'andon-user', action: 'edit', ts: 2 },
        },
      ],
    });
    fixture.detectChanges();

    fixture.componentInstance.toggleHistory('user_role.md');
    http
      .expectOne(
        'http://127.0.0.1:8765/api/memory/D--Repos-andon/provenance?file=user_role.md',
      )
      .flush([
        { session_id: 'andon-user', action: 'edit', ts: 2 },
        { session_id: 'sess-1', action: 'create', ts: 1 },
      ]);
    fixture.detectChanges();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.textContent).toContain('You, in Andon');
    expect(el.querySelector('a[href="/sessions/andon-user"]')).toBeNull();
    expect(el.querySelector('a[href="/sessions/sess-1"]')).toBeTruthy();
  });
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd web && npm test
```

Expected: FAIL — `toggleHistory is not a function`.

- [ ] **Step 3: Write the implementation**

Add to `MemoryComponent`, and add `MemoryTouch` to the `models` import:

```ts
  readonly openHistory = signal<string | null>(null);
  readonly history = signal<Record<string, MemoryTouch[]>>({});

  /** Matches the sentinel written for UI edits and deletes (memory::provenance). */
  isAndonUser(t: MemoryTouch): boolean {
    return t.session_id === 'andon-user';
  }

  toggleHistory(file: string): void {
    if (this.openHistory() === file) {
      this.openHistory.set(null);
      return;
    }
    this.openHistory.set(file);
    const slug = this.slug();
    if (!slug) return;
    this.api.memoryTouches(slug, file).subscribe((ts) => {
      this.history.update((h) => ({ ...h, [file]: ts }));
    });
  }
```

Update the origin block in `memory.component.html` — replace the existing `<div class="text-xs text-neutral-500">…</div>` with:

```html
      <div class="space-y-1 text-xs text-neutral-500">
        <div class="flex items-center gap-2">
          @if (e.origin) {
            <span>
              Last written by
              @if (isAndonUser(e.origin)) {
                <span class="text-neutral-300">You, in Andon</span>
              } @else {
                <a class="underline" [routerLink]="['/sessions', e.origin.session_id]">
                  {{ e.origin.session_id }}
                </a>
              }
            </span>
          } @else {
            <span>Origin unknown — this memory predates provenance tracking.</span>
          }
          <button class="underline" (click)="toggleHistory(e.doc.file)">
            {{ openHistory() === e.doc.file ? 'Hide history' : 'History' }}
          </button>
        </div>

        @if (openHistory() === e.doc.file) {
          <ul class="space-y-0.5 border-l border-neutral-800 pl-2">
            @for (t of history()[e.doc.file] ?? []; track t.ts) {
              <li>
                <span class="text-neutral-400">{{ t.action }}</span>
                —
                @if (isAndonUser(t)) {
                  <span class="text-neutral-300">You, in Andon</span>
                } @else {
                  <a class="underline" [routerLink]="['/sessions', t.session_id]">
                    {{ t.session_id }}
                  </a>
                }
              </li>
            }
          </ul>
        }
      </div>
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd web && npm test
```

Expected: PASS.

- [ ] **Step 5: Full verification**

```bash
cd src-tauri && cargo test --features test-support
cd ../web && npm test && npm run build
```

Expected: all PASS. Then confirm the untouched-hook guarantee one final time:

```bash
git diff main --stat -- src-tauri/src/integration.rs
```

Expected: **empty output.**

- [ ] **Step 6: Commit**

```bash
git add web/src/app/features/memory/
git commit -m "feat(memory): show full provenance touch history on demand"
```

---

## Manual verification before merge

Run the app and drive the real flow — tests do not prove the disk path works against a real `~/.claude`:

```powershell
cargo tauri dev
```

1. Open **Memory**. The switcher lists projects that have memory folders (11 on the author's machine). `D:\Repos\andon` shows `user_role.md`, `repo-formatting-state.md`, `release-build-pwsh-missing.md`.
2. Every memory reads "Origin unknown" — correct, they all predate the ledger. This is the forward-only limit working, not a bug.
3. Edit a memory, save, Refresh. The change persists; History shows "You, in Andon".
4. In a separate Claude Code session, get the model to write a memory in this project. Press Refresh. The new memory appears with a session link that opens Session Detail.
5. Delete a memory. Confirm the file **and** its `MEMORY.md` pointer line are gone from `~/.claude/projects/D--Repos-andon/memory/`.
6. Confirm `~/.claude/settings.json` is byte-identical to before the run.

## Definition of Done

- `cargo test --features test-support` and `npm test` pass; `npm run build` succeeds.
- `src-tauri/src/integration.rs` and `~/.claude/settings.json` are untouched.
- No `unwrap()`/`expect()` in new non-test code.
- Manual verification above completed.
- Update `docs/features.md` with a Memory section and add the page to any page list in `README.md`. Follow the existing entries' shape.
