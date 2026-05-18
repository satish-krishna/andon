use std::sync::Arc;
use std::time::Duration;

use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest,
    metrics::v1::ExportMetricsServiceRequest,
};
use prost::Message;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};

use crate::settings::{ForwarderSettings, SettingsStore};

pub struct Forwarder {
    settings: Arc<SettingsStore>,
    client: reqwest::Client,
}

impl Forwarder {
    pub fn new(settings: Arc<SettingsStore>) -> Self {
        let timeout_ms = settings.forwarder().timeout_ms;
        let client = build_client(timeout_ms);
        Self { settings, client }
    }

    pub fn forward_metrics(&self, req: &ExportMetricsServiceRequest) {
        let fwd = self.settings.forwarder();
        if !fwd.enabled || fwd.endpoint.is_empty() {
            return;
        }
        let body = req.encode_to_vec();
        let url = join_url(&fwd.endpoint, "/v1/metrics");
        self.spawn_post(url, body, fwd);
    }

    pub fn forward_logs(&self, req: &ExportLogsServiceRequest) {
        let fwd = self.settings.forwarder();
        if !fwd.enabled || fwd.endpoint.is_empty() {
            return;
        }
        let body = req.encode_to_vec();
        let url = join_url(&fwd.endpoint, "/v1/logs");
        self.spawn_post(url, body, fwd);
    }

    fn spawn_post(&self, url: String, body: Vec<u8>, fwd: ForwarderSettings) {
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/x-protobuf"));
            for (k, v) in &fwd.headers {
                if let (Ok(name), Ok(val)) = (
                    HeaderName::try_from(k.as_str()),
                    HeaderValue::from_str(v),
                ) {
                    headers.insert(name, val);
                }
            }
            match client.post(&url).headers(headers).body(body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(%url, status = resp.status().as_u16(), "forwarded ok");
                }
                Ok(resp) => {
                    tracing::warn!(%url, status = resp.status().as_u16(), "forwarder remote returned non-2xx");
                }
                Err(e) => {
                    tracing::warn!(%url, error = ?e, "forwarder request failed");
                }
            }
        });
    }
}

pub fn build_client(timeout_ms: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.max(100)))
        .connect_timeout(Duration::from_millis(timeout_ms.max(100)))
        .build()
        .expect("reqwest client build")
}

pub fn join_url(base: &str, suffix: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_strips_trailing_slash() {
        assert_eq!(join_url("https://x/", "/v1/metrics"), "https://x/v1/metrics");
        assert_eq!(join_url("https://x",  "/v1/metrics"), "https://x/v1/metrics");
    }
}
