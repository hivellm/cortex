//! Lane abstractions — vector / keyword / graph clients with mock
//! implementations for tests and HTTP-shaped live implementations for
//! production. The orchestrator interacts with traits only so the
//! same fan-out + RRF logic exercises real and faked backends
//! identically.
//!
//! Live wiring against `vectorizer-sdk`, Meilisearch HTTP, and
//! `nexus-graph-sdk` lives behind the same traits and is selected at
//! daemon startup; the orchestrator and acceptance tests never see
//! the difference.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use cortex_core::events::{ConsolidationGrain, ContradictionKind, Envelope, Kind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::types::{Props, Scope};

/// Keys every lane projection MUST stamp into [`LaneHit::extras`]
/// when the upstream document carries them. This is the spec-11
/// "Lane projection contract" — see
/// `docs/specs/11-query-api.md` §Lane projection contract for the
/// full table of upstream sources and downstream consumers.
///
/// Orchestrator overlays read these directly:
///
/// - `decision_id` / `decision_status` / `supersedes` →
///   [`crate::orchestrator::derive_decisions`]
/// - `turn_id` / `model` / `summary` →
///   [`crate::orchestrator::derive_similar_turns`]
/// - `law_id` (with `severity` cross-checked against the
///   top-level [`LaneHit::severity`] field) →
///   [`crate::orchestrator::derive_laws`]
///
/// Missing keys round-trip as absent — the overlay derivers skip
/// rows that lack the relevant key gracefully. Live lanes
/// (`MeiliKeywordLane`, `VectorizerLane`) round-trip these
/// unchanged when present; tests rely on the `MemoryKeywordLane`
/// double seeding them directly.
pub const LANE_EXTRAS_KEYS: &[&str] = &[
    "decision_id",
    "decision_status",
    "supersedes",
    "turn_id",
    "model",
    "summary",
    "law_id",
    "severity",
];

/// Phase11e §3 — sentinel `doc_id` prefix for synthetic
/// `LaneHit`s lanes emit to flag a missing collection / index.
/// The orchestrator strips these before fusion and translates
/// them into `DebugInfo.notes` entries (gated on
/// `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1`). The prefix is
/// long + double-underscored on both ends so it cannot collide
/// with a real chunk id in practice.
pub const COLLECTION_MISSING_DOC_ID_PREFIX: &str = "__cortex_collection_missing__:";

/// Phase11e §3 — env switch that turns the synthetic-marker
/// emission ON. When unset (the default), lanes still swallow
/// 404s into `Ok(empty)` so the response shape is unchanged
/// for every existing caller.
pub const COLLECTION_MISSING_REPORT_ENV: &str = "CORTEX_QUERY_REPORT_MISSING_COLLECTIONS";

/// Phase11e §3 — read [`COLLECTION_MISSING_REPORT_ENV`] and return
/// `true` when it is `1` / `true` / `yes` (case-insensitive).
/// Centralised so every lane reads the same parsing rules.
pub fn collection_missing_report_enabled() -> bool {
    matches!(
        std::env::var(COLLECTION_MISSING_REPORT_ENV)
            .ok()
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase()),
        Some(ref v) if v == "1" || v == "true" || v == "yes"
    )
}

/// Phase11e §3 — build a synthetic `LaneHit` carrying the
/// missing-collection marker. Lanes emit this in place of an
/// empty `Ok(vec![])` when the env switch is on; the
/// orchestrator strips it before fusion and forwards a
/// [`crate::types::DebugNote`] to the caller.
pub fn collection_missing_marker(collection: impl Into<String>) -> LaneHit {
    let name = collection.into();
    let doc_id = format!("{COLLECTION_MISSING_DOC_ID_PREFIX}{name}");
    let mut extras = Props::new();
    extras.insert(
        "__collection_missing".to_string(),
        serde_json::Value::String(name.clone()),
    );
    LaneHit {
        doc_id,
        text: String::new(),
        repo: None,
        path: None,
        symbol: None,
        content_hash: None,
        score: 0.0,
        ts: 0,
        severity: None,
        extras,
        overlay: Overlay::default(),
    }
}

/// True when `hit` is a synthetic missing-collection marker
/// produced by [`collection_missing_marker`].
pub fn is_collection_missing_marker(hit: &LaneHit) -> bool {
    hit.doc_id.starts_with(COLLECTION_MISSING_DOC_ID_PREFIX)
}

/// ADR-011 — lane of origin for a `LaneHit`. Replaces the
/// stringly-typed `extras["source"] = "vector|keyword|graph"`
/// convention. The orchestrator's lane-of-origin tie-break
/// pattern-matches this directly so a drift between a lane's
/// `name()` and the field it stamps is a compile error.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LaneSource {
    /// `VectorLane` — Vectorizer KNN.
    Vector,
    /// `KeywordLane` — Meili / in-memory BM25.
    Keyword,
    /// `GraphLane` — Nexus graph traversal.
    Graph,
    /// `Source` is unknown — used for synthetic hits the orchestrator
    /// injects (missing-collection markers, debug fixtures). Never
    /// stamped by a live lane. Default so an `Overlay::default()`
    /// stamps the safe value rather than misclaiming a lane.
    #[default]
    Unknown,
}

/// ADR-011 — short label for the orchestrator's lane-of-origin
/// classification. Stable across releases; serialised forms are
/// `"vector"`, `"keyword"`, `"graph"`, `"unknown"`.
impl LaneSource {
    /// Stable snake-case label matching the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            LaneSource::Vector => "vector",
            LaneSource::Keyword => "keyword",
            LaneSource::Graph => "graph",
            LaneSource::Unknown => "unknown",
        }
    }
}

/// ADR-011 — decision lifecycle stamped on overlay rows that point
/// back to a `Kind::Decision`. The pre-ADR-011 contract used the
/// raw string from `extras["decision_status"]`; the typed form
/// catches a lane that stamps a value outside the known set at
/// compile / serialise time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Proposed and awaiting acceptance.
    Proposed,
    /// Accepted as authoritative.
    Accepted,
    /// Superseded by a newer decision (walk `superseded_by`).
    Superseded,
    /// Deprecated without a successor.
    Deprecated,
    /// Rejected (kept for the audit trail).
    Rejected,
}

/// ADR-011 — typed overlay carried on every `LaneHit`. Replaces the
/// stringly-typed `extras: Props` map. Lane impls populate the
/// fields their upstream document carries; missing fields stay
/// `None` instead of "absent map key". The orchestrator's overlay
/// derivers read these directly — see
/// `crates/cortex-api/src/orchestrator.rs::derive_*`.
///
/// Per-lane ownership (which fields each lane is expected to fill):
///
/// | Field                  | Vector | Keyword | Graph |
/// |------------------------|--------|---------|-------|
/// | `decision_id`          |        | ✓ (governance) | ✓ |
/// | `decision_status`      |        | ✓        | ✓ |
/// | `superseded_by`        |        | ✓        | ✓ |
/// | `turn_id`              | ✓      | ✓ (turns)| ✓ |
/// | `model`                | ✓      | ✓ (turns)|   |
/// | `summary`              | ✓ (consolidations) | ✓ (turns) | |
/// | `law_id`               |        | ✓ (governance) | ✓ |
/// | `violation_id`         |        | ✓ (governance) |   |
/// | `severity`             |        | ✓ (governance) |   |
/// | `edge_from` / `edge_to`|        |          | ✓ |
/// | `hops`                 |        |          | ✓ |
/// | `body_truncated`       | ✓      | ✓        | ✓ |
/// | `contradiction_flag`   |        | ✓        | ✓ |
/// | `consolidation_grain`  | ✓ (consolidations) | ✓ | |
/// | `topic_id`             |        | ✓ (topic cards) | ✓ |
/// | `source`               | ✓      | ✓        | ✓ (every lane MUST stamp) |
///
/// Wire format note: serialises as a flat JSON object with
/// `skip_serializing_if = "Option::is_none"` so absent fields do
/// not appear in the `/v1/query` response. Existing callers
/// reading the previous flat `extras` payload see the same shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Overlay {
    /// `Kind::Decision` ULID this hit refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// Lifecycle status of the decision the hit cites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_status: Option<DecisionStatus>,
    /// ULID of the decision that supersedes this one (when
    /// `decision_status == Superseded`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// `Kind::Turn` ULID associated with the hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Model identifier that produced the underlying turn (`Turn` /
    /// `Consolidation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Pre-distilled summary text (consolidations / topic cards).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// `Law` identifier (LAW-NNN) when the hit cites or violates a law.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub law_id: Option<String>,
    /// `Kind::LawViolation` ULID that surfaced this hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub violation_id: Option<String>,
    /// Severity label (mirrors top-level [`LaneHit::severity`] but
    /// stays typed across the overlay path). String for now to
    /// preserve compatibility with the existing string column;
    /// will tighten to a `Severity` enum once
    /// `crate::types::Severity` lands per ADR-011 §followup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Source endpoint of a graph edge (`GraphLane`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_from: Option<String>,
    /// Target endpoint of a graph edge (`GraphLane`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_to: Option<String>,
    /// Traversal distance to the seed node (`GraphLane`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<u8>,
    /// Body was clipped before serialisation. Default `false` (not
    /// `Option<bool>`) because absence is meaningful: every hit
    /// either was truncated or was not.
    #[serde(default, skip_serializing_if = "is_false")]
    pub body_truncated: bool,
    /// Contradiction class when the hit was flagged by the synthesiser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contradiction_flag: Option<ContradictionKind>,
    /// Consolidation grain when the hit is a `Kind::Consolidation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consolidation_grain: Option<ConsolidationGrain>,
    /// Topic-card / cluster identifier the hit belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    /// Kind of the underlying envelope. Stamped by `From<&Envelope>`
    /// so the orchestrator can branch on payload shape without
    /// re-parsing the body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// Lane of origin. Every live lane MUST stamp this; the
    /// `Unknown` variant is reserved for synthetic hits.
    #[serde(default = "default_lane_source")]
    pub source: LaneSource,
}

fn default_lane_source() -> LaneSource {
    LaneSource::Unknown
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// ADR-011 — every overlay field that can be derived directly from
/// the source envelope is filled here; lane-specific overlays
/// (`turn_id`, `decision_status`, `edge_from`, …) are stamped
/// downstream by each lane impl. This `From` keeps the "obvious"
/// fields off the lane impls so they only handle the lane-specific
/// branches.
impl From<&Envelope> for Overlay {
    fn from(env: &Envelope) -> Self {
        Overlay {
            kind: Some(env.kind),
            model: env.model.clone(),
            // turn_id / decision_id / law_id are payload-shape
            // specific; lane impls fill them once they've parsed
            // the payload. The envelope-level fields below are
            // payload-agnostic.
            ..Overlay::default()
        }
    }
}

/// ADR-011 — unified lane contract. Replaces the per-lane
/// `VectorLane` / `KeywordLane` / `GraphLane` triple as the
/// canonical trait the orchestrator depends on. The pre-ADR-011
/// traits remain as adapter targets until §4 ports them; new lane
/// impls (phase11v fine-grained MCP search tools) implement `Lane`
/// directly.
///
/// The trait operates at the orchestrator's abstraction level
/// (`Query` + `Scope` → `Vec<LaneHit>`) rather than each lane's
/// native request shape so the orchestrator does not have to
/// know about per-lane wire formats. Each `impl Lane for …`
/// constructs its own per-lane request internally from the
/// shared `Query` / `Scope` pair.
#[async_trait]
pub trait Lane: Send + Sync {
    /// Short identifier — `"vector"` / `"keyword"` / `"graph"` /
    /// `"<custom>"`. Returned verbatim in debug envelopes so the
    /// caller can audit which lanes fired.
    fn name(&self) -> &'static str;

    /// Lane of origin stamped on every emitted `LaneHit.overlay`.
    /// Defaults to a name-derived match; lane impls override when
    /// the lane is named differently from its `LaneSource` variant.
    fn source(&self) -> LaneSource {
        match self.name() {
            "vector" => LaneSource::Vector,
            "keyword" => LaneSource::Keyword,
            "graph" => LaneSource::Graph,
            _ => LaneSource::Unknown,
        }
    }

    /// Run the lane against `query` / `scope` and return projected
    /// hits. Errors propagate as `LaneError`; an empty result is
    /// `Ok(vec![])`.
    async fn search(
        &self,
        query: &crate::types::QueryRequest,
        scope: &Scope,
    ) -> Result<Vec<LaneHit>, LaneError>;
}

/// One hit returned by any lane. The orchestrator coerces lane
/// outputs into this shape before fusion.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LaneHit {
    /// Globally unique id for fusion (e.g. `<repo>|<path>|<chunk_hash>`).
    pub doc_id: String,
    /// Snippet text (already redacted upstream).
    pub text: String,
    /// Repo identifier when the hit is repo-scoped.
    pub repo: Option<String>,
    /// Repo-relative path when applicable.
    pub path: Option<String>,
    /// Symbol / heading / title for snippet rendering.
    pub symbol: Option<String>,
    /// `sha256:...` over the chunk body.
    pub content_hash: Option<String>,
    /// Native lane score (cosine for vector, BM25-style for keyword).
    pub score: f64,
    /// 0-based timestamp surfaced for tie-break.
    pub ts: i64,
    /// Severity label (used by the violations overlay tie-break).
    pub severity: Option<String>,
    /// Free-form lane metadata round-tripped to debug.
    ///
    /// **Deprecated in ADR-011** — being replaced by the typed
    /// [`LaneHit::overlay`] field. Both fields coexist during the
    /// migration: lane impls populate `overlay` directly; the few
    /// extras keys that have not yet migrated (per-lane debug
    /// counters) stay here. Once §4 completes, this field is
    /// removed and the typed `overlay` field becomes the only
    /// path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: Props,
    /// ADR-011 — typed overlay. Replaces the stringly-typed
    /// `extras` map. Lane impls populate the fields their
    /// upstream document carries; absent fields stay at their
    /// default (`None` / `false` / `LaneSource::Unknown`). The
    /// orchestrator's overlay derivers read these directly.
    #[serde(default, skip_serializing_if = "Overlay::is_empty")]
    pub overlay: Overlay,
}

impl Overlay {
    /// ADR-011 — true when every field is at its default. Used by
    /// the future `LaneHit::overlay` field's
    /// `serde(skip_serializing_if)` so the wire format stays
    /// identical for synthetic / pre-migration hits.
    pub fn is_empty(&self) -> bool {
        matches!(
            self,
            Overlay {
                decision_id: None,
                decision_status: None,
                superseded_by: None,
                turn_id: None,
                model: None,
                summary: None,
                law_id: None,
                violation_id: None,
                severity: None,
                edge_from: None,
                edge_to: None,
                hops: None,
                body_truncated: false,
                contradiction_flag: None,
                consolidation_grain: None,
                topic_id: None,
                kind: None,
                source: LaneSource::Unknown,
            }
        )
    }
}

impl LaneHit {
    /// Phase6c — clamp the lane-native score onto the canonical
    /// `[0.0, 1.0]` interval so the score-aware fusion blend can
    /// combine it with positional RRF without per-lane scaling.
    ///
    /// Lanes that already produce `[0,1]`-valued scores (Vectorizer
    /// cosine similarity, Meili `_rankingScore`) round-trip
    /// unchanged. The Nexus graph lane currently emits `0.0` per
    /// hit, so its native contribution is zero until phase4c
    /// stamps a path-length-derived score
    /// (`1.0` direct neighbour → `0.5` 2-hop → `0.25` 3-hop) — see
    /// `docs/analysis/relevance/01-findings.md` §F-002. NaN /
    /// infinity collapse to `0.0`.
    pub fn normalized_score(&self) -> f64 {
        if self.score.is_nan() || self.score.is_infinite() {
            return 0.0;
        }
        self.score.clamp(0.0, 1.0)
    }
}

/// Failure modes raised by lane clients.
#[derive(Debug, Error)]
pub enum LaneError {
    /// Network / transport-layer failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Backend returned a 4xx / configuration error.
    #[error("rejected: {0}")]
    Rejected(String),
    /// Lane budget exceeded — orchestrator soft-cancels.
    #[error("budget exceeded")]
    BudgetExceeded,
}

/// Vector-lane request — the orchestrator wraps the raw query plus
/// per-collection routing.
#[derive(Debug, Clone)]
pub struct VectorRequest {
    /// Vectorizer collection (per-kind family from spec 06).
    pub collection: String,
    /// Free-text query the lane embeds + searches.
    pub query: String,
    /// KNN top-k.
    pub k: usize,
    /// Repo + topic filter forwarded as Vectorizer metadata predicates.
    pub scope: Scope,
}

/// Keyword-lane request — wraps the Meili index + filter shape.
#[derive(Debug, Clone)]
pub struct KeywordRequest {
    /// Meilisearch index name.
    pub index: String,
    /// Free-text query.
    pub query: String,
    /// Per-field hit cap.
    pub limit: usize,
    /// Scope mapped to filterable attributes.
    pub scope: Scope,
}

/// Graph-lane request — parametrised Cypher (read-only).
#[derive(Debug, Clone)]
pub struct GraphRequest {
    /// Cypher template id (the lane resolves to the templated source).
    pub template: String,
    /// Parameters bound by the template.
    pub params: Value,
    /// Hop budget (1 or 2).
    pub max_hops: u8,
    /// Scope echoed for ACL checks.
    pub scope: Scope,
}

/// Vector-lane abstraction.
#[async_trait]
pub trait VectorLane: Send + Sync {
    /// Run a KNN query.
    async fn search(&self, req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError>;
}

/// Keyword-lane abstraction.
#[async_trait]
pub trait KeywordLane: Send + Sync {
    /// Run a typo-tolerant filterable search.
    async fn search(&self, req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError>;
}

/// Graph-lane abstraction.
#[async_trait]
pub trait GraphLane: Send + Sync {
    /// Run a parametrised read-only Cypher query.
    async fn query(&self, req: &GraphRequest) -> Result<Vec<LaneHit>, LaneError>;
}

// ---------- Memory lanes (test doubles) ---------------------------------

/// In-memory vector lane backed by a fixed-result list — the
/// orchestrator gets the same hit set on every call so RRF / overlay /
/// truncation tests are deterministic.
#[derive(Debug, Default)]
pub struct MemoryVectorLane {
    /// Pre-canned hits per collection name.
    pub hits: Mutex<BTreeMap<String, Vec<LaneHit>>>,
    /// Optional artificial delay for the budget tests.
    pub delay: Option<Duration>,
    /// `true` flips the lane into hard-fail mode for failure-mode tests.
    pub fail: bool,
}

impl MemoryVectorLane {
    /// Fresh lane.
    pub fn new() -> Self {
        Self::default()
    }
    /// Pre-populate `collection`'s hit list.
    pub fn seed(&self, collection: &str, hits: Vec<LaneHit>) {
        if let Ok(mut g) = self.hits.lock() {
            g.insert(collection.to_string(), hits);
        }
    }
    /// Consume an artificial delay configuration.
    pub fn with_delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }
    /// Configure hard-fail mode.
    pub fn with_fail(mut self) -> Self {
        self.fail = true;
        self
    }
}

#[async_trait]
impl VectorLane for MemoryVectorLane {
    async fn search(&self, req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        if self.fail {
            return Err(LaneError::Transport("memory vector hard-fail".into()));
        }
        let hits = self
            .hits
            .lock()
            .ok()
            .and_then(|g| g.get(&req.collection).cloned())
            .unwrap_or_default();
        Ok(hits)
    }
}

/// Memory keyword lane.
#[derive(Debug, Default)]
pub struct MemoryKeywordLane {
    /// Hits keyed by index name.
    pub hits: Mutex<BTreeMap<String, Vec<LaneHit>>>,
    /// Artificial delay for budget tests.
    pub delay: Option<Duration>,
    /// Hard-fail flag.
    pub fail: bool,
}

impl MemoryKeywordLane {
    /// Fresh lane.
    pub fn new() -> Self {
        Self::default()
    }
    /// Pre-populate `index`'s hit list.
    pub fn seed(&self, index: &str, hits: Vec<LaneHit>) {
        if let Ok(mut g) = self.hits.lock() {
            g.insert(index.to_string(), hits);
        }
    }
    /// Configure delay.
    pub fn with_delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }
    /// Configure hard-fail.
    pub fn with_fail(mut self) -> Self {
        self.fail = true;
        self
    }

    /// Distinct repo slugs visible in the seeded snapshot. Each hit's
    /// raw `repo` value is run through
    /// `cortex_storage::names::slug_for_repo` so the result matches the
    /// canonicalised form the query lanes filter on (a request that
    /// scopes to `"Vectorizer"` and a hit stamped with the slug
    /// `"vectorizer"` agree on the same key). Sorted + deduplicated so
    /// callers can `.contains()` cheaply.
    pub fn indexed_repos(&self) -> Vec<String> {
        let Ok(g) = self.hits.lock() else {
            return Vec::new();
        };
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for hits in g.values() {
            for h in hits {
                if let Some(repo) = h.repo.as_deref().filter(|s| !s.is_empty()) {
                    set.insert(cortex_storage::names::slug_for_repo(repo));
                }
            }
        }
        set.into_iter().collect()
    }
}

#[async_trait]
impl KeywordLane for MemoryKeywordLane {
    async fn search(&self, req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        if self.fail {
            return Err(LaneError::Transport("memory keyword hard-fail".into()));
        }
        let hits = self
            .hits
            .lock()
            .ok()
            .and_then(|g| g.get(&req.index).cloned())
            .unwrap_or_default();
        Ok(hits)
    }
}

/// Memory graph lane.
#[derive(Debug, Default)]
pub struct MemoryGraphLane {
    /// Hits keyed by template id.
    pub hits: Mutex<BTreeMap<String, Vec<LaneHit>>>,
    /// Artificial delay for budget tests.
    pub delay: Option<Duration>,
    /// Hard-fail flag.
    pub fail: bool,
}

impl MemoryGraphLane {
    /// Fresh lane.
    pub fn new() -> Self {
        Self::default()
    }
    /// Pre-populate `template`'s hit list.
    pub fn seed(&self, template: &str, hits: Vec<LaneHit>) {
        if let Ok(mut g) = self.hits.lock() {
            g.insert(template.to_string(), hits);
        }
    }
    /// Configure delay.
    pub fn with_delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }
    /// Configure hard-fail.
    pub fn with_fail(mut self) -> Self {
        self.fail = true;
        self
    }
}

#[async_trait]
impl GraphLane for MemoryGraphLane {
    async fn query(&self, req: &GraphRequest) -> Result<Vec<LaneHit>, LaneError> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        if self.fail {
            return Err(LaneError::Transport("memory graph hard-fail".into()));
        }
        let hits = self
            .hits
            .lock()
            .ok()
            .and_then(|g| g.get(&req.template).cloned())
            .unwrap_or_default();
        Ok(hits)
    }
}

#[cfg(test)]
mod adr_011_tests {
    use super::*;
    use cortex_core::events::{Context, Envelope, Kind, Stream};

    fn envelope_fixture(kind: Kind, model: Option<&str>) -> Envelope {
        Envelope {
            event_id: "01TESTENVELOPE0000000000000".to_string(),
            schema_version: "1".to_string(),
            occurred_at: "2026-05-20T00:00:00Z".to_string(),
            ingested_at: None,
            session_id: "01TESTSESSION00000000000000".to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: model.map(str::to_string),
            kind,
            context: Context {
                repo: Some("cortex".to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "win32".to_string(),
                ide: None,
                extras: std::collections::BTreeMap::new(),
            },
            payload: serde_json::json!({}),
            redactions: Vec::new(),
            content_hash: "sha256:0".to_string(),
            parent_event_id: None,
        }
    }

    #[test]
    fn overlay_default_is_empty() {
        assert!(Overlay::default().is_empty());
    }

    #[test]
    fn overlay_is_empty_detects_populated_field() {
        let mut o = Overlay::default();
        o.decision_id = Some("01ABC".into());
        assert!(!o.is_empty());
    }

    #[test]
    fn overlay_from_envelope_stamps_kind_and_model() {
        let env = envelope_fixture(Kind::Turn, Some("claude-opus-4-7"));
        let o = Overlay::from(&env);
        assert_eq!(o.kind, Some(Kind::Turn));
        assert_eq!(o.model.as_deref(), Some("claude-opus-4-7"));
        // Lane-specific fields stay None — the lane impl fills them
        // once it has parsed the payload.
        assert!(o.decision_id.is_none());
        assert!(o.turn_id.is_none());
        assert!(o.edge_from.is_none());
        // Source defaults to Unknown; each lane stamps the real
        // value before emitting the hit.
        assert_eq!(o.source, LaneSource::Unknown);
    }

    #[test]
    fn overlay_from_envelope_handles_missing_model() {
        let env = envelope_fixture(Kind::Decision, None);
        let o = Overlay::from(&env);
        assert_eq!(o.kind, Some(Kind::Decision));
        assert!(o.model.is_none());
    }

    #[test]
    fn lane_source_as_str_matches_serde_repr() {
        assert_eq!(LaneSource::Vector.as_str(), "vector");
        assert_eq!(LaneSource::Keyword.as_str(), "keyword");
        assert_eq!(LaneSource::Graph.as_str(), "graph");
        assert_eq!(LaneSource::Unknown.as_str(), "unknown");
        for s in [
            LaneSource::Vector,
            LaneSource::Keyword,
            LaneSource::Graph,
            LaneSource::Unknown,
        ] {
            let json = serde_json::to_value(s).unwrap();
            assert_eq!(json.as_str(), Some(s.as_str()));
        }
    }

    #[test]
    fn lane_source_default_is_unknown() {
        // The Overlay default must NEVER claim a lane it cannot
        // back — Unknown is the safe sentinel that the
        // orchestrator's lane-of-origin tie-break (line 257 of
        // orchestrator.rs) ignores.
        assert_eq!(LaneSource::default(), LaneSource::Unknown);
    }

    #[test]
    fn decision_status_serialises_as_snake_case() {
        let cases = [
            (DecisionStatus::Proposed, "proposed"),
            (DecisionStatus::Accepted, "accepted"),
            (DecisionStatus::Superseded, "superseded"),
            (DecisionStatus::Deprecated, "deprecated"),
            (DecisionStatus::Rejected, "rejected"),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_value(variant).unwrap();
            assert_eq!(json.as_str(), Some(expected));
        }
    }

    #[test]
    fn overlay_skips_serialising_default_fields() {
        // The wire format MUST stay identical to the pre-ADR-011
        // shape for hits that carry no overlay data — a synthetic
        // marker should serialise to an empty object after the
        // `is_empty` guard kicks in on the LaneHit serde
        // attribute. Test the per-field skip behaviour here so
        // §3 can drop the guard with confidence.
        let o = Overlay::default();
        let json = serde_json::to_value(&o).unwrap();
        // Default-only Overlay → `{}` once every field is
        // skipped (only `source` defaults to a value that may
        // serialise; the `default = "default_lane_source"` serde
        // hint sits on field-level deserialise, not serialise,
        // so we accept either `{}` or `{"source":"unknown"}`).
        let obj = json.as_object().unwrap();
        if !obj.is_empty() {
            assert_eq!(obj.len(), 1);
            assert_eq!(obj.get("source").and_then(|v| v.as_str()), Some("unknown"));
        }
    }
}
