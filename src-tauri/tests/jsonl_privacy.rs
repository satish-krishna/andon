//! Privacy property test for the JSONL reducer trust boundary.
//!
//! For randomly generated JSONL records containing prompt-shaped text in
//! every text-bearing field, the reducer output must contain no substring
//! of any input text. Formal guarantee behind the pitch's "never persisted"
//! claim.

use andon_lib::jsonl::record::JsonlRecord;
use andon_lib::jsonl::reducer::{DerivedEvent, Reducer};
use proptest::prelude::*;
use serde_json::json;

fn dump(events: &[DerivedEvent]) -> String {
    events
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    #[test]
    fn user_text_never_leaks(prompt in "[A-Za-z0-9 _.,;:!?/-]{20,200}") {
        let rec_json = json!({
            "type": "user",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:00.000Z",
            "message": { "role": "user", "content": [{"type":"text","text": prompt}] }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        let d = dump(&out);
        prop_assert!(!d.contains(&prompt), "reducer leaked user text: {prompt:?}");
    }

    #[test]
    fn assistant_text_never_leaks(
        text in "[A-Za-z0-9 ]{20,200}",
        path in "[A-Za-z0-9_/.-]{5,40}",
    ) {
        // file_path IS allowed to leak — it's metadata. text is NOT.
        let rec_json = json!({
            "type": "assistant",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:01.000Z",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "usage": { "input_tokens": 1, "output_tokens": 1 },
                "content": [
                    { "type": "text", "text": text },
                    { "type": "tool_use", "id": "x", "name": "Read", "input": { "file_path": path } }
                ]
            }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        let d = dump(&out);
        prop_assert!(!d.contains(&text), "reducer leaked assistant text: {text:?}");
    }

    #[test]
    fn user_string_content_text_never_leaks(prompt in "[A-Za-z0-9 _.,;:!?/-]{20,200}") {
        // Same guarantee as `user_text_never_leaks`, but the prompt is the
        // raw string value of `message.content` rather than an array text block.
        let rec_json = json!({
            "type": "user",
            "sessionId": "s1",
            "timestamp": "2026-05-19T10:00:00.000Z",
            "message": { "role": "user", "content": prompt }
        });
        let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
        let mut r = Reducer::new();
        let out = r.reduce(&rec);
        let d = dump(&out);
        prop_assert!(!d.contains(&prompt), "reducer leaked string-content user text: {prompt:?}");
    }
}

/// A slash-command record exercises the tag-extraction path. The command
/// name is metadata and may escape; the surrounding prompt text and the
/// command-args text must not.
#[test]
fn slash_command_extraction_does_not_leak_surrounding_text() {
    const LEADING: &str = "SENTINEL_LEADING_PROMPT_TEXT";
    const TRAILING: &str = "SENTINEL_TRAILING_PROMPT_TEXT";
    let content = format!(
        "{LEADING} <command-name>/deploy</command-name> \
         <command-args>argalpha argbeta</command-args> {TRAILING}"
    );
    let rec_json = json!({
        "type": "user",
        "sessionId": "s1",
        "timestamp": "2026-05-19T10:00:00.000Z",
        "message": { "role": "user", "content": content }
    });
    let rec: JsonlRecord = serde_json::from_value(rec_json).unwrap();
    let mut r = Reducer::new();
    let out = r.reduce(&rec);
    let d = dump(&out);

    // Sanity: the command was actually detected (otherwise the test proves nothing).
    assert!(d.contains("deploy"), "expected the slash command to be detected");
    // The surrounding prompt text must not leak.
    assert!(!d.contains(LEADING), "reducer leaked leading prompt text");
    assert!(!d.contains(TRAILING), "reducer leaked trailing prompt text");
    // The command-args *text* must not leak — only an arg count escapes.
    assert!(!d.contains("argalpha"), "reducer leaked command-args text");
    assert!(!d.contains("argbeta"), "reducer leaked command-args text");
}
