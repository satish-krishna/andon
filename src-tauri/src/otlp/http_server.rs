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

use crate::diagnostics::Diagnostics;

use super::forwarder::Forwarder;
use super::{HTTP_ADDR, ingestor::Ingestor};

#[derive(Clone)]
struct HttpState {
    ingestor: Arc<Ingestor>,
    forwarder: Arc<Forwarder>,
}

pub async fn serve(ingestor: Arc<Ingestor>, diagnostics: Diagnostics, forwarder: Arc<Forwarder>) {
    let addr: SocketAddr = HTTP_ADDR.parse().expect("hardcoded otlp http bind valid");
    let state = HttpState { ingestor, forwarder };

    let app = Router::new()
        .route("/v1/metrics", post(metrics))
        .route("/v1/logs", post(logs))
        .route("/", get(|| async { "andon otlp http" }))
        .with_state(state);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = ?e, %addr, "failed to bind otlp HTTP listener");
            diagnostics.record_bind("http", false, Some(e.to_string()));
            return;
        }
    };
    tracing::info!(%addr, "otlp HTTP listening");
    diagnostics.record_bind("http", true, None);
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = ?e, "otlp HTTP server stopped");
        diagnostics.record_bind("http", false, Some(e.to_string()));
    }
}

async fn metrics(State(state): State<HttpState>, body: Bytes) -> impl IntoResponse {
    match ExportMetricsServiceRequest::decode(body) {
        Ok(req) => {
            state.forwarder.forward_metrics(&req);
            let ingestor = state.ingestor.clone();
            let resource_metrics = req.resource_metrics;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = ingestor.ingest_metrics_v2(resource_metrics, "http") {
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
            state.forwarder.forward_logs(&req);
            let ingestor = state.ingestor.clone();
            let resource_logs = req.resource_logs;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = ingestor.ingest_logs_v2(resource_logs, "http") {
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
