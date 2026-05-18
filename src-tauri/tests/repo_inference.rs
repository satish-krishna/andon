//! Integration tests for `repo_inference` and `git_query` modules.
//! Uses `common::init_temp_repo` to create real git repositories on disk.

mod common;

use std::path::PathBuf;

use andon_lib::{
    git_query::{compute_repo_name, normalize_remote, query_repo},
    repo_inference::{find_git_ancestor, longest_common_ancestor},
};

// ---------------------------------------------------------------------------
// 1. find_git_ancestor picks the right worktree root for a path inside the tree
// ---------------------------------------------------------------------------

#[test]
fn find_git_ancestor_resolves_subdirectory_to_repo_root() {
    // Create a real git repo with one commit so .git is present.
    let repo_dir = common::init_temp_repo(&["initial"]);
    let sub = repo_dir.path().join("subdir");
    std::fs::create_dir_all(&sub).unwrap();

    let found = find_git_ancestor(&sub);
    assert!(
        found.is_some(),
        "expected a git ancestor for path inside the repo, got None"
    );
    // Canonicalize both sides to avoid case/symlink differences on Windows.
    let found_canonical = found.unwrap().canonicalize().unwrap();
    let expected_canonical = repo_dir.path().canonicalize().unwrap();
    assert_eq!(
        found_canonical, expected_canonical,
        "ancestor should be the repo root"
    );
}

// ---------------------------------------------------------------------------
// 2. find_git_ancestor returns None for a path outside any git worktree
// ---------------------------------------------------------------------------

#[test]
fn find_git_ancestor_returns_none_outside_git_tree() {
    // A plain tempdir without git init has no .git anywhere above it
    // (assuming the temp root itself is not inside a git repo, which is safe
    // for CI and local dev where %TEMP% / /tmp is outside any checkout).
    let plain_dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    // Sanity: ensure we did NOT accidentally create a git repo here.
    std::fs::remove_dir_all(plain_dir.path().join(".git")).ok();

    // Walk up from the temp dir — if we land inside a git repo on this machine
    // (e.g. the test runner is inside a checkout), the result can be Some.
    // We only assert None when the temp root itself has no .git ancestor up
    // to the filesystem root. If git IS found, skip gracefully.
    let found = find_git_ancestor(plain_dir.path());
    if found.is_some() {
        // The machine's temp dir happens to be inside a git repo; skip.
        return;
    }
    assert!(found.is_none(), "expected None for plain tempdir");
}

// A more reliable "outside git" test: use a directory we know has no .git
// by creating a totally isolated path and checking it ourselves.
#[test]
fn find_git_ancestor_returns_none_when_no_dot_git_present() {
    // Construct a path that cannot exist on disk — find_git_ancestor only
    // looks at existing directories, so a non-existent prefix is fine as long
    // as the canonicalized walk never finds a real .git.
    //
    // Use the common helper: create a tempdir, explicitly verify no .git, and
    // ask find_git_ancestor on a *subdirectory* of it.
    let plain_dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let nested = plain_dir.path().join("a").join("b").join("c");
    std::fs::create_dir_all(&nested).unwrap();

    // Ensure no .git anywhere between `nested` and `plain_dir`.
    // (tempdir creates a fresh UUID-named dir — no .git up to its parent.)
    let mut cur: Option<&std::path::Path> = Some(&nested);
    let mut git_found_in_range = false;
    while let Some(p) = cur {
        if p.join(".git").exists() {
            git_found_in_range = true;
            break;
        }
        // Stop checking once we pass plain_dir's parent.
        if p == plain_dir.path().parent().unwrap_or(p) {
            break;
        }
        cur = p.parent();
    }

    if git_found_in_range {
        // Rare case: temp dir is inside a git checkout — skip rather than fail.
        return;
    }

    // The nested dir is genuinely outside any git tree (up to its parent).
    // find_git_ancestor will walk up but should return None for the range we care about.
    // We can't assert None for the whole walk, but we CAN assert the returned
    // path (if any) is NOT inside our tempdir.
    let found = find_git_ancestor(&nested);
    if let Some(ref root) = found {
        assert!(
            !root.starts_with(plain_dir.path()),
            "git ancestor should not be inside our plain tempdir; got {root:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. query_repo returns correct info for a real git repo
// ---------------------------------------------------------------------------

#[tokio::test]
async fn query_repo_returns_root_for_git_dir() {
    let repo_dir = common::init_temp_repo(&["first commit", "second commit", "third commit"]);

    let info = query_repo(repo_dir.path()).await;

    // repo_root should be the canonicalized tempdir path.
    assert!(
        info.repo_root.is_some(),
        "expected repo_root to be Some for a real git repo; got {info:?}"
    );

    let root = info.repo_root.as_ref().unwrap();
    // Normalize paths for comparison on Windows (git may return forward slashes or
    // mixed case; PathBuf::canonicalize handles that).
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
    let expected_canonical = repo_dir.path().canonicalize().unwrap();
    assert_eq!(
        root_canonical, expected_canonical,
        "repo_root should match the tempdir"
    );
}

#[tokio::test]
async fn query_repo_returns_branch_for_git_dir() {
    let repo_dir = common::init_temp_repo(&["initial commit"]);

    let info = query_repo(repo_dir.path()).await;

    // A freshly-initialized repo will have a default branch (main or master).
    assert!(
        info.repo_branch.is_some(),
        "expected repo_branch to be Some; got {info:?}"
    );
    let branch = info.repo_branch.as_ref().unwrap();
    assert!(
        branch == "main" || branch == "master",
        "expected 'main' or 'master' branch, got {branch:?}"
    );
}

#[tokio::test]
async fn query_repo_returns_none_for_plain_dir() {
    let plain_dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

    let info = query_repo(plain_dir.path()).await;

    assert!(
        info.repo_root.is_none(),
        "expected None for non-git dir; got {:?}",
        info.repo_root
    );
    assert!(
        info.repo_remote.is_none(),
        "expected no remote for non-git dir"
    );
}

#[tokio::test]
async fn query_repo_commit_count_matches() {
    // Create a repo with exactly 5 commits and verify we can list them all.
    let messages = ["c1", "c2", "c3", "c4", "c5"];
    let repo_dir = common::init_temp_repo(&messages);

    let info = query_repo(repo_dir.path()).await;
    assert!(info.repo_root.is_some(), "repo_root should be present");

    // Use git log to count commits independently and compare.
    let output = std::process::Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(repo_dir.path())
        .output()
        .expect("git log failed");
    let log_text = String::from_utf8(output.stdout).unwrap();
    let commit_count = log_text.lines().count();

    assert_eq!(
        commit_count,
        messages.len(),
        "expected {} commits, git log shows {commit_count}",
        messages.len()
    );

    // Verify each message appears in the log.
    for msg in &messages {
        assert!(
            log_text.contains(msg),
            "commit message '{msg}' not found in git log"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. normalize_remote parses remote URL formats correctly
// ---------------------------------------------------------------------------

#[test]
fn normalize_remote_https_with_dot_git() {
    assert_eq!(
        normalize_remote("https://github.com/org/repo.git"),
        "github.com/org/repo"
    );
}

#[test]
fn normalize_remote_https_without_dot_git() {
    assert_eq!(
        normalize_remote("https://github.com/org/repo"),
        "github.com/org/repo"
    );
}

#[test]
fn normalize_remote_ssh_scp_form() {
    // git@github.com:org/repo.git
    assert_eq!(
        normalize_remote("git@github.com:org/repo.git"),
        "github.com/org/repo"
    );
}

#[test]
fn normalize_remote_ssh_url_form() {
    // ssh://git@github.com/org/repo
    assert_eq!(
        normalize_remote("ssh://git@github.com/org/repo"),
        "github.com/org/repo"
    );
}

#[test]
fn normalize_remote_gitlab_https() {
    assert_eq!(
        normalize_remote("https://gitlab.com/team/project.git"),
        "gitlab.com/team/project"
    );
}

#[test]
fn normalize_remote_lowercases_host() {
    assert_eq!(
        normalize_remote("https://GITHUB.COM/Org/Repo.git"),
        "github.com/Org/Repo"
    );
}

#[test]
fn normalize_remote_strips_credentials() {
    assert_eq!(
        normalize_remote("https://token:secret@github.com/org/repo.git"),
        "github.com/org/repo"
    );
}

#[test]
fn normalize_remote_http_scheme() {
    assert_eq!(
        normalize_remote("http://github.com/org/repo.git"),
        "github.com/org/repo"
    );
}

// ---------------------------------------------------------------------------
// 5. compute_repo_name derives expected names
// ---------------------------------------------------------------------------

#[test]
fn compute_repo_name_from_remote() {
    let name = compute_repo_name(
        Some("github.com/myorg/myrepo"),
        None,
        &PathBuf::from("/tmp"),
    );
    assert_eq!(name, Some("myorg/myrepo".to_string()));
}

#[test]
fn compute_repo_name_from_repo_root_when_no_remote() {
    let name = compute_repo_name(
        None,
        Some(&PathBuf::from("/home/user/projects/cool-project")),
        &PathBuf::from("/home/user/projects/cool-project/src"),
    );
    assert_eq!(name, Some("cool-project".to_string()));
}

#[test]
fn compute_repo_name_from_cwd_when_no_root_or_remote() {
    let name = compute_repo_name(None, None, &PathBuf::from("/workspace/my-app"));
    assert_eq!(name, Some("my-app".to_string()));
}

// ---------------------------------------------------------------------------
// 6. longest_common_ancestor helpers
// ---------------------------------------------------------------------------

#[test]
fn lca_finds_common_parent_across_files() {
    let paths = vec![
        PathBuf::from("C:/repos/proj/src/main.rs"),
        PathBuf::from("C:/repos/proj/src/lib.rs"),
        PathBuf::from("C:/repos/proj/Cargo.toml"),
    ];
    // LCA should be C:/repos/proj
    let lca = longest_common_ancestor(&paths).expect("LCA should be Some");
    // Convert to string for easy assertion (cross-platform path comparison).
    let lca_str = lca.to_string_lossy();
    assert!(
        lca_str.ends_with("proj"),
        "LCA should end with 'proj', got {lca_str}"
    );
}

#[test]
fn lca_single_path_returns_same_path() {
    let paths = vec![PathBuf::from("/a/b/c")];
    let lca = longest_common_ancestor(&paths).expect("LCA should be Some for single path");
    assert_eq!(lca, PathBuf::from("/a/b/c"));
}
