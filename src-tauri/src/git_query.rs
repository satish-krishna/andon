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
    let cwd_for_warn = cwd.clone();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let fut = async move {
        let out = Command::new("git")
            .args(&args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
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
            tracing::warn!(cwd = ?cwd_for_warn, "git timed out");
            None
        }
    }
}

pub async fn query_repo(cwd: &Path) -> RepoInfo {
    let (toplevel_raw, remote_raw, branch) = tokio::join!(
        run_git(cwd, &["rev-parse", "--show-toplevel"]),
        run_git(cwd, &["config", "--get", "remote.origin.url"]),
        run_git(cwd, &["branch", "--show-current"]),
    );
    let toplevel = toplevel_raw.map(PathBuf::from);
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
    let no_scheme = raw
        .strip_prefix("https://").or_else(|| raw.strip_prefix("http://"))
        .or_else(|| raw.strip_prefix("ssh://"))
        .unwrap_or(raw);

    // Strip userinfo: any "user@" or "user:pass@" segment before the first '/'.
    // We only strip when the '@' appears before any '/', so paths like
    // `host/org/some@thing` are preserved.
    let no_userinfo = match no_scheme.find('@') {
        Some(at) if !no_scheme[..at].contains('/') => &no_scheme[at + 1..],
        _ => no_scheme,
    };

    // SCP form: host:org/repo -> host/org/repo (only the first colon).
    let slashed = if let Some(idx) = no_userinfo.find(':') {
        let (host, rest) = no_userinfo.split_at(idx);
        format!("{}/{}", host, &rest[1..])
    } else {
        no_userinfo.to_string()
    };
    let no_git = slashed.strip_suffix(".git").unwrap_or(&slashed).to_string();
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

    #[test]
    fn normalize_strips_embedded_credentials() {
        assert_eq!(
            normalize_remote("https://user:pat@github.com/Foo/Bar.git"),
            "github.com/Foo/Bar"
        );
        assert_eq!(
            normalize_remote("https://token@gitlab.com/team/proj"),
            "gitlab.com/team/proj"
        );
    }

    fn uuid_like() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }
}
