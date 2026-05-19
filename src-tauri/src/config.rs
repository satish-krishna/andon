use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub log_path: PathBuf,
}

impl Paths {
    pub fn resolve_and_prepare() -> Result<Self> {
        let home = dirs::home_dir().context("could not resolve user home directory")?;
        let data_dir = home.join(".andon");
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data directory {}", data_dir.display()))?;
        let db_path = data_dir.join("data.db");
        let log_path = data_dir.join("log.txt");
        Ok(Self {
            data_dir,
            db_path,
            log_path,
        })
    }

    /// Build a `Paths` rooted at an arbitrary directory, used for testing.
    /// Does NOT create any directories on disk.
    #[cfg(test)]
    pub fn from_data_dir(data_dir: PathBuf) -> Self {
        let db_path = data_dir.join("data.db");
        let log_path = data_dir.join("log.txt");
        Self {
            data_dir,
            db_path,
            log_path,
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests — pure Rust only, no real `~/.andon` interaction
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    // Verify that `from_data_dir` computes the correct derived paths.
    #[test]
    fn paths_derived_from_data_dir() {
        let dir = tmp();
        let data_dir = dir.path().to_path_buf();
        let paths = Paths::from_data_dir(data_dir.clone());

        assert_eq!(paths.data_dir, data_dir);
        assert_eq!(paths.db_path, data_dir.join("data.db"));
        assert_eq!(paths.log_path, data_dir.join("log.txt"));
    }

    // db_path and log_path must be direct children of data_dir.
    #[test]
    fn paths_db_and_log_are_children_of_data_dir() {
        let dir = tmp();
        let paths = Paths::from_data_dir(dir.path().to_path_buf());

        assert_eq!(
            paths.db_path.parent().expect("db_path has parent"),
            paths.data_dir,
            "db_path must be a direct child of data_dir"
        );
        assert_eq!(
            paths.log_path.parent().expect("log_path has parent"),
            paths.data_dir,
            "log_path must be a direct child of data_dir"
        );
    }

    // File names must not change accidentally.
    #[test]
    fn paths_file_names_are_stable() {
        let dir = tmp();
        let paths = Paths::from_data_dir(dir.path().to_path_buf());

        assert_eq!(
            paths.db_path.file_name().and_then(|n| n.to_str()),
            Some("data.db"),
            "database file must be named data.db"
        );
        assert_eq!(
            paths.log_path.file_name().and_then(|n| n.to_str()),
            Some("log.txt"),
            "log file must be named log.txt"
        );
    }
}
