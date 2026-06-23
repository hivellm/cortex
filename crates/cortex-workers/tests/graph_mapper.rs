//! Integration tests for `cortex_workers::graph::mapper` per-kind payload
//! expansion (graph-writer task §4.4).
//!
//! Covers TOUCHED (ToolCall→Artifact), LINKED_TO (Turn→Decision),
//! SUPERSEDES (Decision→Decision), and OF (LawViolation→Law). Each
//! test asserts the patch shape only — the writer test suite covers
//! the Cypher dispatch layer.

use cortex_core::events::Kind;
use cortex_workers::classifier::types::{ExtractedEntity, ExtractedRelation};
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::graph::{map_event_to_patch, EnrichedEvent, GraphPatch};
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
        entities: Vec::new(),
        relations: Vec::new(),
        sensitivity: Default::default(),
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
        session_id: None,
        occurred_at_ms: 0,
        class_level: None,
        class_compartments: None,
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
    let evt = event(
        "tc1",
        Kind::ToolCall,
        payload,
        Some("hivellm/cortex"),
        None,
        None,
    );
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

// ---------- phase27a §2.3 — structural mapper edge confidence ----------

#[test]
fn structural_mapper_edges_carry_extracted_confidence() {
    // A ToolCall yields structural edges (HAS_TOOL_CALL, TOUCHED, IN_REPO)
    // — all deterministic facts → `Extracted`. The static-fallback
    // classifier emits no relations, so there are no Inferred edges here.
    let payload = json!({
        "tool_name": "Edit",
        "input": {},
        "outcome": "success",
        "duration_ms": 12,
        "touched": [ { "kind": "write", "path": "src/lib.rs" } ]
    });
    let evt = event(
        "tc-conf",
        Kind::ToolCall,
        payload,
        Some("hivellm/cortex"),
        None,
        None,
    );
    let patch = map_event_to_patch(&evt);
    assert!(!patch.edges.is_empty(), "expected structural edges");
    for e in &patch.edges {
        assert_eq!(
            e.props.get("confidence").and_then(|v| v.as_str()),
            Some("extracted"),
            "structural edge {} must carry Extracted confidence",
            e.edge_type
        );
    }
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

#[test]
fn law_violation_with_observed_kind_tool_call_emits_observed_in_edge() {
    let payload = json!({
        "violation_id": "01HXVIO00000000000000000A1",
        "law_id": "LAW-007",
        "severity": "critical",
        "message": "no --no-verify",
        "evidence": null,
        "observed_event_id": "01HXTC0000000000000000000Z",
        "observed_event_kind": "tool_call"
    });
    let evt = event("lv-with-tc", Kind::LawViolation, payload, None, None, None);
    let patch = map_event_to_patch(&evt);

    let observed_in = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "OBSERVED_IN")
        .expect("OBSERVED_IN edge for tool_call kind");
    assert_eq!(observed_in.from_label, "LawViolation");
    assert_eq!(observed_in.from_key, "01HXVIO00000000000000000A1");
    assert_eq!(observed_in.to_label, "ToolCall");
    assert_eq!(observed_in.to_key, "01HXTC0000000000000000000Z");
}

#[test]
fn law_violation_with_observed_kind_turn_picks_turn_label() {
    let payload = json!({
        "violation_id": "01HXVIO00000000000000000A2",
        "law_id": "LAW-014",
        "severity": "notable",
        "message": "scope drift",
        "evidence": null,
        "observed_event_id": "01HXTURN0000000000000000Z",
        "observed_event_kind": "turn"
    });
    let evt = event(
        "lv-with-turn",
        Kind::LawViolation,
        payload,
        None,
        None,
        None,
    );
    let patch = map_event_to_patch(&evt);

    let observed_in = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "OBSERVED_IN")
        .expect("OBSERVED_IN edge for turn kind");
    assert_eq!(observed_in.to_label, "Turn");
    assert_eq!(observed_in.to_key, "01HXTURN0000000000000000Z");
}

#[test]
fn law_violation_without_observed_event_omits_observed_in_edge() {
    let payload = json!({
        "violation_id": "01HXVIO00000000000000000A3",
        "law_id": "LAW-019",
        "severity": "info",
        "message": "soft notice",
        "evidence": null
    });
    let evt = event("lv-bare", Kind::LawViolation, payload, None, None, None);
    let patch = map_event_to_patch(&evt);
    assert!(
        patch.edges.iter().all(|e| e.edge_type != "OBSERVED_IN"),
        "no OBSERVED_IN edge expected when observed_event_id is unset"
    );
}

#[test]
fn law_violation_with_unknown_observed_kind_drops_edge_safely() {
    // Defensive: even though the spec-04 schema's allOf/if-then
    // enforces the discriminator, the mapper must still degrade
    // gracefully on unexpected values that slip past validation.
    let payload = json!({
        "violation_id": "01HXVIO00000000000000000A4",
        "law_id": "LAW-099",
        "severity": "info",
        "message": "guard test",
        "evidence": null,
        "observed_event_id": "01HXOTHER000000000000000Z",
        "observed_event_kind": "agent_call"
    });
    let evt = event("lv-bad-kind", Kind::LawViolation, payload, None, None, None);
    let patch = map_event_to_patch(&evt);
    assert!(
        patch.edges.iter().all(|e| e.edge_type != "OBSERVED_IN"),
        "unknown observed_event_kind must skip the edge, not pick a phantom label"
    );
}

// ---------- Analysis ----------

#[test]
fn imported_analysis_emits_analysis_node_and_analyzes_edge_to_repo() {
    // Bootstrap-shape payload (phase4e): `{ title, status, body,
    // source_path }`. The mapper must produce an `Analysis` node
    // labelled by the title plus an `ANALYZES` edge wired to the
    // owning repo.
    let payload = json!({
        "title": "Cortex — System Analysis (2026-04-28)",
        "status": "draft",
        "body": "# Cortex — System Analysis (2026-04-28)\n\nBody.",
        "source_path": "docs/analysis/cortex/00-index.md"
    });
    let evt = event(
        "01ANALYSIS00000000000000A1",
        Kind::Analysis,
        payload,
        Some("Cortex"),
        Some("docs/analysis/cortex/00-index.md"),
        None,
    );
    let patch = map_event_to_patch(&evt);

    let analysis = patch
        .nodes
        .iter()
        .find(|n| n.label == "Analysis")
        .expect("Analysis node must be present");
    assert_eq!(analysis.natural_key, "01ANALYSIS00000000000000A1");
    assert_eq!(
        analysis.props.get("title").and_then(|v| v.as_str()),
        Some("Cortex — System Analysis (2026-04-28)")
    );
    assert_eq!(
        analysis.props.get("status").and_then(|v| v.as_str()),
        Some("draft")
    );
    assert_eq!(
        analysis.props.get("source_path").and_then(|v| v.as_str()),
        Some("docs/analysis/cortex/00-index.md")
    );

    let repo = patch
        .nodes
        .iter()
        .find(|n| n.label == "Repo" && n.natural_key == "Cortex")
        .expect("Repo node must be present");
    assert_eq!(
        repo.props.get("name").and_then(|v| v.as_str()),
        Some("Cortex")
    );

    let analyzes = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "ANALYZES")
        .expect("ANALYZES edge must be present");
    assert_eq!(analyzes.from_label, "Analysis");
    assert_eq!(analyzes.from_key, "01ANALYSIS00000000000000A1");
    assert_eq!(analyzes.to_label, "Repo");
    assert_eq!(analyzes.to_key, "Cortex");
}

#[test]
fn imported_analysis_without_repo_skips_analyzes_edge() {
    let payload = json!({
        "title": "Loose Analysis",
        "status": "draft",
        "body": "# Loose",
        "source_path": "docs/analysis/loose.md"
    });
    let evt = event(
        "01ANALYSIS00000000000000A2",
        Kind::Analysis,
        payload,
        None,
        None,
        None,
    );
    let patch = map_event_to_patch(&evt);
    assert!(patch.nodes.iter().any(|n| n.label == "Analysis"));
    assert!(
        patch.edges.iter().all(|e| e.edge_type != "ANALYZES"),
        "no ANALYZES edge expected when context_repo is missing"
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

#[test]
fn classifier_extracted_entities_become_typed_nodes_and_edges() {
    // The new semantic-graph layer: when Sonnet stamps `entities` +
    // `relations` on a Turn's classifier output, the mapper must
    // emit one node per entity (typed by `entity_type`) and one
    // edge per relation anchored at the Turn.
    let payload = json!({
        "user_message": "Implementing the RRF fusion change from DEC-0042",
        "assistant_message": null,
    });
    let mut evt = event("turn-9", Kind::Turn, payload, Some("Cortex"), None, None);
    evt.classifier.entities = vec![
        ExtractedEntity {
            entity_type: "decision".into(),
            identifier: "DEC-0042".into(),
            label: Some("Adopt RRF fusion".into()),
        },
        ExtractedEntity {
            entity_type: "artifact".into(),
            identifier: "crates/cortex-api/src/orchestrator.rs".into(),
            label: None,
        },
        ExtractedEntity {
            entity_type: "concept".into(),
            identifier: "rrf-fusion".into(),
            label: None,
        },
    ];
    evt.classifier.relations = vec![
        ExtractedRelation {
            from: "this_event".into(),
            relation: "IMPLEMENTS".into(),
            to: "DEC-0042".into(),
        },
        ExtractedRelation {
            from: "this_event".into(),
            relation: "OBSERVED_IN".into(),
            to: "crates/cortex-api/src/orchestrator.rs".into(),
        },
        ExtractedRelation {
            from: "this_event".into(),
            relation: "DISCUSSES".into(),
            to: "rrf-fusion".into(),
        },
    ];

    let patch = map_event_to_patch(&evt);

    // One typed node per entity, deduplicated by `(entity_type, identifier)`.
    assert!(patch
        .nodes
        .iter()
        .any(|n| n.label == "Decision" && n.natural_key == "decision|DEC-0042"));
    assert!(patch.nodes.iter().any(|n| n.label == "Artifact"
        && n.natural_key == "artifact|crates/cortex-api/src/orchestrator.rs"));
    assert!(patch
        .nodes
        .iter()
        .any(|n| n.label == "Concept" && n.natural_key == "concept|rrf-fusion"));

    // One typed edge per relation, anchored at the Turn.
    assert_eq!(count_edges(&patch, "IMPLEMENTS"), 1);
    assert_eq!(count_edges(&patch, "OBSERVED_IN"), 1);
    assert_eq!(count_edges(&patch, "DISCUSSES"), 1);

    // The IMPLEMENTS edge points from the Turn to the Decision —
    // direction matters for "what implements DEC-0042?" queries.
    let impl_edge = patch
        .edges
        .iter()
        .find(|e| e.edge_type == "IMPLEMENTS")
        .unwrap();
    assert_eq!(impl_edge.from_label, "Turn");
    assert_eq!(impl_edge.from_key, "turn-9");
    assert_eq!(impl_edge.to_label, "Decision");
    assert_eq!(impl_edge.to_key, "decision|DEC-0042");
}

#[test]
fn classifier_drops_phantom_relations_with_no_matching_entity() {
    // Defensive: if Sonnet emits a relation pointing at an entity
    // identifier it didn't list, we drop the relation rather than
    // creating a dangling edge to nowhere.
    let payload = json!({ "user_message": "noop", "assistant_message": null });
    let mut evt = event("turn-10", Kind::Turn, payload, None, None, None);
    evt.classifier.relations = vec![ExtractedRelation {
        from: "this_event".into(),
        relation: "REFERENCES".into(),
        to: "DEC-NEVER-DECLARED".into(),
    }];

    let patch = map_event_to_patch(&evt);
    assert_eq!(count_edges(&patch, "REFERENCES"), 0);
}

#[test]
fn classifier_drops_relations_with_unknown_label() {
    // Defensive: relations outside the closed vocabulary
    // (REFERENCES / IMPLEMENTS / FIXES / DISCUSSES / DEFINES /
    // DEPENDS_ON / SUPERSEDES / OBSERVED_IN / TOUCHED) are dropped
    // so the Nexus schema stays inspectable.
    let payload = json!({ "user_message": "noop", "assistant_message": null });
    let mut evt = event("turn-11", Kind::Turn, payload, None, None, None);
    evt.classifier.entities = vec![ExtractedEntity {
        entity_type: "decision".into(),
        identifier: "DEC-0001".into(),
        label: None,
    }];
    evt.classifier.relations = vec![ExtractedRelation {
        from: "this_event".into(),
        relation: "PROBABLY_BREAKS".into(),
        to: "DEC-0001".into(),
    }];

    let patch = map_event_to_patch(&evt);
    // Node still appears (Sonnet identified the entity) but no
    // bogus edge label sneaks in.
    assert!(patch.nodes.iter().any(|n| n.label == "Decision"));
    assert_eq!(count_edges(&patch, "PROBABLY_BREAKS"), 0);
}

// ---------- phase4c: Symbol + DEFINES extraction ----------

/// Helper for Artifact-event tests. The CodeChunker reads the
/// inline body via `event_text`, which falls back through
/// `content`/`text`/`body` keys — `body` matches the canonical
/// `ArtifactPayload` shape so we use that.
fn artifact_event(repo: &str, path: &str, body: &str) -> EnrichedEvent {
    event(
        "art-1",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": path,
            "body": body,
            "truncated": false
        }),
        Some(repo),
        Some(path),
        None,
    )
}

#[test]
fn artifact_with_rust_source_emits_symbol_and_defines_edges() {
    let body = "\
pub struct PreThinkingTool {\n\
    pub name: String,\n\
}\n\
\n\
pub fn run(input: &str) -> bool {\n\
    !input.is_empty()\n\
}\n";
    let evt = artifact_event("Cortex", "crates/cortex-mcp-server/src/tools.rs", body);
    let patch = map_event_to_patch(&evt);

    // The artifact node + IN_REPO edge still land — additive change.
    assert!(count_nodes(&patch, "Artifact") >= 1);
    assert!(count_edges(&patch, "IN_REPO") >= 1);

    // Symbol nodes for the struct + the function. The chunker
    // detects both top-level declarations and stamps the bare name
    // on `metadata.symbol`; the mapper composes the qualified
    // name as `<path>::<name>` since Rust syntax doesn't carry an
    // FQN at the chunker level.
    let symbol_names: Vec<String> = patch
        .nodes
        .iter()
        .filter(|n| n.label == "Symbol")
        .filter_map(|n| {
            n.props
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    assert!(
        symbol_names.contains(&"PreThinkingTool".to_string()),
        "expected Symbol(name=PreThinkingTool) in {symbol_names:?}"
    );
    assert!(
        symbol_names.contains(&"run".to_string()),
        "expected Symbol(name=run) in {symbol_names:?}"
    );

    // Each Symbol should have one DEFINES edge to the artifact key.
    let defines = count_edges(&patch, "DEFINES");
    assert_eq!(defines, symbol_names.len());

    // The DEFINES endpoints — `from` is the symbol natural key
    // (`Cortex|rust|<qname>`), `to` is the artifact natural key.
    let from_keys: Vec<String> = patch
        .edges
        .iter()
        .filter(|e| e.edge_type == "DEFINES")
        .map(|e| e.from_key.clone())
        .collect();
    for key in &from_keys {
        assert!(
            key.starts_with("Cortex|rust|"),
            "DEFINES.from should be Symbol natural key, got {key}"
        );
    }
}

#[test]
fn artifact_without_recognised_language_stays_artifact_only() {
    // .lock files are not in the chunker's grammar set, so
    // CodeChunker returns Vec::new(). The mapper must NOT fail —
    // it just emits the existing Artifact + IN_REPO patches.
    let evt = artifact_event("Cortex", "Cargo.lock", "# generated by cargo\n");
    let patch = map_event_to_patch(&evt);
    assert_eq!(count_nodes(&patch, "Symbol"), 0);
    assert_eq!(count_edges(&patch, "DEFINES"), 0);
    assert!(count_nodes(&patch, "Artifact") >= 1);
}

#[test]
fn artifact_replay_is_idempotent_under_natural_key() {
    // Mapping the same event twice must produce two patches whose
    // Symbol natural keys are identical — the writer's MERGE will
    // collapse them into a single Nexus node (phase4c §1.3 spec
    // scenario "replay does not duplicate DEFINES").
    let body = "pub fn parse(s: &str) -> bool { !s.is_empty() }\n";
    let evt = artifact_event("Cortex", "crates/foo/src/lib.rs", body);
    let p1 = map_event_to_patch(&evt);
    let p2 = map_event_to_patch(&evt);

    let keys1: Vec<&str> = p1
        .nodes
        .iter()
        .filter(|n| n.label == "Symbol")
        .map(|n| n.natural_key.as_str())
        .collect();
    let keys2: Vec<&str> = p2
        .nodes
        .iter()
        .filter(|n| n.label == "Symbol")
        .map(|n| n.natural_key.as_str())
        .collect();
    assert_eq!(keys1, keys2, "Symbol natural keys must be deterministic");
    assert!(!keys1.is_empty(), "expected at least one Symbol patch");
}

#[test]
fn duplicate_symbol_within_one_event_collapses_to_a_single_node() {
    // Hypothetical: two top-level declarations resolve to the same
    // FQN (e.g. an `impl` block named the same as the `struct`).
    // The mapper's intra-patch dedupe should keep exactly one
    // Symbol node + one DEFINES edge.
    let body = "\
pub struct Foo;\n\
\n\
impl Foo {\n\
    pub fn new() -> Self { Foo }\n\
}\n";
    let evt = artifact_event("Cortex", "crates/foo/src/lib.rs", body);
    let patch = map_event_to_patch(&evt);
    let foo_count = patch
        .nodes
        .iter()
        .filter(|n| n.label == "Symbol")
        .filter(|n| n.props.get("name").and_then(|v| v.as_str()) == Some("Foo"))
        .count();
    assert!(
        foo_count <= 1,
        "expected at most one Symbol(name=Foo) per event, got {foo_count}"
    );
}
