//! Property test: the forwarder's redact_user_prompt pass strips
//! any prompt text before egress, regardless of the prompt's content.

use proptest::prelude::*;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value as AnyV};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use andon_lib::otlp::forwarder::redact_user_prompt;

fn build_logs(body: &str) -> Vec<ResourceLogs> {
    vec![ResourceLogs {
        resource: None,
        scope_logs: vec![ScopeLogs {
            scope: None,
            log_records: vec![LogRecord {
                time_unix_nano: 0, observed_time_unix_nano: 0,
                severity_number: 0, severity_text: String::new(),
                body: Some(AnyValue { value: Some(AnyV::StringValue(body.into())) }),
                attributes: vec![KeyValue {
                    key: "event.name".into(),
                    value: Some(AnyValue { value: Some(AnyV::StringValue("user_prompt".into())) }),
                }],
                dropped_attributes_count: 0, flags: 0, trace_id: vec![], span_id: vec![],
            }],
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }]
}

fn serialise_to_string(logs: &[ResourceLogs]) -> String {
    let mut out = String::new();
    for rl in logs {
        for sl in &rl.scope_logs {
            for rec in &sl.log_records {
                if let Some(b) = &rec.body {
                    if let Some(AnyV::StringValue(s)) = b.value.as_ref() {
                        out.push_str(s);
                        out.push('\n');
                    }
                }
                for kv in &rec.attributes {
                    if let Some(v) = &kv.value {
                        if let Some(AnyV::StringValue(s)) = v.value.as_ref() {
                            out.push_str(s);
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    out
}

proptest! {
    #[test]
    fn forwarder_strips_user_prompt_body(prompt in "[\\p{L} ]{1,200}") {
        // Skip degenerate "no content" inputs that trivially appear in "<redacted>".
        prop_assume!(!prompt.is_empty());
        prop_assume!(!"<redacted>".contains(&*prompt));

        let mut logs = build_logs(&prompt);
        redact_user_prompt(&mut logs);
        let serialised = serialise_to_string(&logs);

        prop_assert!(!serialised.contains(&*prompt),
            "forwarder leaked prompt: {:?} found in {:?}", prompt, serialised);
    }
}
