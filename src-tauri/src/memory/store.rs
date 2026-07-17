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
