use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use super::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .with_state(state)
}

async fn health(State(state): State<ApiState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "db": state.db_path.display().to_string(),
    }))
}
