use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use super::paths::{guard_under, memory_dir};

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
        // `name:` / `description:` are scoped to top-level keys, matching how
        // `type:` below is scoped to `in_metadata && indented`. Without the
        // `!indented` guard, a `name:` or `description:` nested under
        // `metadata:` would be misattributed to the top-level field instead
        // of being ignored.
        if !indented && trimmed.starts_with("name:") {
            if let Some(v) = trimmed.strip_prefix("name:") {
                name = Some(v.trim().to_string());
            }
        } else if !indented && trimmed.starts_with("description:") {
            if let Some(v) = trimmed.strip_prefix("description:") {
                description = Some(v.trim().to_string());
            }
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

/// Every parsed memory in `dir`, excluding the MEMORY.md index, sorted by
/// file name. A missing or unreadable directory yields an empty list: that
/// is the common case, not an error.
fn list_in(dir: &Path) -> Vec<MemoryDoc> {
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

/// Every parsed memory in the project, excluding the MEMORY.md index.
/// A missing folder yields an empty list: that is the common case, not an error.
pub fn list(slug: &str) -> Vec<MemoryDoc> {
    let Some(dir) = memory_dir(slug) else {
        return Vec::new();
    };
    list_in(&dir)
}

fn read_in(dir: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(guard_under(dir, rel)?).ok()
}

pub fn read(slug: &str, rel: &str) -> Option<String> {
    read_in(&memory_dir(slug)?, rel)
}

/// Writes `content` to `rel` under `dir`. Refuses to create a new file:
/// `guard_under` requires the target to already exist (`is_file()`), which
/// pins the deliberate invariant that the UI edits and deletes memories but
/// never creates them -- only the model creates memories.
fn save_in(dir: &Path, rel: &str, content: &str) -> Result<()> {
    let path = guard_under(dir, rel).context("memory path rejected by guard")?;
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
}

pub fn save(slug: &str, rel: &str, content: &str) -> Result<()> {
    let dir = memory_dir(slug).context("memory path rejected by guard")?;
    save_in(&dir, rel, content)
}

/// Removes the memory file and its pointer line from MEMORY.md.
/// Hard delete, no undo: memories are a few lines each and self-regenerating.
fn delete_in(dir: &Path, rel: &str) -> Result<()> {
    let path = guard_under(dir, rel).context("memory path rejected by guard")?;
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;

    if let Some(index) = guard_under(dir, INDEX_FILE) {
        match std::fs::read_to_string(&index) {
            Ok(raw) => {
                let next = strip_index_line(&raw, rel);
                if next != raw {
                    if let Err(e) = std::fs::write(&index, next) {
                        // The memory is already gone; a stale index line is cosmetic.
                        tracing::warn!(error = %e, "memory::delete: could not rewrite MEMORY.md");
                    }
                }
            }
            Err(e) => {
                // Same reasoning as the write failure above: the memory file
                // is already gone, so a MEMORY.md we could not even read is
                // cosmetic fallout, not a reason to fail the delete. Still
                // worth a diagnostic trail rather than silent swallowing.
                tracing::warn!(error = %e, "memory::delete: could not read MEMORY.md");
            }
        }
    }
    Ok(())
}

pub fn delete(slug: &str, rel: &str) -> Result<()> {
    let dir = memory_dir(slug).context("memory path rejected by guard")?;
    delete_in(&dir, rel)
}

/// Drops any MEMORY.md line whose Markdown link targets `file`.
///
/// Assumed invariant: one pointer per physical line, the MEMORY.md
/// convention. If two pointers ever shared a line, deleting one memory
/// would collaterally drop the other's line too -- this function filters
/// whole lines, it does not edit within a line.
///
/// Preserves the input's existing line-ending style (CRLF vs LF) rather
/// than normalizing to `\n`, and never adds a trailing newline that was not
/// already present.
pub fn strip_index_line(index: &str, file: &str) -> String {
    let needle = format!("]({file})");
    let eol = match index.find('\n') {
        Some(i) if i > 0 && index.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    };
    let kept: Vec<&str> = index.lines().filter(|l| !l.contains(&needle)).collect();
    let mut out = kept.join(eol);
    if index.ends_with('\n') && !out.is_empty() {
        out.push_str(eol);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

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
        assert_eq!(d.raw, "just a note", "raw must carry the complete original text on every branch");
    }

    #[test]
    fn nested_name_and_description_under_metadata_do_not_leak_to_top_level() {
        let raw = "---\nmetadata:\n  name: nested-name\n  description: nested-description\n  type: user\n---\n\nbody text\n";
        let d = parse_doc("weird.md", raw);
        assert!(d.parse_ok, "the scoped metadata.type key is present, so this should parse");
        assert_eq!(d.name, None, "name nested under metadata must not populate the top-level name field");
        assert_eq!(
            d.description, None,
            "description nested under metadata must not populate the top-level description field"
        );
        assert_eq!(d.kind.as_deref(), Some("user"));
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

    #[test]
    fn strip_index_line_preserves_crlf_line_endings() {
        let index = "- [Role](user_role.md) - who they are\r\n- [Other](other.md) - keep me\r\n";
        let out = strip_index_line(index, "user_role.md");
        assert_eq!(out, "- [Other](other.md) - keep me\r\n");
        assert!(out.contains("\r\n"));
        assert!(!out.contains("\n\n"), "the join must not have produced a bare-LF/CRLF collision");
    }

    #[test]
    fn strip_index_line_preserves_lf_line_endings() {
        let index = "- [Role](user_role.md) - who they are\n- [Other](other.md) - keep me\n";
        let out = strip_index_line(index, "user_role.md");
        assert_eq!(out, "- [Other](other.md) - keep me\n");
        assert!(!out.contains('\r'), "an LF-only input must not gain a CR");
    }

    #[test]
    fn strip_index_line_without_a_trailing_newline_does_not_gain_one() {
        let index = "- [Role](user_role.md) - who they are\n- [Other](other.md) - keep me";
        let out = strip_index_line(index, "user_role.md");
        assert_eq!(out, "- [Other](other.md) - keep me");
        assert!(!out.ends_with('\n'), "no trailing newline in, none out");
    }

    #[test]
    fn strip_index_line_with_mixed_line_endings_uses_the_first_line_ending() {
        // First line is CRLF, second is bare LF -- the fix commits to the
        // first-seen style rather than trying to preserve a per-line mix.
        let index = "- [Role](user_role.md) - who they are\r\n- [Other](other.md) - keep me\n";
        let out = strip_index_line(index, "user_role.md");
        assert_eq!(out, "- [Other](other.md) - keep me\r\n");
    }

    /// Monotonic counter so parallel test threads (which share a process id)
    /// never race on the same directory name. Mirrors `paths::tests`.
    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn canon(p: &Path) -> PathBuf {
        fs::canonicalize(p).expect("canonicalize temp dir")
    }

    /// A sibling of a base dir (same temp root, different name), containing a
    /// real, existing `.md` file. Referencing it via `../<name>/real.md`
    /// canonicalizes fine and passes the extension/is_file checks, so it
    /// proves containment is actually enforced rather than passing because
    /// canonicalize failed on a nonexistent target.
    fn sibling_outside_base() -> (PathBuf, String) {
        let dir = unique_temp_dir("andon-storetest-sibling");
        fs::write(dir.join("real.md"), "outside base").expect("write real.md");
        let dir = canon(&dir);
        let name = dir.file_name().expect("sibling dir has a name").to_string_lossy().to_string();
        (dir, name)
    }

    #[test]
    fn save_in_writes_content_to_an_existing_file_and_it_round_trips() {
        let base = unique_temp_dir("andon-storetest-save-ok");
        fs::write(base.join("note.md"), "old content").expect("seed note.md");
        let base = canon(&base);

        save_in(&base, "note.md", "new content").expect("save_in must succeed for an existing file");

        let got = fs::read_to_string(base.join("note.md")).expect("read back note.md");
        assert_eq!(got, "new content", "content must round-trip through save_in");
    }

    #[test]
    fn save_in_refuses_to_create_a_file_that_does_not_exist() {
        let base = canon(&unique_temp_dir("andon-storetest-save-create"));

        let result = save_in(&base, "new.md", "content");

        assert!(
            result.is_err(),
            "save_in must not create files -- only the model creates memories, the UI only edits/deletes"
        );
        assert!(!base.join("new.md").exists(), "no file should have been created");
    }

    #[test]
    fn save_in_refuses_a_path_that_escapes_the_base() {
        let base = canon(&unique_temp_dir("andon-storetest-save-escape"));
        let (sibling_dir, sibling_name) = sibling_outside_base();
        let rel = format!("../{sibling_name}/real.md");

        let result = save_in(&base, &rel, "malicious content");

        assert!(result.is_err(), "save_in must refuse a target outside the base directory");
        let original = fs::read_to_string(sibling_dir.join("real.md")).expect("read sibling file");
        assert_eq!(original, "outside base", "the file outside the base must be untouched");
    }

    #[test]
    fn delete_in_removes_the_file_and_strips_its_index_pointer() {
        let base = unique_temp_dir("andon-storetest-delete-ok");
        fs::write(base.join("note.md"), "content to delete").expect("seed note.md");
        fs::write(base.join(INDEX_FILE), "- [Note](note.md) — a note\n- [Other](other.md) — keep me\n")
            .expect("seed MEMORY.md");
        let base = canon(&base);

        delete_in(&base, "note.md").expect("delete_in must succeed");

        assert!(!base.join("note.md").exists(), "memory file must be gone");
        let index = fs::read_to_string(base.join(INDEX_FILE)).expect("read MEMORY.md");
        assert!(!index.contains("note.md"), "the deleted memory's pointer line must be stripped");
        assert!(index.contains("other.md"), "other pointers must survive");
    }

    #[test]
    fn delete_in_refuses_a_path_that_escapes_the_base() {
        let base = canon(&unique_temp_dir("andon-storetest-delete-escape"));
        let (sibling_dir, sibling_name) = sibling_outside_base();
        let rel = format!("../{sibling_name}/real.md");

        let result = delete_in(&base, &rel);

        assert!(result.is_err(), "delete_in must refuse a target outside the base directory");
        assert!(sibling_dir.join("real.md").exists(), "the file outside the base must survive");
    }

    #[test]
    fn list_in_excludes_the_index_and_sorts_by_file_name() {
        let base = unique_temp_dir("andon-storetest-list");
        fs::write(base.join("zeta.md"), "just a note").expect("write zeta.md");
        fs::write(base.join("alpha.md"), "just a note").expect("write alpha.md");
        fs::write(base.join(INDEX_FILE), "- [Zeta](zeta.md)\n- [Alpha](alpha.md)\n").expect("write MEMORY.md");
        let base = canon(&base);

        let docs = list_in(&base);

        let names: Vec<&str> = docs.iter().map(|d| d.file.as_str()).collect();
        assert_eq!(names, vec!["alpha.md", "zeta.md"], "MEMORY.md must be excluded and the rest sorted by file name");
    }
}
