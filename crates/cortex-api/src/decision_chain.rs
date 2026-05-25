//! phase13g §3 — `cortex_decision_chain` ADR supersession walker.
//!
//! Walks `:SUPERSEDES` edges in Nexus from a starting `Decision`
//! node and returns the chronological chain (predecessors +
//! successors merged by `date`). Cycle-detection: a node that
//! already appears in the result set stops that branch.
//!
//! ADR-014 pure-reader contract: the handler returns
//! [`DecisionChainResponse`] verbatim from a [`DecisionChainSource`]
//! implementation. Live wiring builds a `NexusDecisionChainSource`
//! on top of `nexus_sdk::NexusClient`; tests substitute a fixture
//! source that records every walk so the projection logic is
//! exercised without a live graph backend.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

/// Default cap on path length when the caller omits `max_hops`.
pub const DEFAULT_MAX_HOPS: u32 = 16;

/// Server-side hard ceiling on `max_hops` — matches the spec.
/// Walks longer than this would risk runaway Cypher on a dense
/// supersession subgraph.
pub const MAX_HOPS_CEILING: u32 = 16;

/// One node in the chain. Fields mirror the `Decision` node shape
/// the writer produces (spec 13 / phase11k).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionNode {
    /// Decision's `event_id` ULID.
    pub event_id: String,
    /// Slug derived from the title (e.g. `adr-014-...`). Empty when
    /// the node carries no slug (legacy).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slug: String,
    /// Lifecycle status (`proposed` / `accepted` / `superseded` /
    /// `deprecated`).
    pub status: String,
    /// ISO-8601 date stamp. Empty when the node has no date.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    /// Display title.
    pub title: String,
    /// `event_id` of the decision this node supersedes. `None` when
    /// the node is a root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// `event_id` of the decision that supersedes this node. `None`
    /// when the node is a leaf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// Wire response body for `GET /v1/search/decision-chain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionChainResponse {
    /// Merged predecessor + start + successor nodes, sorted by
    /// `date` ascending. Empty when the start node does not exist.
    pub chain: Vec<DecisionNode>,
    /// Number of predecessor hops the walker actually traversed
    /// (excludes the start node).
    pub walked_predecessors: u32,
    /// Number of successor hops the walker actually traversed
    /// (excludes the start node).
    pub walked_successors: u32,
}

/// Source-level errors surfaced by the walker.
#[derive(Debug, thiserror::Error)]
pub enum DecisionChainError {
    /// Backend transport error.
    #[error("backend transport: {0}")]
    Transport(String),
    /// Catch-all for non-transport upstream errors.
    #[error("backend: {0}")]
    Other(String),
}

/// Source the handler reads from.
#[async_trait]
pub trait DecisionChainSource: Send + Sync {
    /// Walk `:SUPERSEDES` edges from `event_id` in both directions
    /// bounded by `max_hops`. Returns the merged chain plus per-
    /// direction hop counts.
    async fn walk(
        &self,
        event_id: &str,
        max_hops: u32,
    ) -> Result<DecisionChainResponse, DecisionChainError>;
}

/// Source that returns an empty chain unconditionally. Used as the
/// boot-time default until a live Nexus client is wired in.
pub struct UnwiredDecisionChainSource;

#[async_trait]
impl DecisionChainSource for UnwiredDecisionChainSource {
    async fn walk(
        &self,
        _event_id: &str,
        _max_hops: u32,
    ) -> Result<DecisionChainResponse, DecisionChainError> {
        Ok(DecisionChainResponse {
            chain: Vec::new(),
            walked_predecessors: 0,
            walked_successors: 0,
        })
    }
}

/// Axum state holding the live source.
#[derive(Clone)]
pub struct DecisionChainState {
    /// Behind an `Arc<dyn _>` so the same handler can serve both
    /// live + fixture sources.
    pub source: Arc<dyn DecisionChainSource>,
}

impl Default for DecisionChainState {
    fn default() -> Self {
        Self {
            source: Arc::new(UnwiredDecisionChainSource),
        }
    }
}

/// Build the `/v1/search/decision-chain` sub-router.
pub fn build_router(state: DecisionChainState) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/v1/search/decision-chain", get(decision_chain_handler))
        .with_state(state)
}

/// Query params for `/v1/search/decision-chain`.
#[derive(Debug, Deserialize)]
pub struct DecisionChainQuery {
    /// Starting `event_id`. Must be a 26-char ULID
    /// (`[0-9A-Z]{26}`).
    pub event_id: String,
    /// Path length cap. Clamped server-side to
    /// `[1, MAX_HOPS_CEILING]`; defaults to [`DEFAULT_MAX_HOPS`].
    #[serde(default)]
    pub max_hops: Option<u32>,
}

/// Axum handler for `GET /v1/search/decision-chain`.
pub async fn decision_chain_handler(
    State(state): State<DecisionChainState>,
    Query(params): Query<DecisionChainQuery>,
) -> Response {
    if !is_ulid(&params.event_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "reason": "bad_input",
                "message": "`event_id` must match [0-9A-Z]{26}",
                "details": { "event_id": params.event_id },
            })),
        )
            .into_response();
    }
    let max_hops = clamp_max_hops(params.max_hops);
    match state.source.walk(&params.event_id, max_hops).await {
        Ok(mut resp) => {
            // Merge guarantee — sort by date asc, then event_id asc
            // as the deterministic tiebreaker. Empty `date` strings
            // sink to the front of the same bucket; the handler
            // surfaces these because the projection is honest.
            resp.chain.sort_by(|a, b| {
                a.date.cmp(&b.date).then_with(|| a.event_id.cmp(&b.event_id))
            });
            Json(resp).into_response()
        }
        Err(DecisionChainError::Transport(msg)) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "reason": "nexus_unreachable",
                "message": msg,
            })),
        )
            .into_response(),
        Err(DecisionChainError::Other(msg)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "reason": "backend_error",
                "message": msg,
            })),
        )
            .into_response(),
    }
}

/// Clamp `max_hops` into `[1, MAX_HOPS_CEILING]`. `None` returns
/// [`DEFAULT_MAX_HOPS`].
pub fn clamp_max_hops(max_hops: Option<u32>) -> u32 {
    max_hops.unwrap_or(DEFAULT_MAX_HOPS).clamp(1, MAX_HOPS_CEILING)
}

/// Validate a ULID — 26 chars, base32 alphabet `[0-9A-Z]` with the
/// Crockford-safe subset (rejects `I`, `L`, `O`, `U`).
pub fn is_ulid(s: &str) -> bool {
    if s.len() != 26 {
        return false;
    }
    s.bytes().all(|b| {
        matches!(
            b,
            b'0'..=b'9'
                | b'A'..=b'H'
                | b'J'
                | b'K'
                | b'M'
                | b'N'
                | b'P'..=b'T'
                | b'V'..=b'Z'
        )
    })
}

/// Builder helper for the live walker. Given a flat list of
/// supersession edges `(pred_id, succ_id)` plus a node lookup
/// table, walks both directions from `start` capped at `max_hops`,
/// breaks on cycles, and returns the merged response.
///
/// Lives at module scope (rather than inside the live source impl)
/// so the cycle / hop logic stays unit-testable without a live
/// Nexus.
pub fn walk_chain(
    start: &str,
    nodes: &BTreeMap<String, DecisionNode>,
    edges: &[(String, String)],
    max_hops: u32,
) -> DecisionChainResponse {
    let mut chain: BTreeMap<String, DecisionNode> = BTreeMap::new();
    let mut walked_predecessors = 0u32;
    let mut walked_successors = 0u32;
    if let Some(seed) = nodes.get(start) {
        chain.insert(start.to_string(), seed.clone());
    } else {
        return DecisionChainResponse {
            chain: Vec::new(),
            walked_predecessors: 0,
            walked_successors: 0,
        };
    }

    // Predecessors: follow edges where the current node is `succ`,
    // hop to `pred`. Stops when a hop revisits a known node.
    let mut frontier = vec![start.to_string()];
    let mut hops = 0u32;
    while !frontier.is_empty() && hops < max_hops {
        let mut next = Vec::new();
        for cur in &frontier {
            for (pred, succ) in edges {
                if succ == cur {
                    if chain.contains_key(pred) {
                        continue;
                    }
                    if let Some(node) = nodes.get(pred) {
                        chain.insert(pred.clone(), node.clone());
                        next.push(pred.clone());
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        walked_predecessors += next.len() as u32;
        hops += 1;
        frontier = next;
    }

    // Successors: follow edges where the current node is `pred`,
    // hop to `succ`. Same cycle break, separate hop counter.
    let mut frontier = vec![start.to_string()];
    let mut hops = 0u32;
    while !frontier.is_empty() && hops < max_hops {
        let mut next = Vec::new();
        for cur in &frontier {
            for (pred, succ) in edges {
                if pred == cur {
                    if chain.contains_key(succ) {
                        continue;
                    }
                    if let Some(node) = nodes.get(succ) {
                        chain.insert(succ.clone(), node.clone());
                        next.push(succ.clone());
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        walked_successors += next.len() as u32;
        hops += 1;
        frontier = next;
    }

    let mut chain: Vec<DecisionNode> = chain.into_values().collect();
    chain.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    DecisionChainResponse {
        chain,
        walked_predecessors,
        walked_successors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulid(n: u8) -> String {
        // 25 zeros + one variable suffix char from the Crockford
        // subset so every test id is a valid ULID.
        format!("{}{}", "0".repeat(25), char::from(b'A' + n))
    }

    fn node(event_id: &str, date: &str, supersedes: Option<&str>, superseded_by: Option<&str>) -> DecisionNode {
        DecisionNode {
            event_id: event_id.into(),
            slug: format!("slug-{event_id}"),
            status: "proposed".into(),
            date: date.into(),
            title: format!("Title {event_id}"),
            supersedes: supersedes.map(str::to_string),
            superseded_by: superseded_by.map(str::to_string),
        }
    }

    #[test]
    fn clamp_max_hops_bounds_and_defaults() {
        assert_eq!(clamp_max_hops(None), DEFAULT_MAX_HOPS);
        assert_eq!(clamp_max_hops(Some(0)), 1);
        assert_eq!(clamp_max_hops(Some(100)), MAX_HOPS_CEILING);
        assert_eq!(clamp_max_hops(Some(8)), 8);
    }

    #[test]
    fn is_ulid_accepts_canonical_form_and_rejects_invalid() {
        assert!(is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(!is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FA")); // 25 chars
        assert!(!is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAV0")); // 27 chars
        assert!(!is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAv")); // lowercase
        assert!(!is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAI")); // I forbidden
        assert!(!is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAL")); // L forbidden
    }

    #[test]
    fn walk_chain_returns_single_node_when_no_supersession() {
        let start = ulid(0);
        let mut nodes = BTreeMap::new();
        nodes.insert(start.clone(), node(&start, "2026-01-01", None, None));
        let edges: Vec<(String, String)> = Vec::new();
        let r = walk_chain(&start, &nodes, &edges, 16);
        assert_eq!(r.chain.len(), 1);
        assert_eq!(r.walked_predecessors, 0);
        assert_eq!(r.walked_successors, 0);
    }

    #[test]
    fn walk_chain_produces_linear_three_node_chain_sorted_by_date() {
        // a (2026-01) -> b (2026-02) -> c (2026-03), start from b.
        let a = ulid(0);
        let b = ulid(1);
        let c = ulid(2);
        let mut nodes = BTreeMap::new();
        nodes.insert(a.clone(), node(&a, "2026-01-01", None, Some(&b)));
        nodes.insert(b.clone(), node(&b, "2026-02-01", Some(&a), Some(&c)));
        nodes.insert(c.clone(), node(&c, "2026-03-01", Some(&b), None));
        let edges = vec![(a.clone(), b.clone()), (b.clone(), c.clone())];
        let r = walk_chain(&b, &nodes, &edges, 16);
        let ids: Vec<&str> = r.chain.iter().map(|n| n.event_id.as_str()).collect();
        assert_eq!(ids, vec![a.as_str(), b.as_str(), c.as_str()]);
        assert_eq!(r.walked_predecessors, 1);
        assert_eq!(r.walked_successors, 1);
    }

    #[test]
    fn walk_chain_returns_both_branches_of_a_fork() {
        // pred1 -> root, pred2 -> root. Walk from root.
        let root = ulid(0);
        let p1 = ulid(1);
        let p2 = ulid(2);
        let mut nodes = BTreeMap::new();
        nodes.insert(p1.clone(), node(&p1, "2026-01-01", None, Some(&root)));
        nodes.insert(p2.clone(), node(&p2, "2026-01-15", None, Some(&root)));
        nodes.insert(root.clone(), node(&root, "2026-02-01", Some(&p1), None));
        let edges = vec![(p1.clone(), root.clone()), (p2.clone(), root.clone())];
        let r = walk_chain(&root, &nodes, &edges, 16);
        assert_eq!(r.chain.len(), 3);
        // Both predecessors traversed in one hop.
        assert_eq!(r.walked_predecessors, 2);
        assert_eq!(r.walked_successors, 0);
    }

    #[test]
    fn walk_chain_breaks_on_cycle_within_two_hops() {
        // a -> b -> a (illegal cycle but the walker must not loop).
        let a = ulid(0);
        let b = ulid(1);
        let mut nodes = BTreeMap::new();
        nodes.insert(a.clone(), node(&a, "2026-01-01", None, Some(&b)));
        nodes.insert(b.clone(), node(&b, "2026-02-01", Some(&a), Some(&a)));
        let edges = vec![(a.clone(), b.clone()), (b.clone(), a.clone())];
        let r = walk_chain(&a, &nodes, &edges, 16);
        // Both nodes appear exactly once — the cycle did not loop.
        assert_eq!(r.chain.len(), 2);
        // Walker runs predecessor pass first: edge `b -> a` adds
        // `b`. Predecessor hop 2 would walk `b`'s predecessor edge
        // `a -> b` but `a` is already in chain, so the walk stops.
        // Successor pass then finds `b` already in the chain (the
        // cycle guard) and adds nothing more.
        assert_eq!(r.walked_predecessors, 1);
        assert_eq!(r.walked_successors, 0);
    }

    #[test]
    fn walk_chain_respects_max_hops_cap() {
        // Linear chain a -> b -> c -> d -> e starting at `a`.
        let ids: Vec<String> = (0..5).map(ulid).collect();
        let mut nodes = BTreeMap::new();
        let mut edges = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let prev = if i == 0 { None } else { Some(ids[i - 1].as_str()) };
            let next = if i == ids.len() - 1 {
                None
            } else {
                Some(ids[i + 1].as_str())
            };
            nodes.insert(
                id.clone(),
                node(id, &format!("2026-0{}-01", i + 1), prev, next),
            );
            if i > 0 {
                edges.push((ids[i - 1].clone(), id.clone()));
            }
        }
        // max_hops = 2 from start `a`: walks b, then c. d and e dropped.
        let r = walk_chain(&ids[0], &nodes, &edges, 2);
        let chain_ids: Vec<&str> = r.chain.iter().map(|n| n.event_id.as_str()).collect();
        assert_eq!(chain_ids, vec![ids[0].as_str(), ids[1].as_str(), ids[2].as_str()]);
        assert_eq!(r.walked_successors, 2);
    }

    #[tokio::test]
    async fn handler_rejects_non_ulid_event_id() {
        let state = DecisionChainState::default();
        let resp = decision_chain_handler(
            State(state),
            Query(DecisionChainQuery {
                event_id: "not-a-ulid".into(),
                max_hops: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_returns_empty_chain_when_source_unwired() {
        let state = DecisionChainState::default();
        let resp = decision_chain_handler(
            State(state),
            Query(DecisionChainQuery {
                event_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                max_hops: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let value: DecisionChainResponse = serde_json::from_slice(&body).unwrap();
        assert!(value.chain.is_empty());
        assert_eq!(value.walked_predecessors, 0);
        assert_eq!(value.walked_successors, 0);
    }
}
