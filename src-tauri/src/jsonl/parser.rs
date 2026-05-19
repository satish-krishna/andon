//! Streaming JSONL parser. Captures per-line errors so callers can route them
//! to the `jsonl_errors` table.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::jsonl::record::{parse_line, JsonlRecord};

#[derive(Debug)]
pub struct ParseErr {
    pub file: PathBuf,
    pub line_no: usize,
    pub kind: ErrKind,
    pub msg: String,
    pub cc_version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrKind {
    JsonParse,
    UnknownType,
    MissingField,
    ReducerPanic,
}

impl ErrKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrKind::JsonParse => "json_parse",
            ErrKind::UnknownType => "unknown_type",
            ErrKind::MissingField => "missing_field",
            ErrKind::ReducerPanic => "reducer_panic",
        }
    }
}

/// Iterate every JSONL line in `path`. Returning `false` aborts iteration.
pub fn for_each_record<F>(path: &Path, mut cb: F) -> std::io::Result<()>
where
    F: FnMut(Result<JsonlRecord, ParseErr>) -> bool,
{
    let f = File::open(path)?;
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line_no = i + 1;
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let cont = cb(Err(ParseErr {
                    file: path.to_path_buf(),
                    line_no,
                    kind: ErrKind::JsonParse,
                    msg: format!("read error: {e}"),
                    cc_version: None,
                }));
                if !cont {
                    return Ok(());
                } else {
                    continue;
                }
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let ev = match parse_line(&line) {
            Ok(rec) => Ok(rec),
            Err(e) => Err(ParseErr {
                file: path.to_path_buf(),
                line_no,
                kind: ErrKind::JsonParse,
                msg: e.to_string(),
                cc_version: None,
            }),
        };
        if !cb(ev) {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parses_valid_lines_and_captures_invalid() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"type":"user","sessionId":"s1"}}"#).unwrap();
        writeln!(f, r#"not valid json"#).unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, r#"{{"type":"assistant","sessionId":"s1"}}"#).unwrap();
        let (mut oks, mut errs) = (0, 0);
        for_each_record(f.path(), |r| {
            match r {
                Ok(_) => oks += 1,
                Err(_) => errs += 1,
            }
            true
        })
        .unwrap();
        assert_eq!(oks, 2);
        assert_eq!(errs, 1);
    }
}
