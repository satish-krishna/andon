pub mod dto;
pub mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::Router;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

use crate::db::DbPool;
use crate::diagnostics::Diagnostics;
use crate::integration::IntegrationStatus;
use crate::otlp::IngestionControl;
use crate::otlp::forwarder::Forwarder;
use crate::settings::SettingsStore;

const BIND_ADDR: &str = "127.0.0.1:8765";

#[derive(Clone)]
pub struct ApiState {
    pub pool: Arc<DbPool>,
    pub db_path: PathBuf,
    pub control: IngestionControl,
    pub integration: Arc<Mutex<IntegrationStatus>>,
    pub diagnostics: Diagnostics,
    pub settings: Arc<SettingsStore>,
    pub forwarder: Arc<Forwarder>,
}

pub async fn serve(state: ApiState) -> Result<()> {
    let addr: SocketAddr = BIND_ADDR.parse().expect("hardcoded bind address is valid");

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let diag = state.diagnostics.clone();
    let app: Router = routes::router(state).layer(cors);

    let listener = TcpListener::bind(addr).await.with_context(|| {
        format!("failed to bind api server to {addr} — is another andon instance running?")
    })?;
    tracing::info!(%addr, "api server listening");
    diag.record_bind("api", true, None);

    axum::serve(listener, app)
        .await
        .context("api server crashed")?;
    Ok(())
}
