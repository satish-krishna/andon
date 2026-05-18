#![allow(dead_code)] // helpers are shared across many test files

use std::sync::Arc;

use andon_lib::db::DbPool;
use tempfile::TempDir;

/// Build an isolated SQLite pool backed by a temp file (WAL requires a real file).
/// Returns the pool plus the TempDir guard — drop the guard to delete the DB.
pub fn fixture_pool() -> (Arc<DbPool>, TempDir) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("test.db");
    let pool = andon_lib::db::init(&db_path).expect("open pool and run migrations");
    (Arc::new(pool), dir)
}
