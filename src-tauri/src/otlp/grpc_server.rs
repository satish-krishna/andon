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

use super::{GRPC_ADDR, ingestor::Ingestor};

pub async fn serve(ingestor: Arc<Ingestor>) {
    let addr: SocketAddr = GRPC_ADDR.parse().expect("hardcoded grpc bind address valid");

    let metrics_svc = MetricsServiceImpl {
        ingestor: ingestor.clone(),
    };
    let logs_svc = LogsServiceImpl {
        ingestor: ingestor.clone(),
    };

    tracing::info!(%addr, "otlp gRPC listening");
    if let Err(e) = Server::builder()
        .add_service(MetricsServiceServer::new(metrics_svc))
        .add_service(LogsServiceServer::new(logs_svc))
        .serve(addr)
        .await
    {
        tracing::error!(error = ?e, "otlp gRPC server stopped");
    }
}

struct MetricsServiceImpl {
    ingestor: Arc<Ingestor>,
}

#[tonic::async_trait]
impl MetricsService for MetricsServiceImpl {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let req = request.into_inner();
        let ingestor = self.ingestor.clone();
        // Run blocking SQLite work off the async runtime.
        tokio::task::spawn_blocking(move || {
            if let Err(e) = ingestor.ingest_metrics(req.resource_metrics) {
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
}

#[tonic::async_trait]
impl LogsService for LogsServiceImpl {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let req = request.into_inner();
        let ingestor = self.ingestor.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = ingestor.ingest_logs(req.resource_logs) {
                tracing::warn!(error = ?e, "logs ingestion error");
            }
        })
        .await
        .ok();
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}
