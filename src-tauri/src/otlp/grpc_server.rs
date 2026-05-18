use std::net::SocketAddr;
use std::sync::Arc;

use opentelemetry_proto::tonic::collector::{
    logs::v1::{
        ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    },
    metrics::v1::{
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
        metrics_service_server::{MetricsService, MetricsServiceServer},
    },
};
use tonic::{Request, Response, Status, transport::Server};

use crate::diagnostics::Diagnostics;

use super::forwarder::Forwarder;
use super::{GRPC_ADDR, ingestor::Ingestor};

pub async fn serve(ingestor: Arc<Ingestor>, diagnostics: Diagnostics, forwarder: Arc<Forwarder>) {
    let addr: SocketAddr = GRPC_ADDR.parse().expect("hardcoded grpc bind address valid");

    let metrics_svc = MetricsServiceImpl {
        ingestor: ingestor.clone(),
        forwarder: forwarder.clone(),
    };
    let logs_svc = LogsServiceImpl {
        ingestor: ingestor.clone(),
        forwarder,
    };

    tracing::info!(%addr, "otlp gRPC listening");
    diagnostics.record_bind("grpc", true, None);
    if let Err(e) = Server::builder()
        .add_service(MetricsServiceServer::new(metrics_svc))
        .add_service(LogsServiceServer::new(logs_svc))
        .serve(addr)
        .await
    {
        tracing::error!(error = ?e, "otlp gRPC server stopped");
        diagnostics.record_bind("grpc", false, Some(e.to_string()));
    }
}

struct MetricsServiceImpl {
    ingestor: Arc<Ingestor>,
    forwarder: Arc<Forwarder>,
}

#[tonic::async_trait]
impl MetricsService for MetricsServiceImpl {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let req = request.into_inner();
        self.forwarder.forward_metrics(&req);
        let ingestor = self.ingestor.clone();
        let resource_metrics = req.resource_metrics;
        // Run blocking SQLite work off the async runtime.
        tokio::task::spawn_blocking(move || {
            if let Err(e) = ingestor.ingest_metrics_v2(resource_metrics, "grpc") {
                tracing::warn!(error = ?e, "metrics ingestion error");
            }
        })
        .await
        .ok();
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}

struct LogsServiceImpl {
    ingestor: Arc<Ingestor>,
    forwarder: Arc<Forwarder>,
}

#[tonic::async_trait]
impl LogsService for LogsServiceImpl {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let req = request.into_inner();
        self.forwarder.forward_logs(&req);
        let ingestor = self.ingestor.clone();
        let resource_logs = req.resource_logs;
        tokio::task::spawn_blocking(move || {
            if let Err(e) = ingestor.ingest_logs_v2(resource_logs, "grpc") {
                tracing::warn!(error = ?e, "logs ingestion error");
            }
        })
        .await
        .ok();
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}
