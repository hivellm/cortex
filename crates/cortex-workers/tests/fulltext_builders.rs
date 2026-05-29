//! Integration tests for the per-kind doc builders.
//!
//! Spec 08 §Per-kind document builder requires one builder per
//! event family with shared-core fields plus `ext.<family>`
//! extensions; these tests pin the resulting document shape so
//! downstream filters (spec 11 query orchestrator) stay stable.

use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::fulltext::{build_doc, BuildOutcome, EnrichedEvent};
use serde_json::json;

fn classifier(event_id: &str, summary: Option<&str>) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.to_string(),
        kind_refinement: None,
        topics: vec!["rust".into()],
        severity: Severity::Notable,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: summary.map(String::from),
        entities: Vec::new(),
        relations: Vec::new(),
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

fn event(
    event_id: &str,
    kind: Kind,
    payload: serde_json::Value,
    repo: Option<&str>,
    path: Option<&str>,
    summary: Option<&str>,
) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id, summary),
        context_repo: repo.map(String::from),
        context_path: path.map(String::from),
        parent_event_id: None,
        session_id: None,
    occurred_at_ms: 0,
    }
}

#[test]
fn turn_doc_carries_shared_core_and_no_extension() {
    let evt = event(
        "turn-1",
        Kind::Turn,
        json!({ "user_message": "Refactor the HNSW search", "assistant_message": "Ack" }),
        Some("Vectorizer"),
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("turn must build"),
    };
    assert_eq!(doc.id, "turn-1");
    assert_eq!(doc.event_id, "turn-1");
    assert_eq!(doc.kind, "turn");
    assert_eq!(doc.repo.as_deref(), Some("Vectorizer"));
    assert_eq!(doc.severity, "notable");
    assert_eq!(doc.pii_risk, "low");
    assert!(doc.body.contains("Refactor the HNSW search"));
    assert!(doc.body.contains("Ack"));
    assert!(doc.ext.is_empty(), "Turn carries no ext");
    // Phase11k §1.5 — every turn document carries a top-level
    // `turn_id` (= envelope event_id) so `derive_similar_turns`
    // groups hits by turn without re-deriving the id from the doc id.
    assert_eq!(doc.turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn tool_call_doc_carries_ext_with_tool_name_and_outcome() {
    let evt = event(
        "tc-1",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": { "command": "rm -rf /tmp" },
            "outcome": "success",
            "duration_ms": 25,
            "touched": []
        }),
        Some("Vectorizer"),
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("tool_call must build"),
    };
    assert_eq!(doc.kind, "tool_call");
    assert_eq!(doc.title, "Edit");
    let ext = doc.ext.get("tool_call").expect("ext.tool_call");
    assert_eq!(ext["tool_name"], "Edit");
    assert_eq!(ext["outcome"], "success");
    assert_eq!(ext["duration_ms"], 25);
}

#[test]
fn decision_doc_carries_ext_with_status_and_supersedes() {
    let evt = event(
        "dec-1",
        Kind::Decision,
        json!({
            "decision_id": "DEC-0042",
            "title": "Adopt Meilisearch",
            "status": "accepted",
            "body": "We pick Meili over Lexum for v1.",
            "supersedes": "DEC-0001",
            "tags": ["search", "infra"]
        }),
        None,
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("decision must build"),
    };
    assert_eq!(doc.title, "Adopt Meilisearch");
    let ext = doc.ext.get("decision").expect("ext.decision");
    assert_eq!(ext["decision_id"], "DEC-0042");
    assert_eq!(ext["status"], "accepted");
    assert_eq!(ext["supersedes"], "DEC-0001");
    assert!(ext["tags"].is_array());
    // Phase11k §1.5 — top-level governance projection: the spec-11
    // lane projection contract reads these directly off
    // `extras_raw`, so they MUST be stamped on the wire payload in
    // addition to the legacy `ext.decision.*` nesting (which the
    // dashboard's facet view still uses for its filterable schema).
    assert_eq!(doc.decision_id.as_deref(), Some("DEC-0042"));
    assert_eq!(doc.decision_title.as_deref(), Some("Adopt Meilisearch"));
    assert_eq!(doc.decision_status.as_deref(), Some("accepted"));
    assert_eq!(doc.decision_supersedes.as_deref(), Some("DEC-0001"));
}

#[test]
fn law_violation_doc_carries_ext_with_law_id_and_tier() {
    let evt = event(
        "lv-1",
        Kind::LawViolation,
        json!({
            "violation_id": "VIO-0007",
            "law_id": "LAW-007",
            "severity": "critical",
            "tier": 2,
            "message": "Forbidden destructive op",
            "evidence": null
        }),
        None,
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("law_violation must build"),
    };
    let ext = doc.ext.get("law_violation").expect("ext.law_violation");
    assert_eq!(ext["violation_id"], "VIO-0007");
    assert_eq!(ext["law_id"], "LAW-007");
    assert_eq!(ext["tier"], 2);
    // Phase11k §1.5 — top-level governance projection mirrors the
    // legacy `ext.law_violation.*` nesting onto `law_id` /
    // `law_severity` / `law_tier` so the spec-11 lane projection
    // contract surfaces the violation overlay.
    assert_eq!(doc.law_id.as_deref(), Some("LAW-007"));
    assert_eq!(doc.law_severity.as_deref(), Some("critical"));
    assert_eq!(doc.law_tier.as_deref(), Some("2"));
}

#[test]
fn bootstrap_doc_id_uses_repo_path_hash() {
    let evt = event(
        "evt-bootstrap",
        Kind::Artifact,
        json!({ "artifact_type": "file", "path": "src/lib.rs", "body": "fn main() {}" }),
        Some("Vectorizer"),
        Some("src/lib.rs"),
        None,
    );
    let out = build_doc(&evt, true, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("artifact must build"),
    };
    assert_eq!(doc.id, "bootstrap:Vectorizer:src/lib.rs:h-evt-bootstrap");
}

#[test]
fn empty_payload_returns_skipped_outcome() {
    let evt = event(
        "evt-empty",
        Kind::Artifact,
        json!({ "artifact_type": "file" }),
        None,
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    assert_eq!(out, BuildOutcome::Skipped);
}

#[test]
fn oversize_with_summary_uses_summary_as_body() {
    let big = "x".repeat(8 * 1024);
    let evt = event(
        "evt-big",
        Kind::Decision,
        json!({
            "decision_id": "DEC-1",
            "title": "Big",
            "status": "accepted",
            "body": big,
        }),
        None,
        None,
        Some("Concise summary"),
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("must build"),
    };
    assert_eq!(doc.body, "Concise summary");
    assert!(!doc.truncated);
}

#[test]
fn body_truncates_to_max_bytes_and_flips_flag() {
    let raw = "y".repeat(12_000);
    let evt = event(
        "evt-trunc",
        Kind::Turn,
        json!({ "user_message": raw }),
        None,
        None,
        None,
    );
    let out = build_doc(&evt, false, 8 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("must build"),
    };
    assert!(doc.truncated);
    assert!(doc.body.len() <= 8 * 1024);
}

/// Regression for phase2_static_classifier_summary_preserves_text §3.3:
/// when the classifier does not synthesise a summary (the static path
/// since the 2026-04-27 fix), the Meili document's `body` MUST hold the
/// envelope's source text — not the literal string `"static summary: N
/// chars"` that was being copied into the index before the fix.
#[test]
fn body_falls_back_to_envelope_text_when_classifier_summary_is_none() {
    let payload_text = "fn search(query: &str) -> Vec<Hit> { /* HNSW recall */ }";
    let evt = event(
        "evt-no-summary",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "src/search.rs",
            "body": payload_text,
        }),
        Some("Vectorizer"),
        Some("src/search.rs"),
        None, // classifier.summary = None — the static-path contract
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("must build with raw fallback"),
    };
    assert_eq!(
        doc.body, payload_text,
        "body must mirror envelope text when summary is None"
    );
    assert!(
        doc.summary.is_none(),
        "doc.summary stays None — clients fall back to body / source text"
    );
    assert!(!doc.truncated);
}

#[test]
fn artifact_doc_carries_path_prefixes_for_filterable_scope() {
    // Phase11b §1.3 — every projected document gets a
    // `path_prefixes` array spanning every ancestor directory plus
    // the full path. The Meili lane filters on this array via
    // `path_prefixes IN [...]` because Meilisearch rejects
    // `path STARTS WITH ...` outright.
    let evt = event(
        "art-prefix",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "crates/cortex-api/src/meili_lane.rs",
            "body": "fn build_filter() {}",
        }),
        Some("Cortex"),
        Some("crates/cortex-api/src/meili_lane.rs"),
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("artifact must build"),
    };
    assert_eq!(
        doc.path_prefixes,
        vec![
            "crates/".to_string(),
            "crates/cortex-api/".to_string(),
            "crates/cortex-api/src/".to_string(),
            "crates/cortex-api/src/meili_lane.rs".to_string(),
        ],
    );
}

#[test]
fn doc_without_context_path_has_empty_path_prefixes() {
    // Phase11b §1.3 — turns / tool calls without `context_path`
    // get an empty `path_prefixes` so the field stays absent in the
    // serialised JSON (skip_serializing_if = "Vec::is_empty").
    let evt = event(
        "turn-no-path",
        Kind::Turn,
        json!({ "user_message": "hi", "assistant_message": "hi" }),
        Some("Cortex"),
        None,
        None,
    );
    let out = build_doc(&evt, false, 1024 * 1024);
    let doc = match out {
        BuildOutcome::Ready(d) => *d,
        BuildOutcome::Skipped => panic!("turn must build"),
    };
    assert!(doc.path_prefixes.is_empty());
}
