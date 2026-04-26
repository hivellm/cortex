//! Integration tests for `cortex_core::validate`.

use cortex_core::{validate_event, ValidationError};
use serde_json::{json, Value};

fn fresh_envelope() -> Value {
    json!({
        "event_id": "01HXYZABCDEF0123456789ABCD",
        "schema_version": "1",
        "occurred_at": "2026-04-17T12:34:56.789Z",
        "session_id": "01HXYZABCDEF0123456789ABCD",
        "stream": "live",
        "tool": "claude-code",
        "kind": "tool_call",
        "context": { "platform": "linux" },
        "payload": {
            "tool_name": "Bash",
            "input": { "command": "ls" },
            "outcome": "success"
        },
        "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    })
}

#[test]
fn valid_event_passes() {
    validate_event(&fresh_envelope()).expect("valid event should pass");
}

#[test]
fn missing_required_field_fails() {
    let mut v = fresh_envelope();
    v.as_object_mut().unwrap().remove("content_hash");
    let errors = validate_event(&v).unwrap_err();
    assert!(!errors.is_empty());
}

#[test]
fn unknown_kind_fails() {
    let mut v = fresh_envelope();
    v["kind"] = json!("nonsense_kind");
    let errors = validate_event(&v).unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, ValidationError::Schema { .. })));
}

#[test]
fn payload_shape_mismatch_fails() {
    let mut v = fresh_envelope();
    v["payload"] = json!({ "tool_name": "Bash" });
    let errors = validate_event(&v).unwrap_err();
    assert!(errors.iter().any(|e| match e {
        ValidationError::Schema { path, .. } => path.starts_with("/payload"),
        _ => false,
    }));
}

#[test]
fn bad_content_hash_fails() {
    let mut v = fresh_envelope();
    v["content_hash"] = json!("md5:abc");
    let errors = validate_event(&v).unwrap_err();
    assert!(!errors.is_empty());
}
