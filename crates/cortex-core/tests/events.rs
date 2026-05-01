//! Integration tests for `cortex_core::events`.

use cortex_core::events::{Kind, Stream, ToolCall, ToolCallOutput};
use serde_json::json;

#[test]
fn kind_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&Kind::ToolCall).unwrap(),
        "\"tool_call\""
    );
    assert_eq!(
        serde_json::to_string(&Kind::LawViolation).unwrap(),
        "\"law_violation\""
    );
}

#[test]
fn stream_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&Stream::Live).unwrap(), "\"live\"");
    assert_eq!(
        serde_json::to_string(&Stream::Bootstrap).unwrap(),
        "\"bootstrap\""
    );
}

#[test]
fn round_trip_tool_call() {
    let tc = ToolCall {
        tool_name: "Bash".into(),
        input: json!({ "command": "ls" }),
        output: Some(ToolCallOutput {
            stdout: Some("foo".into()),
            exit_code: Some(0),
            ..Default::default()
        }),
        duration_ms: Some(12),
        touched: vec![],
        outcome: "success".into(),
    };
    let encoded = serde_json::to_string(&tc).unwrap();
    let decoded: ToolCall = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, tc);
}

#[test]
fn kind_schema_stems() {
    assert_eq!(Kind::ToolCall.schema_stem(), "tool_call");
    assert_eq!(Kind::LawViolation.schema_stem(), "law_violation");
}
