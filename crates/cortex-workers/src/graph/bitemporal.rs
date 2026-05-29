//! phase18 §2.1 — Bitemporal property stamper for graph nodes.
//!
//! Every entity node label gains 7 bitemporal / scoping columns:
//!
//! | column            | type    | rule                                            |
//! |-------------------|---------|-------------------------------------------------|
//! | `project_id`      | String  | Derived from `event.context_repo` (lowercased). |
//! | `branch_id`       | String  | Defaults to `"main"` until §2.12 lands branches.|
//! | `valid_from`      | String  | UTC RFC3339 second-precision (ADR-018).         |
//! | `valid_to`        | None    | NULL = still valid (ADR-018 §1.1).              |
//! | `recorded_at`     | String  | When Cortex first persisted the fact.           |
//! | `superseded_at`   | None    | NULL = still believed (stamped later by SUPERSEDES edge). |
//! | `lifecycle`       | String  | `active` by default; payload-derived overrides. |
//!
//! The stamp is applied at the END of `map_event_to_patch` after
//! every per-kind emitter has filled in its own `props`. We never
//! overwrite a value that an emitter already set — that way the
//! `decision_id`-driven `lifecycle = superseded` overrides the
//! default `active` when a `DecisionPayload.status` says so.
//!
//! Branch lookup is intentionally a constant `"main"` for the P1
//! migration window. Phase18 §2.12 introduces per-project branches
//! and the writer-side branch resolution; until then every node
//! lands on `main` which the temporal classifier (§3) handles as
//! the default scope.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::embedder::EnrichedEvent;

use super::patch::{GraphPatch, NodeOp};

/// Bitemporal column keys. Centralised so the writer and the
/// migration backfill share the literal names.
pub mod keys {
    /// Project scope — lower-cased repo slug.
    pub const PROJECT_ID: &str = "project_id";
    /// Branch scope — defaults to `"main"`.
    pub const BRANCH_ID: &str = "branch_id";
    /// When the fact starts being true (RFC3339, second precision).
    pub const VALID_FROM: &str = "valid_from";
    /// When the fact stops; absent = still valid.
    pub const VALID_TO: &str = "valid_to";
    /// When Cortex first persisted the fact.
    pub const RECORDED_AT: &str = "recorded_at";
    /// When Cortex stopped believing this version.
    pub const SUPERSEDED_AT: &str = "superseded_at";
    /// `proposed | active | superseded | deprecated | abandoned | merged`.
    pub const LIFECYCLE: &str = "lifecycle";
}

/// Default branch every node lands on until §2.12 ships per-project
/// branches. The reserved name is fixed by ADR-019 §1.2.
pub const DEFAULT_BRANCH: &str = "main";

/// Phase18 §2.10 — backfill defaults for rows that pre-date the
/// bitemporal stamp. The migration CLI walks every node and applies
/// these rules when the column is absent:
///
/// - `valid_from` = `recorded_at` OR `created_at` OR `EPOCH_FALLBACK`.
/// - `valid_to`   = NULL (absent).
/// - `branch_id`  = `"main"`.
/// - `lifecycle`  = derived from existing `status` when present:
///     - `"superseded"` → superseded
///     - `"deprecated"` → deprecated
///     - anything else  → `"active"`
/// - `project_id` = lower-cased `context_repo` OR `"unknown"`.
///
/// `EPOCH_FALLBACK` is the canonical pre-Cortex zero value — a
/// node with no timestamp anchor lands here so range probes never
/// see `valid_from = NULL`.
pub const EPOCH_FALLBACK: &str = "1970-01-01T00:00:00Z";

/// Phase18 §2.10 — derive a lifecycle label from an existing
/// `status` value. Maps the historical Decision /
/// LawViolation / Analysis `status` vocabulary onto the bitemporal
/// `lifecycle` axis used by the temporal classifier (§3). Anything
/// not in the table maps to `"active"`.
pub fn lifecycle_from_status(status: Option<&str>) -> &'static str {
    match status.map(str::trim).unwrap_or("") {
        "superseded" => "superseded",
        "deprecated" => "deprecated",
        "abandoned" => "abandoned",
        "merged" => "merged",
        "proposed" => "proposed",
        _ => "active",
    }
}

/// Phase18 §2.10 — pick a `valid_from` per the backfill chain.
/// `recorded_at` wins; `created_at` next; `EPOCH_FALLBACK` last.
/// All inputs are RFC3339 strings; the helper does NOT re-parse —
/// the migration CLI is responsible for normalising at read time.
pub fn valid_from_fallback_chain<'a>(
    recorded_at: Option<&'a str>,
    created_at: Option<&'a str>,
) -> &'a str {
    if let Some(s) = recorded_at.map(str::trim).filter(|s| !s.is_empty()) {
        return s;
    }
    if let Some(s) = created_at.map(str::trim).filter(|s| !s.is_empty()) {
        return s;
    }
    EPOCH_FALLBACK
}

/// Apply the bitemporal stamp to every node in `patch`. Idempotent —
/// values already set by the per-kind emitter survive. The call
/// belongs at the END of `map_event_to_patch`.
pub fn stamp_bitemporal_props_on_patch(event: &EnrichedEvent, patch: &mut GraphPatch) {
    let project_id = derive_project_id(event);
    let branch_id = DEFAULT_BRANCH.to_string();
    let valid_from = derive_valid_from(event);
    let recorded_at = valid_from.clone();
    for node in patch.nodes.iter_mut() {
        stamp_one_node(node, &project_id, &branch_id, &valid_from, &recorded_at);
    }
}

/// Stamp the bitemporal columns onto a single NodeOp. Public so the
/// per-emit unit tests can exercise the same path the patch walker
/// runs through.
pub fn stamp_one_node(
    node: &mut NodeOp,
    project_id: &str,
    branch_id: &str,
    valid_from: &str,
    recorded_at: &str,
) {
    insert_if_absent(
        &mut node.props,
        keys::PROJECT_ID,
        Value::String(project_id.to_string()),
    );
    insert_if_absent(
        &mut node.props,
        keys::BRANCH_ID,
        Value::String(branch_id.to_string()),
    );
    insert_if_absent(
        &mut node.props,
        keys::VALID_FROM,
        Value::String(valid_from.to_string()),
    );
    insert_if_absent(
        &mut node.props,
        keys::RECORDED_AT,
        Value::String(recorded_at.to_string()),
    );
    // `valid_to` + `superseded_at` default to absent (null = still
    // valid). Only stamp `lifecycle` when nothing is there — the
    // Decision emitter, for example, overrides this with the
    // payload's status.
    insert_if_absent(
        &mut node.props,
        keys::LIFECYCLE,
        Value::String("active".to_string()),
    );
}

fn insert_if_absent(props: &mut BTreeMap<String, Value>, key: &str, value: Value) {
    props.entry(key.to_string()).or_insert(value);
}

/// Derive the project_id from the envelope's context_repo (lower-cased).
/// Falls back to `"unknown"` when the upstream did not stamp a repo
/// hint so the column stays non-null (the bitemporal contract requires
/// every node to carry a project; null would break per-project
/// retrieval).
fn derive_project_id(event: &EnrichedEvent) -> String {
    event
        .context_repo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Derive `valid_from` from `event.occurred_at_ms` (phase20 §5.2).
/// Falls back to the current UTC second when the upstream stamped
/// `0` (the no-timestamp sentinel) so the column never lands on the
/// 1970-01-01 epoch.
fn derive_valid_from(event: &EnrichedEvent) -> String {
    let ms = if event.occurred_at_ms > 0 {
        event.occurred_at_ms
    } else {
        chrono::Utc::now().timestamp_millis()
    };
    let secs = ms / 1_000;
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .unwrap_or_else(chrono::Utc::now);
    // Format with seconds + `Z` suffix per ADR-018 §1.1.
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
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
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn make_event(repo: Option<&str>, occurred_at_ms: i64) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "01HZTEST00000000000000000A".into(),
            kind: Kind::Decision,
            content_hash: "sha256:test".into(),
            redacted_payload: serde_json::json!({}),
            classifier: make_classifier("01HZTEST00000000000000000A"),
            context_repo: repo.map(String::from),
            context_path: None,
            parent_event_id: None,
            session_id: Some("01HSESS00000000000000000A0".into()),
            occurred_at_ms,
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
    fn stamp_sets_project_branch_valid_from_lifecycle() {
        let event = make_event(Some("Cortex"), 1_700_000_000_000);
        let mut patch = GraphPatch::empty();
        patch.nodes.push(make_node());
        stamp_bitemporal_props_on_patch(&event, &mut patch);
        let n = &patch.nodes[0];
        assert_eq!(n.props.get(keys::PROJECT_ID).and_then(|v| v.as_str()), Some("cortex"));
        assert_eq!(n.props.get(keys::BRANCH_ID).and_then(|v| v.as_str()), Some("main"));
        assert_eq!(
            n.props.get(keys::VALID_FROM).and_then(|v| v.as_str()),
            Some("2023-11-14T22:13:20Z")
        );
        assert_eq!(
            n.props.get(keys::RECORDED_AT).and_then(|v| v.as_str()),
            Some("2023-11-14T22:13:20Z")
        );
        assert_eq!(n.props.get(keys::LIFECYCLE).and_then(|v| v.as_str()), Some("active"));
        assert!(!n.props.contains_key(keys::VALID_TO), "valid_to defaults absent");
        assert!(
            !n.props.contains_key(keys::SUPERSEDED_AT),
            "superseded_at defaults absent"
        );
    }

    #[test]
    fn stamp_does_not_overwrite_emitter_set_lifecycle() {
        let event = make_event(Some("cortex"), 1_700_000_000_000);
        let mut patch = GraphPatch::empty();
        let mut node = make_node();
        node.props.insert(
            keys::LIFECYCLE.to_string(),
            Value::String("superseded".into()),
        );
        patch.nodes.push(node);
        stamp_bitemporal_props_on_patch(&event, &mut patch);
        assert_eq!(
            patch.nodes[0]
                .props
                .get(keys::LIFECYCLE)
                .and_then(|v| v.as_str()),
            Some("superseded")
        );
    }

    #[test]
    fn stamp_uses_now_when_occurred_at_is_zero() {
        let event = make_event(Some("cortex"), 0);
        let mut patch = GraphPatch::empty();
        patch.nodes.push(make_node());
        stamp_bitemporal_props_on_patch(&event, &mut patch);
        let v = patch.nodes[0]
            .props
            .get(keys::VALID_FROM)
            .and_then(|v| v.as_str())
            .expect("valid_from set");
        // ts > 0 ⇒ a Zulu-suffixed second-precision string was minted;
        // we don't pin the year because the clock is the system clock.
        assert!(v.ends_with('Z'));
        assert_eq!(v.len(), "YYYY-MM-DDTHH:MM:SSZ".len());
    }

    #[test]
    fn stamp_falls_back_to_unknown_when_repo_absent() {
        let event = make_event(None, 1_700_000_000_000);
        let mut patch = GraphPatch::empty();
        patch.nodes.push(make_node());
        stamp_bitemporal_props_on_patch(&event, &mut patch);
        assert_eq!(
            patch.nodes[0]
                .props
                .get(keys::PROJECT_ID)
                .and_then(|v| v.as_str()),
            Some("unknown")
        );
    }

    #[test]
    fn lifecycle_from_status_maps_known_values() {
        assert_eq!(lifecycle_from_status(Some("superseded")), "superseded");
        assert_eq!(lifecycle_from_status(Some("deprecated")), "deprecated");
        assert_eq!(lifecycle_from_status(Some("abandoned")), "abandoned");
        assert_eq!(lifecycle_from_status(Some("merged")), "merged");
        assert_eq!(lifecycle_from_status(Some("proposed")), "proposed");
    }

    #[test]
    fn lifecycle_from_status_defaults_to_active_for_unknown_or_empty() {
        assert_eq!(lifecycle_from_status(None), "active");
        assert_eq!(lifecycle_from_status(Some("")), "active");
        assert_eq!(lifecycle_from_status(Some("   ")), "active");
        assert_eq!(lifecycle_from_status(Some("accepted")), "active");
        assert_eq!(lifecycle_from_status(Some("draft")), "active");
    }

    #[test]
    fn valid_from_fallback_chain_prefers_recorded_then_created_then_epoch() {
        assert_eq!(
            valid_from_fallback_chain(Some("2026-04-01T00:00:00Z"), Some("2025-12-01T00:00:00Z")),
            "2026-04-01T00:00:00Z"
        );
        assert_eq!(
            valid_from_fallback_chain(None, Some("2025-12-01T00:00:00Z")),
            "2025-12-01T00:00:00Z"
        );
        assert_eq!(valid_from_fallback_chain(None, None), EPOCH_FALLBACK);
        // Empty / whitespace inputs skip the slot.
        assert_eq!(
            valid_from_fallback_chain(Some("   "), Some("2025-12-01T00:00:00Z")),
            "2025-12-01T00:00:00Z"
        );
    }

    #[test]
    fn epoch_fallback_is_canonical_rfc3339() {
        assert_eq!(EPOCH_FALLBACK, "1970-01-01T00:00:00Z");
        assert!(EPOCH_FALLBACK.ends_with('Z'));
        assert_eq!(EPOCH_FALLBACK.len(), "YYYY-MM-DDTHH:MM:SSZ".len());
    }

    #[test]
    fn stamp_walks_every_node_in_patch() {
        let event = make_event(Some("cortex"), 1_700_000_000_000);
        let mut patch = GraphPatch::empty();
        for i in 0..3 {
            let mut node = make_node();
            node.natural_key = format!("DEC-{i:03}");
            patch.nodes.push(node);
        }
        stamp_bitemporal_props_on_patch(&event, &mut patch);
        for node in &patch.nodes {
            assert!(node.props.contains_key(keys::PROJECT_ID));
            assert!(node.props.contains_key(keys::BRANCH_ID));
            assert!(node.props.contains_key(keys::VALID_FROM));
            assert!(node.props.contains_key(keys::LIFECYCLE));
        }
    }
}
