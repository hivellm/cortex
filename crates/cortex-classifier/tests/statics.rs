//! Integration tests for `cortex_classifier::statics`.

use cortex_classifier::statics::StaticClassifier;
use cortex_classifier::types::{Classifier, ClassifierSource, EnrichmentInput, PiiRisk, Severity};
use cortex_core::events::Kind;
use serde_json::{json, Value};

fn input(kind: Kind, payload: Value) -> EnrichmentInput {
    EnrichmentInput {
        event_id: "01H".into(),
        kind,
        content_hash: "sha256:abc".into(),
        redacted_payload: payload,
        context_repo: None,
    }
}

#[tokio::test]
async fn tool_call_git_push_is_notable() {
    let c = StaticClassifier::new();
    let out = c
        .classify_batch(&[input(
            Kind::ToolCall,
            json!({
                "tool_name": "Bash",
                "input": { "command": "git push origin main" },
                "outcome": "success"
            }),
        )])
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].severity, Severity::Notable);
    assert_eq!(out[0].kind_refinement.as_deref(), Some("git_push"));
    assert!(out[0].topics.iter().any(|t| t == "git_push"));
    assert_eq!(out[0].source, ClassifierSource::StaticFallback);
}

#[tokio::test]
async fn tool_call_blocked_by_law_is_critical() {
    let c = StaticClassifier::new();
    let out = c
        .classify_batch(&[input(
            Kind::ToolCall,
            json!({
                "tool_name": "Bash",
                "input": { "command": "git commit --no-verify" },
                "outcome": "blocked_by_law:LAW-007"
            }),
        )])
        .await
        .unwrap();
    assert_eq!(out[0].severity, Severity::Critical);
    assert!(out[0].topics.iter().any(|t| t == "law"));
    assert!(out[0].topics.iter().any(|t| t == "governance"));
}

#[tokio::test]
async fn redacted_content_marks_high_pii() {
    let c = StaticClassifier::new();
    let out = c
        .classify_batch(&[input(
            Kind::ToolCall,
            json!({
                "tool_name": "Bash",
                "input": { "command": "curl -H 'Authorization: Bearer [REDACTED:github_token]'" },
                "outcome": "success"
            }),
        )])
        .await
        .unwrap();
    assert_eq!(out[0].pii_risk, PiiRisk::High);
}

/// The static path used to emit `summary = "static summary: <N> chars"`
/// for any payload over 4 KB. The fulltext worker then copied that
/// string verbatim into Meilisearch's `body` field, destroying full-text
/// search on every artifact (the indexed body had no real tokens). The
/// static path now returns `summary: None` and downstream consumers
/// fall back to the source `text`. Test pinned to lock the contract.
#[tokio::test]
async fn oversize_payload_does_not_synthesise_summary() {
    let c = StaticClassifier::new();
    let big = "x".repeat(5000);
    let out = c
        .classify_batch(&[input(
            Kind::Turn,
            json!({
                "user_message": big
            }),
        )])
        .await
        .unwrap();
    assert!(
        out[0].summary.is_none(),
        "static path must not synthesise a summary; got: {:?}",
        out[0].summary
    );
}

#[tokio::test]
async fn batch_preserves_order() {
    let c = StaticClassifier::new();
    let out = c
        .classify_batch(&[
            EnrichmentInput {
                event_id: "a".into(),
                kind: Kind::Turn,
                content_hash: "sha256:a".into(),
                redacted_payload: json!({ "user_message": "hi" }),
                context_repo: None,
            },
            EnrichmentInput {
                event_id: "b".into(),
                kind: Kind::Turn,
                content_hash: "sha256:b".into(),
                redacted_payload: json!({ "user_message": "bye" }),
                context_repo: None,
            },
        ])
        .await
        .unwrap();
    assert_eq!(out[0].event_id, "a");
    assert_eq!(out[1].event_id, "b");
}

#[tokio::test]
async fn decision_is_notable() {
    let c = StaticClassifier::new();
    let out = c
        .classify_batch(&[input(
            Kind::Decision,
            json!({
                "decision_id": "DEC-0042",
                "title": "Raise HNSW ef_search default to 128",
                "status": "accepted",
                "body": "..."
            }),
        )])
        .await
        .unwrap();
    assert_eq!(out[0].severity, Severity::Notable);
    assert!(out[0].topics.iter().any(|t| t == "decision"));
}
