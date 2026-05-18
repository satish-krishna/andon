# Session repo capture — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture the git repository (or, failing that, the working folder) for every Claude Code session and surface it as a first-class dimension in the UI — displayed on session rows, filterable on Sessions and Files, aggregated on Overview.

**Architecture:** A `SessionStart` hook in `~/.claude/settings.json` (a one-line `curl` matching the pattern Andon already uses for `PostToolUse` and `SessionEnd`) streams Claude Code's hook payload to `POST /api/session/context`. The handler persists `cwd` synchronously and returns 200 immediately, then runs `git` queries against `cwd` in a spawned task to enrich `repo_root` / `repo_remote` / `repo_branch` / `repo_name`. An inference fallback walks per-session file paths for sessions captured before the hook landed (or where the hook didn't fire).

**Tech Stack:** Rust 1.95 (axum, tokio, rusqlite, serde, tracing) · Angular 21 (standalone components, signals, Tailwind 4) · SQLite (WAL).

**Spec:** [`docs/superpowers/specs/2026-05-17-session-repo-capture-design.md`](../specs/2026-05-17-session-repo-capture-design.md)

**Branch:** `repo-capture` (already created and checked out)

---

## File structure

### Create
- `src-tauri/src/git_query.rs` — Async helpers that shell out to `git` from a path: `query_repo`, `normalize_remote`, `compute_repo_name`. Unit tests.
- `src-tauri/src/repo_inference.rs` — `longest_common_ancestor`, `find_git_ancestor`, `infer_repo_for_session`. Unit tests.
- `web/src/app/features/overview/top-repos-tile.component.ts` — Standalone Angular component for the Overview "TOP REPOS · PERIOD" tile.

### Modify
- `src-tauri/src/lib.rs` — register the two new modules.
- `src-tauri/src/db/migrations.rs` — add `MIGRATION_V3` (5 columns + 2 indexes on `sessions`).
- `src-tauri/src/integration.rs` — add `install_session_start_hook` / `remove_session_start_hook` / `has_session_start_hook` mirroring the existing SessionEnd helpers; wire into `try_ensure` and `unpatch_claude_settings`.
- `src-tauri/src/api/routes.rs` — add `POST /api/session/context`, `POST /api/repo/backfill`, `GET /api/repos`, and `GET /api/overview/top-repos`. Project repo fields on existing session-list and session-detail responses. Accept `repo` query param on session/file filters.
- `src-tauri/src/api/dto.rs` — add repo fields to session DTOs; new `RepoSummary` and `TopRepoEntry` DTOs.
- `web/src/app/core/models.ts` — add repo fields on session types; add `RepoSummary`, `TopRepoEntry`.
- `web/src/app/core/api.service.ts` — add typed wrappers for the new endpoints and `repo` filter param plumbing.
- `web/src/app/core/filter.service.ts` — add `repos = signal<string[]>([])` to the shared filter state; include in query-param marshalling.
- `web/src/app/features/sessions/sessions.component.{ts,html}` — REPO column; REPO filter chip group; delete the "NOT EMITTED BY CLAUDE CODE" placeholder.
- `web/src/app/features/sessions/session-detail.component.ts` — header subtitle line for repo + branch.
- `web/src/app/features/files/files.component.{ts,html}` — REPO filter chip group; render paths relative to `repo_root` when exactly one repo is selected.
- `web/src/app/features/overview/overview.component.{ts,html}` — slot in the new `<app-top-repos-tile />`.
- `web/src/app/features/settings/settings.component.{ts,html}` — "Backfill repo info" button under the Data section.
- `README.md` — extend Privacy section to mention the SessionStart hook.

---

## Phase 1 — DB & helpers

### Task 1: Migration v3 — add repo columns to `sessions`

**Files:**
- Modify: `src-tauri/src/db/migrations.rs:99`
- Test: `src-tauri/src/db/migrations.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Add migration constant**

After `MIGRATION_V2` (line 97), add:

```rust
const MIGRATION_V3: &str = r#"
ALTER TABLE sessions ADD COLUMN cwd TEXT;
ALTER TABLE sessions ADD COLUMN repo_root TEXT;
ALTER TABLE sessions ADD COLUMN repo_remote TEXT;
ALTER TABLE sessions ADD COLUMN repo_branch TEXT;
ALTER TABLE sessions ADD COLUMN repo_name TEXT;
CREATE INDEX idx_sessions_repo_remote ON sessions(repo_remote);
CREATE INDEX idx_sessions_repo_root   ON sessions(repo_root);
"#;
```

- [ ] **Step 2: Register it**

Change the `MIGRATIONS` slice:

```rust
const MIGRATIONS: &[(i32, &str)] = &[(1, MIGRATION_V1), (2, MIGRATION_V2), (3, MIGRATION_V3)];
```

- [ ] **Step 3: Write the failing test**

At the bottom of `migrations.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn v3_adds_repo_columns_and_indexes() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(|r| r.unwrap()).collect();
        for expected in ["cwd", "repo_root", "repo_remote", "repo_branch", "repo_name"] {
            assert!(cols.contains(&expected.to_string()), "missing column {expected}");
        }

        let idxs: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='sessions'").unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap()
            .map(|r| r.unwrap()).collect();
        assert!(idxs.contains(&"idx_sessions_repo_remote".to_string()));
        assert!(idxs.contains(&"idx_sessions_repo_root".to_string()));

        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(v, 3);
    }

    #[test]
    fn migrations_are_idempotent_across_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        apply(&mut conn).unwrap(); // second call must be a no-op
        let v: i32 = conn.query_row(
            "SELECT MAX(version) FROM schema_version", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(v, 3);
    }
}
```

- [ ] **Step 4: Run test**

```powershell
cargo test -p andon --lib db::migrations
```

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/db/migrations.rs
git commit -m "feat(db): migration v3 — add cwd/repo_root/repo_remote/repo_branch/repo_name to sessions"
```

---

### Task 2: `git_query` module — shell out to git with timeout, normalize remote

**Files:**
- Create: `src-tauri/src/git_query.rs`
- Modify: `src-tauri/src/lib.rs` (register module)

- [ ] **Step 1: Scaffold the module**

Create `src-tauri/src/git_query.rs`:

```rust
//! Run `git` from a given working directory and return repo metadata.
//! Each subcommand has a hard 2-second timeout so hung git never blocks
//! a tokio task indefinitely.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const GIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoInfo {
    pub repo_root: Option<PathBuf>,
    pub repo_remote: Option<String>, // normalized
    pub repo_branch: Option<String>,
    pub repo_name: Option<String>,
}

/// Run `git <args>` from `cwd` with a hard timeout. Returns trimmed stdout
/// on success, None on any failure (non-zero exit, timeout, spawn error).
async fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let cwd = cwd.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let fut = async move {
        let out = Command::new("git")
            .args(&args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?;
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    };
    match timeout(GIT_TIMEOUT, fut).await {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!(?cwd, "git timed out");
            None
        }
    }
}

pub async fn query_repo(cwd: &Path) -> RepoInfo {
    let toplevel = run_git(cwd, &["rev-parse", "--show-toplevel"]).await
        .map(PathBuf::from);
    let remote_raw = run_git(cwd, &["config", "--get", "remote.origin.url"]).await;
    let branch = run_git(cwd, &["branch", "--show-current"]).await;
    let remote = remote_raw.as_deref().map(normalize_remote);
    let name = compute_repo_name(remote.as_deref(), toplevel.as_deref(), cwd);
    RepoInfo {
        repo_root: toplevel,
        repo_remote: remote,
        repo_branch: branch,
        repo_name: Some(name),
    }
}

/// Normalize a git remote URL to host/org/repo form, lowercased host, no `.git` suffix.
/// Examples:
///   https://github.com/Foo/Bar.git -> github.com/Foo/Bar
///   git@github.com:Foo/Bar.git     -> github.com/Foo/Bar
///   ssh://git@github.com/Foo/Bar   -> github.com/Foo/Bar
pub fn normalize_remote(raw: &str) -> String {
    let raw = raw.trim();
    // Strip leading scheme + auth.
    let no_scheme = raw
        .strip_prefix("https://").or_else(|| raw.strip_prefix("http://"))
        .or_else(|| raw.strip_prefix("ssh://"))
        .unwrap_or(raw);
    let no_auth = no_scheme
        .strip_prefix("git@")
        .unwrap_or(no_scheme);
    // SCP form: host:org/repo -> host/org/repo (only the first colon).
    let slashed = if let Some(idx) = no_auth.find(':') {
        let (host, rest) = no_auth.split_at(idx);
        format!("{}/{}", host, &rest[1..])
    } else {
        no_auth.to_string()
    };
    // Strip trailing .git.
    let no_git = slashed.strip_suffix(".git").unwrap_or(&slashed).to_string();
    // Lowercase host portion only (first segment).
    if let Some(first_slash) = no_git.find('/') {
        let (host, rest) = no_git.split_at(first_slash);
        format!("{}{}", host.to_lowercase(), rest)
    } else {
        no_git.to_lowercase()
    }
}

/// Derive a display name for the repo. Prefers org/repo from the remote;
/// otherwise basename of repo_root; otherwise basename of cwd.
pub fn compute_repo_name(remote: Option<&str>, repo_root: Option<&Path>, cwd: &Path) -> String {
    if let Some(r) = remote {
        // Take the last two segments, e.g. github.com/foo/bar -> foo/bar.
        let parts: Vec<&str> = r.split('/').collect();
        if parts.len() >= 3 {
            return format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
        }
        return r.to_string();
    }
    let base = |p: &Path| {
        p.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    if let Some(rr) = repo_root {
        let b = base(rr);
        if !b.is_empty() { return b; }
    }
    base(cwd)
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add alongside the other `mod` statements:

```rust
pub mod git_query;
```

(If `lib.rs` uses `mod` rather than `pub mod` for sibling modules, match the surrounding style.)

- [ ] **Step 3: Write unit tests**

Append to `src-tauri/src/git_query.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn normalize_https() {
        assert_eq!(normalize_remote("https://github.com/Foo/Bar.git"), "github.com/Foo/Bar");
    }

    #[test]
    fn normalize_ssh_scp() {
        assert_eq!(normalize_remote("git@github.com:Foo/Bar.git"), "github.com/Foo/Bar");
    }

    #[test]
    fn normalize_ssh_url() {
        assert_eq!(normalize_remote("ssh://git@github.com/Foo/Bar"), "github.com/Foo/Bar");
    }

    #[test]
    fn normalize_no_dotgit() {
        assert_eq!(normalize_remote("https://gitlab.com/team/proj"), "gitlab.com/team/proj");
    }

    #[test]
    fn normalize_lowercases_host_only() {
        assert_eq!(normalize_remote("https://GITHUB.COM/Foo/Bar"), "github.com/Foo/Bar");
    }

    #[test]
    fn name_from_remote() {
        assert_eq!(
            compute_repo_name(Some("github.com/satish-krishna/andon"), None, &PathBuf::from("/tmp")),
            "satish-krishna/andon"
        );
    }

    #[test]
    fn name_from_root_when_no_remote() {
        assert_eq!(
            compute_repo_name(None, Some(&PathBuf::from("/tmp/andon")), &PathBuf::from("/tmp/andon/sub")),
            "andon"
        );
    }

    #[test]
    fn name_from_cwd_when_no_root() {
        assert_eq!(
            compute_repo_name(None, None, &PathBuf::from("/tmp/scratch")),
            "scratch"
        );
    }

    #[tokio::test]
    async fn query_repo_returns_none_for_non_git_dir() {
        let tmp = std::env::temp_dir().join(format!("andon-test-{}", uuid_like()));
        std::fs::create_dir_all(&tmp).unwrap();
        let info = query_repo(&tmp).await;
        assert!(info.repo_root.is_none(), "expected None for non-git dir, got {:?}", info.repo_root);
        std::fs::remove_dir_all(&tmp).ok();
    }

    fn uuid_like() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }
}
```

- [ ] **Step 4: Run tests**

```powershell
cargo test -p andon --lib git_query
```

Expected: all unit tests PASS. The async test PASSes only if `git` is available on PATH and the temp dir is not a git repo.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/git_query.rs src-tauri/src/lib.rs
git commit -m "feat: git_query module with 2s timeout + remote URL normalization"
```

---

### Task 3: `repo_inference` module — derive repo from per-session file paths

**Files:**
- Create: `src-tauri/src/repo_inference.rs`
- Modify: `src-tauri/src/lib.rs` (register module)

- [ ] **Step 1: Scaffold the module**

Create `src-tauri/src/repo_inference.rs`:

```rust
//! Best-effort repo inference for sessions that were captured before the
//! SessionStart hook landed (or where the hook didn't fire). Collects the
//! per-session file paths, finds their longest common ancestor, and walks
//! up looking for a .git directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use rusqlite::params;

use crate::db::DbPool;
use crate::git_query::{RepoInfo, query_repo};

/// Longest path that is an ancestor of every input path. Returns None when
/// the input is empty or the paths share no common root.
pub fn longest_common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut iter = paths.iter();
    let first = iter.next()?;
    let mut acc: Vec<&std::ffi::OsStr> = first.components().map(|c| c.as_os_str()).collect();
    for p in iter {
        let parts: Vec<&std::ffi::OsStr> = p.components().map(|c| c.as_os_str()).collect();
        let common = acc.iter().zip(parts.iter()).take_while(|(a, b)| a == b).count();
        acc.truncate(common);
        if acc.is_empty() { return None; }
    }
    let mut out = PathBuf::new();
    for part in acc { out.push(part); }
    Some(out)
}

/// Walk up from `start` looking for a directory containing `.git`. Returns
/// the deepest such ancestor, or None.
pub fn find_git_ancestor(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Pull every absolute file_path for the session out of file_changes and
/// tool_decisions, find the LCA, walk up for .git, and run git_query
/// against the discovered ancestor.
pub async fn infer_repo_for_session(
    pool: Arc<DbPool>,
    session_id: String,
) -> Result<Option<RepoInfo>> {
    let paths: Vec<PathBuf> = tokio::task::spawn_blocking({
        let pool = pool.clone();
        let sid = session_id.clone();
        move || -> Result<Vec<PathBuf>> {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT file_path FROM file_changes WHERE session_id = ?1 AND file_path IS NOT NULL
                 UNION
                 SELECT file_path FROM tool_decisions WHERE session_id = ?1 AND file_path IS NOT NULL"
            )?;
            let rows = stmt.query_map(params![sid], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                let s = r?;
                let p = PathBuf::from(&s);
                if p.is_absolute() { out.push(p); }
            }
            Ok(out)
        }
    }).await??;

    if paths.is_empty() {
        return Ok(None);
    }

    let lca = match longest_common_ancestor(&paths) {
        Some(p) => p,
        None => return Ok(None),
    };

    let start = if lca.is_dir() {
        lca.clone()
    } else {
        // LCA might be a file or a non-existent ancestor; walk up to a real dir.
        let mut cur = lca.as_path();
        while !cur.is_dir() {
            match cur.parent() {
                Some(p) => cur = p,
                None => return Ok(None),
            }
        }
        cur.to_path_buf()
    };

    let git_root = find_git_ancestor(&start).unwrap_or(start);
    let info = query_repo(&git_root).await;
    if info.repo_root.is_some() || info.repo_remote.is_some() {
        Ok(Some(info))
    } else {
        // Non-git folder — still return repo_root as the LCA so the column
        // is non-NULL after inference.
        Ok(Some(RepoInfo {
            repo_root: Some(git_root.clone()),
            repo_name: Some(crate::git_query::compute_repo_name(None, Some(&git_root), &git_root)),
            ..Default::default()
        }))
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add:

```rust
pub mod repo_inference;
```

- [ ] **Step 3: Write unit tests**

Append to `src-tauri/src/repo_inference.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn lca_single_folder() {
        let paths = vec![
            p("/repo/src/a.rs"),
            p("/repo/src/b.rs"),
            p("/repo/Cargo.toml"),
        ];
        assert_eq!(longest_common_ancestor(&paths), Some(p("/repo")));
    }

    #[test]
    fn lca_multi_folder() {
        let paths = vec![p("/a/x"), p("/b/y")];
        // Both start with the root component, so LCA is "/".
        // On Windows the equivalent paths wouldn't share a root; that's fine.
        assert_eq!(longest_common_ancestor(&paths), Some(p("/")));
    }

    #[test]
    fn lca_empty_returns_none() {
        assert_eq!(longest_common_ancestor(&[]), None);
    }

    #[test]
    fn find_git_ancestor_missing() {
        let tmp = std::env::temp_dir().join(format!("andon-inf-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(find_git_ancestor(&tmp), None);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn find_git_ancestor_present() {
        let tmp = std::env::temp_dir().join(format!("andon-inf-git-{}", std::process::id()));
        let sub  = tmp.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        assert_eq!(find_git_ancestor(&sub), Some(tmp.clone()));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 4: Run tests**

```powershell
cargo test -p andon --lib repo_inference
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/repo_inference.rs src-tauri/src/lib.rs
git commit -m "feat: repo_inference — LCA + .git ancestor walk for session backfill"
```

---

## Phase 2 — API endpoint + integration patcher

### Task 4: `POST /api/session/context` — synchronous persist, async git enrichment

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/src/api/dto.rs`

- [ ] **Step 1: Read the existing hook handler for the pattern**

Read `src-tauri/src/api/routes.rs` from line 817 (the `// ---------- session-end hook + reports ----------` section) so the new handler mirrors structure, error handling, and `tokio::task::spawn_blocking` style.

- [ ] **Step 2: Add the DTO**

In `src-tauri/src/api/dto.rs`, add:

```rust
#[derive(serde::Deserialize, Debug)]
pub struct SessionContextPayload {
    pub session_id: String,
    pub cwd: Option<String>,
    // Tolerated and ignored:
    #[serde(default)] pub source: Option<String>,
    #[serde(default)] pub transcript_path: Option<String>,
    #[serde(default)] pub hook_event_name: Option<String>,
    #[serde(default)] pub model: Option<String>,
}
```

- [ ] **Step 3: Register the route**

In `src-tauri/src/api/routes.rs` near line 47 (where `/api/hooks/tool-use` is registered), add:

```rust
.route("/api/session/context", post(hook_session_context))
```

- [ ] **Step 4: Add the handler**

Append in the hook section (near `hook_session_end`):

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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0);
    let cwd_str = p.cwd.clone().unwrap_or_default();

    // 1) Persist cwd synchronously. Never overwrite an existing non-NULL value.
    {
        let pool = state.pool.clone();
        let sid = sid.clone();
        let cwd_str = cwd_str.clone();
        tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE session_id = ?1",
                params![sid], |r| r.get(0),
            ).unwrap_or(0);
            if exists == 0 {
                conn.execute(
                    "INSERT INTO sessions (session_id, started_at, cwd) VALUES (?1, ?2, ?3)",
                    params![sid, now, &cwd_str],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions SET cwd = COALESCE(cwd, ?2) WHERE session_id = ?1",
                    params![sid, &cwd_str],
                )?;
            }
            Ok(())
        }).await.ok();
    }

    // 2) Enrich asynchronously — git queries don't block the hook.
    if !cwd_str.is_empty() {
        let pool = state.pool.clone();
        let sid_for_task = sid.clone();
        let cwd_path = std::path::PathBuf::from(&cwd_str);
        tokio::spawn(async move {
            let info = crate::git_query::query_repo(&cwd_path).await;
            let pool_inner = pool.clone();
            let sid_inner  = sid_for_task.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let Ok(conn) = pool_inner.get() else { return };
                let root  = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
                let rem   = info.repo_remote.clone();
                let brn   = info.repo_branch.clone();
                let name  = info.repo_name.clone();
                let _ = conn.execute(
                    "UPDATE sessions
                       SET repo_root   = COALESCE(repo_root,   ?2),
                           repo_remote = COALESCE(repo_remote, ?3),
                           repo_branch = COALESCE(repo_branch, ?4),
                           repo_name   = COALESCE(repo_name,   ?5)
                     WHERE session_id = ?1",
                    params![sid_inner, root, rem, brn, name],
                );
            }).await;
        });
    }

    Ok(Json(json!({ "ok": true })))
}
```

- [ ] **Step 5: Write integration test**

Add (or extend) an integration test in `src-tauri/src/api/routes.rs` `#[cfg(test)]` module (look for the existing test module; if there isn't one, create it). Tests must:

```rust
#[tokio::test]
async fn session_context_rejects_missing_session_id() {
    // Build a test ApiState with an in-memory pool (mirror what the
    // existing tests do — search the file for `apply(` and copy the
    // pattern). POST {} and assert 400.
}

#[tokio::test]
async fn session_context_inserts_stub_row_with_cwd() {
    // POST { session_id: "s1", cwd: "/tmp/whatever" }
    // Assert row exists with cwd="/tmp/whatever" and started_at != 0.
}

#[tokio::test]
async fn session_context_does_not_overwrite_cwd() {
    // INSERT a row with cwd="/orig", POST { session_id, cwd: "/new" }.
    // Assert cwd is still "/orig".
}
```

If no test harness exists yet in this file, scaffold it using `tower::ServiceExt::oneshot` on the axum `Router`.

- [ ] **Step 6: Run tests**

```powershell
cargo test -p andon --lib api::routes
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs
git commit -m "feat(api): POST /api/session/context — sync cwd, async git enrichment"
```

---

### Task 5: SessionStart hook in integration patcher

**Files:**
- Modify: `src-tauri/src/integration.rs`

- [ ] **Step 1: Add the constants**

Near the existing `SESSION_END_COMMAND` (line 23 area), add:

```rust
const SESSION_START_COMMAND: &str =
    "curl -s -X POST http://127.0.0.1:8765/api/session/context -H \"Content-Type: application/json\" --data-binary @-";
const SESSION_START_MARKER: &str = "/api/session/context";
```

- [ ] **Step 2: Add the three helpers**

Mirror `has_session_end_hook` / `install_session_end_hook` / `remove_session_end_hook` (lines 236-287). Add directly below them:

```rust
fn has_session_start_hook(value: &Value) -> bool {
    let Some(arr) = value
        .get("hooks").and_then(|h| h.get("SessionStart")).and_then(|a| a.as_array())
    else { return false };
    for entry in arr {
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            for h in hooks {
                if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                    if cmd.contains(SESSION_START_MARKER) { return true; }
                }
            }
        }
    }
    false
}

fn install_session_start_hook(value: &mut Value) {
    if has_session_start_hook(value) { return; }
    let Some(obj) = value.as_object_mut() else { return };
    let hooks = obj.entry("hooks".to_string()).or_insert_with(|| json!({}));
    if !hooks.is_object() { *hooks = json!({}); }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let arr = hooks_obj
        .entry("SessionStart".to_string())
        .or_insert_with(|| json!([]));
    if !arr.is_array() { *arr = json!([]); }
    arr.as_array_mut().unwrap().push(json!({
        "matcher": "",
        "hooks": [{ "type": "command", "command": SESSION_START_COMMAND }]
    }));
}

fn remove_session_start_hook(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else { return false };
    let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) else { return false };
    let Some(arr) = hooks.get_mut("SessionStart").and_then(|a| a.as_array_mut()) else { return false };
    let before = arr.len();
    arr.retain(|entry| {
        let has = entry.get("hooks").and_then(|h| h.as_array()).map(|inner| {
            inner.iter().any(|h| h.get("command").and_then(|c| c.as_str())
                .map(|s| s.contains(SESSION_START_MARKER)).unwrap_or(false))
        }).unwrap_or(false);
        !has
    });
    let removed = arr.len() < before;
    if arr.is_empty() { hooks.remove("SessionStart"); }
    if hooks.is_empty() { obj.remove("hooks"); }
    removed
}
```

- [ ] **Step 3: Wire into `try_ensure`**

In `try_ensure` (line 45 area), change the `hook_installed` line:

```rust
    let hook_installed = has_our_hook(&existing)
        && has_session_end_hook(&existing)
        && has_session_start_hook(&existing);
```

And after `install_session_end_hook(&mut merged_val);` (line 128), add:

```rust
    install_session_start_hook(&mut merged_val);
```

- [ ] **Step 4: Wire into `unpatch_claude_settings`**

After `remove_session_end_hook(&mut value)` (line 317 area), add:

```rust
    if remove_session_start_hook(&mut value) {
        removed_any = true;
    }
```

- [ ] **Step 5: Add unit tests**

Look for an existing `#[cfg(test)]` block in `integration.rs`. If present, add cases. If absent, scaffold one:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn install_into_empty_settings() {
        let mut v = json!({});
        install_session_start_hook(&mut v);
        assert!(has_session_start_hook(&v));
    }

    #[test]
    fn install_is_idempotent() {
        let mut v = json!({});
        install_session_start_hook(&mut v);
        install_session_start_hook(&mut v);
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn install_preserves_unrelated_session_start_hook() {
        let mut v = json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "",
                    "hooks": [{ "type": "command", "command": "echo someone-elses-hook" }]
                }]
            }
        });
        install_session_start_hook(&mut v);
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn remove_only_ours() {
        let mut v = json!({
            "hooks": {
                "SessionStart": [
                    { "matcher": "", "hooks": [{ "type": "command", "command": "echo theirs" }] }
                ]
            }
        });
        install_session_start_hook(&mut v);
        assert!(remove_session_start_hook(&mut v));
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert!(arr[0]["hooks"][0]["command"].as_str().unwrap().contains("theirs"));
    }

    #[test]
    fn remove_cleans_up_empty_containers() {
        let mut v = json!({});
        install_session_start_hook(&mut v);
        assert!(remove_session_start_hook(&mut v));
        assert!(v.get("hooks").is_none());
    }
}
```

- [ ] **Step 6: Run tests**

```powershell
cargo test -p andon --lib integration::tests
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```powershell
git add src-tauri/src/integration.rs
git commit -m "feat(integration): install/remove SessionStart hook for /api/session/context"
```

---

### Task 5.5: Run inference at session-end

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (extend `hook_session_end`)

- [ ] **Step 1: Read the existing session-end handler**

```powershell
# Already shown above. Look at the `tokio::spawn(async move { ... })` block
# that generates the report — we'll add a sibling spawn for inference.
```

- [ ] **Step 2: Spawn an inference task when repo_root is still NULL**

Inside `hook_session_end`, after the report-generation `tokio::spawn` (around the line that calls `crate::reports::generate_report`), add:

```rust
    // Best-effort repo inference for sessions the hook didn't cover.
    let pool_for_inf = state.pool.clone();
    let sid_for_inf = sid.clone();
    tokio::spawn(async move {
        // Skip if repo_root is already populated.
        let needs = {
            let pool = pool_for_inf.clone();
            let sid = sid_for_inf.clone();
            tokio::task::spawn_blocking(move || -> bool {
                let Ok(conn) = pool.get() else { return false };
                conn.query_row(
                    "SELECT repo_root IS NULL FROM sessions WHERE session_id = ?1",
                    params![sid], |r| r.get::<_, i64>(0)
                ).map(|n| n != 0).unwrap_or(false)
            }).await.unwrap_or(false)
        };
        if !needs { return; }

        let Ok(Some(info)) = crate::repo_inference::infer_repo_for_session(
            pool_for_inf.clone(), sid_for_inf.clone()
        ).await else { return };

        let pool = pool_for_inf.clone();
        let sid  = sid_for_inf.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let Ok(conn) = pool.get() else { return };
            let root = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
            let _ = conn.execute(
                "UPDATE sessions
                   SET repo_root   = COALESCE(repo_root,   ?2),
                       repo_remote = COALESCE(repo_remote, ?3),
                       repo_branch = COALESCE(repo_branch, ?4),
                       repo_name   = COALESCE(repo_name,   ?5)
                 WHERE session_id = ?1",
                params![sid, root, info.repo_remote, info.repo_branch, info.repo_name],
            );
        }).await;
    });
```

- [ ] **Step 3: Build**

```powershell
cargo build -p andon
```

Expected: succeeds.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): run repo inference at session-end when repo_root is NULL"
```

---

### Task 6: Backfill endpoint — `POST /api/repo/backfill`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/src/api/dto.rs`

- [ ] **Step 1: Add a response DTO**

In `src-tauri/src/api/dto.rs`:

```rust
#[derive(serde::Serialize)]
pub struct BackfillResult {
    pub scanned: usize,
    pub updated: usize,
}
```

- [ ] **Step 2: Register the route**

Add near the other `/api/...` routes:

```rust
.route("/api/repo/backfill", post(repo_backfill))
```

- [ ] **Step 3: Add the handler**

```rust
async fn repo_backfill(
    State(state): State<ApiState>,
) -> Json<crate::api::dto::BackfillResult> {
    // Collect candidate session ids (repo_root still NULL).
    let pool = state.pool.clone();
    let ids: Vec<String> = match tokio::task::spawn_blocking({
        let pool = pool.clone();
        move || -> rusqlite::Result<Vec<String>> {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let mut stmt = conn.prepare(
                "SELECT session_id FROM sessions WHERE repo_root IS NULL ORDER BY started_at DESC"
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        }
    }).await {
        Ok(Ok(v)) => v,
        _ => return Json(crate::api::dto::BackfillResult { scanned: 0, updated: 0 }),
    };

    let scanned = ids.len();
    let mut updated = 0usize;
    for sid in ids {
        let Ok(Some(info)) = crate::repo_inference::infer_repo_for_session(
            pool.clone(), sid.clone()
        ).await else { continue };

        let pool_inner = pool.clone();
        let written = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let conn = pool_inner.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let root = info.repo_root.as_ref().map(|p| p.to_string_lossy().into_owned());
            conn.execute(
                "UPDATE sessions
                   SET repo_root   = COALESCE(repo_root,   ?2),
                       repo_remote = COALESCE(repo_remote, ?3),
                       repo_branch = COALESCE(repo_branch, ?4),
                       repo_name   = COALESCE(repo_name,   ?5)
                 WHERE session_id = ?1",
                params![sid, root, info.repo_remote, info.repo_branch, info.repo_name],
            )
        }).await.unwrap_or(Ok(0)).unwrap_or(0);
        if written > 0 { updated += 1; }
    }
    Json(crate::api::dto::BackfillResult { scanned, updated })
}
```

- [ ] **Step 4: Smoke-test manually**

Build and run:

```powershell
cargo build -p andon
# then with the app running:
curl -s -X POST http://127.0.0.1:8765/api/repo/backfill
```

Expected JSON: `{"scanned": N, "updated": M}` where N >= M >= 0.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs
git commit -m "feat(api): POST /api/repo/backfill — run inference for sessions missing repo info"
```

---

## Phase 3 — API read-side: project repo, list repos, top repos

### Task 7: Add repo fields to session list + detail responses

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/src/api/dto.rs`
- Modify: `web/src/app/core/models.ts`

- [ ] **Step 1: Find the session-list and session-detail DTOs**

In `src-tauri/src/api/dto.rs`, locate the structs returned by `/api/v2/sessions` and `/api/sessions/:id` (typically `V2Session` and `SessionDetail`). Add fields:

```rust
#[serde(default)] pub cwd: Option<String>,
#[serde(default)] pub repo_root: Option<String>,
#[serde(default)] pub repo_remote: Option<String>,
#[serde(default)] pub repo_branch: Option<String>,
#[serde(default)] pub repo_name: Option<String>,
```

- [ ] **Step 2: Project the columns in the SQL**

Find every `SELECT ... FROM sessions` in `src-tauri/src/api/routes.rs` that builds a session-list or session-detail response and append the five new columns to the select list. Order: `cwd, repo_root, repo_remote, repo_branch, repo_name`. Wire them through `rusqlite::Row::get::<_, Option<String>>(idx)` calls in the row-mapping closures.

- [ ] **Step 3: Mirror in the TypeScript model**

In `web/src/app/core/models.ts`, find the session interfaces (`V2Session`, `SessionSummary`, `SessionDetail`) and add:

```ts
cwd?: string | null;
repo_root?: string | null;
repo_remote?: string | null;
repo_branch?: string | null;
repo_name?: string | null;
```

- [ ] **Step 4: Verify**

```powershell
cargo build -p andon
cd web; npm run build; cd ..
```

Expected: both succeed.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs web/src/app/core/models.ts
git commit -m "feat(api+models): project repo_* columns on session list and detail"
```

---

### Task 8: `GET /api/repos` and `GET /api/overview/top-repos`

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/src/api/dto.rs`

- [ ] **Step 1: Add the DTOs**

```rust
#[derive(serde::Serialize)]
pub struct RepoSummary {
    /// Grouping key: COALESCE(repo_remote, repo_root, cwd, '—').
    pub key: String,
    /// Display label: repo_name (preferred), else basename of key.
    pub label: String,
    /// `true` when repo_remote is set.
    pub has_remote: bool,
    pub session_count: i64,
}

#[derive(serde::Serialize)]
pub struct TopRepoEntry {
    pub key: String,
    pub label: String,
    pub cost_usd: f64,
    pub session_count: i64,
    /// Daily cost series for the period, oldest-first, same length as the period.
    pub spark: Vec<f64>,
}
```

- [ ] **Step 2: Register the routes**

```rust
.route("/api/repos", get(list_repos))
.route("/api/overview/top-repos", get(overview_top_repos))
```

- [ ] **Step 3: `list_repos` — used by the REPO filter chips**

```rust
#[derive(Deserialize)]
struct ListReposQuery {
    #[serde(default)] from: Option<i64>,
    #[serde(default)] to:   Option<i64>,
    #[serde(default)] limit: Option<usize>,
}

async fn list_repos(
    State(state): State<ApiState>,
    Query(q): Query<ListReposQuery>,
) -> Json<Vec<crate::api::dto::RepoSummary>> {
    let limit = q.limit.unwrap_or(50);
    let from = q.from.unwrap_or(0);
    let to   = q.to.unwrap_or(i64::MAX);
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<crate::api::dto::RepoSummary>> {
        let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(repo_remote, repo_root, cwd, '—') AS k,
                COALESCE(repo_name,
                         CASE WHEN repo_remote IS NOT NULL THEN repo_remote ELSE NULL END,
                         repo_root, cwd, '—') AS label,
                CASE WHEN repo_remote IS NOT NULL THEN 1 ELSE 0 END AS has_remote,
                COUNT(*) AS n
             FROM sessions
             WHERE started_at BETWEEN ?1 AND ?2
             GROUP BY k
             ORDER BY n DESC
             LIMIT ?3"
        )?;
        let rows = stmt.query_map(params![from, to, limit as i64], |r| {
            Ok(crate::api::dto::RepoSummary {
                key: r.get::<_, String>(0)?,
                label: r.get::<_, String>(1)?,
                has_remote: r.get::<_, i64>(2)? != 0,
                session_count: r.get::<_, i64>(3)?,
            })
        })?;
        rows.collect()
    }).await.unwrap_or(Ok(vec![])).unwrap_or_default();
    Json(rows)
}
```

- [ ] **Step 4: `overview_top_repos` — used by the Overview tile**

```rust
#[derive(Deserialize)]
struct TopReposQuery {
    from: i64,
    to: i64,
    #[serde(default)] limit: Option<usize>,
}

async fn overview_top_repos(
    State(state): State<ApiState>,
    Query(q): Query<TopReposQuery>,
) -> Json<Vec<crate::api::dto::TopRepoEntry>> {
    let limit = q.limit.unwrap_or(5);
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<crate::api::dto::TopRepoEntry>> {
        let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;

        // 1) Top-N repos by cost.
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') AS k,
                COALESCE(s.repo_name, s.repo_remote, s.repo_root, s.cwd, '—') AS label,
                SUM(c.cost_usd) AS cost,
                COUNT(DISTINCT s.session_id) AS n
             FROM sessions s
             JOIN cost_entries c ON c.session_id = s.session_id
             WHERE s.started_at BETWEEN ?1 AND ?2
             GROUP BY k
             ORDER BY cost DESC
             LIMIT ?3"
        )?;
        let summary: Vec<(String, String, f64, i64)> =
            stmt.query_map(params![q.from, q.to, limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?.collect::<rusqlite::Result<_>>()?;

        // 2) Per-day cost per repo for sparklines.
        // Compute the day count once.
        let day_ms = 86_400_000i64;
        let days = ((q.to - q.from) / day_ms).max(1) as usize;

        let mut out = Vec::with_capacity(summary.len());
        for (k, label, cost, n) in summary {
            let mut spark = vec![0.0; days];
            let mut sp = conn.prepare(
                "SELECT (c.timestamp - ?2) / ?3 AS day_idx, SUM(c.cost_usd)
                 FROM cost_entries c
                 JOIN sessions s ON s.session_id = c.session_id
                 WHERE c.timestamp BETWEEN ?2 AND ?4
                   AND COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') = ?1
                 GROUP BY day_idx"
            )?;
            let rows = sp.query_map(params![k, q.from, day_ms, q.to], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (idx, v) = row?;
                if (0..spark.len() as i64).contains(&idx) {
                    spark[idx as usize] = v;
                }
            }
            out.push(crate::api::dto::TopRepoEntry { key: k, label, cost_usd: cost, session_count: n, spark });
        }
        Ok(out)
    }).await.unwrap_or(Ok(vec![])).unwrap_or_default();
    Json(rows)
}
```

- [ ] **Step 5: Smoke-test**

```powershell
cargo run -p andon
# then in another shell:
curl -s "http://127.0.0.1:8765/api/repos?limit=10"
curl -s "http://127.0.0.1:8765/api/overview/top-repos?from=0&to=99999999999999&limit=5"
```

Expected: arrays of objects with the documented shape.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/api/routes.rs src-tauri/src/api/dto.rs
git commit -m "feat(api): GET /api/repos and GET /api/overview/top-repos"
```

---

### Task 9: Accept `repo` filter on existing session/file endpoints

**Files:**
- Modify: `src-tauri/src/api/routes.rs`

- [ ] **Step 1: Extend the query structs**

Find the `Query` deserialization structs for the v2 sessions and v2 files endpoints (search for `V2Sessions` or similar). Add:

```rust
#[serde(default)] pub repo: Vec<String>, // multi-select
```

- [ ] **Step 2: Wire the filter into SQL**

For each affected endpoint, build a `WHERE COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') IN (?, ?, ...)` clause when `repo` is non-empty. Use rusqlite's parameter slicing: build the placeholder string at runtime and pass an `[&dyn ToSql]` slice.

Pattern:

```rust
let mut sql = String::from("SELECT ... FROM sessions s WHERE 1=1");
let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![];
if !q.repo.is_empty() {
    let placeholders = (1..=q.repo.len()).map(|i| format!("?{}", params_vec.len() + i)).collect::<Vec<_>>().join(",");
    sql.push_str(&format!(" AND COALESCE(s.repo_remote, s.repo_root, s.cwd, '—') IN ({})", placeholders));
    for r in &q.repo {
        params_vec.push(Box::new(r.clone()));
    }
}
// ... continue building the rest of the WHERE clause similarly.
let refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
let mut stmt = conn.prepare(&sql)?;
let rows = stmt.query_map(refs.as_slice(), |r| { ... });
```

- [ ] **Step 3: Manually verify**

```powershell
cargo run -p andon
# Once running:
curl -s "http://127.0.0.1:8765/api/v2/sessions?repo=github.com/satish-krishna/andon&from=0&to=99999999999999"
```

Expected: only sessions whose repo identity matches.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): repo[] filter on /api/v2/sessions and /api/v2/files"
```

---

## Phase 4 — Frontend

> **Before each frontend task, read the file you're modifying first** so your edits follow the existing component's style (signals, naming, Tailwind class conventions).

### Task 10: Frontend models + API client wrappers

**Files:**
- Modify: `web/src/app/core/models.ts`
- Modify: `web/src/app/core/api.service.ts`
- Modify: `web/src/app/core/filter.service.ts`

- [ ] **Step 1: Add the model types**

In `models.ts`, add:

```ts
export interface RepoSummary {
  key: string;
  label: string;
  has_remote: boolean;
  session_count: number;
}

export interface TopRepoEntry {
  key: string;
  label: string;
  cost_usd: number;
  session_count: number;
  spark: number[];
}

export interface BackfillResult {
  scanned: number;
  updated: number;
}
```

- [ ] **Step 2: Add API methods**

In `api.service.ts`, alongside the existing typed methods:

```ts
listRepos(args: { from?: number; to?: number; limit?: number }) {
  return this.http.get<RepoSummary[]>(`${BASE}/api/repos`, { params: toParams(args) });
}

topRepos(args: { from: number; to: number; limit?: number }) {
  return this.http.get<TopRepoEntry[]>(`${BASE}/api/overview/top-repos`, { params: toParams(args) });
}

backfillRepos() {
  return this.http.post<BackfillResult>(`${BASE}/api/repo/backfill`, {});
}
```

If `toParams` doesn't currently handle `repo` arrays, extend it to emit `repo=a&repo=b` (Angular's `HttpParams` supports this with `appendAll`).

- [ ] **Step 3: Add `repos` to shared filter state**

In `filter.service.ts`, add:

```ts
readonly repos = signal<string[]>([]);
```

Include `repos` in any `asParams()` / query-marshalling helper exported by this file so that downstream API calls automatically pick it up.

- [ ] **Step 4: Build**

```powershell
cd web; npm run build; cd ..
```

Expected: build succeeds, no type errors.

- [ ] **Step 5: Commit**

```powershell
git add web/src/app/core/
git commit -m "feat(web/core): models + API client for repo endpoints; repos filter signal"
```

---

### Task 11: Sessions page — REPO column + filter chips + delete the old placeholder

**Files:**
- Modify: `web/src/app/features/sessions/sessions.component.ts`
- Modify: `web/src/app/features/sessions/sessions.component.html`

- [ ] **Step 1: Read the component**

```powershell
cat web/src/app/features/sessions/sessions.component.ts
cat web/src/app/features/sessions/sessions.component.html
```

Find:
- The chip-group block for MODEL filters (template).
- Where the SESSION column header / cell is defined.
- The placeholder text "REPO — NOT EMITTED BY CLAUDE CODE (FILTER BY SESSION INSTEAD)".

- [ ] **Step 2: Replace the placeholder with a real REPO chip group**

In the template, replace the `REPO — NOT EMITTED…` block with a chip group that mirrors the MODEL chips, bound to a `repoOptions = signal<RepoSummary[]>([])` populated in `ngOnInit` via `apiService.listRepos(...)`. Multi-select; toggling a chip pushes/removes from the shared `filterService.repos` signal.

- [ ] **Step 3: Add the REPO column**

Insert a new `<th>` and `<td>` between SESSION and MODEL. Cell renders `session.repo_name ?? '—'`, with a `title` attribute set to `session.repo_root ?? session.cwd ?? ''`. When `repo_root` is set but `repo_remote` is null, append `(not git)` in a muted class (Tailwind `text-zinc-500 text-xs`).

- [ ] **Step 4: Verify in the browser**

Build the SPA, then with the app running serve `web/dist/web/browser` (as we did during the screenshot work):

```powershell
cd web; npm run build; cd ..
python -m http.server 8088 --bind 127.0.0.1 --directory web/dist/web/browser
# Open http://127.0.0.1:8088/#/sessions in a browser.
```

Expected: REPO column shows real values; chip group lists the repos; toggling chips filters the table.

- [ ] **Step 5: Commit**

```powershell
git add web/src/app/features/sessions/
git commit -m "feat(web/sessions): REPO column + filter chip group; remove not-emitted placeholder"
```

---

### Task 12: Session detail — header subtitle with repo + branch

**Files:**
- Modify: `web/src/app/features/sessions/session-detail.component.ts`

- [ ] **Step 1: Read the component**

```powershell
cat web/src/app/features/sessions/session-detail.component.ts
```

Find the header block that currently renders the session ID and timestamp.

- [ ] **Step 2: Add the subtitle**

Below the session ID, render `{{ session.repo_name }} · {{ session.repo_branch }}` when `repo_name` is set. Below that, render `<div class="text-xs text-zinc-500">{{ session.repo_root || session.cwd }}</div>`. When `repo_remote` is set, wrap the repo name in an `<a href="https://{{ session.repo_remote }}" target="_blank" rel="noopener">`.

When `repo_name` is null, render nothing — no placeholder.

- [ ] **Step 3: Verify**

Rebuild and load `/#/sessions/<id>` for a session that has repo info.

- [ ] **Step 4: Commit**

```powershell
git add web/src/app/features/sessions/session-detail.component.ts
git commit -m "feat(web/session-detail): repo · branch subtitle with optional remote link"
```

---

### Task 13: Files page — REPO filter + relative paths when a single repo is selected

**Files:**
- Modify: `web/src/app/features/files/files.component.ts`
- Modify: `web/src/app/features/files/files.component.html`

- [ ] **Step 1: Add the same REPO chip group**

Reuse the implementation from Task 11. Bind to the same shared `filterService.repos`.

- [ ] **Step 2: Compute the active repo_root for path-stripping**

Add a computed signal in `files.component.ts`:

```ts
private readonly singleRepoRoot = computed<string | null>(() => {
  const repos = this.filterService.repos();
  if (repos.length !== 1) return null;
  // Look up the selected repo's repo_root by joining against the rows we
  // already have. Fall back to the key itself if it's a path.
  const row = this.rows().find(r => r.repo_remote === repos[0] || r.repo_root === repos[0]);
  return row?.repo_root ?? (repos[0].startsWith('/') || /^[A-Z]:\\/.test(repos[0]) ? repos[0] : null);
});
```

(`rows()` is the existing files-list signal — name it to whatever the component uses.)

In the template, render `displayPath(row)`:

```ts
displayPath(row: { file_path: string }) {
  const root = this.singleRepoRoot();
  if (!root || !row.file_path?.startsWith(root)) return row.file_path;
  const rel = row.file_path.slice(root.length);
  return rel.replace(/^[/\\]/, '');
}
```

- [ ] **Step 3: Verify**

Rebuild, open `/#/files`. With no repo selected, paths are absolute. Select exactly one repo from the chip group — paths under that repo render relative.

- [ ] **Step 4: Commit**

```powershell
git add web/src/app/features/files/
git commit -m "feat(web/files): REPO chip filter; relative paths when one repo is selected"
```

---

### Task 14: Overview — TOP REPOS tile

**Files:**
- Create: `web/src/app/features/overview/top-repos-tile.component.ts`
- Modify: `web/src/app/features/overview/overview.component.html`
- Modify: `web/src/app/features/overview/overview.component.ts`

- [ ] **Step 1: Scaffold the tile component**

```ts
import { Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { TopRepoEntry } from '../../core/models';

@Component({
  selector: 'app-top-repos-tile',
  standalone: true,
  imports: [CommonModule, RouterLink],
  template: `
    <section class="card">
      <header class="text-xs tracking-wider text-zinc-500 mb-3">TOP REPOS · PERIOD</header>
      @if (entries().length === 0) {
        <div class="text-sm text-zinc-500">No repo-attributed cost in this period.</div>
      } @else {
        <ul class="space-y-2">
          @for (e of entries(); track e.key) {
            <li class="flex items-center gap-3">
              <a [routerLink]="['/sessions']" [queryParams]="{ repo: e.key }"
                 class="flex-1 truncate text-sm hover:underline">{{ e.label }}</a>
              <svg class="h-6 w-16" viewBox="0 0 64 24" preserveAspectRatio="none">
                <polyline [attr.points]="sparkPoints(e.spark)" fill="none" stroke="currentColor" stroke-width="1" />
              </svg>
              <span class="w-20 text-right tabular-nums">{{ e.cost_usd | currency:'USD' }}</span>
            </li>
          }
        </ul>
      }
    </section>
  `,
})
export class TopReposTileComponent implements OnInit {
  from = input.required<number>();
  to   = input.required<number>();
  private api = inject(ApiService);
  readonly entries = signal<TopRepoEntry[]>([]);

  ngOnInit(): void {
    this.api.topRepos({ from: this.from(), to: this.to(), limit: 5 })
      .subscribe(rows => this.entries.set(rows));
  }

  sparkPoints(spark: number[]): string {
    if (!spark.length) return '';
    const max = Math.max(...spark, 1);
    const dx = 64 / Math.max(spark.length - 1, 1);
    return spark.map((v, i) => `${i * dx},${24 - (v / max) * 22}`).join(' ');
  }
}
```

(Class name `card` is the existing tile container — verify by reading another Overview tile and reuse whatever class it uses.)

- [ ] **Step 2: Slot the tile into Overview**

In `overview.component.ts`, add `import { TopReposTileComponent } from './top-repos-tile.component';` and include it in the component's `imports: [...]`.

In `overview.component.html`, place `<app-top-repos-tile [from]="rangeFrom()" [to]="rangeTo()" />` directly below the existing "COST BY MODEL · PERIOD" tile. (Names of the range signals should match what the Overview component already exposes — verify by reading the file.)

- [ ] **Step 3: Verify**

Rebuild, open `/#/overview`. Expect a Top Repos card listing the top 5 by cost in the active range, each row clickable and routing to `/sessions?repo=<key>`.

- [ ] **Step 4: Commit**

```powershell
git add web/src/app/features/overview/
git commit -m "feat(web/overview): Top Repos tile with sparkline + click-through to filtered Sessions"
```

---

### Task 15: Settings → Data — Backfill button

**Files:**
- Modify: `web/src/app/features/settings/settings.component.ts`
- Modify: `web/src/app/features/settings/settings.component.html`

- [ ] **Step 1: Add the action**

In `settings.component.ts`:

```ts
backfillResult = signal<{ scanned: number; updated: number } | null>(null);
backfilling = signal(false);

runBackfill() {
  this.backfilling.set(true);
  this.api.backfillRepos().subscribe({
    next: r => { this.backfillResult.set(r); this.backfilling.set(false); },
    error: () => this.backfilling.set(false),
  });
}
```

- [ ] **Step 2: Render in the Data section**

In `settings.component.html`, find the Data section (the one that shows "Database" path and row counts). Add a row:

```html
<div class="flex items-center justify-between py-2">
  <div>
    <div class="text-sm">Backfill repo info</div>
    <div class="text-xs text-zinc-500">Walk file paths for sessions without a repo and infer the repo root.</div>
  </div>
  <div class="flex items-center gap-3">
    @if (backfillResult(); as r) {
      <span class="text-xs text-zinc-500">{{ r.updated }}/{{ r.scanned }} updated</span>
    }
    <button class="btn" [disabled]="backfilling()" (click)="runBackfill()">
      {{ backfilling() ? 'Running…' : 'Backfill' }}
    </button>
  </div>
</div>
```

(Replace `btn` with the existing button class used by other Settings buttons — read the file first.)

- [ ] **Step 3: Verify**

Rebuild, open `/#/settings`, click "Backfill". Expect a status like `3/9 updated`. Refresh the Sessions page; sessions previously missing repo info now show one.

- [ ] **Step 4: Commit**

```powershell
git add web/src/app/features/settings/
git commit -m "feat(web/settings): Backfill repo info button under Data"
```

---

### Task 15.5: "Missing repo info" banner on Sessions and Files

**Files:**
- Modify: `src-tauri/src/api/routes.rs` (extend the session-list response with a coverage hint)
- Modify: `web/src/app/core/models.ts`
- Modify: `web/src/app/features/sessions/sessions.component.{ts,html}`
- Modify: `web/src/app/features/files/files.component.{ts,html}`

- [ ] **Step 1: Server — add coverage to the session list response**

In the v2 sessions list endpoint, after computing the result set, also run:

```rust
let (total, with_repo): (i64, i64) = {
    // same WHERE clause as the main query, minus any repo[] filter
    let mut stmt = conn.prepare(
        "SELECT COUNT(*), SUM(CASE WHEN repo_root IS NOT NULL OR repo_remote IS NOT NULL THEN 1 ELSE 0 END)
         FROM sessions WHERE started_at BETWEEN ?1 AND ?2"
    )?;
    stmt.query_row(params![from, to], |r| Ok((r.get(0)?, r.get(1)?)))?
};
```

Wrap the existing response in a new object:

```rust
#[derive(serde::Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<V2Session>,
    pub coverage: CoverageHint,
}

#[derive(serde::Serialize)]
pub struct CoverageHint {
    pub total: i64,
    pub with_repo: i64,
}
```

If the v2 sessions endpoint currently returns `Vec<V2Session>` directly, change the return type to `SessionListResponse` and update callers in `api.service.ts` accordingly.

- [ ] **Step 2: TypeScript types**

```ts
export interface CoverageHint { total: number; with_repo: number; }
export interface SessionListResponse {
  sessions: V2Session[];
  coverage: CoverageHint;
}
```

- [ ] **Step 3: Banner in the Sessions component**

Add a computed signal:

```ts
readonly missingRepoPct = computed(() => {
  const c = this.coverage();
  if (!c || c.total === 0) return 0;
  return 1 - c.with_repo / c.total;
});
```

In the template, just above the sessions table:

```html
@if (missingRepoPct() > 0.2) {
  <div class="rounded border border-amber-700/40 bg-amber-950/30 px-3 py-2 text-xs flex items-center justify-between mb-3">
    <span>{{ (missingRepoPct() * 100) | number:'1.0-0' }}% of sessions in view are missing repo info.</span>
    <span class="flex gap-2">
      <button class="underline" (click)="runBackfill()">Backfill from file paths</button>
      <a class="underline" routerLink="/settings" fragment="integration">Re-apply hook</a>
    </span>
  </div>
}
```

`runBackfill()` calls `apiService.backfillRepos()` and then re-fetches the session list.

- [ ] **Step 4: Mirror the banner on the Files page**

Use the same threshold. The Files page can compute coverage off its own row set (`rows().filter(r => r.repo_root || r.repo_remote).length / rows().length`) without needing a new server field.

- [ ] **Step 5: Verify**

Build + open Sessions in a state where >20% lack repo info (e.g. immediately after the migration, before re-apply). Confirm the banner appears; click Backfill; confirm the banner goes away after the call completes and the list re-fetches.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/api/ web/src/app/core/ web/src/app/features/sessions/ web/src/app/features/files/
git commit -m "feat(web): show banner when >20% of sessions in view are missing repo info"
```

---

## Phase 5 — Docs

### Task 16: README — note the SessionStart hook in Privacy

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Privacy section**

Find the existing Privacy bullets in `README.md` ("All ports bind to `127.0.0.1`…", "No outbound calls…", …) and add:

```
- Andon installs a Claude Code `SessionStart` hook (in addition to the existing `PostToolUse` and `SessionEnd` hooks) that POSTs the session id and working directory to `http://127.0.0.1:8765/api/session/context`. Git metadata (toplevel, remote, branch) is computed by Andon locally — git is invoked from the cwd you launched Claude Code from. Nothing leaves the machine.
```

- [ ] **Step 2: Commit**

```powershell
git add README.md
git commit -m "docs(README): document the SessionStart hook in Privacy"
```

---

## Self-review checklist (do not skip)

After completing all tasks:

- [ ] **Build clean:** `cargo build -p andon` and `cd web; npm run build` both succeed without warnings related to this feature.
- [ ] **All tests pass:** `cargo test -p andon` is green.
- [ ] **End-to-end sanity:**
  1. Re-apply integration from Settings → Integration (or wipe `~/.claude/settings.json`).
  2. Confirm the SessionStart hook appears in `~/.claude/settings.json`.
  3. Start a fresh Claude Code session from a git repo.
  4. Within a few seconds, the session appears in `/#/sessions` with the repo populated.
  5. Click into the session → repo · branch shows in the header.
  6. Filter Sessions by that repo → only matching sessions show.
  7. Overview → TOP REPOS tile lists the repo with non-zero cost.
  8. Settings → Backfill → returns a result (likely 0/0 if nothing's missing).
- [ ] **Unpatch works:** click Settings → Danger zone → Unpatch. Re-read `~/.claude/settings.json` — the SessionStart hook is gone, but other unrelated hooks remain.
- [ ] **Open the PR:**
  ```powershell
  git push -u origin repo-capture
  gh pr create --title "feat: session repo capture (hook + inference)" --body "@see docs/superpowers/specs/2026-05-17-session-repo-capture-design.md"
  ```
