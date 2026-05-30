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

/// Returns true when `p` has at least one named component beyond the root
/// prefix. This rejects bare roots like `/` (Unix) or `C:\` (Windows) which
/// are not meaningful repository roots.
pub fn is_meaningful_root(p: &Path) -> bool {
    let mut named = 0usize;
    for c in p.components() {
        if matches!(c, std::path::Component::Normal(_)) {
            named += 1;
        }
    }
    named >= 1
}

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
            // sessions.cwd is the most reliable hint for JSONL-only or
            // telemetry-light sessions — it's set by the JSONL reducer from
            // the first user turn. Folding it into the LCA input via UNION
            // means a single-path session (cwd only) will short-circuit to
            // cwd itself as the LCA, then walk up for .git as usual.
            let mut stmt = conn.prepare(
                "SELECT file_path FROM file_changes WHERE session_id = ?1 AND file_path IS NOT NULL
                 UNION
                 SELECT file_path FROM tool_decisions WHERE session_id = ?1 AND file_path IS NOT NULL
                 UNION
                 SELECT cwd FROM sessions WHERE session_id = ?1 AND cwd IS NOT NULL"
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

    // Reject bare filesystem roots (e.g. `/` or `C:\`) — they are not
    // meaningful repo roots and walking up from them finds nothing useful.
    if !is_meaningful_root(&lca) {
        return Ok(None);
    }

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

    // Guard again after walking up — in case the walk landed on a root.
    if !is_meaningful_root(&git_root) {
        return Ok(None);
    }
    let info = query_repo(&git_root).await;
    if info.repo_root.is_some() || info.repo_remote.is_some() {
        Ok(Some(info))
    } else {
        // Non-git folder — still return repo_root as the LCA so the column
        // is non-NULL after inference.
        Ok(Some(RepoInfo {
            repo_root: Some(git_root.clone()),
            repo_name: crate::git_query::compute_repo_name(None, Some(&git_root), &git_root),
            ..Default::default()
        }))
    }
}

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

    #[test]
    fn infer_returns_none_when_lca_is_root_only() {
        // LCA of /a/x and /b/y is "/", which should not be persisted.
        let paths = vec![PathBuf::from("/a/x"), PathBuf::from("/b/y")];
        let lca = longest_common_ancestor(&paths).unwrap();
        assert!(!is_meaningful_root(&lca));
    }
}
