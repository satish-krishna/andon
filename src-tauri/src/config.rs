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
}
