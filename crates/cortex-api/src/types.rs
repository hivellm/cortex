//! Wire types for the query API. Mirrors `docs/specs/11-query-api.md`
//! §Inputs/Outputs verbatim — every field name and shape lines up
//! with the spec so MCP and HTTP callers see the same JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Intent the orchestrator dispatches against — drives lane selection
/// and the overlay set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Pre-thinking context bundle for the Claude Code adapter
    /// (spec 12 owns the adapter-side wrapper).
    PreChangeContext,
    /// Decision lookup with supersession chain.
    DecisionLookup,
    /// Similar-problems search across past turns + analyses.
    SimilarProblems,
    /// Active laws + recent violations within scope.
    LawCheck,
    /// Free-form keyword + vector blend with no overlays.
    FreeSearch,
}

impl Intent {
    /// Snake-case label for telemetry + audit.
    pub fn label(self) -> &'static str {
        match self {
            Intent::PreChangeContext => "pre_change_context",
            Intent::DecisionLookup => "decision_lookup",
            Intent::SimilarProblems => "similar_problems",
            Intent::LawCheck => "law_check",
            Intent::FreeSearch => "free_search",
        }
    }
}

/// Result fields the caller wants surfaced. Matches the spec-11
/// `include` array verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncludeField {
    /// Code / doc snippets (vector + keyword fusion).
    Snippets,
    /// Decision overlays.
    Decisions,
    /// Law-violation overlays.
    Violations,
    /// Graph-neighbor expansion.
    GraphNeighbors,
    /// Similar-turn KNN derivation.
    SimilarTurns,
}

/// Canonicalised scope filter — populated from request and validated
/// against ACLs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Scope {
    /// Repo identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// File-glob filters (Meilisearch + Vectorizer compatible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Topic filters (controlled vocab from the classifier).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    /// ISO-8601 lower bound on `ts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// Request body for `POST /v1/query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    /// Intent — required.
    pub intent: Intent,
    /// Optional scope filter.
    #[serde(default)]
    pub scope: Scope,
    /// Free-text query.
    pub query: String,
    /// Per-field result cap.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// KNN top-k cap for the vector lane.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Result fields the caller wants populated.
    #[serde(default = "default_include")]
    pub include: Vec<IncludeField>,
    /// Total budget the orchestrator must stay under.
    #[serde(default = "default_budget_ms")]
    pub budget_ms: u64,
}

fn default_limit() -> usize {
    20
}
fn default_k() -> usize {
    50
}
fn default_budget_ms() -> u64 {
    500
}
fn default_include() -> Vec<IncludeField> {
    vec![
        IncludeField::Snippets,
        IncludeField::Decisions,
        IncludeField::Violations,
        IncludeField::GraphNeighbors,
        IncludeField::SimilarTurns,
    ]
}

/// One snippet returned by the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    /// 1-based fused rank.
    pub rank: usize,
    /// `vector` | `keyword` | `graph`.
    pub source: String,
    /// Vectorizer collection / Meili index name (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Repo identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Repo-relative path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Symbol (Tree-sitter for code, H1 for doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// `sha256:...` over the chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Snippet text (already redacted by upstream, re-redacted by api).
    pub text: String,
    /// Fused score.
    pub score: f64,
    /// Why-this-result blurb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// One decision overlay entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRef {
    /// 1-based rank within the overlay.
    pub rank: usize,
    /// Decision id (`DEC-NNNN` or ULID).
    pub id: String,
    /// Title.
    pub title: String,
    /// Status (`proposed` / `accepted` / `superseded` / `deprecated`).
    pub status: String,
    /// Decision timestamp in ms epoch.
    pub ts: i64,
    /// Score from the underlying lane.
    pub score: f64,
    /// Optional links to source files / urls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

/// One law-violation overlay entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViolationRef {
    /// Violation id (ULID).
    pub id: String,
    /// Law id (`LAW-NNN`).
    pub law_id: String,
    /// `info` / `notable` / `critical`.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Anchor — `turn:<id>` / `tool_call:<id>`.
    pub observed_in: String,
}

/// One graph-neighbor entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNeighbor {
    /// Source node identifier.
    pub from: String,
    /// Edge type.
    pub relation: String,
    /// Target node identifier.
    pub to: String,
    /// Hop distance from the seed (1 or 2).
    pub hops: u8,
}

/// One similar-turn entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimilarTurn {
    /// Turn ULID.
    pub turn_id: String,
    /// Turn timestamp.
    pub ts: i64,
    /// Model identifier.
    pub model: String,
    /// Classifier-supplied summary.
    pub summary: String,
    /// KNN score.
    pub score: f64,
}

/// Active law surfaced as an overlay (separate from violations).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LawRef {
    /// Law id.
    pub id: String,
    /// Severity.
    pub severity: String,
    /// Title.
    pub title: String,
}

/// Per-lane / overall budget breakdown surfaced in the response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BudgetReport {
    /// Total wall-clock used (ms).
    pub used_ms: u64,
    /// Cap from the request.
    pub cap_ms: u64,
    /// `hit` | `miss`.
    pub cache: String,
}

/// Lane-level latency / partial flags surfaced under `debug.lanes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaneTimings {
    /// Wall-clock taken by the vector lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_ms: Option<u64>,
    /// Wall-clock taken by the keyword lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_ms: Option<u64>,
    /// Wall-clock taken by the graph lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_ms: Option<u64>,
}

/// Debug bag attached to every response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebugInfo {
    /// Lane wall-clock breakdown.
    pub lanes: LaneTimings,
    /// Per-lane errors observed during fan-out.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, String>,
    /// `true` when the orchestrator returned partial results because
    /// the total budget elapsed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Top-level response envelope. Matches spec 11 §Response schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryResponse {
    /// Echoed intent label.
    pub intent: String,
    /// Per-request ULID (echoed in audit + dashboard).
    pub query_id: String,
    /// Canonicalised scope.
    pub scope_resolved: Scope,
    /// Per-field result groups.
    pub results: ResultsBag,
    /// Active laws overlay (top-level for cheap rendering).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub laws_active: Vec<LawRef>,
    /// Budget breakdown.
    pub budget: BudgetReport,
    /// Debug bag.
    pub debug: DebugInfo,
}

/// Result groups bag carried under `results`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResultsBag {
    /// Snippets (vector + keyword + graph fused).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
    /// Decision overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionRef>,
    /// Violation overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<ViolationRef>,
    /// Graph-neighbor overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_neighbors: Vec<GraphNeighbor>,
    /// Similar-turn KNN.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub similar_turns: Vec<SimilarTurn>,
}

/// Convenience helper to build an empty response stamped with a fresh
/// `query_id` and the request's intent label.
pub fn empty_response(req: &QueryRequest) -> QueryResponse {
    QueryResponse {
        intent: req.intent.label().to_string(),
        query_id: ulid::Ulid::new().to_string(),
        scope_resolved: req.scope.clone(),
        results: ResultsBag::default(),
        laws_active: Vec::new(),
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: req.budget_ms,
            cache: "miss".to_string(),
        },
        debug: DebugInfo::default(),
    }
}

/// Free-form properties bag — used by lanes to round-trip raw
/// upstream metadata into the orchestrator without exposing
/// vendor-specific types.
pub type Props = BTreeMap<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_round_trips_through_json() {
        let r: QueryRequest = serde_json::from_value(serde_json::json!({
            "intent": "pre_change_context",
            "query": "tune ef_search",
        }))
        .unwrap();
        assert_eq!(r.intent, Intent::PreChangeContext);
        assert_eq!(r.limit, default_limit());
        assert_eq!(r.k, default_k());
        assert_eq!(r.budget_ms, default_budget_ms());
    }

    #[test]
    fn include_default_covers_every_field() {
        let r: QueryRequest = serde_json::from_value(serde_json::json!({
            "intent": "free_search",
            "query": "x",
        }))
        .unwrap();
        let inc = r.include;
        assert!(inc.contains(&IncludeField::Snippets));
        assert!(inc.contains(&IncludeField::Decisions));
        assert!(inc.contains(&IncludeField::Violations));
        assert!(inc.contains(&IncludeField::GraphNeighbors));
        assert!(inc.contains(&IncludeField::SimilarTurns));
    }
}
