use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Bytes;
use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
};
use prost::Message;
use tokio::net::TcpListener;

use super::{HTTP_ADDR, ingestor::Ingestor};

#[derive(Clone)]
struct HttpState {
    ingestor: Arc<Ingestor>,
}

pub async fn serve(ingestor: Arc<Ingestor>) {
    let addr: SocketAddr = HTTP_ADDR.parse().expect("hardcoded otlp http bind valid");
    let state = HttpState { ingestor };

    let app = Router::new()
        .route("/v1/metrics", post(metrics))
        .route("/v1/logs", post(logs))
        .route("/", get(|| async { "andon otlp http" }))
        .with_state(state);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = ?e, %addr, "failed to bind otlp HTTP listener");
            return;
        }
    };
    tracing::info!(%addr, "otlp HTTP listening");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = ?e, "otlp HTTP server stopped");
    }
}

async fn metrics(State(state): State<HttpState>, body: Bytes) -> impl IntoResponse {
    match ExportMetricsServiceRequest::decode(body) {
        Ok(req) => {
            let ingestor = state.ingestor.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = ingestor.ingest_metrics(req.resource_metrics) {
                    tracing::warn!(error = ?e, "metrics ingestion error (http)");
                }
            })
            .await
            .ok();
            (StatusCode::OK, "")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "failed to decode OTLP metrics protobuf");
            (StatusCode::OK, "")
        }
    }
}

async fn logs(State(state): State<HttpState>, body: Bytes) -> impl IntoResponse {
    match ExportLogsServiceRequest::decode(body) {
        Ok(req) => {
            let ingestor = state.ingestor.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = ingestor.ingest_logs(req.resource_logs) {
                    tracing::warn!(error = ?e, "logs ingestion error (http)");
                }
            })
            .await
            .ok();
            (StatusCode::OK, "")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "failed to decode OTLP logs protobuf");
            (StatusCode::OK, "")
        }
    }
}
