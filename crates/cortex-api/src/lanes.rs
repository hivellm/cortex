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
    }
}

/// True when `hit` is a synthetic missing-collection marker
/// produced by [`collection_missing_marker`].
pub fn is_collection_missing_marker(hit: &LaneHit) -> bool {
    hit.doc_id.starts_with(COLLECTION_MISSING_DOC_ID_PREFIX)
}

/// One hit returned by any lane. The orchestrator coerces lane
/// outputs into this shape before fusion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Spec-11 lane projection contract ([`LANE_EXTRAS_KEYS`]):
    /// lane impls MUST stamp the contract keys here when the
    /// upstream document carries them, so the orchestrator's
    /// overlay derivers can read them out 1:1. Missing keys
    /// round-trip as absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: Props,
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
