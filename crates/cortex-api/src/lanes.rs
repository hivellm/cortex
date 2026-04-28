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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: Props,
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
