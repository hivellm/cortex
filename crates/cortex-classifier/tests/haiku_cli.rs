//! Integration tests for `cortex_classifier::haiku_cli`.

use cortex_classifier::haiku_cli::{
    normalise_topics, ClassifierOutputBatch, ClaudeJsonResponse,
};
use cortex_classifier::types::Severity;

#[test]
fn normalise_drops_out_of_vocab() {
    let topics = vec!["code".into(), "not_a_topic".into(), "test".into()];
    let out = normalise_topics(topics);
    assert!(out.iter().any(|t| t == "code"));
    assert!(out.iter().any(|t| t == "test"));
    assert!(!out.iter().any(|t| t == "not_a_topic"));
}

#[test]
fn parses_expected_cli_shape() {
    let sample = r#"
    {
        "text": "{\"events\":[{\"event_id\":\"01H\",\"kind_refinement\":\"git_push\",\"topics\":[\"git\",\"git_push\"],\"severity\":\"notable\",\"pii_risk\":\"low\",\"redaction_suggestions\":[],\"summary\":null}]}",
        "tokens": { "input": 42, "output": 9 }
    }"#;
    let outer: ClaudeJsonResponse = serde_json::from_str(sample).unwrap();
    assert!(outer.text.is_some());
    let inner: ClassifierOutputBatch = serde_json::from_str(outer.text.unwrap().trim()).unwrap();
    assert_eq!(inner.events.len(), 1);
    assert_eq!(inner.events[0].severity, Severity::Notable);
}
