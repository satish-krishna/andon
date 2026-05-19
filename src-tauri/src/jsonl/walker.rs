//! Enumerate `<claude_home>/projects/<slug>/*.jsonl`.

use std::path::{Path, PathBuf};

pub fn enumerate(claude_home: &Path) -> Vec<PathBuf> {
    let projects = claude_home.join("projects");
    let mut out = vec![];
    let Ok(slugs) = std::fs::read_dir(&projects) else {
        return out;
    };
    for slug in slugs.flatten() {
        if !slug.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(slug.path()) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
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
}
