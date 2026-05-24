//! Trust boundary between JSONL (raw, contains prompt text) and the rest
//! of the ingest pipeline.
//!
//! Prior to the 2026-05-24 privacy-contract amendment, no reducer output
//! variant could carry prompt text. The amendment introduces one
//! exception: `DerivedEvent::PromptTurn` carries the raw prompt for
//! Skill Finder and coach detectors. The reducer remains the SINGLE
//! chokepoint for prompts entering the local DB — no other module reads
//! `record::Message.content[].text`. See
//! docs/superpowers/specs/2026-05-24-ai-engineering-coach-integration-design.md
//! §Privacy contract amendment.

use std::collections::HashMap;

use crate::jsonl::record::{ContentBlock, JsonlRecord, Message};

/// Output of the reducer.
///
/// All variants are text-free EXCEPT `PromptTurn`, which is the single
/// amendment introduced by the 2026-05-24 privacy-contract update.
/// `PromptTurn` carries the raw user prompt text for Skill Finder and coach
/// detectors. The privacy property test in `tests/jsonl_privacy.rs` carves
/// out `PromptTurn` from its "no text leak" assertion accordingly.
#[derive(Debug, Clone)]
pub enum DerivedEvent {
    /// Carries the raw user prompt text.
    /// Privacy-contract amendment: this is the ONLY variant permitted to
    /// hold prompt text. All other variants remain text-free.
    PromptTurn {
        session_id: String,
        request_id: Option<String>,
        turn_index: i64,
        ts_ms: i64,
        text: String,
        norm_hash: String,
        command: Option<String>,
        length: i64,
        has_file_ref: bool,
        has_code: bool,
        has_constraint: bool,
    },
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

/// Options that control reducer behaviour. Pass `ReduceOptions::default()`
/// (via `Reducer::new()`) for no behaviour change relative to pre-B4.
#[derive(Default, Clone)]
pub struct ReduceOptions {
    /// Keywords whose presence in a user prompt sets `has_constraint = true`
    /// on the emitted `PromptTurn`. Case-insensitive substring match.
    pub constraint_keywords: Vec<String>,
}

#[derive(Default)]
pub struct Reducer {
    first_turn_seen: bool,
    seen_requests: std::collections::HashSet<String>,
    /// Per-session count of user turns seen (for `turn_index`).
    turn_counters: HashMap<String, i64>,
    opts: ReduceOptions,
}

impl Reducer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a reducer with custom options (e.g. constraint keywords for
    /// the AI Engineering Coach).
    pub fn with_options(opts: ReduceOptions) -> Self {
        Self {
            opts,
            ..Self::default()
        }
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
        // Emit PromptTurn for every user record that carries non-empty text.
        // The turn_index is a per-session ordinal starting at 0.
        let turn_index = {
            let counter = self.turn_counters.entry(sid.to_string()).or_insert(0);
            let idx = *counter;
            *counter += 1;
            idx
        };
        if let Some(ev) = build_prompt_turn(rec, sid, turn_index, ts, &self.opts) {
            out.push(ev);
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

/// Build a `PromptTurn` event from a user record if the record carries
/// non-empty text content.
///
/// `ts` is passed in (already computed by the caller) to avoid a redundant
/// parse. `turn_index` is a per-session ordinal maintained by `Reducer`.
fn build_prompt_turn(
    rec: &JsonlRecord,
    sid: &str,
    turn_index: i64,
    ts: i64,
    opts: &ReduceOptions,
) -> Option<DerivedEvent> {
    let msg = rec.message.as_ref()?;
    if msg.role.as_deref() != Some("user") {
        return None;
    }

    let text: String = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text: Some(t) } => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if text.is_empty() {
        return None;
    }

    let length = text.chars().count() as i64;

    // `has_file_ref`: '@' sigil, absolute Unix paths (/…), or Windows-style
    // drive letters (C:\…) detected by a ':' as the second byte of a word.
    let has_file_ref = text.contains('@')
        || text.split_whitespace().any(|w| {
            w.starts_with('/') || (w.len() > 2 && w.as_bytes().get(1) == Some(&b':'))
        });

    let has_code = text.contains("```");

    let lc = text.to_lowercase();
    let has_constraint = opts
        .constraint_keywords
        .iter()
        .any(|kw| lc.contains(&kw.to_lowercase()));

    // A slash command in the plain text (not the XML-tag form handled by
    // `detect_slash_command`). Only set when the ENTIRE prompt starts with '/'.
    let command = text
        .trim()
        .strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
        .map(|s| s.to_string());

    // Placeholder hash: purely byte-based blake3. Task G1 replaces this with
    // a keyed-hash normaliser that collapses semantically equivalent prompts.
    let norm_hash = blake3::hash(text.as_bytes()).to_string();

    Some(DerivedEvent::PromptTurn {
        session_id: sid.to_string(),
        request_id: rec.request_id.clone(),
        turn_index,
        ts_ms: ts,
        text,
        norm_hash,
        command,
        length,
        has_file_ref,
        has_code,
        has_constraint,
    })
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

    // ── B4: PromptTurn tests ────────────────────────────────────────────────

    #[test]
    fn user_message_emits_prompt_turn_with_derived_flags() {
        let mut r = Reducer::with_options(ReduceOptions {
            constraint_keywords: vec!["must".into(), "should".into()],
            ..Default::default()
        });
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Refactor @src/foo.rs — must be idempotent."}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());

        let (text, has_file_ref, has_constraint, length) = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn {
                    text,
                    has_file_ref,
                    has_constraint,
                    length,
                    ..
                } => Some((text.clone(), *has_file_ref, *has_constraint, *length)),
                _ => None,
            })
            .expect("a PromptTurn event must be emitted");

        assert_eq!(text, "Refactor @src/foo.rs \u{2014} must be idempotent.");
        assert!(has_file_ref, "has_file_ref should be true — '@src/foo.rs'");
        assert!(has_constraint, "has_constraint should be true — 'must'");
        // char count: "Refactor @src/foo.rs — must be idempotent." — em-dash is 1 char
        assert_eq!(length, text.chars().count() as i64);
    }

    #[test]
    fn user_message_norm_hash_differs_for_different_text() {
        // Placeholder hash (B4): purely text-based blake3, no normalisation.
        // G1 will introduce normalisation that collapses semantically equivalent prompts.
        let a = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Package the extension"}]}}"#;
        let b = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"Ship the release"}]}}"#;

        let mut ra_reducer = Reducer::new();
        let mut rb_reducer = Reducer::new();
        let ea = ra_reducer.reduce(&parse_line(a).unwrap());
        let eb = rb_reducer.reduce(&parse_line(b).unwrap());

        let ha = ea
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn { norm_hash, .. } => Some(norm_hash.clone()),
                _ => None,
            })
            .unwrap();
        let hb = eb
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn { norm_hash, .. } => Some(norm_hash.clone()),
                _ => None,
            })
            .unwrap();

        assert_ne!(ha, hb, "different text must produce different placeholder hashes");
    }

    #[test]
    fn prompt_turn_turn_index_increments_per_session() {
        let mut r = Reducer::new();
        let l1 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"first turn"}]}}"#;
        let l2 = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"second turn"}]}}"#;

        let out1 = r.reduce(&parse_line(l1).unwrap());
        let out2 = r.reduce(&parse_line(l2).unwrap());

        let idx1 = out1
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn { turn_index, .. } => Some(*turn_index),
                _ => None,
            })
            .expect("PromptTurn for first turn");
        let idx2 = out2
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn { turn_index, .. } => Some(*turn_index),
                _ => None,
            })
            .expect("PromptTurn for second turn");

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
    }

    #[test]
    fn empty_content_does_not_emit_prompt_turn() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        assert!(
            !out.iter().any(|e| matches!(e, DerivedEvent::PromptTurn { .. })),
            "empty content must not emit PromptTurn"
        );
    }

    #[test]
    fn has_code_flag_set_when_triple_backtick_present() {
        let mut r = Reducer::new();
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Fix this:\n```rust\nfn main() {}\n```"}]}}"#;
        let out = r.reduce(&parse_line(line).unwrap());
        let has_code = out
            .iter()
            .find_map(|e| match e {
                DerivedEvent::PromptTurn { has_code, .. } => Some(*has_code),
                _ => None,
            })
            .expect("PromptTurn emitted");
        assert!(has_code, "has_code should be true when ``` is present");
    }
}
