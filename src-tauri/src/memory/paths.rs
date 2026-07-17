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
