//! Phase11r §4 — MCP tool surface for topic cards.
//!
//! Five tools land here, each with a JSON descriptor and a uniform
//! `invoke_*` async handler that the runtime calls with parsed input.
//! Identical surface-by-construction to the spec-11 `cortex_query`
//! pattern in [`crate::mcp`] so behaviour stays aligned across tools:
//!
//! - [`topic_get_descriptor`] / [`invoke_topic_get`] — fetch the
//!   top topic card for a slug-or-query.
//! - [`topic_drill_descriptor`] / [`invoke_topic_drill`] — drill
//!   into a topic-card dimension (`evidence`, `contradictions`,
//!   `history`, `open_questions`, `related`). *(arrives in §4.2)*
//! - [`topic_neighbors_descriptor`] / [`invoke_topic_neighbors`] —
//!   subgraph walk via Nexus. *(arrives in §4.3)*
//! - [`topic_diff_descriptor`] / [`invoke_topic_diff`] — synthesis
//!   diff between two revisions. *(arrives in §4.4)*
//! - [`synthesize_descriptor`] / [`invoke_synthesize`] — operator
//!   escape hatch that runs the synthesiser ad-hoc. *(arrives in §4.5)*
//!
//! Editorial deviation from the §4 brief: the task description placed
//! these tools "in `crates/cortex-api/src/mcp.rs`". They land in this
//! sibling module instead because (a) `mcp.rs` already owns
//! `cortex_query` end-to-end and the additional ~600 lines for the
//! topic-card tools would dilute it, (b) the topic-card surface is
//! self-contained — its own error type, lookup trait, and payload
//! contract — and groups cleanly behind a single `mcp_topic_card`
//! module path. `lib.rs` re-exports the public names so callers stay
//! ignorant of the file layout.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_core::events::{Contradiction, EvidenceKind, EvidenceRef, TopicCardPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::audit::{record_topic_card_call, AuditPublisher};
use crate::types::Scope;

/// MCP tool name for §4.1. Identifier-safe per MCP 2024-11-05.
pub const TOOL_NAME_TOPIC_GET: &str = "cortex_topic_get";

/// MCP tool name for §4.2.
pub const TOOL_NAME_TOPIC_DRILL: &str = "cortex_topic_drill";

/// MCP tool name for §4.3.
pub const TOOL_NAME_TOPIC_NEIGHBORS: &str = "cortex_topic_neighbors";

/// MCP tool name for §4.4.
pub const TOOL_NAME_TOPIC_DIFF: &str = "cortex_topic_diff";

/// MCP tool name for §4.5 — operator escape hatch.
pub const TOOL_NAME_SYNTHESIZE: &str = "cortex_synthesize";

/// §4.3 — maximum nodes returned from a neighbours walk before the
/// Cypher `LIMIT` clause clips. Matches the spec brief: "subgraph
/// with nodes + edges, clipped at 64 nodes".
pub const TOPIC_NEIGHBORS_NODE_CAP: usize = 64;

/// §4.3 — default traversal depth when the caller omits the param.
pub const TOPIC_NEIGHBORS_DEFAULT_DEPTH: u8 = 2;

/// Confidence floor below which `cortex_topic_get`'s query path
/// returns `None` instead of the top-1 hit. Spec §4.1: "returns
/// top-1 if confidence ≥ 0.6". A weak match adds noise to the
/// agent prompt; the tool prefers a hard `null` over a low-signal
/// card.
pub const TOPIC_GET_CONFIDENCE_FLOOR: f32 = 0.6;

/// MCP-side error shape for the topic-card tools.
#[derive(Debug, Error)]
pub enum TopicCardMcpError {
    /// 422-equivalent — `scope.repo` was empty or absent. Topic
    /// cards are repo-scoped (per spec 11r §1.2 the canonical card
    /// id is derived from `(slug, repo_scope)`); cross-repo reads
    /// are answered through `cortex_query` with explicit overrides
    /// instead of this tool.
    #[error("scope.repo is required")]
    ScopeRepoRequired,
    /// 400-equivalent — the request was unparsable (missing
    /// `query_or_slug`, malformed JSON).
    #[error("invalid input: {0}")]
    Invalid(String),
    /// 5xx-equivalent — the lookup backend (Meili / Vectorizer)
    /// returned a transport / decode error.
    #[error("backend error: {0}")]
    Backend(String),
    /// 429-equivalent — the §4.5 cost budget was exhausted before
    /// the synthesiser could run. Carries the budget cap and the
    /// already-spent amount so the operator can surface both
    /// numbers in the dashboard.
    #[error("budget exhausted: {used_cents} / {cap_cents} cents")]
    BudgetExhausted {
        /// Already-spent cents in the active budget window.
        used_cents: u32,
        /// Configured cap.
        cap_cents: u32,
    },
}

/// True when `s` matches the canonical topic-card slug shape per
/// `topic_card.schema.json` regex `^[a-z0-9](?:[a-z0-9-]{0,78}[a-z0-9])?$`:
/// kebab-case, 1-80 chars, starts/ends with `[a-z0-9]`. Used by the
/// §4.1 slug-exact short-circuit so `auth-rewrite` lands the
/// `get_by_slug` lane while a free-text query (`how does auth
/// rewrite work?`) drops through to the search lane.
pub fn is_valid_topic_slug(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 80 {
        return false;
    }
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) {
        return false;
    }
    bytes.iter().all(|&c| alnum(c) || c == b'-')
}

/// Read-side backend the §4.1 tool dispatches against. The
/// production implementation runs Meili search against
/// `cortex_topic_cards` filtered on `repos` (and the per-repo
/// Vectorizer collection for the slug-exact path); tests substitute
/// an in-memory fake. Both paths return the canonical
/// `TopicCardPayload` so the MCP surface stays uniform with the
/// envelope contract from spec 11r §1.
#[async_trait]
pub trait TopicCardLookup: Send + Sync {
    /// Resolve a card by exact slug (slug-exact path of §4.1).
    /// Returns `None` when the slug is unknown in the scope.
    async fn get_by_slug(
        &self,
        slug: &str,
        scope: &Scope,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError>;

    /// Run hybrid search and return the top-1 card. The §4.1
    /// confidence gate (≥ 0.6) is applied by the caller in
    /// [`invoke_topic_get`]; backends return whatever they
    /// retrieve, no pre-filtering.
    async fn search(
        &self,
        query: &str,
        scope: &Scope,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError>;
}

/// Build the MCP descriptor for `cortex_topic_get`.
pub fn topic_get_descriptor() -> Value {
    json!({
        "name": TOOL_NAME_TOPIC_GET,
        "description": "Fetch the top topic card for a slug-or-query, scoped to a single repo. \
                        Slug-exact short-circuits (kebab-case ≤80 chars); free-text queries \
                        run hybrid search over `cortex_topic_cards` and return the top-1 hit \
                        when confidence ≥ 0.6. Returns null when no card matches the floor.",
        "inputSchema": {
            "type": "object",
            "required": ["query_or_slug", "scope"],
            "properties": {
                "query_or_slug": {
                    "type": "string",
                    "description": "Either a canonical kebab-case slug (`auth-rewrite`) or a \
                                    free-text query. The dispatcher chooses the lane based on \
                                    whether the input matches the slug regex.",
                },
                "scope": {
                    "type": "object",
                    "required": ["repo"],
                    "properties": {
                        "repo": { "type": "string" },
                    },
                },
            },
        },
        "outputSchema": {
            "type": "object",
            "description": "Either a `TopicCardPayload` (per spec 11r §1) or `null` when no \
                            card matches.",
        },
    })
}

/// Dispatch logic for `cortex_topic_get`. Slug-exact path runs first
/// when the input is a valid slug; the query path is the fallback.
/// The confidence floor applies only on the search path — a slug-
/// exact match is always returned verbatim because the caller named
/// the card directly.
///
/// Every call (success, miss, or rejection) emits one
/// `topic_card_mcp_audit` envelope through `audit_publisher` per
/// spec 11r §4.1 so the dashboard's audit lane surfaces drill / get /
/// neighbor traffic alongside the existing `cortex_query` path.
pub async fn invoke_topic_get(
    lookup: Arc<dyn TopicCardLookup>,
    audit_publisher: &dyn AuditPublisher,
    caller: &str,
    scope: Scope,
    query_or_slug: String,
) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
    let scope_repo = scope.repo.as_deref().filter(|s| !s.is_empty());
    if scope_repo.is_none() {
        record_topic_card_call(
            audit_publisher,
            caller,
            TOOL_NAME_TOPIC_GET,
            None,
            json!({ "error": "scope_repo_required" }),
        )
        .await;
        return Err(TopicCardMcpError::ScopeRepoRequired);
    }

    let mut via = "search";
    let mut result: Option<TopicCardPayload> = None;
    if is_valid_topic_slug(&query_or_slug) {
        if let Some(card) = lookup.get_by_slug(&query_or_slug, &scope).await? {
            via = "slug_exact";
            result = Some(card);
        }
    }
    if result.is_none() {
        let hit = lookup.search(&query_or_slug, &scope).await?;
        result = hit.filter(|c| c.confidence >= TOPIC_GET_CONFIDENCE_FLOOR);
    }

    let summary = match result.as_ref() {
        Some(card) => json!({
            "hit": card.topic_slug,
            "topic_card_id": card.topic_card_id,
            "revision": card.revision,
            "confidence": card.confidence,
            "via": via,
        }),
        None => json!({ "hit": null, "via": via }),
    };
    record_topic_card_call(
        audit_publisher,
        caller,
        TOOL_NAME_TOPIC_GET,
        scope_repo,
        summary,
    )
    .await;
    Ok(result)
}

// ---------------------------------------------------------------------------
// §4.2 — cortex_topic_drill
// ---------------------------------------------------------------------------

/// Dimension a `cortex_topic_drill` call expands. Snake-case so the
/// MCP wire shape stays aligned with the rest of the topic-card
/// surface; matches the `DrillDimension` enum the §4.2 brief named.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrillDimension {
    /// Hydrate every `EvidenceRef` with its source title +
    /// `occurred_at` so the agent prompt reads a real citation
    /// instead of a bare ULID.
    Evidence,
    /// Return the card's `contradictions[]` verbatim — surfaced
    /// for review without the per-evidence hydration.
    Contradictions,
    /// Walk the revision chain (newest → oldest) and return one
    /// [`TopicCardRevision`] per revision.
    History,
    /// Return the card's `open_questions[]` verbatim.
    OpenQuestions,
    /// Return the topic-card ids reachable through one
    /// `:RELATED_TO` hop in Nexus.
    Related,
}

impl DrillDimension {
    /// Snake-case label for telemetry + audit.
    pub fn label(self) -> &'static str {
        match self {
            DrillDimension::Evidence => "evidence",
            DrillDimension::Contradictions => "contradictions",
            DrillDimension::History => "history",
            DrillDimension::OpenQuestions => "open_questions",
            DrillDimension::Related => "related",
        }
    }
}

/// One hydrated evidence entry — the `EvidenceRef` plus the
/// title / `occurred_at` looked up from the source envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HydratedEvidenceItem {
    /// Source kind discriminator (consolidation / decision / law / turn).
    pub kind: EvidenceKind,
    /// Source envelope id.
    pub id: String,
    /// Human-readable title pulled from the source envelope. Empty
    /// when the hydrator could not resolve a title (e.g. the source
    /// envelope is gone / redacted).
    pub title: String,
    /// `occurred_at` (RFC 3339) from the source envelope. Empty
    /// when unavailable.
    pub occurred_at: String,
    /// Revision the synthesiser was on when it cited this evidence.
    pub cited_at_rev: u32,
    /// Caller-assigned weight in `0.0..=1.0`, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// One entry in a topic card's revision chain. Returned by the
/// `History` dimension so callers can plot the synthesis evolution
/// without re-fetching every payload — `synthesis_diff_hash` lets a
/// caller skip a `cortex_topic_diff` when nothing changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicCardRevision {
    /// Deterministic topic-card id (same across every revision).
    pub topic_card_id: String,
    /// Monotonic revision number.
    pub revision: u32,
    /// Wall-clock timestamp of the revision (RFC 3339).
    pub last_rev_at: String,
    /// SHA-256 of the synthesis body for this revision (hex-encoded,
    /// 64 chars). Identical across two revisions means no rewrite
    /// happened despite the version bump.
    pub synthesis_diff_hash: String,
}

/// Result envelope for `cortex_topic_drill`. Exactly one collection
/// is non-empty per call (the one matching the requested
/// `dimension`); the rest are skipped on the wire via
/// `skip_serializing_if`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DrillResult {
    /// Echoed canonical topic-card id (so the caller can pin the
    /// response to the request).
    pub topic_card_id: String,
    /// Echoed dimension.
    pub dimension: DrillDimension,
    /// Hydrated evidence items (`Evidence` dimension).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<HydratedEvidenceItem>,
    /// Surfaced contradictions (`Contradictions` dimension).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradictions: Vec<Contradiction>,
    /// Revision chain (`History` dimension).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<TopicCardRevision>,
    /// Open questions (`OpenQuestions` dimension).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_questions: Vec<String>,
    /// Adjacent topic-card ids (`Related` dimension).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl DrillResult {
    /// Empty result for `topic_card_id` + `dimension`.
    fn empty(topic_card_id: String, dimension: DrillDimension) -> Self {
        Self {
            topic_card_id,
            dimension,
            evidence: Vec::new(),
            contradictions: Vec::new(),
            history: Vec::new(),
            open_questions: Vec::new(),
            related: Vec::new(),
        }
    }
}

/// Read-side backend the §4.2 tool dispatches against. The
/// production implementation reads the canonical card from Synap,
/// hydrates evidence via per-envelope reads, walks history via the
/// `parent_event_id` chain, and queries Nexus for `:RELATED_TO`
/// edges; tests substitute an in-memory fake.
#[async_trait]
pub trait TopicCardDrill: Send + Sync {
    /// Resolve a card by its deterministic id. Returns `None` when
    /// the id is unknown.
    async fn get_card(
        &self,
        topic_card_id: &str,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError>;
    /// Hydrate the evidence list with title + `occurred_at` per
    /// item. The output stays in the same order as the input.
    async fn hydrate_evidence(
        &self,
        evidence: &[EvidenceRef],
    ) -> Result<Vec<HydratedEvidenceItem>, TopicCardMcpError>;
    /// Walk the revision chain newest-first.
    async fn history(
        &self,
        topic_card_id: &str,
    ) -> Result<Vec<TopicCardRevision>, TopicCardMcpError>;
    /// Return outgoing `:RELATED_TO` neighbours (one hop, deduped).
    async fn related(&self, topic_card_id: &str) -> Result<Vec<String>, TopicCardMcpError>;
}

/// Build the MCP descriptor for `cortex_topic_drill`.
pub fn topic_drill_descriptor() -> Value {
    json!({
        "name": TOOL_NAME_TOPIC_DRILL,
        "description": "Drill into one dimension of a topic card. \
                        `evidence` hydrates each citation with title + occurred_at; \
                        `contradictions` returns the surfaced contradictions verbatim; \
                        `history` walks the revision chain; `open_questions` returns \
                        unresolved questions; `related` returns adjacent topic-card ids.",
        "inputSchema": {
            "type": "object",
            "required": ["topic_card_id", "dimension"],
            "properties": {
                "topic_card_id": {
                    "type": "string",
                    "pattern": "^topic-[0-9a-f]{24}$",
                },
                "dimension": {
                    "type": "string",
                    "enum": [
                        "evidence",
                        "contradictions",
                        "history",
                        "open_questions",
                        "related",
                    ],
                },
            },
        },
        "outputSchema": {
            "type": "object",
            "description": "DrillResult — echoes topic_card_id + dimension; exactly one \
                            of evidence / contradictions / history / open_questions / \
                            related is populated.",
        },
    })
}

/// Dispatch logic for `cortex_topic_drill`. Looks up the card,
/// dispatches into the per-dimension lane, and emits one audit
/// envelope at the tail. A missing topic card returns
/// `TopicCardMcpError::Invalid("topic card not found: <id>")`.
pub async fn invoke_topic_drill(
    drill: Arc<dyn TopicCardDrill>,
    audit_publisher: &dyn AuditPublisher,
    caller: &str,
    topic_card_id: String,
    dimension: DrillDimension,
) -> Result<DrillResult, TopicCardMcpError> {
    let card = drill.get_card(&topic_card_id).await?.ok_or_else(|| {
        TopicCardMcpError::Invalid(format!("topic card not found: {topic_card_id}"))
    })?;

    let mut out = DrillResult::empty(topic_card_id.clone(), dimension);
    let count: usize = match dimension {
        DrillDimension::Evidence => {
            out.evidence = drill.hydrate_evidence(&card.evidence).await?;
            out.evidence.len()
        }
        DrillDimension::Contradictions => {
            out.contradictions = card.contradictions.clone();
            out.contradictions.len()
        }
        DrillDimension::History => {
            out.history = drill.history(&topic_card_id).await?;
            out.history.len()
        }
        DrillDimension::OpenQuestions => {
            out.open_questions = card.open_questions.clone();
            out.open_questions.len()
        }
        DrillDimension::Related => {
            out.related = drill.related(&topic_card_id).await?;
            out.related.len()
        }
    };

    record_topic_card_call(
        audit_publisher,
        caller,
        TOOL_NAME_TOPIC_DRILL,
        card.repos.first().map(|s| s.as_str()),
        json!({
            "topic_card_id": topic_card_id,
            "dimension": dimension.label(),
            "count": count,
        }),
    )
    .await;
    Ok(out)
}

// ---------------------------------------------------------------------------
// §4.3 — cortex_topic_neighbors
// ---------------------------------------------------------------------------

/// One node in a neighbours subgraph. Mirrors the `:TopicCard` shape
/// the §3.2 graph mapper writes — id + slug + revision so the
/// dashboard's graph view + the agent prompt can both render the
/// node without a follow-up read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NeighborNode {
    /// Deterministic topic-card id.
    pub topic_card_id: String,
    /// Human-readable slug.
    pub topic_slug: String,
    /// Latest revision the graph carries.
    pub revision: u32,
}

/// One edge in a neighbours subgraph. Edge type is one of
/// `RELATED_TO` (sibling cards) or `EVIDENCE_OF` (card → source);
/// the wire shape passes the literal Cypher type so callers can
/// branch downstream without re-encoding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NeighborEdge {
    /// `RELATED_TO` | `EVIDENCE_OF`.
    pub edge_type: String,
    /// `from` node natural key.
    pub from: String,
    /// `to` node natural key.
    pub to: String,
}

/// Subgraph the §4.3 tool returns — ≤ 64 nodes after the
/// [`TOPIC_NEIGHBORS_NODE_CAP`] clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NeighborGraph {
    /// Echoed root.
    pub topic_card_id: String,
    /// Traversal depth used (after clamping to `[1, 5]`).
    pub depth: u8,
    /// Subgraph nodes.
    pub nodes: Vec<NeighborNode>,
    /// Subgraph edges.
    pub edges: Vec<NeighborEdge>,
    /// `true` when the result was clipped at the node cap; `false`
    /// when the underlying walk returned ≤ 64 nodes.
    pub truncated: bool,
}

/// Read-side backend for §4.3. The production implementation runs
/// the Cypher
///   `MATCH (t:TopicCard {id: $id})-[:RELATED_TO|EVIDENCE_OF*1..$depth]-(n)
///    RETURN n LIMIT 64`
/// against Nexus and decodes the result into `NeighborGraph`.
#[async_trait]
pub trait TopicCardNeighbors: Send + Sync {
    /// Walk one hop in `1..=depth` from the root node. Implementers
    /// must clip at [`TOPIC_NEIGHBORS_NODE_CAP`] nodes — the
    /// dispatcher does not re-clip.
    async fn neighbors(
        &self,
        topic_card_id: &str,
        depth: u8,
    ) -> Result<NeighborGraph, TopicCardMcpError>;
}

/// Build the MCP descriptor for `cortex_topic_neighbors`.
pub fn topic_neighbors_descriptor() -> Value {
    json!({
        "name": TOOL_NAME_TOPIC_NEIGHBORS,
        "description": "Walk the topic-card subgraph one to N hops outward via \
                        `:RELATED_TO` and `:EVIDENCE_OF` edges. Returns nodes + edges \
                        clipped at 64 nodes; `truncated = true` when the cap fired.",
        "inputSchema": {
            "type": "object",
            "required": ["topic_card_id"],
            "properties": {
                "topic_card_id": {
                    "type": "string",
                    "pattern": "^topic-[0-9a-f]{24}$",
                },
                "depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "default": TOPIC_NEIGHBORS_DEFAULT_DEPTH,
                },
            },
        },
        "outputSchema": {
            "type": "object",
            "description": "NeighborGraph — nodes + edges + truncated flag.",
        },
    })
}

/// Dispatch logic for `cortex_topic_neighbors`. Clamps the depth
/// into `[1, 5]` (the upper bound matches the JSON Schema), runs
/// the walk, and emits one audit envelope.
pub async fn invoke_topic_neighbors(
    neighbors: Arc<dyn TopicCardNeighbors>,
    audit_publisher: &dyn AuditPublisher,
    caller: &str,
    topic_card_id: String,
    depth: Option<u8>,
) -> Result<NeighborGraph, TopicCardMcpError> {
    let resolved_depth = depth.unwrap_or(TOPIC_NEIGHBORS_DEFAULT_DEPTH).clamp(1, 5);
    let graph = neighbors.neighbors(&topic_card_id, resolved_depth).await?;
    record_topic_card_call(
        audit_publisher,
        caller,
        TOOL_NAME_TOPIC_NEIGHBORS,
        None,
        json!({
            "topic_card_id": topic_card_id,
            "depth": resolved_depth,
            "nodes": graph.nodes.len(),
            "edges": graph.edges.len(),
            "truncated": graph.truncated,
        }),
    )
    .await;
    Ok(graph)
}

// ---------------------------------------------------------------------------
// §4.4 — cortex_topic_diff
// ---------------------------------------------------------------------------

/// Result envelope for `cortex_topic_diff`. `from_rev` < `to_rev`,
/// always — the dispatcher rejects a `since_rev` that is ≥ the
/// current revision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicCardDiff {
    /// Echoed canonical id.
    pub topic_card_id: String,
    /// The revision the caller asked to diff against.
    pub from_rev: u32,
    /// The current revision.
    pub to_rev: u32,
    /// Unified diff over the markdown body. Empty when the bodies
    /// are identical (e.g. only metadata changed across revisions).
    pub synthesis_diff: String,
    /// Evidence items present in `to` but not in `from`, keyed on
    /// `(kind, id)`.
    pub evidence_added: Vec<EvidenceRef>,
    /// Evidence items present in `from` but not in `to`.
    pub evidence_removed: Vec<EvidenceRef>,
    /// Contradictions surfaced after `from`.
    pub contradictions_added: Vec<Contradiction>,
    /// Contradictions whose status flipped to `Reconciled` /
    /// `Deprecated` between `from` and `to`.
    pub contradictions_resolved: Vec<Contradiction>,
}

/// Read-side backend for §4.4. The production implementation walks
/// the `parent_event_id` chain to find the closest revision matching
/// `since_rev` and returns it alongside the current head.
#[async_trait]
pub trait TopicCardDiffer: Send + Sync {
    /// Fetch the (from, to) pair for a topic card. `Ok(None)` when
    /// either side is missing (e.g. the requested `since_rev` does
    /// not exist in the chain).
    async fn revision_pair(
        &self,
        topic_card_id: &str,
        since_rev: u32,
    ) -> Result<Option<(TopicCardPayload, TopicCardPayload)>, TopicCardMcpError>;
}

/// Build the MCP descriptor for `cortex_topic_diff`.
pub fn topic_diff_descriptor() -> Value {
    json!({
        "name": TOOL_NAME_TOPIC_DIFF,
        "description": "Compute the diff between revision `since_rev` and the current head \
                        of a topic card. Returns a unified-diff body for the synthesis plus \
                        set-diffs for evidence and contradictions.",
        "inputSchema": {
            "type": "object",
            "required": ["topic_card_id", "since_rev"],
            "properties": {
                "topic_card_id": {
                    "type": "string",
                    "pattern": "^topic-[0-9a-f]{24}$",
                },
                "since_rev": { "type": "integer", "minimum": 1 },
            },
        },
        "outputSchema": {
            "type": "object",
            "description": "TopicCardDiff — from_rev, to_rev, synthesis_diff (unified), \
                            evidence_added/removed, contradictions_added/resolved.",
        },
    })
}

/// Render a unified diff over `from` and `to`. Each diverging line
/// is prefixed with `- ` (removed) or `+ ` (added); a trailing
/// newline is preserved. Empty result when the inputs are byte-
/// identical. Lines-only is sufficient because the synthesis body
/// is prose markdown — the goal is to surface what changed for an
/// agent prompt, not produce IDE-grade hunks.
fn render_synthesis_diff(from: &str, to: &str) -> String {
    if from == to {
        return String::new();
    }
    let from_lines: Vec<&str> = from.lines().collect();
    let to_lines: Vec<&str> = to.lines().collect();

    // Elide common prefix + suffix so a small edit in the middle
    // of a long body doesn't read as "every line changed".
    let mut head = 0;
    while head < from_lines.len() && head < to_lines.len() && from_lines[head] == to_lines[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < from_lines.len() - head
        && tail < to_lines.len() - head
        && from_lines[from_lines.len() - 1 - tail] == to_lines[to_lines.len() - 1 - tail]
    {
        tail += 1;
    }

    let mut out = String::new();
    for line in &from_lines[head..from_lines.len() - tail] {
        out.push_str("- ");
        out.push_str(line);
        out.push('\n');
    }
    for line in &to_lines[head..to_lines.len() - tail] {
        out.push_str("+ ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Compute set-diff between two evidence lists keyed on `(kind, id)`.
/// Returns `(added, removed)`. Order is preserved from the source
/// list so the output reads predictably.
fn diff_evidence(from: &[EvidenceRef], to: &[EvidenceRef]) -> (Vec<EvidenceRef>, Vec<EvidenceRef>) {
    let key = |e: &EvidenceRef| (format!("{:?}", e.kind), e.id.clone());
    let from_keys: std::collections::BTreeSet<_> = from.iter().map(key).collect();
    let to_keys: std::collections::BTreeSet<_> = to.iter().map(key).collect();
    let added: Vec<EvidenceRef> = to
        .iter()
        .filter(|e| !from_keys.contains(&key(e)))
        .cloned()
        .collect();
    let removed: Vec<EvidenceRef> = from
        .iter()
        .filter(|e| !to_keys.contains(&key(e)))
        .cloned()
        .collect();
    (added, removed)
}

/// Diff two contradiction lists. `added` = present in `to` keyed on
/// `(kind, evidence_a, evidence_b)` but absent from `from`.
/// `resolved` = present in both with `from.status == Open` but
/// `to.status ∈ {Reconciled, Deprecated}`.
fn diff_contradictions(
    from: &[Contradiction],
    to: &[Contradiction],
) -> (Vec<Contradiction>, Vec<Contradiction>) {
    use cortex_core::events::ContradictionStatus;
    let key = |c: &Contradiction| {
        (
            format!("{:?}", c.kind),
            c.evidence_a.clone(),
            c.evidence_b.clone(),
        )
    };
    let from_by_key: std::collections::BTreeMap<_, &Contradiction> =
        from.iter().map(|c| (key(c), c)).collect();
    let to_by_key: std::collections::BTreeMap<_, &Contradiction> =
        to.iter().map(|c| (key(c), c)).collect();
    let added: Vec<Contradiction> = to
        .iter()
        .filter(|c| !from_by_key.contains_key(&key(c)))
        .cloned()
        .collect();
    let resolved: Vec<Contradiction> = to
        .iter()
        .filter(|c| {
            !matches!(c.status, ContradictionStatus::Open)
                && from_by_key
                    .get(&key(c))
                    .map(|prior| matches!(prior.status, ContradictionStatus::Open))
                    .unwrap_or(false)
        })
        .cloned()
        .collect();
    let _ = to_by_key; // referenced for symmetry; not used directly
    (added, resolved)
}

/// Dispatch logic for `cortex_topic_diff`. Fetches the (from, to)
/// pair, computes set-diffs over evidence + contradictions, renders
/// the synthesis diff, emits one audit envelope.
pub async fn invoke_topic_diff(
    differ: Arc<dyn TopicCardDiffer>,
    audit_publisher: &dyn AuditPublisher,
    caller: &str,
    topic_card_id: String,
    since_rev: u32,
) -> Result<TopicCardDiff, TopicCardMcpError> {
    let pair = differ
        .revision_pair(&topic_card_id, since_rev)
        .await?
        .ok_or_else(|| {
            TopicCardMcpError::Invalid(format!(
                "revision pair not found for {topic_card_id} since_rev={since_rev}"
            ))
        })?;
    let (from, to) = pair;
    if from.revision >= to.revision {
        return Err(TopicCardMcpError::Invalid(format!(
            "since_rev {} must be older than current revision {}",
            from.revision, to.revision
        )));
    }
    let synthesis_diff = render_synthesis_diff(&from.synthesis_markdown, &to.synthesis_markdown);
    let (evidence_added, evidence_removed) = diff_evidence(&from.evidence, &to.evidence);
    let (contradictions_added, contradictions_resolved) =
        diff_contradictions(&from.contradictions, &to.contradictions);
    let result = TopicCardDiff {
        topic_card_id: topic_card_id.clone(),
        from_rev: from.revision,
        to_rev: to.revision,
        synthesis_diff,
        evidence_added,
        evidence_removed,
        contradictions_added,
        contradictions_resolved,
    };
    record_topic_card_call(
        audit_publisher,
        caller,
        TOOL_NAME_TOPIC_DIFF,
        to.repos.first().map(|s| s.as_str()),
        json!({
            "topic_card_id": topic_card_id,
            "from_rev": result.from_rev,
            "to_rev": result.to_rev,
            "evidence_added": result.evidence_added.len(),
            "evidence_removed": result.evidence_removed.len(),
            "contradictions_added": result.contradictions_added.len(),
            "contradictions_resolved": result.contradictions_resolved.len(),
        }),
    )
    .await;
    Ok(result)
}

// ---------------------------------------------------------------------------
// §4.5 — cortex_synthesize
// ---------------------------------------------------------------------------

/// Request body for `cortex_synthesize`. The MCP runtime parses
/// this directly from the tool input; field names match the
/// `synthesize_descriptor` schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesizeRequest {
    /// Free-text query / topic seed the synthesiser should rewrite
    /// against. Drives the topic_slug derivation (the implementer
    /// kebab-cases the query when no existing card matches).
    pub query: String,
    /// Repo scope. `scope.repo` is required.
    pub scope: Scope,
    /// When `true`, escalates to the deeper (more expensive) model
    /// even if the trigger heuristics would have stayed on the
    /// shallow path. Per §2.7 — `force_deep` flips the orchestrator
    /// to Opus.
    #[serde(default)]
    pub force: bool,
    /// When `true`, the produced card is emitted as a `Kind::TopicCard`
    /// envelope (which the indexer pipeline picks up). When `false`,
    /// the result is returned to the caller without persisting —
    /// useful for previews / dry-runs. Either way the cost ledger
    /// records the burn.
    #[serde(default)]
    pub persist: bool,
}

/// Result envelope for `cortex_synthesize`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesizeResult {
    /// The produced card.
    pub topic_card: TopicCardPayload,
    /// Realised cost of the rewrite in cents (`cost_telemetry::cost_cents`
    /// convention).
    pub cost_cents: u32,
    /// `true` when an envelope was emitted.
    pub persisted: bool,
}

/// Backend the §4.5 tool dispatches against. The production
/// implementation wraps the cortex-workers `topic_cards::orchestrator`
/// surface — calling `Orchestrator::run` and (when `persist=true`)
/// emitting the resulting payload as a `Kind::TopicCard` envelope
/// through Synap. Tests substitute an in-memory fake.
#[async_trait]
pub trait TopicCardSynthesizer: Send + Sync {
    /// Run the synthesiser and (optionally) emit the envelope. The
    /// implementer is responsible for the cost-budget gate — exhausting
    /// the cap returns `Err(TopicCardMcpError::BudgetExhausted{..})`.
    async fn synthesize(
        &self,
        req: SynthesizeRequest,
    ) -> Result<SynthesizeResult, TopicCardMcpError>;
}

/// Build the MCP descriptor for `cortex_synthesize`.
pub fn synthesize_descriptor() -> Value {
    json!({
        "name": TOOL_NAME_SYNTHESIZE,
        "description": "Operator escape hatch — run the topic-card synthesiser ad-hoc \
                        through the orchestrator. `persist=true` emits a Kind::TopicCard \
                        envelope normally; `persist=false` returns the payload without \
                        indexing. Counts against the cost budget either way; refuses with \
                        BudgetExhausted when over cap.",
        "inputSchema": {
            "type": "object",
            "required": ["query", "scope"],
            "properties": {
                "query": { "type": "string", "minLength": 1 },
                "scope": {
                    "type": "object",
                    "required": ["repo"],
                    "properties": { "repo": { "type": "string" } },
                },
                "force": { "type": "boolean", "default": false },
                "persist": { "type": "boolean", "default": false },
            },
        },
        "outputSchema": {
            "type": "object",
            "description": "SynthesizeResult — topic_card + cost_cents + persisted flag.",
        },
    })
}

/// Dispatch logic for `cortex_synthesize`. Validates `scope.repo`,
/// hands the request to the backend, emits one audit envelope.
pub async fn invoke_synthesize(
    synthesizer: Arc<dyn TopicCardSynthesizer>,
    audit_publisher: &dyn AuditPublisher,
    caller: &str,
    req: SynthesizeRequest,
) -> Result<SynthesizeResult, TopicCardMcpError> {
    let scope_repo = req.scope.repo.clone().filter(|s| !s.is_empty());
    if scope_repo.is_none() {
        record_topic_card_call(
            audit_publisher,
            caller,
            TOOL_NAME_SYNTHESIZE,
            None,
            json!({ "error": "scope_repo_required" }),
        )
        .await;
        return Err(TopicCardMcpError::ScopeRepoRequired);
    }
    if req.query.trim().is_empty() {
        record_topic_card_call(
            audit_publisher,
            caller,
            TOOL_NAME_SYNTHESIZE,
            scope_repo.as_deref(),
            json!({ "error": "empty_query" }),
        )
        .await;
        return Err(TopicCardMcpError::Invalid("query is empty".to_string()));
    }
    let force = req.force;
    let persist = req.persist;
    match synthesizer.synthesize(req).await {
        Ok(result) => {
            record_topic_card_call(
                audit_publisher,
                caller,
                TOOL_NAME_SYNTHESIZE,
                scope_repo.as_deref(),
                json!({
                    "topic_card_id": result.topic_card.topic_card_id,
                    "topic_slug": result.topic_card.topic_slug,
                    "revision": result.topic_card.revision,
                    "cost_cents": result.cost_cents,
                    "persisted": result.persisted,
                    "force": force,
                    "persist": persist,
                }),
            )
            .await;
            Ok(result)
        }
        Err(TopicCardMcpError::BudgetExhausted {
            used_cents,
            cap_cents,
        }) => {
            record_topic_card_call(
                audit_publisher,
                caller,
                TOOL_NAME_SYNTHESIZE,
                scope_repo.as_deref(),
                json!({
                    "error": "budget_exhausted",
                    "used_cents": used_cents,
                    "cap_cents": cap_cents,
                }),
            )
            .await;
            Err(TopicCardMcpError::BudgetExhausted {
                used_cents,
                cap_cents,
            })
        }
        Err(other) => {
            record_topic_card_call(
                audit_publisher,
                caller,
                TOOL_NAME_SYNTHESIZE,
                scope_repo.as_deref(),
                json!({ "error": format!("{other}") }),
            )
            .await;
            Err(other)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audit::MemoryAuditPublisher;
    use cortex_core::events::derive_topic_card_id;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// In-memory `TopicCardLookup` for unit tests. Holds two maps —
    /// one for slug-exact lookups, one for the search lane — so
    /// each path can be exercised independently.
    #[derive(Default)]
    struct FakeLookup {
        by_slug: Mutex<BTreeMap<String, TopicCardPayload>>,
        search_hit: Mutex<Option<TopicCardPayload>>,
        get_calls: Mutex<Vec<String>>,
        search_calls: Mutex<Vec<String>>,
    }

    impl FakeLookup {
        fn with_slug(slug: &str, repo: &str, confidence: f32) -> Self {
            let me = Self::default();
            me.by_slug
                .lock()
                .unwrap()
                .insert(slug.to_string(), card(slug, repo, confidence, 1));
            me
        }
    }

    #[async_trait]
    impl TopicCardLookup for FakeLookup {
        async fn get_by_slug(
            &self,
            slug: &str,
            _scope: &Scope,
        ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
            self.get_calls.lock().unwrap().push(slug.to_string());
            Ok(self.by_slug.lock().unwrap().get(slug).cloned())
        }
        async fn search(
            &self,
            query: &str,
            _scope: &Scope,
        ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
            self.search_calls.lock().unwrap().push(query.to_string());
            Ok(self.search_hit.lock().unwrap().clone())
        }
    }

    fn card(slug: &str, repo: &str, confidence: f32, revision: u32) -> TopicCardPayload {
        TopicCardPayload {
            topic_card_id: derive_topic_card_id(slug, repo),
            topic_slug: slug.to_string(),
            repos: vec![repo.to_string()],
            revision,
            synthesis_markdown:
                "Synthesis body that exceeds the 200-byte minimum so the validator does not \
                trip if this payload is ever round-tripped through the JSON Schema. The body \
                only needs to read sensibly for the test reader; the bytes here are filler."
                    .to_string(),
            evidence: Vec::new(),
            contradictions: Vec::new(),
            open_questions: Vec::new(),
            related_topic_ids: Vec::new(),
            confidence,
            last_rev_at: "2026-05-03T12:00:00Z".to_string(),
            events_since_last_rev: 0,
            synthesis_model: "claude-haiku-4-5".to_string(),
            synthesis_cost_cents: 80,
        }
    }

    fn scope(repo: &str) -> Scope {
        Scope {
            repo: Some(repo.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn is_valid_topic_slug_accepts_kebab_case() {
        assert!(is_valid_topic_slug("auth-rewrite"));
        assert!(is_valid_topic_slug("a"));
        assert!(is_valid_topic_slug("a1"));
        assert!(is_valid_topic_slug("phase11r-topic-cards"));
    }

    #[test]
    fn is_valid_topic_slug_rejects_uppercase_underscores_and_edge_dashes() {
        assert!(!is_valid_topic_slug(""));
        assert!(!is_valid_topic_slug("Auth-Rewrite"));
        assert!(!is_valid_topic_slug("auth_rewrite"));
        assert!(!is_valid_topic_slug("-auth"));
        assert!(!is_valid_topic_slug("auth-"));
        assert!(!is_valid_topic_slug("how does auth work"));
        // 81 chars — one over the schema's 80-char ceiling.
        assert!(!is_valid_topic_slug(&"a".repeat(81)));
    }

    #[tokio::test]
    async fn invoke_topic_get_slug_exact_short_circuits_to_get_by_slug() {
        let fake = Arc::new(FakeLookup::with_slug("auth-rewrite", "cortex", 0.5));
        let audit = MemoryAuditPublisher::new();
        let result = invoke_topic_get(
            fake.clone(),
            &audit,
            "claude-code",
            scope("cortex"),
            "auth-rewrite".into(),
        )
        .await
        .expect("happy path");
        let card = result.expect("slug-exact returns the card verbatim");
        // The confidence floor (≥ 0.6) is bypassed on the slug-exact
        // lane — the caller named the card directly, so even a
        // weak-confidence card is returned. Spec §4.1 contract.
        assert_eq!(card.topic_slug, "auth-rewrite");
        assert!(card.confidence < TOPIC_GET_CONFIDENCE_FLOOR);
        // Search must not have been called when the slug-exact path
        // resolved.
        assert_eq!(fake.search_calls.lock().unwrap().len(), 0);
        // One audit envelope must land per call, with `via=slug_exact`.
        let envs = audit.snapshot();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0]["tool"], TOOL_NAME_TOPIC_GET);
        assert_eq!(envs[0]["result"]["via"], "slug_exact");
        assert_eq!(envs[0]["result"]["hit"], "auth-rewrite");
    }

    #[tokio::test]
    async fn invoke_topic_get_query_path_filters_low_confidence_hits() {
        // A weak top-1 hit (confidence < floor) returns None even
        // when the search backend produced a candidate — preserves
        // the agent prompt's signal-to-noise ratio.
        let fake = Arc::new(FakeLookup::default());
        *fake.search_hit.lock().unwrap() = Some(card("auth-rewrite", "cortex", 0.55, 1));
        let audit = MemoryAuditPublisher::new();
        let result = invoke_topic_get(
            fake.clone(),
            &audit,
            "claude-code",
            scope("cortex"),
            "how does auth work".into(),
        )
        .await
        .expect("happy path");
        assert!(result.is_none(), "below-floor hit must be dropped");
        // The query was free-text (not a valid slug), so the slug
        // lane was bypassed.
        assert_eq!(fake.get_calls.lock().unwrap().len(), 0);
        assert_eq!(fake.search_calls.lock().unwrap().len(), 1);
        let envs = audit.snapshot();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0]["result"]["hit"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn invoke_topic_get_query_path_returns_strong_hit() {
        let fake = Arc::new(FakeLookup::default());
        *fake.search_hit.lock().unwrap() = Some(card("auth-rewrite", "cortex", 0.82, 3));
        let audit = MemoryAuditPublisher::new();
        let result = invoke_topic_get(
            fake.clone(),
            &audit,
            "claude-code",
            scope("cortex"),
            "how does auth work".into(),
        )
        .await
        .expect("happy path");
        let card = result.expect("above-floor hit returned");
        assert_eq!(card.topic_slug, "auth-rewrite");
        assert_eq!(card.revision, 3);
        let envs = audit.snapshot();
        assert_eq!(envs[0]["result"]["via"], "search");
        // f32 → f64 widening picks up trailing ULPs (0.82_f32 ≈
        // 0.81999999) so an exact equality on the JSON number is
        // brittle. The contract worth pinning is "the value the
        // card carries lands in the envelope" — re-cast through
        // f32 and compare against the source confidence.
        let recorded = envs[0]["result"]["confidence"]
            .as_f64()
            .expect("confidence is a number");
        assert!((recorded as f32 - 0.82_f32).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn invoke_topic_get_requires_scope_repo() {
        let fake = Arc::new(FakeLookup::default());
        let audit = MemoryAuditPublisher::new();
        let empty = Scope {
            repo: None,
            ..Scope::default()
        };
        let err = invoke_topic_get(
            fake.clone(),
            &audit,
            "claude-code",
            empty,
            "auth-rewrite".into(),
        )
        .await
        .expect_err("must reject empty scope.repo");
        assert!(matches!(err, TopicCardMcpError::ScopeRepoRequired));
        // Neither lookup lane was called — failure surfaces before
        // the dispatcher decides slug vs query.
        assert_eq!(fake.get_calls.lock().unwrap().len(), 0);
        assert_eq!(fake.search_calls.lock().unwrap().len(), 0);

        // Empty-string repo is treated the same as None — the spec
        // requires a real repo identifier.
        let blank = Scope {
            repo: Some(String::new()),
            ..Default::default()
        };
        let err = invoke_topic_get(
            fake.clone(),
            &audit,
            "claude-code",
            blank,
            "auth-rewrite".into(),
        )
        .await
        .expect_err("must reject empty-string scope.repo");
        assert!(matches!(err, TopicCardMcpError::ScopeRepoRequired));
        // Both rejections still emitted an audit envelope so the
        // dashboard surfaces malformed callers — the dashboard's
        // misconfig detection in phase6a relies on every rejection
        // being recorded.
        let envs = audit.snapshot();
        assert_eq!(envs.len(), 2);
        for env in &envs {
            assert_eq!(env["result"]["error"], "scope_repo_required");
        }
    }

    // -----------------------------------------------------------------
    // §4.2 — cortex_topic_drill
    // -----------------------------------------------------------------

    #[derive(Default)]
    struct FakeDrill {
        cards: Mutex<BTreeMap<String, TopicCardPayload>>,
        evidence_titles: Mutex<BTreeMap<String, (String, String)>>, // id -> (title, occurred_at)
        history: Mutex<BTreeMap<String, Vec<TopicCardRevision>>>,
        related: Mutex<BTreeMap<String, Vec<String>>>,
    }

    #[async_trait]
    impl TopicCardDrill for FakeDrill {
        async fn get_card(
            &self,
            topic_card_id: &str,
        ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
            Ok(self.cards.lock().unwrap().get(topic_card_id).cloned())
        }
        async fn hydrate_evidence(
            &self,
            evidence: &[EvidenceRef],
        ) -> Result<Vec<HydratedEvidenceItem>, TopicCardMcpError> {
            let titles = self.evidence_titles.lock().unwrap();
            Ok(evidence
                .iter()
                .map(|e| {
                    let (title, occurred_at) = titles
                        .get(&e.id)
                        .cloned()
                        .unwrap_or_else(|| (String::new(), String::new()));
                    HydratedEvidenceItem {
                        kind: e.kind,
                        id: e.id.clone(),
                        title,
                        occurred_at,
                        cited_at_rev: e.cited_at_rev,
                        weight: e.weight,
                    }
                })
                .collect())
        }
        async fn history(
            &self,
            topic_card_id: &str,
        ) -> Result<Vec<TopicCardRevision>, TopicCardMcpError> {
            Ok(self
                .history
                .lock()
                .unwrap()
                .get(topic_card_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn related(&self, topic_card_id: &str) -> Result<Vec<String>, TopicCardMcpError> {
            Ok(self
                .related
                .lock()
                .unwrap()
                .get(topic_card_id)
                .cloned()
                .unwrap_or_default())
        }
    }

    fn evidence(kind: EvidenceKind, id: &str, rev: u32) -> EvidenceRef {
        EvidenceRef {
            kind,
            id: id.to_string(),
            weight: None,
            cited_at_rev: rev,
        }
    }

    fn populated_card() -> TopicCardPayload {
        let mut c = card("auth-rewrite", "cortex", 0.82, 3);
        c.evidence = vec![
            evidence(EvidenceKind::Decision, "DEC-0042", 3),
            evidence(EvidenceKind::Law, "LAW-CORTEX-001", 2),
        ];
        c.contradictions = vec![Contradiction {
            kind: cortex_core::events::ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-0042".to_string(),
            evidence_b: "DEC-0001".to_string(),
            surfaced_at_rev: 3,
            status: cortex_core::events::ContradictionStatus::Open,
        }];
        c.open_questions = vec![
            "Should rotation TTL be configurable?".to_string(),
            "How does the gateway handle replays?".to_string(),
        ];
        c.related_topic_ids = vec!["topic-".to_string() + &"a".repeat(24)];
        c
    }

    #[tokio::test]
    async fn invoke_topic_drill_evidence_hydrates_each_item() {
        let drill = Arc::new(FakeDrill::default());
        let card = populated_card();
        let card_id = card.topic_card_id.clone();
        drill.cards.lock().unwrap().insert(card_id.clone(), card);
        drill.evidence_titles.lock().unwrap().insert(
            "DEC-0042".to_string(),
            (
                "Adopt JWT rotation".to_string(),
                "2026-04-01T09:00:00Z".to_string(),
            ),
        );
        // LAW-CORTEX-001 is intentionally absent from the title map
        // — defensive: the hydrator must surface an empty title /
        // occurred_at rather than fail the whole drill.
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            card_id.clone(),
            DrillDimension::Evidence,
        )
        .await
        .expect("drill ok");
        assert_eq!(out.dimension, DrillDimension::Evidence);
        assert_eq!(out.evidence.len(), 2);
        assert_eq!(out.evidence[0].id, "DEC-0042");
        assert_eq!(out.evidence[0].title, "Adopt JWT rotation");
        assert_eq!(out.evidence[1].id, "LAW-CORTEX-001");
        assert_eq!(out.evidence[1].title, ""); // missing source
                                               // Audit envelope carries the count.
        let envs = audit.snapshot();
        assert_eq!(envs[0]["tool"], TOOL_NAME_TOPIC_DRILL);
        assert_eq!(envs[0]["result"]["dimension"], "evidence");
        assert_eq!(envs[0]["result"]["count"], 2);
    }

    #[tokio::test]
    async fn invoke_topic_drill_contradictions_returns_card_field_verbatim() {
        let drill = Arc::new(FakeDrill::default());
        let card = populated_card();
        let card_id = card.topic_card_id.clone();
        drill.cards.lock().unwrap().insert(card_id.clone(), card);
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            card_id,
            DrillDimension::Contradictions,
        )
        .await
        .expect("drill ok");
        assert_eq!(out.contradictions.len(), 1);
        assert_eq!(out.contradictions[0].evidence_a, "DEC-0042");
        // Other lanes stay empty so the wire shape doesn't leak
        // through `skip_serializing_if`.
        assert!(out.evidence.is_empty());
        assert!(out.history.is_empty());
    }

    #[tokio::test]
    async fn invoke_topic_drill_history_walks_revision_chain() {
        let drill = Arc::new(FakeDrill::default());
        let card = populated_card();
        let card_id = card.topic_card_id.clone();
        drill.cards.lock().unwrap().insert(card_id.clone(), card);
        drill.history.lock().unwrap().insert(
            card_id.clone(),
            vec![
                TopicCardRevision {
                    topic_card_id: card_id.clone(),
                    revision: 3,
                    last_rev_at: "2026-05-03T12:00:00Z".to_string(),
                    synthesis_diff_hash: "deadbeef".repeat(8),
                },
                TopicCardRevision {
                    topic_card_id: card_id.clone(),
                    revision: 2,
                    last_rev_at: "2026-04-20T09:00:00Z".to_string(),
                    synthesis_diff_hash: "cafef00d".repeat(8),
                },
            ],
        );
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            card_id,
            DrillDimension::History,
        )
        .await
        .expect("drill ok");
        assert_eq!(out.history.len(), 2);
        // Newest revision first — pin the order so a future
        // implementation does not silently flip it.
        assert_eq!(out.history[0].revision, 3);
        assert_eq!(out.history[1].revision, 2);
    }

    #[tokio::test]
    async fn invoke_topic_drill_open_questions_returns_card_field_verbatim() {
        let drill = Arc::new(FakeDrill::default());
        let card = populated_card();
        let card_id = card.topic_card_id.clone();
        drill.cards.lock().unwrap().insert(card_id.clone(), card);
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            card_id,
            DrillDimension::OpenQuestions,
        )
        .await
        .expect("drill ok");
        assert_eq!(out.open_questions.len(), 2);
        assert!(out.open_questions[0].contains("rotation TTL"));
    }

    #[tokio::test]
    async fn invoke_topic_drill_related_returns_neighbour_ids() {
        let drill = Arc::new(FakeDrill::default());
        let card = populated_card();
        let card_id = card.topic_card_id.clone();
        drill.cards.lock().unwrap().insert(card_id.clone(), card);
        drill.related.lock().unwrap().insert(
            card_id.clone(),
            vec![
                "topic-".to_string() + &"a".repeat(24),
                "topic-".to_string() + &"b".repeat(24),
            ],
        );
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            card_id,
            DrillDimension::Related,
        )
        .await
        .expect("drill ok");
        assert_eq!(out.related.len(), 2);
    }

    #[tokio::test]
    async fn invoke_topic_drill_returns_invalid_when_card_missing() {
        let drill = Arc::new(FakeDrill::default());
        let audit = MemoryAuditPublisher::new();
        let err = invoke_topic_drill(
            drill,
            &audit,
            "claude-code",
            "topic-".to_string() + &"0".repeat(24),
            DrillDimension::Evidence,
        )
        .await
        .expect_err("missing card must error");
        match err {
            TopicCardMcpError::Invalid(msg) => {
                assert!(msg.contains("not found"), "msg: {msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §4.3 — cortex_topic_neighbors
    // -----------------------------------------------------------------

    /// In-memory `TopicCardNeighbors` fake. Records the depth the
    /// dispatcher sent through (so the unit tests can pin the
    /// clamping contract) and returns whatever subgraph the test
    /// pre-loaded under the requested root.
    #[derive(Default)]
    struct FakeNeighbors {
        graphs: Mutex<BTreeMap<String, NeighborGraph>>,
        last_depth: Mutex<Option<u8>>,
    }

    #[async_trait]
    impl TopicCardNeighbors for FakeNeighbors {
        async fn neighbors(
            &self,
            topic_card_id: &str,
            depth: u8,
        ) -> Result<NeighborGraph, TopicCardMcpError> {
            *self.last_depth.lock().unwrap() = Some(depth);
            Ok(self
                .graphs
                .lock()
                .unwrap()
                .get(topic_card_id)
                .cloned()
                .unwrap_or_else(|| NeighborGraph {
                    topic_card_id: topic_card_id.to_string(),
                    depth,
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    truncated: false,
                }))
        }
    }

    fn neighbor_node(idx: usize) -> NeighborNode {
        NeighborNode {
            topic_card_id: format!("topic-{:024x}", idx),
            topic_slug: format!("slug-{idx}"),
            revision: 1,
        }
    }

    #[tokio::test]
    async fn invoke_topic_neighbors_default_depth_is_2() {
        // Phase11r §4.3 — when the caller omits `depth`, the
        // dispatcher uses the default constant. Pin the contract
        // so a future bump surfaces here.
        let neighbors = Arc::new(FakeNeighbors::default());
        let audit = MemoryAuditPublisher::new();
        let root = "topic-".to_string() + &"a".repeat(24);
        let _ = invoke_topic_neighbors(neighbors.clone(), &audit, "claude-code", root, None)
            .await
            .expect("neighbours ok");
        assert_eq!(
            neighbors.last_depth.lock().unwrap().unwrap(),
            TOPIC_NEIGHBORS_DEFAULT_DEPTH
        );
        assert_eq!(TOPIC_NEIGHBORS_DEFAULT_DEPTH, 2);
    }

    #[tokio::test]
    async fn invoke_topic_neighbors_returns_nodes_and_edges_at_depth_1() {
        let neighbors = Arc::new(FakeNeighbors::default());
        let root = "topic-".to_string() + &"a".repeat(24);
        let neighbour_id = "topic-".to_string() + &"b".repeat(24);
        let graph = NeighborGraph {
            topic_card_id: root.clone(),
            depth: 1,
            nodes: vec![
                NeighborNode {
                    topic_card_id: root.clone(),
                    topic_slug: "auth-rewrite".to_string(),
                    revision: 3,
                },
                NeighborNode {
                    topic_card_id: neighbour_id.clone(),
                    topic_slug: "session-store".to_string(),
                    revision: 1,
                },
            ],
            edges: vec![NeighborEdge {
                edge_type: "RELATED_TO".to_string(),
                from: root.clone(),
                to: neighbour_id.clone(),
            }],
            truncated: false,
        };
        neighbors
            .graphs
            .lock()
            .unwrap()
            .insert(root.clone(), graph.clone());
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_neighbors(neighbors, &audit, "claude-code", root, Some(1))
            .await
            .expect("neighbours ok");
        assert_eq!(out.nodes.len(), 2);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].edge_type, "RELATED_TO");
        let envs = audit.snapshot();
        assert_eq!(envs[0]["result"]["nodes"], 2);
        assert_eq!(envs[0]["result"]["edges"], 1);
        assert_eq!(envs[0]["result"]["depth"], 1);
        assert_eq!(envs[0]["result"]["truncated"], false);
    }

    #[tokio::test]
    async fn invoke_topic_neighbors_clip_at_64_nodes_sets_truncated_flag() {
        // Phase11r §4.3 — backends are expected to clip at the
        // node cap. The dispatcher trusts the flag; we pin the
        // contract so a backend that returns 65+ nodes without
        // setting `truncated` surfaces as a unit-test break.
        let root = "topic-".to_string() + &"a".repeat(24);
        let nodes: Vec<NeighborNode> = (0..TOPIC_NEIGHBORS_NODE_CAP).map(neighbor_node).collect();
        let graph = NeighborGraph {
            topic_card_id: root.clone(),
            depth: 2,
            nodes,
            edges: Vec::new(),
            truncated: true,
        };
        let neighbors = Arc::new(FakeNeighbors::default());
        neighbors.graphs.lock().unwrap().insert(root.clone(), graph);
        let audit = MemoryAuditPublisher::new();
        let out = invoke_topic_neighbors(neighbors, &audit, "claude-code", root, Some(2))
            .await
            .expect("neighbours ok");
        assert_eq!(out.nodes.len(), TOPIC_NEIGHBORS_NODE_CAP);
        assert!(out.truncated);
        assert_eq!(audit.snapshot()[0]["result"]["truncated"], true);
    }

    // -----------------------------------------------------------------
    // §4.4 — cortex_topic_diff
    // -----------------------------------------------------------------

    #[derive(Default)]
    struct FakeDiffer {
        pairs: Mutex<BTreeMap<(String, u32), (TopicCardPayload, TopicCardPayload)>>,
    }

    #[async_trait]
    impl TopicCardDiffer for FakeDiffer {
        async fn revision_pair(
            &self,
            topic_card_id: &str,
            since_rev: u32,
        ) -> Result<Option<(TopicCardPayload, TopicCardPayload)>, TopicCardMcpError> {
            Ok(self
                .pairs
                .lock()
                .unwrap()
                .get(&(topic_card_id.to_string(), since_rev))
                .cloned())
        }
    }

    #[test]
    fn render_synthesis_diff_elides_common_prefix_and_suffix() {
        let from = "Intro\nMiddle old\nOutro";
        let to = "Intro\nMiddle new\nOutro";
        let diff = render_synthesis_diff(from, to);
        // Only the changed middle line is surfaced; intro + outro
        // get elided so the diff reads cleanly in the agent prompt.
        assert!(diff.contains("- Middle old"));
        assert!(diff.contains("+ Middle new"));
        assert!(!diff.contains("Intro"));
        assert!(!diff.contains("Outro"));
    }

    #[test]
    fn render_synthesis_diff_returns_empty_when_bodies_equal() {
        let body = "abc\ndef";
        assert_eq!(render_synthesis_diff(body, body), "");
    }

    #[tokio::test]
    async fn invoke_topic_diff_set_diffs_evidence_and_contradictions() {
        let differ = Arc::new(FakeDiffer::default());
        let card_id = derive_topic_card_id("auth-rewrite", "cortex");

        let mut from = card("auth-rewrite", "cortex", 0.7, 1);
        from.synthesis_markdown = "Old synthesis body".repeat(20); // ≥ 200 bytes
        from.evidence = vec![evidence(EvidenceKind::Decision, "DEC-0001", 1)];
        from.contradictions = vec![Contradiction {
            kind: cortex_core::events::ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-0001".to_string(),
            evidence_b: "DEC-0000".to_string(),
            surfaced_at_rev: 1,
            status: cortex_core::events::ContradictionStatus::Open,
        }];

        let mut to = card("auth-rewrite", "cortex", 0.85, 3);
        to.synthesis_markdown = "New synthesis body".repeat(20);
        to.evidence = vec![
            evidence(EvidenceKind::Decision, "DEC-0001", 3),
            evidence(EvidenceKind::Law, "LAW-CORTEX-001", 3),
        ];
        to.contradictions = vec![
            // Same contradiction, now reconciled — diff_resolved.
            Contradiction {
                kind: cortex_core::events::ContradictionKind::DecisionSupersession,
                evidence_a: "DEC-0001".to_string(),
                evidence_b: "DEC-0000".to_string(),
                surfaced_at_rev: 1,
                status: cortex_core::events::ContradictionStatus::Reconciled,
            },
            // Brand new contradiction — diff_added.
            Contradiction {
                kind: cortex_core::events::ContradictionKind::OutcomeDivergence,
                evidence_a: "DEC-0001".to_string(),
                evidence_b: "LAW-CORTEX-001".to_string(),
                surfaced_at_rev: 3,
                status: cortex_core::events::ContradictionStatus::Open,
            },
        ];

        differ
            .pairs
            .lock()
            .unwrap()
            .insert((card_id.clone(), 1), (from, to));

        let audit = MemoryAuditPublisher::new();
        let diff = invoke_topic_diff(differ, &audit, "claude-code", card_id.clone(), 1)
            .await
            .expect("diff ok");
        assert_eq!(diff.from_rev, 1);
        assert_eq!(diff.to_rev, 3);
        assert_eq!(diff.evidence_added.len(), 1);
        assert_eq!(diff.evidence_added[0].id, "LAW-CORTEX-001");
        assert!(diff.evidence_removed.is_empty());
        assert_eq!(diff.contradictions_added.len(), 1);
        assert_eq!(
            diff.contradictions_added[0].kind,
            cortex_core::events::ContradictionKind::OutcomeDivergence
        );
        assert_eq!(diff.contradictions_resolved.len(), 1);
        assert!(matches!(
            diff.contradictions_resolved[0].status,
            cortex_core::events::ContradictionStatus::Reconciled
        ));
        assert!(!diff.synthesis_diff.is_empty());

        let envs = audit.snapshot();
        assert_eq!(envs[0]["tool"], TOOL_NAME_TOPIC_DIFF);
        assert_eq!(envs[0]["result"]["evidence_added"], 1);
        assert_eq!(envs[0]["result"]["contradictions_resolved"], 1);
    }

    #[tokio::test]
    async fn invoke_topic_diff_rejects_since_rev_at_or_above_current() {
        // since_rev=3 against a head at rev 3 must reject — diffing
        // a card against itself is meaningless and would produce
        // misleading empty results.
        let differ = Arc::new(FakeDiffer::default());
        let card_id = derive_topic_card_id("auth-rewrite", "cortex");
        let from = card("auth-rewrite", "cortex", 0.7, 3);
        let to = card("auth-rewrite", "cortex", 0.7, 3);
        differ
            .pairs
            .lock()
            .unwrap()
            .insert((card_id.clone(), 3), (from, to));
        let audit = MemoryAuditPublisher::new();
        let err = invoke_topic_diff(differ, &audit, "claude-code", card_id, 3)
            .await
            .expect_err("must reject");
        match err {
            TopicCardMcpError::Invalid(msg) => assert!(msg.contains("must be older")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §4.5 — cortex_synthesize
    // -----------------------------------------------------------------

    #[derive(Default)]
    struct FakeSynthesizer {
        budget_used: Mutex<u32>,
        budget_cap: Mutex<u32>,
        last_request: Mutex<Option<SynthesizeRequest>>,
    }

    impl FakeSynthesizer {
        fn under_budget() -> Self {
            let me = Self::default();
            *me.budget_cap.lock().unwrap() = 10_000;
            *me.budget_used.lock().unwrap() = 100;
            me
        }
        fn at_budget() -> Self {
            let me = Self::default();
            *me.budget_cap.lock().unwrap() = 1_000;
            *me.budget_used.lock().unwrap() = 1_000;
            me
        }
    }

    #[async_trait]
    impl TopicCardSynthesizer for FakeSynthesizer {
        async fn synthesize(
            &self,
            req: SynthesizeRequest,
        ) -> Result<SynthesizeResult, TopicCardMcpError> {
            let used = *self.budget_used.lock().unwrap();
            let cap = *self.budget_cap.lock().unwrap();
            if used >= cap {
                return Err(TopicCardMcpError::BudgetExhausted {
                    used_cents: used,
                    cap_cents: cap,
                });
            }
            *self.last_request.lock().unwrap() = Some(req.clone());
            let mut produced = card("auth-rewrite", "cortex", 0.78, 1);
            produced.synthesis_markdown =
                "Synthesised body that easily exceeds the schema's 200-byte floor; padded \
                with filler so the validator does not trip in tests."
                    .to_string();
            Ok(SynthesizeResult {
                topic_card: produced,
                cost_cents: if req.force { 4_000 } else { 100 },
                persisted: req.persist,
            })
        }
    }

    fn synth_req(query: &str, force: bool, persist: bool) -> SynthesizeRequest {
        SynthesizeRequest {
            query: query.to_string(),
            scope: scope("cortex"),
            force,
            persist,
        }
    }

    #[tokio::test]
    async fn invoke_synthesize_persist_false_returns_payload_without_emitting() {
        let synth = Arc::new(FakeSynthesizer::under_budget());
        let audit = MemoryAuditPublisher::new();
        let out = invoke_synthesize(
            synth.clone(),
            &audit,
            "claude-code",
            synth_req("auth rewrite", false, false),
        )
        .await
        .expect("synth ok");
        assert!(!out.persisted);
        assert_eq!(out.cost_cents, 100);
        let envs = audit.snapshot();
        assert_eq!(envs[0]["tool"], TOOL_NAME_SYNTHESIZE);
        assert_eq!(envs[0]["result"]["persisted"], false);
        assert_eq!(envs[0]["result"]["force"], false);
    }

    #[tokio::test]
    async fn invoke_synthesize_persist_true_marks_envelope_emitted() {
        let synth = Arc::new(FakeSynthesizer::under_budget());
        let audit = MemoryAuditPublisher::new();
        let out = invoke_synthesize(
            synth,
            &audit,
            "claude-code",
            synth_req("auth rewrite", false, true),
        )
        .await
        .expect("synth ok");
        assert!(out.persisted);
        let envs = audit.snapshot();
        assert_eq!(envs[0]["result"]["persisted"], true);
    }

    #[tokio::test]
    async fn invoke_synthesize_force_flag_passes_through() {
        let synth = Arc::new(FakeSynthesizer::under_budget());
        let audit = MemoryAuditPublisher::new();
        let out = invoke_synthesize(
            synth.clone(),
            &audit,
            "claude-code",
            synth_req("auth rewrite", true, false),
        )
        .await
        .expect("synth ok");
        // The fake's branching on `req.force` is the contract that
        // the dispatcher passes the flag through unchanged. A
        // `force=true` rewrite costs 4_000 cents in this fake;
        // `force=false` costs 100.
        assert_eq!(out.cost_cents, 4_000);
        let last = synth.last_request.lock().unwrap().clone().unwrap();
        assert!(last.force);
        assert_eq!(audit.snapshot()[0]["result"]["force"], true);
    }

    #[tokio::test]
    async fn invoke_synthesize_returns_budget_exhausted_when_over_cap() {
        let synth = Arc::new(FakeSynthesizer::at_budget());
        let audit = MemoryAuditPublisher::new();
        let err = invoke_synthesize(
            synth,
            &audit,
            "claude-code",
            synth_req("auth rewrite", false, false),
        )
        .await
        .expect_err("over-cap must reject");
        match err {
            TopicCardMcpError::BudgetExhausted {
                used_cents,
                cap_cents,
            } => {
                assert_eq!(used_cents, 1_000);
                assert_eq!(cap_cents, 1_000);
            }
            other => panic!("expected BudgetExhausted, got {other:?}"),
        }
        // The audit envelope still lands so the dashboard surfaces
        // the rejection.
        let envs = audit.snapshot();
        assert_eq!(envs[0]["result"]["error"], "budget_exhausted");
        assert_eq!(envs[0]["result"]["used_cents"], 1_000);
        assert_eq!(envs[0]["result"]["cap_cents"], 1_000);
    }

    #[tokio::test]
    async fn invoke_synthesize_requires_scope_repo_and_non_empty_query() {
        let synth = Arc::new(FakeSynthesizer::under_budget());
        let audit = MemoryAuditPublisher::new();

        // Empty scope.repo
        let err = invoke_synthesize(
            synth.clone(),
            &audit,
            "claude-code",
            SynthesizeRequest {
                query: "auth".to_string(),
                scope: Scope::default(),
                force: false,
                persist: false,
            },
        )
        .await
        .expect_err("must reject");
        assert!(matches!(err, TopicCardMcpError::ScopeRepoRequired));

        // Empty query
        let err = invoke_synthesize(synth, &audit, "claude-code", synth_req("   ", false, false))
            .await
            .expect_err("must reject");
        assert!(matches!(err, TopicCardMcpError::Invalid(_)));

        let envs = audit.snapshot();
        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0]["result"]["error"], "scope_repo_required");
        assert_eq!(envs[1]["result"]["error"], "empty_query");
    }

    #[tokio::test]
    async fn invoke_topic_diff_returns_invalid_when_pair_missing() {
        let differ = Arc::new(FakeDiffer::default());
        let audit = MemoryAuditPublisher::new();
        let err = invoke_topic_diff(
            differ,
            &audit,
            "claude-code",
            "topic-".to_string() + &"0".repeat(24),
            5,
        )
        .await
        .expect_err("must reject");
        assert!(matches!(err, TopicCardMcpError::Invalid(_)));
    }

    #[tokio::test]
    async fn invoke_topic_neighbors_clamps_out_of_range_depth() {
        // Depth=0 and depth=99 both clamp into [1, 5]. The JSON
        // Schema rejects them upstream when the runtime validates,
        // but the dispatcher's clamp is the load-bearing safety
        // net for callers that bypass schema validation.
        let neighbors = Arc::new(FakeNeighbors::default());
        let audit = MemoryAuditPublisher::new();
        let root = "topic-".to_string() + &"a".repeat(24);

        let _ = invoke_topic_neighbors(neighbors.clone(), &audit, "x", root.clone(), Some(0))
            .await
            .unwrap();
        assert_eq!(neighbors.last_depth.lock().unwrap().unwrap(), 1);

        let _ = invoke_topic_neighbors(neighbors.clone(), &audit, "x", root, Some(99))
            .await
            .unwrap();
        assert_eq!(neighbors.last_depth.lock().unwrap().unwrap(), 5);
    }
}
