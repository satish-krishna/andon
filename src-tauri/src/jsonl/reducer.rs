//! Trust boundary between JSONL (raw, contains prompt text) and the rest
//! of the ingest pipeline (text-free by type). Anything that reads
//! `record::Message.content[].text` must do so inside this module and
//! drop the text before returning.

use crate::jsonl::record::{ContentBlock, JsonlRecord, Message};

/// Output of the reducer. No variant carries prompt or response text.
/// The privacy property test in `tests/jsonl_privacy.rs` enforces this empirically;
/// the type system enforces it structurally.
#[derive(Debug, Clone)]
pub enum DerivedEvent {
    SessionLifecycle {
        session_id: String,
        started_at: i64,
        ended_at: Option<i64>,
        cc_version: Option<String>,
        cwd: Option<String>,
        git_branch: Option<String>,
    },
    TokenUsage {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        input: i64,
        output: i64,
        cache_create: i64,
        cache_read: i64,
        is_subagent: bool,
    },
    CostEntry {
        session_id: String,
        request_id: String,
        ts: i64,
        model: String,
        cost_usd: f64,
        is_subagent: bool,
    },
    ToolCall {
        session_id: String,
        ts: i64,
        tool_name: String,
        file_path: Option<String>,
        model: Option<String>,
    },
    SlashCommand {
        session_id: String,
        ts: i64,
        name: String,
        arg_count: i64,
    },
    SubAgentCall {
        parent_id: String,
        child_id: Option<String>,
        subagent_type: Option<String>,
        started_at: i64,
    },
}

#[derive(Default)]
pub struct Reducer {
    first_turn_seen: bool,
    seen_requests: std::collections::HashSet<String>,
}

impl Reducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reduce(&mut self, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let Some(sid) = rec.session_id.as_deref().map(|s| s.to_string()) else {
            return vec![];
        };
        let ts = parse_ts(rec.timestamp.as_deref()).unwrap_or(0);
        match rec.kind.as_deref() {
            Some("user") => self.reduce_user(&sid, ts, rec),
            Some("assistant") => self.reduce_assistant(&sid, ts, rec),
            _ => vec![],
        }
    }

    fn reduce_user(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let mut out = vec![];
        if !self.first_turn_seen {
            self.first_turn_seen = true;
            out.push(DerivedEvent::SessionLifecycle {
                session_id: sid.to_string(),
                started_at: ts,
                ended_at: None,
                cc_version: rec.version.clone(),
                cwd: rec.cwd.clone(),
                git_branch: rec.git_branch.clone(),
            });
        }
        if let Some(msg) = rec.message.as_ref() {
            if let Some((name, arg_count)) = detect_slash_command(msg) {
                out.push(DerivedEvent::SlashCommand {
                    session_id: sid.to_string(),
                    ts,
                    name,
                    arg_count,
                });
            }
        }
        out
    }

    fn reduce_assistant(&mut self, sid: &str, ts: i64, rec: &JsonlRecord) -> Vec<DerivedEvent> {
        let mut out = vec![];
        let Some(msg) = rec.message.as_ref() else {
            return out;
        };
        let model = msg.model.clone().unwrap_or_else(|| "unknown".into());

        // Claude Code writes one assistant record per content block; every record
        // of an API call carries the same requestId and the identical usage.
        // Emit token/cost exactly once per requestId, at its first-seen record.
        // Records with no requestId (synthetic / api-error) carry no priceable
        // usage and are skipped — they cannot be safely deduplicated.
        if let (Some(request_id), Some(u)) = (rec.request_id.as_deref(), msg.usage.as_ref()) {
            if self.seen_requests.insert(request_id.to_string())
                && u.input_tokens + u.output_tokens + u.cache_read + u.cache_creation > 0
            {
                out.push(DerivedEvent::TokenUsage {
                    session_id: sid.to_string(),
                    request_id: request_id.to_string(),
                    ts,
                    model: model.clone(),
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_create: u.cache_creation,
                    cache_read: u.cache_read,
                    is_subagent: rec.is_sidechain,
                });
                if let Some(cost) = crate::jsonl::pricing::cost_for(
                    &model,
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_read,
                    u.cache_creation,
                ) {
                    if cost > 0.0 {
                        out.push(DerivedEvent::CostEntry {
                            session_id: sid.to_string(),
                            request_id: request_id.to_string(),
                            ts,
                            model: model.clone(),
                            cost_usd: cost,
                            is_subagent: rec.is_sidechain,
                        });
                    }
                }
            }
        }

        for block in &msg.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                let Some(tool_name) = name.clone() else {
                    continue;
                };
                let file_path = input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                out.push(DerivedEvent::ToolCall {
                    session_id: sid.to_string(),
                    ts,
                    tool_name: tool_name.clone(),
                    file_path,
                    model: Some(model.clone()),
                });
                if tool_name == "Task" {
                    let child_id = input
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let subagent_type = input
                        .get("subagent_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    out.push(DerivedEvent::SubAgentCall {
                        parent_id: sid.to_string(),
                        child_id,
                        subagent_type,
                        started_at: ts,
                    });
                }
            }
        }
        out
    }
}

fn parse_ts(s: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s?)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn detect_slash_command(msg: &Message) -> Option<(String, i64)> {
    for block in &msg.content {
        if let ContentBlock::Text { text: Some(t) } = block {
            if let Some(name) = extract_tag(t, "command-name") {
                let arg_count = extract_tag(t, "command-args")
                    .map(|a| a.split_whitespace().count() as i64)
                    .unwrap_or(0);
                let trimmed = name.trim().trim_start_matches('/').to_string();
                if !trimmed.is_empty() {
                    return Some((trimmed, arg_count));
                }
            }
        }
    }
    None
}

fn extract_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(&s[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonl::record::parse_line;

    #[test]
    fn user_record_emits_lifecycle_on_first_turn() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        assert!(matches!(out[0], DerivedEvent::SessionLifecycle { .. }));
    }

    #[test]
    fn second_user_record_does_not_repeat_lifecycle() {
        let mut r = Reducer::new();
        let l1 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        let l2 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:05.000Z","message":{"role":"user","content":[]}}"#;
        let _ = r.reduce(&parse_line(l1).unwrap());
        let out = r.reduce(&parse_line(l2).unwrap());
        assert!(out.is_empty());
    }

    #[test]
    fn assistant_emits_token_usage_and_cost() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_emit","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let has_tok = out
            .iter()
            .any(|e| matches!(e, DerivedEvent::TokenUsage { .. }));
        let cost = out.iter().find_map(|e| match e {
            DerivedEvent::CostEntry { cost_usd, .. } => Some(*cost_usd),
            _ => None,
        });
        assert!(has_tok);
        assert!(
            (cost.unwrap() - 15.0).abs() < 1e-9,
            "1M input tokens × $15/Mtok = $15"
        );
    }

    #[test]
    fn assistant_tool_use_emits_tool_call_with_file_path() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let call = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::ToolCall {
                    tool_name,
                    file_path,
                    ..
                } => Some((tool_name.clone(), file_path.clone())),
                _ => None,
            })
            .expect("tool call emitted");
        assert_eq!(call.0, "Read");
        assert_eq!(call.1.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn assistant_task_tool_emits_subagent() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"t1","name":"Task","input":{"subagent_type":"Explore"}}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let st = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::SubAgentCall { subagent_type, .. } => Some(subagent_type.clone()),
                _ => None,
            })
            .expect("subagent emitted");
        assert_eq!(st.as_deref(), Some("Explore"));
    }

    #[test]
    fn user_command_name_tag_emits_slash_command() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"<command-name>/review</command-name><command-args>PR 42</command-args>"}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let sc = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::SlashCommand {
                    name, arg_count, ..
                } => Some((name.clone(), *arg_count)),
                _ => None,
            })
            .expect("slash command emitted");
        assert_eq!(sc, ("review".to_string(), 2));
    }

    #[test]
    fn assistant_with_is_sidechain_marks_usage_as_subagent() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_a","isSidechain":true,"timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-haiku-4-5","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());

        let (token_flag, cost_flag) = out.iter().fold((None, None), |acc, e| match e {
            DerivedEvent::TokenUsage { is_subagent, .. } => (Some(*is_subagent), acc.1),
            DerivedEvent::CostEntry { is_subagent, .. } => (acc.0, Some(*is_subagent)),
            _ => acc,
        });
        assert_eq!(token_flag, Some(true), "TokenUsage missing or wrong is_subagent");
        assert_eq!(cost_flag, Some(true), "CostEntry missing or wrong is_subagent");
    }

    #[test]
    fn assistant_without_is_sidechain_marks_usage_as_main() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","requestId":"req_b","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());

        let token_flag = out.iter().find_map(|e| match e {
            DerivedEvent::TokenUsage { is_subagent, .. } => Some(*is_subagent),
            _ => None,
        });
        assert_eq!(token_flag, Some(false), "TokenUsage should default to is_subagent=false");
        let cost_flag = out.iter().find_map(|e| match e {
            DerivedEvent::CostEntry { is_subagent, .. } => Some(*is_subagent),
            _ => None,
        });
        assert_eq!(cost_flag, Some(false), "CostEntry should default to is_subagent=false");
    }

    #[test]
    fn no_session_id_no_output() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        assert!(r.reduce(&parse_line(line).unwrap()).is_empty());
    }

    #[test]
    fn multi_record_request_emits_usage_once() {
        // Claude Code splits one API call across multiple records, each carrying
        // the same requestId and identical usage. Usage must be counted once;
        // every tool_use block must still produce a ToolCall.
        let mut r = Reducer::new();
        let rec1 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#;
        let rec2 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:02.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.rs"}}]}}"#;
        let rec3 = r#"{"type":"assistant","sessionId":"s1","requestId":"req_A","timestamp":"2026-05-19T10:00:03.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000000,"output_tokens":0},"content":[{"type":"tool_use","id":"t2","name":"Grep","input":{}}]}}"#;
        let mut all = vec![];
        for line in [rec1, rec2, rec3] {
            all.extend(r.reduce(&parse_line(line).unwrap()));
        }
        let tok = all.iter().filter(|e| matches!(e, DerivedEvent::TokenUsage { .. })).count();
        let cost = all.iter().filter(|e| matches!(e, DerivedEvent::CostEntry { .. })).count();
        let tools = all.iter().filter(|e| matches!(e, DerivedEvent::ToolCall { .. })).count();
        assert_eq!(tok, 1, "usage counted once per requestId");
        assert_eq!(cost, 1, "cost counted once per requestId");
        assert_eq!(tools, 2, "every tool_use block still recorded");
    }

    #[test]
    fn assistant_without_request_id_emits_no_usage() {
        let mut r = Reducer::new();
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":2000}}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        assert!(!out.iter().any(|e| matches!(
            e, DerivedEvent::TokenUsage { .. } | DerivedEvent::CostEntry { .. }
        )));
    }

    #[test]
    fn string_content_user_record_emits_slash_command() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":"<command-name>/review</command-name><command-args>PR 42</command-args>"}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let sc = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::SlashCommand { name, arg_count, .. } => {
                    Some((name.clone(), *arg_count))
                }
                _ => None,
            })
            .expect("slash command emitted from string-form content");
        assert_eq!(sc, ("review".to_string(), 2));
    }

    #[test]
    fn string_content_first_turn_emits_session_lifecycle() {
        let mut r = Reducer::new();
        // A session whose very first user record carries string-form content
        // (previously a hard parse error, so the line was dropped entirely).
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","cwd":"/r","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":"<command-name>/config</command-name>"}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let has_lifecycle = out
            .iter()
            .any(|e| matches!(e, DerivedEvent::SessionLifecycle { .. }));
        assert!(has_lifecycle, "string-content first turn must emit SessionLifecycle");
    }
}
