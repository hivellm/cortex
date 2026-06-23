//! phase21 §2.2 — Classification property stamper for graph nodes.
//!
//! Mirrors `graph/bitemporal.rs` exactly: after every per-kind emitter
//! fills in its props, `stamp_classification_props_on_patch` walks
//! every node and stamps the two classification columns using
//! `entry().or_insert()` semantics — a value already set by the emitter
//! (the "explicit override" merge rule from ADR-032) is never touched.
//!
//! Column contract:
//!
//! | column              | type          | rule                                         |
//! |---------------------|---------------|----------------------------------------------|
//! | `class_level`       | u64 (JSON)    | Ordinal: public=0, internal=1, confidential=2, restricted=3. |
//! | `class_compartments`| String[] (JSON)| Need-to-know labels, e.g. `["financial","hr"]`. |
//!
//! The `default_level` is taken from `AccessControlConfig.default_level` (§7.1).
//! Until that config block lands, callers pass a `u8` constant directly.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::embedder::EnrichedEvent;

use super::patch::{GraphPatch, NodeOp};

/// Classification column key constants. Centralised so the writer, the
/// classification stamper, and the migration backfill share literal names.
pub mod keys {
    /// Sensitivity level ordinal (u64 in JSON, u8 in Rust).
    pub const CLASS_LEVEL: &str = "class_level";
    /// Orthogonal need-to-know compartment labels (JSON string array).
    pub const CLASS_COMPARTMENTS: &str = "class_compartments";
}

/// Apply the classification stamp to every node in `patch`. Idempotent —
/// values already set by the per-kind emitter survive unchanged (ADR-032
/// "explicit emitter override wins"). Call this AFTER
/// `stamp_bitemporal_props_on_patch` and all per-kind emitters.
pub fn stamp_classification_props_on_patch(
    event: &EnrichedEvent,
    patch: &mut GraphPatch,
    default_level: u8,
) {
    let level = event.class_level.unwrap_or(default_level);
    let compartments = event.class_compartments.clone().unwrap_or_default();
    for node in patch.nodes.iter_mut() {
        stamp_one_node(node, level, &compartments);
    }
}

/// Stamp the classification columns onto a single [`NodeOp`]. Public so
/// per-emit unit tests can exercise the same path the patch walker runs.
pub fn stamp_one_node(node: &mut NodeOp, level: u8, compartments: &[String]) {
    insert_if_absent(
        &mut node.props,
        keys::CLASS_LEVEL,
        Value::Number(level.into()),
    );
    insert_if_absent(
        &mut node.props,
        keys::CLASS_COMPARTMENTS,
        Value::Array(
            compartments
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        ),
    );
}

fn insert_if_absent(props: &mut BTreeMap<String, Value>, key: &str, value: Value) {
    props.entry(key.to_string()).or_insert(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use crate::graph::patch::ConflictPolicy;
    use cortex_core::events::Kind;

    fn make_classifier(event_id: &str) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.into(),
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
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

    fn make_event(
        class_level: Option<u8>,
        class_compartments: Option<Vec<String>>,
    ) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "01HZTEST00000000000000000A".into(),
            kind: Kind::Decision,
            content_hash: "sha256:test".into(),
            redacted_payload: serde_json::json!({}),
            classifier: make_classifier("01HZTEST00000000000000000A"),
            context_repo: Some("cortex".into()),
            context_path: None,
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level,
            class_compartments,
        }
    }

    fn make_node() -> NodeOp {
        NodeOp {
            label: "Decision".into(),
            natural_key: "DEC-001".into(),
            external_id: Some("DEC-001".into()),
            conflict_policy: ConflictPolicy::default(),
            props: BTreeMap::new(),
        }
    }

    #[test]
    fn stamp_sets_class_level_and_compartments_from_event() {
        let event = make_event(Some(2), Some(vec!["financial".into()]));
        let mut patch = GraphPatch::empty();
        patch.nodes.push(make_node());
        stamp_classification_props_on_patch(&event, &mut patch, 1);
        let n = &patch.nodes[0];
        assert_eq!(
            n.props.get(keys::CLASS_LEVEL).and_then(Value::as_u64),
            Some(2),
            "event class_level wins over default"
        );
        let comps = n
            .props
            .get(keys::CLASS_COMPARTMENTS)
            .and_then(Value::as_array)
            .expect("compartments present");
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].as_str(), Some("financial"));
    }

    #[test]
    fn stamp_falls_back_to_default_level_when_event_has_none() {
        let event = make_event(None, None);
        let mut patch = GraphPatch::empty();
        patch.nodes.push(make_node());
        stamp_classification_props_on_patch(&event, &mut patch, 1);
        assert_eq!(
            patch.nodes[0]
                .props
                .get(keys::CLASS_LEVEL)
                .and_then(Value::as_u64),
            Some(1),
            "default_level used when event.class_level is None"
        );
        let comps = patch.nodes[0]
            .props
            .get(keys::CLASS_COMPARTMENTS)
            .and_then(Value::as_array)
            .expect("compartments present");
        assert!(comps.is_empty(), "empty compartments when event has none");
    }

    #[test]
    fn stamp_does_not_overwrite_emitter_set_class_level() {
        let event = make_event(Some(3), None);
        let mut patch = GraphPatch::empty();
        let mut node = make_node();
        // Emitter pre-set a stricter level — stamp must not overwrite.
        node.props
            .insert(keys::CLASS_LEVEL.to_string(), Value::Number(0u64.into()));
        patch.nodes.push(node);
        stamp_classification_props_on_patch(&event, &mut patch, 1);
        assert_eq!(
            patch.nodes[0]
                .props
                .get(keys::CLASS_LEVEL)
                .and_then(Value::as_u64),
            Some(0),
            "emitter-set value survives stamp"
        );
    }

    #[test]
    fn stamp_does_not_overwrite_emitter_set_compartments() {
        let event = make_event(None, Some(vec!["hr".into()]));
        let mut patch = GraphPatch::empty();
        let mut node = make_node();
        node.props.insert(
            keys::CLASS_COMPARTMENTS.to_string(),
            Value::Array(vec![Value::String("legal".into())]),
        );
        patch.nodes.push(node);
        stamp_classification_props_on_patch(&event, &mut patch, 1);
        let comps = patch.nodes[0]
            .props
            .get(keys::CLASS_COMPARTMENTS)
            .and_then(Value::as_array)
            .expect("compartments present");
        assert_eq!(
            comps[0].as_str(),
            Some("legal"),
            "emitter-set compartments survive"
        );
    }

    #[test]
    fn stamp_walks_every_node_in_patch() {
        let event = make_event(Some(2), Some(vec!["security".into()]));
        let mut patch = GraphPatch::empty();
        for i in 0..4 {
            let mut n = make_node();
            n.natural_key = format!("DEC-{i:03}");
            patch.nodes.push(n);
        }
        stamp_classification_props_on_patch(&event, &mut patch, 0);
        for node in &patch.nodes {
            assert!(node.props.contains_key(keys::CLASS_LEVEL));
            assert!(node.props.contains_key(keys::CLASS_COMPARTMENTS));
        }
    }

    #[test]
    fn stamp_one_node_public_level_zero_empty_compartments() {
        let mut node = make_node();
        stamp_one_node(&mut node, 0, &[]);
        assert_eq!(
            node.props.get(keys::CLASS_LEVEL).and_then(Value::as_u64),
            Some(0)
        );
        let comps = node
            .props
            .get(keys::CLASS_COMPARTMENTS)
            .and_then(Value::as_array)
            .expect("compartments present");
        assert!(comps.is_empty());
    }

    #[test]
    fn stamp_one_node_restricted_with_multiple_compartments() {
        let mut node = make_node();
        stamp_one_node(
            &mut node,
            3,
            &["financial".into(), "hr".into(), "legal".into()],
        );
        assert_eq!(
            node.props.get(keys::CLASS_LEVEL).and_then(Value::as_u64),
            Some(3)
        );
        let comps: Vec<&str> = node
            .props
            .get(keys::CLASS_COMPARTMENTS)
            .and_then(Value::as_array)
            .expect("compartments present")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(comps, vec!["financial", "hr", "legal"]);
    }
}
