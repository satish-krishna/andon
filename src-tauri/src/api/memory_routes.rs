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
              WHERE repo_root IS NOT NULL AND repo_root != ''",
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
                let count = store::list(&slug).len();
                let label = label_for(&slug, &roots);
                MemoryProject { slug, label, count }
            })
            .collect())
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))?
    ?;
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
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))?
    ?;
    Ok(Json(out))
}

#[tracing::instrument(skip(state, body))]
async fn memory_save(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<SaveBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        // store::save resolves through the containment guard and fails closed.
        store::save(&slug, &body.file, &body.content)
            .map_err(|e| ApiError::bad_request(&format!("save rejected: {e}")))?;

        let conn = pool.get().map_err(ApiError::pool)?;
        if let Err(e) = provenance::record(
            &conn,
            provenance::ANDON_USER,
            &slug,
            &body.file,
            provenance::Action::Edit,
            now_ms(),
        ) {
            tracing::warn!(error = %e, "memory_save: ledger insert failed");
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))?
    ?;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state, body))]
async fn memory_delete(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Json(body): Json<DeleteBody>,
) -> Result<StatusCode, ApiError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        // Record before removing: the delete row is the churn signal, and it
        // must survive even if the ledger write is the thing that fails.
        let conn = pool.get().map_err(ApiError::pool)?;
        if let Err(e) = provenance::record(
            &conn,
            provenance::ANDON_USER,
            &slug,
            &body.file,
            provenance::Action::Delete,
            now_ms(),
        ) {
            tracing::warn!(error = %e, "memory_delete: ledger insert failed");
        }
        drop(conn);

        store::delete(&slug, &body.file)
            .map_err(|e| ApiError::bad_request(&format!("delete rejected: {e}")))
    })
    .await
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))?
    ?;
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
    .map_err(|e| ApiError::bad_request(&format!("join: {e}")))?
    ?;
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
}
