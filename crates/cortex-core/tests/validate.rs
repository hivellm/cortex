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
    assert!(errors
        .iter()
        .any(|e| matches!(e, ValidationError::Schema { .. })));
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

fn law_violation_envelope() -> Value {
    json!({
        "event_id": "01HXYZABCDEF0123456789ABD5",
        "schema_version": "1",
        "occurred_at": "2026-04-17T13:45:00.000Z",
        "session_id": "01HXYZABCDEF0123456789ABCE",
        "stream": "live",
        "tool": "claude-code",
        "kind": "law_violation",
        "context": { "platform": "linux" },
        "payload": {
            "violation_id": "01HXYZABCDEF0123456789ABD5",
            "law_id": "LAW-007",
            "severity": "critical",
            "message": "no --no-verify"
        },
        "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    })
}

#[test]
fn law_violation_with_observed_id_requires_observed_kind() {
    // The graph-writer needs `observed_event_kind` to pick the right
    // MERGE label without phantom-node risk. The schema's allOf/if-then
    // turns "observed_event_id present" into a hard requirement on
    // `observed_event_kind`.
    let mut v = law_violation_envelope();
    v["payload"]["observed_event_id"] = json!("01HXYZABCDEF0123456789ABCF");
    let errors = validate_event(&v).expect_err("missing observed_event_kind must fail");
    assert!(errors.iter().any(|e| match e {
        ValidationError::Schema { path, .. } => path.starts_with("/payload"),
        _ => false,
    }));
}

#[test]
fn law_violation_with_observed_kind_passes() {
    let mut v = law_violation_envelope();
    v["payload"]["observed_event_id"] = json!("01HXYZABCDEF0123456789ABCF");
    v["payload"]["observed_event_kind"] = json!("tool_call");
    validate_event(&v).expect("kind+id together must pass");
}

#[test]
fn law_violation_without_observed_id_does_not_require_observed_kind() {
    // Detector-only contexts that haven't pinned a parent event yet
    // can still publish a violation. The conditional-require only
    // fires when observed_event_id is set.
    validate_event(&law_violation_envelope()).expect("bare violation passes");
}

#[test]
fn law_violation_observed_kind_must_be_canonical_kind_value() {
    let mut v = law_violation_envelope();
    v["payload"]["observed_event_id"] = json!("01HXYZABCDEF0123456789ABCF");
    v["payload"]["observed_event_kind"] = json!("agent_call");
    let errors = validate_event(&v).expect_err("agent_call is not a valid observed kind");
    assert!(!errors.is_empty());
}
