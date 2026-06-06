//! phase18 §2.4 — Temporal + branching edge kinds.
//!
//! ADR-023 §1.6 locks the semantics. Writers that emit these edges
//! live across multiple subsystems: the consolidator emits
//! EVOLVES_FROM; the branch CLI emits FORKED_FROM / MERGED_INTO;
//! the deprecation tooling emits OBSOLETES; the cross-project
//! backfill emits CROSS_PROJECT_REF. The constants live here so
//! every writer references the same identifier — drift between
//! writer and classifier would silently break retrieval.

use std::collections::BTreeMap;

use serde_json::Value;

use super::patch::EdgeOp;

/// `SUPERSEDES(from, to)` — `from` REPLACES `to` (ADR-023).
/// Already emitted by `mapper::emit_decision` for ADR chains; the
/// constant lives here for write-side consistency.
pub const SUPERSEDES: &str = "SUPERSEDES";

/// `OBSOLETES(from, to)` — `from` makes `to` inapplicable WITHOUT
/// replacing it. Stamps `to.lifecycle = deprecated`; leaves
/// `to.valid_to` unset.
pub const OBSOLETES: &str = "OBSOLETES";

/// `CONFLICTS_WITH(from, to)` — two facts that disagree; the
/// resolution lands as a separate ADR / decision.
pub const CONFLICTS_WITH: &str = "CONFLICTS_WITH";

/// `FORKED_FROM(child_branch, parent_branch)` — branch ancestry.
/// Written by the `cortex-ops branch create` CLI.
pub const FORKED_FROM: &str = "FORKED_FROM";

/// `MERGED_INTO(branch, parent_branch)` — branch fold-back. Carries
/// the merge `strategy` on the edge props per ADR-021. Written by
/// the `cortex-ops branch merge` CLI.
pub const MERGED_INTO: &str = "MERGED_INTO";

/// `EVOLVES_FROM(from, to)` — non-replacing precursor link
/// (ADR-023). Written by the consolidator when an envelope summarises
/// underlying topic cards.
pub const EVOLVES_FROM: &str = "EVOLVES_FROM";

/// `CROSS_PROJECT_REF(from, to, version_constraint)` — cross-project
/// dependency. Carries `valid_from` / `valid_to` on the edge props
/// so the temporal classifier can demote constraint-expired links.
pub const CROSS_PROJECT_REF: &str = "CROSS_PROJECT_REF";

/// All temporal / branching edge kinds the phase18 graph layer
/// emits. Used by the §3 classifier to walk the right edges and by
/// the §7.2 dashboard panel that breaks down edge kind counts.
pub const ALL_TEMPORAL_EDGES: &[&str] = &[
    SUPERSEDES,
    OBSOLETES,
    CONFLICTS_WITH,
    FORKED_FROM,
    MERGED_INTO,
    EVOLVES_FROM,
    CROSS_PROJECT_REF,
];

/// Build an `OBSOLETES` edge between two entities (any label that
/// carries the bitemporal axis). The writer is responsible for
/// stamping `to.lifecycle = deprecated` separately when this edge
/// lands.
pub fn build_obsoletes(from_label: &str, from_key: &str, to_label: &str, to_key: &str) -> EdgeOp {
    EdgeOp {
        edge_type: OBSOLETES.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props: BTreeMap::new(),
    }
}

/// Build an `EVOLVES_FROM(from -> to)` edge with no props.
pub fn build_evolves_from(
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
) -> EdgeOp {
    EdgeOp {
        edge_type: EVOLVES_FROM.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props: BTreeMap::new(),
    }
}

/// Build a `CONFLICTS_WITH` edge between two entities. The
/// resolution decision lives as a separate ADR; this edge only
/// records the disagreement.
pub fn build_conflicts_with(
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
) -> EdgeOp {
    EdgeOp {
        edge_type: CONFLICTS_WITH.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props: BTreeMap::new(),
    }
}

/// Build a `FORKED_FROM(child_branch_id -> parent_branch_id)` edge.
/// Branch nodes use composite ids (`<project>:<name>`) per ADR-019.
pub fn build_forked_from(child_id: &str, parent_id: &str) -> EdgeOp {
    EdgeOp {
        edge_type: FORKED_FROM.to_string(),
        from_label: super::branch::BRANCH_LABEL.to_string(),
        from_key: child_id.to_string(),
        to_label: super::branch::BRANCH_LABEL.to_string(),
        to_key: parent_id.to_string(),
        props: BTreeMap::new(),
    }
}

/// Build a `MERGED_INTO(branch -> parent_branch)` edge with the
/// merge strategy stamped on the edge props per ADR-021 §1.4.
pub fn build_merged_into(
    branch_id: &str,
    parent_id: &str,
    strategy: super::branch::MergeStrategy,
    merge_point_event_id: &str,
) -> EdgeOp {
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    props.insert(
        "strategy".to_string(),
        Value::String(strategy.as_str().to_string()),
    );
    props.insert(
        "merge_point_event_id".to_string(),
        Value::String(merge_point_event_id.to_string()),
    );
    EdgeOp {
        edge_type: MERGED_INTO.to_string(),
        from_label: super::branch::BRANCH_LABEL.to_string(),
        from_key: branch_id.to_string(),
        to_label: super::branch::BRANCH_LABEL.to_string(),
        to_key: parent_id.to_string(),
        props,
    }
}

/// Build a `CROSS_PROJECT_REF(from -> to)` edge with the
/// `version_constraint` + bitemporal `valid_from`/`valid_to`
/// stamped on the edge props. `valid_to` is `None` when the
/// dependency is still active.
pub fn build_cross_project_ref(
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
    version_constraint: &str,
    valid_from: &str,
    valid_to: Option<&str>,
) -> EdgeOp {
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    props.insert(
        "version_constraint".to_string(),
        Value::String(version_constraint.to_string()),
    );
    props.insert(
        super::bitemporal::keys::VALID_FROM.to_string(),
        Value::String(valid_from.to_string()),
    );
    if let Some(vt) = valid_to {
        props.insert(
            super::bitemporal::keys::VALID_TO.to_string(),
            Value::String(vt.to_string()),
        );
    }
    EdgeOp {
        edge_type: CROSS_PROJECT_REF.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props,
    }
}

#[cfg(test)]
mod tests {
    use super::super::branch::MergeStrategy;
    use super::*;

    #[test]
    fn all_seven_edges_are_registered() {
        assert_eq!(ALL_TEMPORAL_EDGES.len(), 7);
        assert!(ALL_TEMPORAL_EDGES.contains(&SUPERSEDES));
        assert!(ALL_TEMPORAL_EDGES.contains(&OBSOLETES));
        assert!(ALL_TEMPORAL_EDGES.contains(&CONFLICTS_WITH));
        assert!(ALL_TEMPORAL_EDGES.contains(&FORKED_FROM));
        assert!(ALL_TEMPORAL_EDGES.contains(&MERGED_INTO));
        assert!(ALL_TEMPORAL_EDGES.contains(&EVOLVES_FROM));
        assert!(ALL_TEMPORAL_EDGES.contains(&CROSS_PROJECT_REF));
    }

    #[test]
    fn build_obsoletes_carries_no_props() {
        let e = build_obsoletes("Decision", "DEC-001", "Decision", "DEC-002");
        assert_eq!(e.edge_type, OBSOLETES);
        assert!(e.props.is_empty());
        assert_eq!(e.from_key, "DEC-001");
        assert_eq!(e.to_key, "DEC-002");
    }

    #[test]
    fn build_merged_into_stamps_strategy_and_merge_point() {
        let e = build_merged_into(
            "cortex:feat/x",
            "cortex:main",
            MergeStrategy::Partial,
            "01HZMERGE000000000000000A",
        );
        assert_eq!(e.edge_type, MERGED_INTO);
        assert_eq!(
            e.props.get("strategy").and_then(|v| v.as_str()),
            Some("partial")
        );
        assert_eq!(
            e.props.get("merge_point_event_id").and_then(|v| v.as_str()),
            Some("01HZMERGE000000000000000A")
        );
    }

    #[test]
    fn build_forked_from_uses_branch_label_on_both_ends() {
        let e = build_forked_from("cortex:feat/x", "cortex:main");
        assert_eq!(e.edge_type, FORKED_FROM);
        assert_eq!(e.from_label, super::super::branch::BRANCH_LABEL);
        assert_eq!(e.to_label, super::super::branch::BRANCH_LABEL);
    }

    #[test]
    fn build_cross_project_ref_stamps_bitemporal_props() {
        let e = build_cross_project_ref(
            "Artifact",
            "cortex|cargo.toml|abc",
            "Artifact",
            "nexus|cargo.toml|def",
            "2.1",
            "2026-05-29T07:00:00Z",
            None,
        );
        assert_eq!(e.edge_type, CROSS_PROJECT_REF);
        assert_eq!(
            e.props.get("version_constraint").and_then(|v| v.as_str()),
            Some("2.1")
        );
        assert_eq!(
            e.props
                .get(super::super::bitemporal::keys::VALID_FROM)
                .and_then(|v| v.as_str()),
            Some("2026-05-29T07:00:00Z")
        );
        assert!(!e
            .props
            .contains_key(super::super::bitemporal::keys::VALID_TO));
    }

    #[test]
    fn build_cross_project_ref_with_valid_to_stamps_it() {
        let e = build_cross_project_ref(
            "Artifact",
            "a",
            "Artifact",
            "b",
            "1.0",
            "2026-04-01T00:00:00Z",
            Some("2026-05-01T00:00:00Z"),
        );
        assert_eq!(
            e.props
                .get(super::super::bitemporal::keys::VALID_TO)
                .and_then(|v| v.as_str()),
            Some("2026-05-01T00:00:00Z")
        );
    }
}
