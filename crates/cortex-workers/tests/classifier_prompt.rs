//! Integration tests for `cortex_workers::classifier::prompt`.

use cortex_core::events::Kind;
use cortex_workers::classifier::prompt::PROMPT_V1;
use cortex_workers::classifier::types::EnrichmentInput;
use serde_json::json;

#[test]
fn renders_placeholders() {
    let inputs = vec![EnrichmentInput {
        event_id: "01H".to_string(),
        kind: Kind::ToolCall,
        content_hash: "sha256:abc".into(),
        redacted_payload: json!({ "tool_name": "Bash", "input": { "command": "ls" }, "outcome": "success" }),
        context_repo: Some("Cortex".into()),
    }];
    let rendered = PROMPT_V1.render(&inputs).unwrap();
    assert!(!rendered.contains("{{TOPIC_VOCAB}}"));
    assert!(!rendered.contains("{{EVENTS_JSON}}"));
    assert!(rendered.contains("\"event_id\":\"01H\""));
    assert!(rendered.contains("code"));
}
