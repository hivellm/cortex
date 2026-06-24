//! Phase15b — edge extractor scaffold.
//!
//! The graph mapper today emits only `IN_REPO` + `REMEMBERS` +
//! identity-level `HAS_TURN`/`HAS_TOOL_CALL` linkage. The graph spec
//! at `docs/specs/07-graph.md` defines ~12 edge kinds (`CALLS`,
//! `IMPORTS`, `DEFINES`, `RETURNS`, `SUPERSEDES`, `CONTRADICTS`,
//! `EMITTED_BY`, `ABOUT`, `ANSWERED_BY`, `CITES`, `MENTIONS_FILE`,
//! `RELATES_TO`). This module is the home for the per-kind
//! extractors that close the gap, wired into the projection
//! pipeline so re-projecting the same envelope is a no-op
//! (idempotency under the `(from, to, kind)` unique constraint).
//!
//! ## Extractor contract
//!
//! Each extractor is a free function:
//!
//! ```ignore
//! pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge>;
//! ```
//!
//! `Edge` is the same [`super::patch::EdgeOp`] the mapper already
//! emits — re-exported here under a friendlier alias so each
//! extractor file can `use super::Edge` instead of pulling in the
//! whole patch module. The extractor MUST be:
//!
//! - **Pure**: same `(env, ctx)` → same `Vec<Edge>`. No I/O, no
//!   global state, no clock reads. The pure contract is what
//!   makes §3.2 idempotency hold structurally — the projection
//!   pipeline can call `extract` twice and the second call
//!   produces a byte-identical edge set that hits the
//!   `(from, to, kind)` unique constraint as a no-op.
//! - **Empty on miss**: when the envelope carries nothing
//!   extractable for the kind (e.g. a `Turn` for the `CALLS`
//!   extractor), return `Vec::new()` rather than fabricating
//!   speculative edges.
//! - **Stamped with `source_event_id`**: every edge MUST carry
//!   the envelope's `event_id` under the
//!   `source_event_id` prop so the stale-edge sweeper at
//!   `crates/cortex-workers/src/graph/stale_sweeper.rs` can
//!   retire edges whose source envelope was deleted.
//!
//! Extractors that need supplementary inputs (e.g. tree-sitter
//! parser handles, the classifier's typed-entity list, the
//! similarity-cache for `RELATES_TO`) take them on the
//! [`ExtractCtx`] so the signature stays uniform.

use super::patch::EdgeOp;
use crate::embedder::EnrichedEvent;

pub mod about;
pub mod answered_by;
pub mod calls;
pub mod cites;
pub mod contradicts;
pub mod defines;
pub mod emitted_by;
pub mod imports;
pub mod mentions_file;
pub mod relates_to;
pub mod returns;
pub mod supersedes;

/// Friendly re-export — `Edge` is the same shape as
/// [`super::patch::EdgeOp`]; the alias lets each extractor file
/// say `use super::Edge` without pulling the whole patch module.
pub type Edge = EdgeOp;

/// Shared context handed to every extractor. Keeps the per-extractor
/// signature uniform so the projection pipeline at
/// [`super::projection`] (lands in §3.1) can dispatch through a
/// single table of function pointers.
///
/// Today the context only carries the analyzer version stamp + the
/// monotonic millis the projection started; the
/// `RELATES_TO` extractor (§2.12) will extend it with the
/// `SimilarityIndex` handle when that extractor lands.
#[derive(Debug, Clone)]
pub struct ExtractCtx {
    /// Stable analyzer-version label stamped on every emitted edge
    /// under the `analyzer_version` prop so the
    /// [`super::stale_sweeper`] can retire output from a superseded
    /// extractor run. Mirrors the
    /// [`super::analyzer::AnalyzerVersion`] string the mapper
    /// already writes for inferred `CALLS` / `IMPORTS` edges (when
    /// those land — phase15b §2.1 / §2.2).
    pub analyzer_version: String,
    /// Wall-clock milliseconds when the projection batch started.
    /// Stamped on every emitted edge under the `created_at_ms` prop
    /// so the sweeper's `older_than_ms` filter can retire stale
    /// edges deterministically.
    pub now_ms: i64,
}

impl ExtractCtx {
    /// Build an [`ExtractCtx`] with the supplied analyzer version
    /// and a clock-derived `now_ms`. Tests prefer
    /// [`Self::with_now_ms`] so the timestamp is reproducible.
    pub fn new(analyzer_version: impl Into<String>) -> Self {
        Self {
            analyzer_version: analyzer_version.into(),
            now_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Test-friendly constructor — pins the `now_ms` so the
    /// per-extractor unit tests can assert on stamped properties
    /// without a clock dependency.
    pub fn with_now_ms(analyzer_version: impl Into<String>, now_ms: i64) -> Self {
        Self {
            analyzer_version: analyzer_version.into(),
            now_ms,
        }
    }
}

/// Stamp the canonical "this edge came from extractor X on
/// envelope Y at time Z" property bag onto an [`Edge`] before
/// returning it from an extractor. Pulls the analyzer version +
/// timestamp off the [`ExtractCtx`] so individual extractors don't
/// re-implement the stamp. Returns the input edge for easy
/// chaining inside a `.map(...)`.
pub fn stamp_provenance(mut edge: Edge, env: &EnrichedEvent, ctx: &ExtractCtx) -> Edge {
    use serde_json::Value;
    edge.props.insert(
        "source_event_id".to_string(),
        Value::String(env.event_id.clone()),
    );
    edge.props.insert(
        "analyzer_version".to_string(),
        Value::String(ctx.analyzer_version.clone()),
    );
    edge.props
        .insert("created_at_ms".to_string(), Value::from(ctx.now_ms));
    edge
}

/// Map the canonical envelope kind to the matching graph node
/// label so the `"this_event"` endpoint anchor resolves to the
/// right `(label, key)` pair. Mirrors the kind → label mapping
/// the mapper at [`super::mapper`] uses for the identity-only
/// patches.
pub(crate) fn kind_to_label(kind: cortex_core::events::Kind) -> &'static str {
    use cortex_core::events::Kind;
    match kind {
        Kind::Turn => "Turn",
        Kind::ToolCall => "ToolCall",
        Kind::AgentCall => "AgentCall",
        Kind::Memory => "Memory",
        Kind::Decision => "Decision",
        Kind::Analysis => "Analysis",
        Kind::Law => "Law",
        Kind::LawViolation => "LawViolation",
        Kind::Artifact => "Artifact",
        Kind::Knowledge => "Knowledge",
        Kind::Learning => "Learning",
        Kind::Consolidation => "Consolidation",
        Kind::TopicCard => "TopicCard",
    }
}

/// Map the classifier's `entity_type` controlled vocab to the
/// graph node label. Unknown entity types return `None` so
/// callers drop the edge rather than guessing the label.
pub(crate) fn entity_type_to_label(entity_type: &str) -> Option<&'static str> {
    let label = match entity_type {
        "decision" => "Decision",
        "law" => "Law",
        "analysis" => "Analysis",
        "artifact" => "Artifact",
        "repo" => "Repo",
        "topic" => "Topic",
        "concept" => "Concept",
        "tool" => "Tool",
        "person" => "Person",
        "session" => "Session",
        "turn" => "Turn",
        _ => return None,
    };
    Some(label)
}

/// Resolve a relation endpoint string to `(label, natural_key)`.
/// The `anchor` literal (`"this_event"` per the classifier
/// contract) anchors at the envelope itself using the envelope's
/// kind → label mapping. Any other identifier resolves through
/// the classifier's `entities` table; returns `None` when the
/// identifier or entity type is unknown so callers drop the edge
/// rather than fabricating a phantom node.
pub(crate) fn resolve_endpoint(env: &EnrichedEvent, identifier: &str) -> Option<(String, String)> {
    if identifier == ANCHOR_THIS_EVENT {
        return Some((kind_to_label(env.kind).to_string(), env.event_id.clone()));
    }
    let entity = env
        .classifier
        .entities
        .iter()
        .find(|e| e.identifier == identifier)?;
    let label = entity_type_to_label(&entity.entity_type)?;
    Some((label.to_string(), entity.identifier.clone()))
}

/// The classifier's anchor literal for "the envelope itself".
pub(crate) const ANCHOR_THIS_EVENT: &str = "this_event";

/// Walk `classifier.relations`, keep entries whose label matches
/// `edge_type` case-insensitively, and project each into a stamped
/// [`Edge`]. Endpoints resolve through [`resolve_endpoint`]; a
/// relation whose endpoint resolution fails is dropped silently
/// (the same-record entity-table invariant means it would land
/// as a phantom-node edge otherwise).
///
/// Shared by every classifier-relation-driven extractor
/// (`CALLS` / `IMPORTS` / `DEFINES` / `RETURNS` / etc.) so the
/// per-kind file only declares the edge-type constant + the
/// envelope-kind filter.
pub(crate) fn extract_via_classifier_relations(
    env: &EnrichedEvent,
    ctx: &ExtractCtx,
    edge_type: &str,
) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    for rel in &env.classifier.relations {
        if !rel.relation.eq_ignore_ascii_case(edge_type) {
            continue;
        }
        let from = resolve_endpoint(env, rel.from.as_str());
        let to = resolve_endpoint(env, rel.to.as_str());
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let edge = Edge {
            edge_type: edge_type.to_string(),
            from_label: from.0,
            from_key: from.1,
            to_label: to.0,
            to_key: to.1,
            props: std::collections::BTreeMap::new(),
            ..Default::default()
        };
        out.push(stamp_provenance(edge, env, ctx));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_core::events::Kind;
    use serde_json::json;

    fn classifier_output(event_id: &str) -> ClassifierOutput {
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

    pub(crate) fn fixture_envelope(event_id: &str, kind: Kind) -> EnrichedEvent {
        EnrichedEvent {
            event_id: event_id.into(),
            kind,
            content_hash: "sha256:test".into(),
            redacted_payload: json!({}),
            classifier: classifier_output(event_id),
            context_repo: Some("cortex".into()),
            context_path: None,
            parent_event_id: None,
            session_id: Some("01HXSESS00000000000000000A".into()),
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        }
    }

    fn sample_edge() -> Edge {
        Edge {
            edge_type: "CALLS".into(),
            from_label: "Symbol".into(),
            from_key: "cortex|rust|crate::a".into(),
            to_label: "Symbol".into(),
            to_key: "cortex|rust|crate::b".into(),
            props: std::collections::BTreeMap::new(),
            ..Default::default()
        }
    }

    #[test]
    fn extract_ctx_new_stamps_analyzer_version_and_recent_now_ms() {
        let ctx = ExtractCtx::new("phase15b-v1");
        assert_eq!(ctx.analyzer_version, "phase15b-v1");
        assert!(ctx.now_ms > 0);
    }

    #[test]
    fn extract_ctx_with_now_ms_pins_the_clock_for_tests() {
        let ctx = ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000);
        assert_eq!(ctx.now_ms, 1_700_000_000_000);
    }

    #[test]
    fn stamp_provenance_adds_source_event_id_analyzer_version_created_at_ms() {
        let env = fixture_envelope("01HXEVT01", Kind::Turn);
        let ctx = ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000);
        let edge = stamp_provenance(sample_edge(), &env, &ctx);
        assert_eq!(
            edge.props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXEVT01")
        );
        assert_eq!(
            edge.props.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase15b-v1")
        );
        assert_eq!(
            edge.props.get("created_at_ms").and_then(|v| v.as_i64()),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn stamp_provenance_overwrites_prior_provenance_to_stay_idempotent() {
        let env = fixture_envelope("01HXEVT02", Kind::Turn);
        let ctx = ExtractCtx::with_now_ms("phase15b-v1", 42);
        let mut edge = sample_edge();
        edge.props.insert(
            "source_event_id".to_string(),
            serde_json::Value::String("stale".into()),
        );
        let stamped = stamp_provenance(edge, &env, &ctx);
        assert_eq!(
            stamped
                .props
                .get("source_event_id")
                .and_then(|v| v.as_str()),
            Some("01HXEVT02"),
            "stamping a second time MUST overwrite stale provenance"
        );
    }
}
