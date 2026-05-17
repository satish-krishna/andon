pub mod migrations;
pub mod queries;

use std::path::Path;

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn init(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path)
        .with_flags(OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE)
        .with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode = WAL;\n\
                 PRAGMA foreign_keys = ON;\n\
                 PRAGMA synchronous = NORMAL;",
            )
        });

    let pool = Pool::builder()
        .max_size(8)
        .build(manager)
        .with_context(|| format!("opening sqlite pool at {}", db_path.display()))?;

    let mut conn = pool.get().context("checking out connection for migrations")?;
    migrations::apply(&mut conn).context("applying migrations")?;

    Ok(pool)
}
