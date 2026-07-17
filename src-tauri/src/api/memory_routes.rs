use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};

use crate::api::ApiState;
use crate::api::routes::ApiError;
use crate::memory::{paths, provenance, store};

#[derive(Debug, Serialize)]
pub struct MemoryProject {
    pub slug: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct MemoryEntry {
    pub doc: store::MemoryDoc,
    /// Headline origin: the last session that wrote this file. `None` means the
    /// memory predates the ledger and must be labeled "origin unknown".
    pub origin: Option<provenance::Touch>,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub slug: String,
    /// Raw MEMORY.md text, if the project has one.
    pub index: Option<String>,
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SaveBody {
    pub file: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteBody {
    pub file: String,
}

#[derive(Debug, Deserialize)]
pub struct ProvenanceQuery {
    pub file: String,
}

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/memory/projects", get(memory_projects))
        .route("/api/memory/:slug", get(memory_list))
        .route("/api/memory/:slug/file", put(memory_save))
        .route("/api/memory/:slug/delete", post(memory_delete))
        .route("/api/memory/:slug/provenance", get(memory_touches))
}

/// Labels a slug with the repo root that mangles to it, falling back to the raw
/// slug. Slugs are enumerated from disk; the mangle rule is only used to match.
fn label_for(slug: &str, repo_roots: &[String]) -> String {
    repo_roots
        .iter()
        .find(|r| paths::slug_for_project(r) == slug)
        .cloned()
        .unwrap_or_else(|| slug.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tracing::instrument(skip(state))]
async fn memory_projects(State(state): State<ApiState>) -> Result<Json<Vec<MemoryProject>>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<Vec<MemoryProject>, ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT repo_root FROM sessions
              WHERE repo_root IS NOT NULL AND repo_root != ''
              ORDER BY repo_root",
        )?;
        let roots: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(paths::projects_with_memory()
            .into_iter()
            .map(|slug| {
                let count = store::count(&slug);
                let label = label_for(&slug, &roots);
                MemoryProject { slug, label, count }
            })
            .collect())
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "memory_projects: blocking task failed");
        ApiError::internal("request failed")
    })??;
    Ok(Json(out))
}

#[tracing::instrument(skip(state))]
async fn memory_list(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
) -> Result<Json<MemoryListResponse>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<MemoryListResponse, ApiError> {
        let docs = store::list(&slug);
        let index = store::read(&slug, store::INDEX_FILE);

        let conn = pool.get().map_err(ApiError::pool)?;
        let entries = docs
            .into_iter()
            .map(|doc| {
                let origin = provenance::last_touch(&conn, &slug, &doc.file);
                MemoryEntry { doc, origin }
            })
            .collect();
        drop(conn);

        Ok(MemoryListResponse { slug, index, entries })
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "memory_list: blocking task failed");
        ApiError::internal("request failed")
    })??;
    Ok(Json(out))
}

/// Saves the file, then best-effort records the ledger row. The fs write goes
/// first: an `edit` row must never be recorded for a save that did not
/// actually happen. `store::save` resolves through the containment guard and
/// fails closed, so a guard rejection here writes no row at all.
///
/// The underlying error may carry an absolute filesystem path (including the
/// OS username); it is logged server-side but never returned to the client.
fn save_and_record(
    conn: &rusqlite::Connection,
    slug: &str,
    file: &str,
    content: &str,
    ts: i64,
) -> Result<(), ApiError> {
    store::save(slug, file, content).map_err(|e| {
        tracing::warn!(error = %e, "memory_save: store::save failed");
        ApiError::bad_request("save rejected")
    })?;

    if let Err(e) = provenance::record(conn, provenance::ANDON_USER, slug, file, provenance::Action::Edit, ts) {
        tracing::warn!(error = %e, "memory_save: ledger insert failed");
    }
    Ok(())
}

#[tracing::instrument(skip(state, body))]
async fn memory_save(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<SaveBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        save_and_record(&conn, &slug, &body.file, &body.content, now_ms())
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "memory_save: blocking task failed");
        ApiError::internal("request failed")
    })??;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the file, then best-effort records the ledger row. The fs delete
/// goes first, deliberately: the ledger is append-only with no function to
/// remove a row (see `memory::provenance`), so recording a `delete` before
/// the delete is confirmed would risk a permanent, false audit entry if
/// `store::delete` failed afterward (guard rejection, locked file,
/// permission denied). A missing row on a rare best-effort ledger-write
/// failure is a lesser harm than a row that lies about what happened.
///
/// The underlying error may carry an absolute filesystem path (including the
/// OS username); it is logged server-side but never returned to the client.
fn delete_and_record(conn: &rusqlite::Connection, slug: &str, file: &str, ts: i64) -> Result<(), ApiError> {
    store::delete(slug, file).map_err(|e| {
        tracing::warn!(error = %e, "memory_delete: store::delete failed");
        ApiError::bad_request("delete rejected")
    })?;

    if let Err(e) = provenance::record(conn, provenance::ANDON_USER, slug, file, provenance::Action::Delete, ts) {
        tracing::warn!(error = %e, "memory_delete: ledger insert failed");
    }
    Ok(())
}

#[tracing::instrument(skip(state, body))]
async fn memory_delete(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<DeleteBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        delete_and_record(&conn, &slug, &body.file, now_ms())
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "memory_delete: blocking task failed");
        ApiError::internal("request failed")
    })??;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state))]
async fn memory_touches(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Query(q): Query<ProvenanceQuery>,
) -> Result<Json<Vec<provenance::Touch>>, ApiError> {
    let pool = state.pool.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<Vec<provenance::Touch>, ApiError> {
        let conn = pool.get().map_err(ApiError::pool)?;
        Ok(provenance::touches(&conn, &slug, &q.file)?)
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "memory_touches: blocking task failed");
        ApiError::internal("request failed")
    })??;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_a_matching_repo_root_over_the_raw_slug() {
        let roots = vec!["D:\\Repos\\andon".to_string(), "D:\\Repos\\blog".to_string()];
        assert_eq!(label_for("D--Repos-andon", &roots), "D:\\Repos\\andon");
    }

    #[test]
    fn label_falls_back_to_the_slug_when_no_repo_matches() {
        let roots = vec!["D:\\Repos\\andon".to_string()];
        assert_eq!(label_for("C--cmder", &roots), "C--cmder");
    }

    #[test]
    fn save_body_round_trips_as_json() {
        let b: SaveBody = serde_json::from_str(r#"{"file":"a.md","content":"hi"}"#)
            .expect("deserialize SaveBody");
        assert_eq!(b.file, "a.md");
        assert_eq!(b.content, "hi");
    }

    #[test]
    fn delete_body_round_trips_as_json() {
        let b: DeleteBody =
            serde_json::from_str(r#"{"file":"a.md"}"#).expect("deserialize DeleteBody");
        assert_eq!(b.file, "a.md");
    }

    fn migrated_conn() -> rusqlite::Connection {
        let mut c = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::apply(&mut c).expect("apply migrations");
        c
    }

    fn provenance_row_count(conn: &rusqlite::Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM memory_provenance", [], |r| r.get(0))
            .expect("count memory_provenance rows")
    }

    /// Regression test for the critical ordering bug: a delete the containment
    /// guard rejects (slug ".." can never resolve to a real memory_dir) must
    /// return an error AND must not leave a `delete` row in the append-only
    /// ledger, because the ledger has no function to remove a false row.
    ///
    /// This test fails if `delete_and_record` is reverted to record-then-delete:
    /// verified by temporarily swapping the order and confirming the assertion
    /// on row count fails (see task report for the mutation-test transcript).
    #[test]
    fn delete_and_record_writes_no_ledger_row_when_the_guard_rejects_the_delete() {
        let conn = migrated_conn();

        let result = delete_and_record(&conn, "..", "escape.md", 100);

        assert!(result.is_err(), "a guard-rejected delete must return Err");
        assert_eq!(
            provenance_row_count(&conn),
            0,
            "a failed delete must not leave a ledger row behind -- the ledger cannot be corrected"
        );
    }

    /// Same shape for save: a save the guard rejects must not record an
    /// `edit` row either, though save was already fs-first before this fix.
    #[test]
    fn save_and_record_writes_no_ledger_row_when_the_guard_rejects_the_save() {
        let conn = migrated_conn();

        let result = save_and_record(&conn, "..", "escape.md", "content", 100);

        assert!(result.is_err(), "a guard-rejected save must return Err");
        assert_eq!(
            provenance_row_count(&conn),
            0,
            "a failed save must not leave a ledger row behind"
        );
    }
}
