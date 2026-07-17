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
    // Reject a slug that could escape the projects root. Beyond the obvious
    // separator/traversal forms, a slug must parse as exactly one
    // `Component::Normal`: this catches Windows drive-relative forms like
    // "C:" or "C:foo", which contain none of '/', '\\', or ".." yet make
    // `Path::join` discard the base entirely -- `root.join("C:").join("memory")`
    // becomes the literal drive-relative path `C:memory`, dropping `root`.
    if slug.is_empty() || slug.contains('/') || slug.contains('\\') || slug.contains("..") || slug.contains(':')
    {
        return None;
    }
    let mut comps = Path::new(slug).components();
    let is_single_normal_component =
        matches!(comps.next(), Some(std::path::Component::Normal(_))) && comps.next().is_none();
    if !is_single_normal_component {
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
    // Reject NTFS alternate data stream syntax outright. `Path::extension()`
    // operates on the raw string, so `secrets.json:evil.md` would otherwise
    // canonicalize fine, report extension "md", and pass `is_file()` -- the
    // check below would resolve the `secrets.json` file's `evil.md` ADS
    // rather than a real Markdown file. No legitimate memory file name
    // contains a colon.
    if rel.contains(':') {
        return None;
    }
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
    let rel = rel.to_string_lossy().replace('\\', "/");
    // Same ADS concern as `guard_under`: `path.extension()` above would
    // happily report "md" for an alternate data stream on a non-Markdown
    // file (e.g. `secrets.json:evil.md`). Reject it here too.
    if rel.contains(':') {
        return None;
    }
    Some((slug, rel))
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

    /// Monotonic counter so parallel test threads (which share a process id)
    /// never race on the same directory name.
    fn unique_temp_name(prefix: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{prefix}-{}-{n}", std::process::id())
    }

    fn temp_base() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(unique_temp_name("andon-memtest"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create temp base");
        fs::write(base.join("ok.md"), "hello").expect("write ok.md");
        fs::canonicalize(&base).expect("canonicalize base")
    }

    /// A sibling of `temp_base()`'s directory (same temp root, different
    /// name so the two helpers' `remove_dir_all` calls never clobber each
    /// other), containing a real, existing `.md` file. Used to prove
    /// `guard_under`'s containment check actually rejects a target that
    /// would otherwise pass every other check (canonicalizes, is Markdown,
    /// is a file) -- the only thing standing in the way is
    /// `path.starts_with(&base)`.
    fn sibling_outside_base() -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(unique_temp_name("andon-memtest-sibling"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create sibling dir");
        fs::write(dir.join("real.md"), "outside base").expect("write real.md");
        let dir = fs::canonicalize(&dir).expect("canonicalize sibling dir");
        let name = dir
            .file_name()
            .expect("sibling dir has a name")
            .to_string_lossy()
            .to_string();
        (dir, name)
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
        // These are rejected only because the traversed target does not
        // exist -- canonicalize fails, and `starts_with` is never reached.
        assert!(guard_under(&base, "../../../windows/system32/drivers/etc/hosts").is_none());
        assert!(guard_under(&base, "..\\ok.md").is_none());

        // This traverses to a real, existing `.md` file outside `base`. It
        // canonicalizes fine and passes the extension check, so this
        // assertion can only pass because of `path.starts_with(&base)`.
        let (_sibling_dir, sibling_name) = sibling_outside_base();
        assert!(guard_under(&base, &format!("../{}/real.md", sibling_name)).is_none());
        assert!(guard_under(&base, &format!("..\\{}\\real.md", sibling_name)).is_none());
    }

    #[test]
    fn guard_rejects_an_absolute_path() {
        // Path::join replaces the base when the argument is absolute.
        let base = temp_base();
        // `hosts` has no extension, so this is rejected by the extension
        // check, not containment.
        assert!(guard_under(&base, "C:\\Windows\\System32\\drivers\\etc\\hosts").is_none());
        assert!(guard_under(&base, "/etc/passwd").is_none());

        // Every Windows absolute path carries a drive-letter colon, so an
        // absolute route to a real `.md` file outside `base` is now caught
        // by the ADS colon rejection before it can reach `starts_with` --
        // it can no longer isolate containment on its own. The relative
        // traversal case in `guard_rejects_parent_traversal` below exercises
        // the same, single `starts_with(&base)` check without a colon in
        // the way, and is what actually proves containment.
        let (sibling_dir, _sibling_name) = sibling_outside_base();
        let abs_md = sibling_dir.join("real.md").to_string_lossy().to_string();
        assert!(guard_under(&base, &abs_md).is_none());
    }

    #[test]
    fn guard_rejects_ntfs_alternate_data_stream_suffix() {
        let base = temp_base();
        // Create a real alternate data stream on an existing non-Markdown
        // file: NTFS treats `secrets.json:evil.md` as a distinct data
        // stream on `secrets.json`. `Path::extension()` only inspects the
        // raw string, so without the explicit `:` rejection this would
        // canonicalize, report extension "md", and pass `is_file()`.
        fs::write(base.join("secrets.json"), "{}").expect("write secrets.json");
        let ads_rel = "secrets.json:evil.md";
        fs::write(base.join(ads_rel), "leaked").expect("write ADS stream");
        assert!(guard_under(&base, ads_rel).is_none());
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

    #[test]
    fn memory_dir_rejects_a_drive_relative_slug() {
        // Path::join treats a bare or drive-relative prefix as replacing the
        // base entirely: `projects_root().join("C:").join("memory")` becomes
        // the literal drive-relative path `C:memory`, discarding the
        // projects root. None of these slugs contain '/', '\\', or "..", so
        // they must be caught by the single-normal-component check.
        assert!(memory_dir("C:").is_none());
        assert!(memory_dir("D:").is_none());
        assert!(memory_dir("C:foo").is_none());
    }
}
