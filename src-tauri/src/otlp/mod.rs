pub mod grpc_server;
pub mod http_server;
pub mod ingestor;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use crate::db::DbPool;
use ingestor::Ingestor;

pub const GRPC_ADDR: &str = "127.0.0.1:4317";
pub const HTTP_ADDR: &str = "127.0.0.1:4318";

#[derive(Clone)]
pub struct IngestionControl {
    paused: Arc<AtomicBool>,
}

impl IngestionControl {
    pub fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn set_paused(&self, v: bool) {
        self.paused.store(v, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

impl Default for IngestionControl {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn serve(pool: Arc<DbPool>, control: IngestionControl) -> Result<()> {
    let ingestor = Arc::new(Ingestor::new(pool, control));

    let grpc = tokio::spawn(grpc_server::serve(ingestor.clone()));
    let http = tokio::spawn(http_server::serve(ingestor.clone()));

    let (g, h) = tokio::join!(grpc, http);
    if let Err(e) = g {
        tracing::error!(error = ?e, "grpc server task panicked");
    }
    if let Err(e) = h {
        tracing::error!(error = ?e, "http server task panicked");
    }
    Ok(())
}
