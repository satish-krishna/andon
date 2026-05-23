//! Enumerate Claude Code transcripts under `<claude_home>/projects/`.
//!
//! Claude Code nests each session's subagent transcripts in a per-session
//! subdirectory (`<slug>/<session>/subagents/agent-*.jsonl`), so enumeration
//! must recurse: a single-level read of each slug finds only the flat main
//! transcripts and silently skips every subagent file.

use std::path::{Path, PathBuf};

/// Every `.jsonl` file anywhere beneath `<claude_home>/projects/`, sorted for
/// deterministic ingest order.
pub fn enumerate(claude_home: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    collect_jsonl(&claude_home.join("projects"), &mut out);
    out.sort();
    out
}

/// Recursively collect `.jsonl` files under `dir`. An unreadable directory or
/// entry is skipped, never fatal — consistent with the lenient never-abort
/// philosophy. Symlinked directories are not followed, so cycles cannot occur.
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_jsonl(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn enumerates_jsonl_only() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("projects").join("p1");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.jsonl"), b"{}").unwrap();
        fs::write(proj.join("x.txt"), b"x").unwrap();
        fs::write(proj.join("b.jsonl"), b"{}").unwrap();
        assert_eq!(enumerate(tmp.path()).len(), 2);
    }

    #[test]
    fn missing_home_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(enumerate(tmp.path()).is_empty());
    }

    #[test]
    fn descends_into_nested_subagent_directories() {
        // Claude Code nests per-session transcripts:
        //   projects/<slug>/<session>/subagents/agent-*.jsonl
        // A single-level read of each slug sees only the flat main transcript
        // and silently skips every subagent file — the walker must recurse.
        let tmp = tempfile::tempdir().unwrap();
        let slug = tmp.path().join("projects").join("p1");
        let subagents = slug.join("session-abc").join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(slug.join("session-abc.jsonl"), b"{}").unwrap();
        fs::write(subagents.join("agent-1.jsonl"), b"{}").unwrap();
        fs::write(subagents.join("agent-2.jsonl"), b"{}").unwrap();
        fs::write(subagents.join("notes.txt"), b"x").unwrap();
        let found = enumerate(tmp.path());
        assert_eq!(
            found.len(),
            3,
            "flat main transcript + 2 nested .jsonl; the .txt is ignored"
        );
    }
}
