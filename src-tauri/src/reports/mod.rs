pub mod assets;
pub mod model;
pub mod render;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::DbPool;

pub fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

pub fn report_path(reports_dir: &std::path::Path, session_id: &str) -> PathBuf {
    reports_dir.join(format!("{}.html", sanitize_id(session_id)))
}

pub fn generate_report(
    pool: Arc<DbPool>,
    reports_dir: &std::path::Path,
    session_id: &str,
) -> Result<PathBuf> {
    let data = model::ReportData::load(&pool, session_id)
        .context("load report data")?;
    let html = render::render(&data).context("render template")?;
    std::fs::create_dir_all(reports_dir)
        .with_context(|| format!("mkdir {}", reports_dir.display()))?;
    let path = report_path(reports_dir, session_id);
    let tmp = path.with_extension("html.tmp");
    std::fs::write(&tmp, html).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize_id("abc-123_xyz"), "abc-123_xyz");
        assert_eq!(sanitize_id("../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_id("a/b\\c"), "abc");
    }
}
