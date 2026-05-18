# Session repo capture — design

**Status:** draft · awaiting user review
**Branch:** `repo-capture`
**Date:** 2026-05-17

## Problem

Claude Code does not emit the working directory or repository as part of its OTLP resource attributes. The Sessions and Files pages today carry a literal placeholder — "REPO — NOT EMITTED BY CLAUDE CODE (FILTER BY SESSION INSTEAD)" — because there is no way to attribute a session to the project it was run against. Users cannot filter sessions by repo, aggregate cost per repo, or even tell at a glance which checkout a session belonged to.

## Goal

Make repository (or, failing that, working directory) a first-class dimension on every session, so that the UI can **display** it on session rows, **filter** sessions and files by it, and **aggregate** cost across it.

## Non-goals (v1)

- A dedicated `/repos` page with per-repo breakdowns. Defer until users have enough repos to justify it.
- Cross-machine aggregation by repo remote. That belongs with the future team-mode work.
- Mid-session `cd` tracking. Claude Code sessions are assumed to have a stable working directory; the hook fires once at session start.
- Cross-platform hook scripts. Andon is Windows-only today; PowerShell only.

## Approach

**Hook + inference fallback.** A `SessionStart` hook in `~/.claude/settings.json` POSTs the cwd, git toplevel, git remote, and git branch to a new Andon API endpoint at session start. For sessions without hook data (existing history, hook disabled, or hook failure), an inference function reconstructs the repo root by finding the longest common ancestor of file paths in the session and walking up to the nearest `.git` directory.

The hook is authoritative. Inference is a safety net and backfill mechanism.

## Data model

Add five columns to the `sessions` table (single migration):

| Column | Type | Source | Notes |
|---|---|---|---|
| `cwd` | TEXT | hook | Raw working directory at session start. |
| `repo_root` | TEXT | hook or inferred | Result of `git rev-parse --show-toplevel`. May equal `cwd` for non-git folders if inference fails. |
| `repo_remote` | TEXT | hook | Normalized remote URL (e.g. `github.com/satish-krishna/andon`). NULL when no remote or no git. |
| `repo_branch` | TEXT | hook | Result of `git branch --show-current`. NULL when no git. |
| `repo_name` | TEXT | derived | Display name. From remote: `org/name`. From path: basename. Computed on write to keep queries simple. |

Remote URL normalization rules (applied on write):
- Strip protocol (`https://`, `git@`).
- Replace `:` with `/` for SSH form (`git@github.com:foo/bar` → `github.com/foo/bar`).
- Strip trailing `.git`.
- Lowercase the host portion only.

Repo identity (the grouping key used by the UI) is computed at read time as `COALESCE(repo_remote, repo_root, cwd)`. Stored as a SQL view or expression — no denormalized column, so the precedence rule can be changed later without a migration.

## Capture path

### 1. SessionStart hook (primary)

Andon writes a PowerShell script to `~/.andon/hooks/session_start.ps1` on first launch:

```powershell
$ErrorActionPreference = 'SilentlyContinue'
$sid  = $env:CLAUDE_SESSION_ID
if (-not $sid) { exit 0 }
$cwd  = (Get-Location).Path
$top  = (git rev-parse --show-toplevel 2>$null)
$rem  = (git config --get remote.origin.url 2>$null)
$brn  = (git branch --show-current 2>$null)
$body = @{
    session_id  = $sid
    cwd         = $cwd
    repo_root   = $top
    repo_remote = $rem
    repo_branch = $brn
} | ConvertTo-Json -Compress
try {
    Invoke-RestMethod -Uri 'http://127.0.0.1:8765/api/session/context' `
        -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 2 | Out-Null
} catch {}
exit 0
```

Design constraints on the script:
- **Never block Claude Code.** 2-second timeout, swallow all errors, always exit 0.
- **Never write to stdout/stderr.** Claude Code may surface hook output.
- **Be self-contained.** No PowerShell modules beyond what ships with Windows.

`~/.claude/settings.json` gets a hook entry pointing at the script:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "",
        "hooks": [
          { "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"%USERPROFILE%\\.andon\\hooks\\session_start.ps1\"" }
        ]
      }
    ]
  }
}
```

Patcher behaviour (`src-tauri/src/integration.rs`):
- On first launch / Re-apply: merge the hook entry into existing `hooks.SessionStart` (don't clobber other hooks).
- Backup file gets the original `hooks` block too, not just `env`.
- If a `SessionStart` hook already exists from another source and Andon's is not present, add Andon's alongside it.
- "Unpatch" removes only Andon's entry; if other entries remain, the `hooks.SessionStart` array stays.

### 2. API endpoint

`POST /api/session/context`

Request body:
```json
{
  "session_id": "uuid",
  "cwd": "E:\\Repos\\andon",
  "repo_root": "E:\\Repos\\andon",
  "repo_remote": "https://github.com/satish-krishna/andon.git",
  "repo_branch": "main"
}
```

Behaviour:
- Idempotent upsert on `session_id`. If the session row doesn't exist yet, insert a stub row with the context fields; the ingestor will fill in the rest when telemetry arrives (the existing `INSERT OR IGNORE` pattern on the sessions table needs to become an upsert that doesn't clobber repo columns).
- Normalize `repo_remote` before storing.
- Compute and store `repo_name`.
- Always returns 200. Errors are logged via `tracing`, never propagated.

### 3. Inference fallback

A function `Ingestor::infer_repo(session_id) -> Result<Option<RepoInfo>>` that:

1. Collects all `file_path` values for the session from `tool_decisions` ∪ `file_changes`.
2. Filters to absolute paths only. Returns `None` if zero.
3. Computes the longest common ancestor path.
4. Walks up from that ancestor (and from each individual file path if the ancestor itself isn't a real directory) looking for the nearest `.git` directory.
5. If found, reads `.git/config` to extract the `remote.origin.url` and current branch (HEAD ref).
6. Returns the discovered `repo_root`, `repo_remote`, `repo_branch`. Writes them to the `sessions` row only if those columns are still NULL (never overwrites hook data).

When inference runs:
- **At session-end detection.** Andon already does session-end work (per the recent forwarder spec, session-end DB writes were moved to `spawn_blocking`). Hook into that path to run inference when `repo_root IS NULL`.
- **On-demand backfill.** A button on Settings → Data: "Backfill repo info for sessions missing it." Runs inference over every session with `repo_root IS NULL` and shows progress.

## UI surface

### Sessions list (`/sessions`)
- New column **REPO** between SESSION and MODEL. Renders `repo_name`, with `(not git)` suffix when `repo_root` is set but `repo_remote` is not. Truncated; tooltip shows the full `repo_root`.
- New filter chip group **REPO** in the left filter rail. Lists the top N (default 10) repos by session count for the current range, with a search box for the long tail. Multi-select.
- Delete the existing "REPO — NOT EMITTED BY CLAUDE CODE" placeholder.

### Session detail (`/sessions/:id`)
- Header subtitle line: `repo_name · branch` (e.g. `satish-krishna/andon · main`), with `repo_root` as a smaller secondary line. When `repo_remote` is set, render `repo_name` as a link to the remote URL (reconstructed: `https://<repo_remote>`).

### Files page (`/files`)
- Same REPO filter group as Sessions.
- When a single repo is selected, file paths render relative to `repo_root` (e.g. `src-tauri/src/lib.rs` instead of `E:\Repos\andon\src-tauri\src\lib.rs`). When zero or multiple repos are selected, fall back to absolute paths.

### Overview (`/overview`)
- New tile **TOP REPOS · PERIOD**, placed below the Cost-by-model tile. Horizontal bar list, top 5 repos by cost in the active range, with a small sparkline of daily spend per repo. Each row links to `/sessions?repo=<key>`.

### Unknown / empty states
- Sessions missing repo info render `—` in the column and group under a single "no repo" bucket in filters.
- A subtle banner on Sessions and Files when >20% of sessions in the current view are missing repo info: *"Some sessions are missing repo info. [Backfill from file paths] [Re-apply hook]"* — buttons go to the inference backfill action and the Settings → Integration re-apply, respectively.

## Migration

Single new migration (next sequential migration number, e.g. `00X_session_repo.sql`):

```sql
ALTER TABLE sessions ADD COLUMN cwd TEXT;
ALTER TABLE sessions ADD COLUMN repo_root TEXT;
ALTER TABLE sessions ADD COLUMN repo_remote TEXT;
ALTER TABLE sessions ADD COLUMN repo_branch TEXT;
ALTER TABLE sessions ADD COLUMN repo_name TEXT;
CREATE INDEX idx_sessions_repo_remote ON sessions(repo_remote);
CREATE INDEX idx_sessions_repo_root ON sessions(repo_root);
```

No backfill is performed by the migration itself. Existing sessions get repo info either when the user clicks the backfill button, or lazily as they're viewed (deferred — v1 keeps it explicit).

## Testing

- **Hook script:** PowerShell unit tests (Pester) covering: no git repo, git repo without remote, SSH remote, HTTPS remote, missing `CLAUDE_SESSION_ID`, API unreachable (must still exit 0).
- **Endpoint:** Rust integration tests in `src-tauri/src/api/routes.rs` covering: insert when session row absent, upsert when row exists, idempotency, normalization of remote URL forms.
- **Inference:** unit tests with synthetic file_changes rows: single-folder session, multi-folder session, no absolute paths, paths under a `.git` ancestor, paths with no `.git` anywhere.
- **Patcher:** existing integration tests extended to cover hook merge / unmerge alongside the env-var patching.
- **UI:** manual verification per page (no automated frontend tests in repo today).

## Rollout

1. Migration runs on next launch. No data loss; all new columns are nullable.
2. Patcher applies hook on next launch (same trigger as env-var patching). Users who declined env patching also decline hook patching.
3. UI changes ship together — no half-state where columns exist but aren't displayed.
4. README updated to mention the SessionStart hook in the Privacy section (it issues one localhost POST per session; no external network).

## Open risks

- **Claude Code hook envelope changes.** The injected env var name (`CLAUDE_SESSION_ID`) and the `SessionStart` hook event must match what Claude Code actually emits. Verify against current CLI before implementation — if the name differs (`CLAUDE_CODE_SESSION_ID`, etc.), update the script.
- **Multiple Andon-managed hooks in future.** If we add more hooks later (e.g. SessionEnd), the patcher needs a way to identify "its" hook entries for clean unpatch. Tag Andon entries with a sentinel comment or a wrapper command name.
- **Inference false positives.** A session that edited a file outside its actual repo (e.g. a temp file in `/tmp`) could pull the common ancestor away from the real repo. Mitigation: prefer the deepest `.git`-bearing ancestor of *most* file paths, not all.
