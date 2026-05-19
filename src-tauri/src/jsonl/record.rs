//! Lenient deserialiser for Claude Code's per-session JSONL transcripts.
//! Every field is `Option<T>` so unknown/missing fields cannot abort parse.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlRecord {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub version: Option<String>,
    pub cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    pub message: Option<Message>,
    #[serde(rename = "isMeta", default)]
    pub is_meta: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    pub cache_creation: i64,
    #[serde(default, rename = "cache_read_input_tokens")]
    pub cache_read: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: Option<String> },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: Option<String>,
        name: Option<String>,
        #[serde(default)]
        input: Value,
    },
    #[serde(other)]
    Other,
}

pub fn parse_line(line: &str) -> Result<JsonlRecord, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_record() {
        let line = r#"{"type":"user","sessionId":"s1","timestamp":"2026-05-19T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let r = parse_line(line).expect("parse");
        assert_eq!(r.kind.as_deref(), Some("user"));
        assert_eq!(r.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn parses_assistant_with_tool_use() {
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-19T10:00:01.000Z","message":{"role":"assistant","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x/y.rs"}}]}}"#;
        let r = parse_line(line).expect("parse");
        let msg = r.message.as_ref().expect("message");
        assert_eq!(msg.model.as_deref(), Some("claude-opus-4-7"));
        let u = msg.usage.as_ref().expect("usage");
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.cache_read, 5);
        match &msg.content[0] {
            ContentBlock::ToolUse { name, .. } => assert_eq!(name.as_deref(), Some("Read")),
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn unknown_record_type_does_not_fail() {
        let line = r#"{"type":"super_event_2027","sessionId":"s1"}"#;
        assert_eq!(
            parse_line(line).unwrap().kind.as_deref(),
            Some("super_event_2027")
        );
    }

    #[test]
    fn missing_fields_default_to_none() {
        let r = parse_line(r#"{"type":"summary"}"#).unwrap();
        assert!(r.session_id.is_none());
        assert!(r.message.is_none());
    }

    #[test]
    fn extra_unknown_fields_ignored() {
        let line = r#"{"type":"user","sessionId":"s1","futureField":42}"#;
        assert_eq!(parse_line(line).unwrap().kind.as_deref(), Some("user"));
    }
}
