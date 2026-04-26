//! Integration tests for `cortex_graph::mapper` per-kind payload
//! expansion (graph-writer task §4.4).
//!
//! Covers TOUCHED (ToolCall→Artifact), LINKED_TO (Turn→Decision),
//! SUPERSEDES (Decision→Decision), and OF (LawViolation→Law). Each
//! test asserts the patch shape only — the writer test suite covers
//! the Cypher dispatch layer.

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_graph::{map_event_to_patch, EnrichedEvent, GraphPatch};
use serde_json::json;

fn classifier(event_id: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.to_string(),
        kind_refinement: None,
        topics: vec![],
        severity: Severity::Info,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: None,
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
    parent: Option<&str>,
) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("hash-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: repo.map(String::from),
        context_path: path.map(String::from),
        parent_event_id: parent.map(String::from),
    }
}

fn count_edges(patch: &GraphPatch, edge_type: &str) -> usize {
    patch
        .edges
        .iter()
        .filter(|e| e.edge_type == edge_type)
        .count()
}

fn count_nodes(patch: &GraphPatch, label: &str) -> usize {
    patch.nodes.iter().filter(|n| n.label == label).count()
}

// ---------- ToolCall: TOUCHED ----------

#[test]
fn tool_call_with_touched_files_emits_touched_edges() {
    let payload = json!({
        "tool_name": "Edit",
        "input": {},
        "outcome": "success",
        "duration_ms": 12,
        "touched": [
            { "kind": "write", "path": "src/lib.rs" },
            { "kind": "read", "path": "src/main.rs" },
        ]
    });
    let evt = event("tc1", Kind::ToolCall, payload, Some("hivellm/cortex"), None, None);
    let patch = map_event_to_patch(&evt);

    assert_eq!(count_edges(&patch, "TOUCHED"), 2);
    assert_eq!(count_nodes(&patch, "Artifact"), 2);
    let touched_ops: Vec<_> = patch
        .edges
        .iter()
        .filter(|e| e.edge_type == "TOUCHED")
        .map(|e| e.props.get("operation").cloned())
        .collect();
    assert!(touched_ops
        .iter()
        .any(|v| v.as_ref().and_then(|x| x.as_str()) == Some("write")));
    assert!(touched_ops
        .iter()
        .any(|v| v.as_ref().and_then(|x| x.as_str()) == Some("read")));
}

#[test]
fn tool_call_props_carry_tool_name_and_outcome() {
    let payload = json!({
        "tool_name": "Bash",
        "input": { "command": "[REDACTED]" },
        "outcome": "error",
        "touched": []
    });
    let evt = event("tc2", Kind::ToolCall, payload, None, None, None);
    let patch = map_event_to_patch(&evt);

    let tc_node = patch
        .nodes
        .iter()
        .find(|n| n.label == "ToolCall")
        .expect("ToolCall node");
    assert_eq!(
        tc_node.props.get("tool_name").and_then(|v| v.as_str()),
        Some("Bash")
    );
    assert_eq!(
        tc_node.props.get("outcome").and_then(|v| v.as_str()),
        Some("error")
    );
}

#[test]
fn tool_call_with_parent_anchors_under_turn() {
    let payload = json!({
        "tool_name": "Read",
        "input": {},
        "outcome": "success",
        "touched": []
    });
    let evt = event(
        "tc3",
        Kind::ToolCall,
        payload,
        None,
        None,
        Some("turn-parent-id"),
    );
    let patch = map_event_to_patch(&evt);
    let edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "HAS_TOOL_CALL")
        .expect("HAS_TOOL_CALL edge");
    assert_eq!(edge.from_label, "Turn");
    assert_eq!(edge.from_key, "turn-parent-id");
}

#[test]
fn tool_call_without_parent_falls_back_to_session_anchor() {
    let payload = json!({
        "tool_name": "Read",
        "input": {},
        "outcome": "success",
        "touched": []
    });
    let evt = event("tc4", Kind::ToolCall, payload, None, None, None);
    let patch = map_event_to_patch(&evt);
    let edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "HAS_TOOL_CALL")
        .expect("HAS_TOOL_CALL edge");
    assert_eq!(edge.from_label, "Session");
}

// ---------- Decision: LINKED_TO + SUPERSEDES ----------

#[test]
fn decision_with_parent_emits_linked_to_with_status_role() {
    let payload = json!({
        "decision_id": "DEC-0042",
        "title": "Use Meilisearch",
        "status": "accepted",
        "body": "...",
        "tags": ["search", "infra"]
    });
    let evt = event(
        "dec1",
        Kind::Decision,
        payload,
        None,
        None,
        Some("turn-decided-here"),
    );
    let patch = map_event_to_patch(&evt);
    let edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "LINKED_TO")
        .expect("LINKED_TO edge");
    assert_eq!(edge.from_label, "Turn");
    assert_eq!(edge.from_key, "turn-decided-here");
    assert_eq!(edge.to_label, "Decision");
    assert_eq!(edge.to_key, "DEC-0042");
    assert_eq!(
        edge.props.get("role").and_then(|v| v.as_str()),
        Some("accepted")
    );
}

#[test]
fn decision_with_supersedes_emits_supersedes_edge() {
    let payload = json!({
        "decision_id": "DEC-0099",
        "title": "Supersede ADR 0042",
        "status": "accepted",
        "body": "Replaces 0042",
        "supersedes": "DEC-0042"
    });
    let evt = event("dec2", Kind::Decision, payload, None, None, None);
    let patch = map_event_to_patch(&evt);
    let edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "SUPERSEDES")
        .expect("SUPERSEDES edge");
    assert_eq!(edge.from_label, "Decision");
    assert_eq!(edge.from_key, "DEC-0099");
    assert_eq!(edge.to_label, "Decision");
    assert_eq!(edge.to_key, "DEC-0042");
}

#[test]
fn decision_uses_payload_id_as_natural_key() {
    let payload = json!({
        "decision_id": "DEC-0001",
        "title": "first",
        "status": "proposed",
        "body": "text"
    });
    let evt = event("dec3", Kind::Decision, payload, None, None, None);
    let patch = map_event_to_patch(&evt);
    let node = patch
        .nodes
        .iter()
        .find(|n| n.label == "Decision")
        .expect("Decision node");
    assert_eq!(node.natural_key, "DEC-0001");
}

// ---------- LawViolation: OF ----------

#[test]
fn law_violation_emits_of_edge_and_law_node() {
    let payload = json!({
        "violation_id": "VIO-0007",
        "law_id": "LAW-007",
        "severity": "critical",
        "tier": 2,
        "message": "Forbidden destructive operation",
        "evidence": null
    });
    let evt = event("lv1", Kind::LawViolation, payload, None, None, None);
    let patch = map_event_to_patch(&evt);

    // Law node is upserted as an id-only entry — spec 13 enriches.
    let law = patch
        .nodes
        .iter()
        .find(|n| n.label == "Law")
        .expect("Law node");
    assert_eq!(law.natural_key, "LAW-007");

    let edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "OF")
        .expect("OF edge");
    assert_eq!(edge.from_label, "LawViolation");
    assert_eq!(edge.from_key, "VIO-0007");
    assert_eq!(edge.to_label, "Law");
    assert_eq!(edge.to_key, "LAW-007");

    let violation = patch
        .nodes
        .iter()
        .find(|n| n.label == "LawViolation")
        .expect("LawViolation node");
    assert_eq!(
        violation.props.get("severity").and_then(|v| v.as_str()),
        Some("critical")
    );
    assert_eq!(
        violation.props.get("tier").and_then(|v| v.as_u64()),
        Some(2)
    );
}

// ---------- Robustness ----------

#[test]
fn malformed_payload_falls_back_to_identity_only() {
    // Payload missing `tool_name` / `outcome` — serde fails, mapper
    // must still produce a connected patch keyed on event_id.
    let payload = json!({ "garbage": true });
    let evt = event("tc-malformed", Kind::ToolCall, payload, None, None, None);
    let patch = map_event_to_patch(&evt);

    assert!(patch.nodes.iter().any(|n| n.label == "ToolCall"));
    assert!(patch.nodes.iter().any(|n| n.label == "Session"));
    assert_eq!(count_edges(&patch, "HAS_TOOL_CALL"), 1);
    assert_eq!(count_edges(&patch, "TOUCHED"), 0);
}
