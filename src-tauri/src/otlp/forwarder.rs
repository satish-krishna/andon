use std::sync::{Arc, RwLock};
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
    client: RwLock<ClientWithTimeout>,
}

struct ClientWithTimeout {
    timeout_ms: u64,
    client: reqwest::Client,
}

impl Forwarder {
    pub fn new(settings: Arc<SettingsStore>) -> Self {
        let timeout_ms = settings.forwarder().timeout_ms;
        let client = ClientWithTimeout {
            timeout_ms,
            client: build_client(timeout_ms),
        };
        Self {
            settings,
            client: RwLock::new(client),
        }
    }

    /// Snapshot the current client, rebuilding it if the settings timeout has drifted.
    fn current_client(&self) -> reqwest::Client {
        let desired = self.settings.forwarder().timeout_ms;
        if let Ok(guard) = self.client.read() {
            if guard.timeout_ms == desired {
                return guard.client.clone();
            }
        }
        let new = build_client(desired);
        if let Ok(mut guard) = self.client.write() {
            guard.timeout_ms = desired;
            guard.client = new.clone();
        }
        new
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
        let mut resource_logs = req.resource_logs.clone();
        redact_user_prompt(&mut resource_logs);
        let outgoing = ExportLogsServiceRequest { resource_logs };
        let body = outgoing.encode_to_vec();
        let url = join_url(&fwd.endpoint, "/v1/logs");
        self.spawn_post(url, body, fwd);
    }

    fn spawn_post(&self, url: String, body: Vec<u8>, fwd: ForwarderSettings) {
        let client = self.current_client();
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

/// Strip the `body` of any `user_prompt` log record before forwarding.
/// See docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md
/// §Privacy & safety rule 5.
pub fn redact_user_prompt(
    resource_logs: &mut [opentelemetry_proto::tonic::logs::v1::ResourceLogs],
) {
    use opentelemetry_proto::tonic::common::v1::{AnyValue, any_value::Value as AnyV};

    for rl in resource_logs.iter_mut() {
        for sl in rl.scope_logs.iter_mut() {
            for rec in sl.log_records.iter_mut() {
                let is_user_prompt = rec.attributes.iter().any(|kv| {
                    kv.key == "event.name"
                        && matches!(
                            kv.value.as_ref().and_then(|v| v.value.as_ref()),
                            Some(AnyV::StringValue(s)) if s == "user_prompt"
                        )
                });
                if is_user_prompt {
                    rec.body = Some(AnyValue {
                        value: Some(AnyV::StringValue("<redacted>".into())),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_strips_trailing_slash() {
        assert_eq!(join_url("https://x/", "/v1/metrics"), "https://x/v1/metrics");
        assert_eq!(join_url("https://x",  "/v1/metrics"), "https://x/v1/metrics");
    }

    #[test]
    fn redact_user_prompt_strips_body() {
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};

        let mut logs = vec![ResourceLogs {
            resource: None,
            scope_logs: vec![ScopeLogs {
                scope: None,
                log_records: vec![LogRecord {
                    time_unix_nano: 0, observed_time_unix_nano: 0,
                    severity_number: 0, severity_text: String::new(),
                    body: Some(AnyValue { value: Some(AnyV::StringValue("secret prompt".into())) }),
                    attributes: vec![KeyValue {
                        key: "event.name".into(),
                        value: Some(AnyValue { value: Some(AnyV::StringValue("user_prompt".into())) }),
                    }],
                    dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
                }],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }];

        redact_user_prompt(&mut logs);

        let body = logs[0].scope_logs[0].log_records[0].body.as_ref().unwrap();
        if let AnyV::StringValue(s) = body.value.as_ref().unwrap() {
            assert_eq!(s, "<redacted>");
        } else {
            panic!("body should be string");
        }
    }

    #[test]
    fn redact_user_prompt_leaves_other_events_alone() {
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};

        let mut logs = vec![ResourceLogs {
            resource: None,
            scope_logs: vec![ScopeLogs {
                scope: None,
                log_records: vec![LogRecord {
                    time_unix_nano: 0, observed_time_unix_nano: 0,
                    severity_number: 0, severity_text: String::new(),
                    body: Some(AnyValue { value: Some(AnyV::StringValue("tool output".into())) }),
                    attributes: vec![KeyValue {
                        key: "event.name".into(),
                        value: Some(AnyValue { value: Some(AnyV::StringValue("tool_decision".into())) }),
                    }],
                    dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
                }],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }];

        redact_user_prompt(&mut logs);

        let body = logs[0].scope_logs[0].log_records[0].body.as_ref().unwrap();
        if let AnyV::StringValue(s) = body.value.as_ref().unwrap() {
            assert_eq!(s, "tool output", "non-user_prompt bodies must be untouched");
        }
    }
}
