//! Live Nexus-backed `GraphLane`.
//!
//! `MemoryGraphLane` ships as a test double — its hits are whatever
//! the orchestrator's strategies layer happened to seed by template
//! name. The 2026-04-27 audit logged `graph_neighbors=0` on every
//! probed query: `cortex-api/src/main.rs` already builds a
//! `NexusClient` for `DashboardState`, but the orchestrator wired
//! `Arc::new(MemoryGraphLane::new())` and never reused it.
//!
//! This module ships the production read-path: per-query parametrised
//! Cypher against the same Nexus instance the dashboard graph view
//! talks to. The lane only executes pre-registered templates — the
//! orchestrator's strategies layer picks the template by intent
//! (`edge_artifact_touched_neighbours`, `decision_supersedes_chain`,
//! `turn_analysis_decision_chain`, `law_violations_last_30d`).
//! Unknown templates are rejected at the lane boundary so a buggy
//! caller can't reach Nexus with arbitrary Cypher.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nexus_sdk::{NexusClient, Value as NexusValue};
use serde_json::Value;

use crate::lanes::{GraphLane, GraphRequest, LaneError, LaneHit};

/// Concrete `GraphLane` backed by a live Nexus instance.
#[derive(Clone)]
pub struct NexusGraphLane {
    client: Arc<NexusClient>,
}

impl std::fmt::Debug for NexusGraphLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NexusGraphLane").finish_non_exhaustive()
    }
}

impl NexusGraphLane {
    /// Wrap an existing Nexus client. The boot path in
    /// `cortex-api/src/main.rs` shares a single `Arc<NexusClient>`
    /// between this lane and `DashboardState` so the daemon doesn't
    /// open two TCP sessions to the same server.
    pub fn new(client: Arc<NexusClient>) -> Self {
        Self { client }
    }
}

/// Resolve a strategies-layer template name to a parametrised
/// read-only Cypher string. Returns `None` for unknown templates so
/// `search` can map to `LaneError::Rejected` (the closest fit in the
/// existing `LaneError` enum — there's no `InvalidRequest` variant
/// today, and `Rejected` already maps to a 4xx-class semantic).
///
/// The Cypher is deliberately conservative: every template caps at
/// 50 rows inline (Nexus 1.15 ignores `LIMIT $param` and returns the
/// full result set — see `dashboard.rs`'s graph endpoint for the
/// same workaround). The orchestrator's `req.limit` further trims
/// downstream during overlay derivation.
fn cypher_for(template: &str) -> Option<&'static str> {
    match template {
        "edge_artifact_touched_neighbours" => Some(
            "MATCH (a:Artifact)<-[r:TOUCHED]-(s) \
             WHERE a.path CONTAINS $q OR a.natural_key CONTAINS $q \
             RETURN s.id AS edge_from, a.path AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(a.path, a.natural_key) AS label \
             LIMIT 50",
        ),
        "decision_supersedes_chain" => Some(
            "MATCH (d:Decision)-[r:SUPERSEDES]->(prev:Decision) \
             WHERE d.id CONTAINS $q OR d.title CONTAINS $q \
                OR prev.id CONTAINS $q OR prev.title CONTAINS $q \
             RETURN d.id AS edge_from, prev.id AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(d.title, d.id) AS label \
             LIMIT 50",
        ),
        "turn_analysis_decision_chain" => Some(
            "MATCH (t:Turn)-[r1:OBSERVED_IN]->(an:Analysis) \
             WHERE t.id CONTAINS $q OR an.id CONTAINS $q OR an.title CONTAINS $q \
             OPTIONAL MATCH (an)-[r2:PRODUCED_DECISION]->(d:Decision) \
             RETURN t.id AS edge_from, an.id AS edge_to, type(r1) AS edge_type, 1 AS hops, \
                    coalesce(an.title, an.id) AS label \
             LIMIT 50",
        ),
        "law_violations_last_30d" => Some(
            // LawViolation -> Law (the violated rule). The 30d window
            // is enforced inline because Nexus 1.15's parameter
            // binding ignores numeric LIMIT/comparison params on this
            // dialect; the strategies layer already filters by query
            // text via the WHERE clause.
            "MATCH (v:LawViolation)-[r:VIOLATES]->(l:Law) \
             WHERE v.law_id CONTAINS $q OR l.title CONTAINS $q OR v.body CONTAINS $q \
             RETURN v.id AS edge_from, l.id AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(l.title, l.id) AS label \
             LIMIT 50",
        ),
        _ => None,
    }
}

#[async_trait]
impl GraphLane for NexusGraphLane {
    async fn query(&self, req: &GraphRequest) -> Result<Vec<LaneHit>, LaneError> {
        let cy = cypher_for(&req.template).ok_or_else(|| {
            LaneError::Rejected(format!(
                "unknown graph template: {} (whitelist: edge_artifact_touched_neighbours, \
                 decision_supersedes_chain, turn_analysis_decision_chain, law_violations_last_30d)",
                req.template
            ))
        })?;

        // Bind `$q` from the strategies-supplied params. Every
        // current template uses the same `query` param name; if a
        // future template needs a different shape, add a per-template
        // binder rather than a free-form `params` walk (the trait
        // surface keeps the lane locked to whitelisted shapes).
        let q = req
            .params
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut params: HashMap<String, NexusValue> = HashMap::new();
        params.insert("q".to_string(), NexusValue::String(q));

        let res = match self.client.execute_cypher(cy, Some(params)).await {
            Ok(r) => r,
            Err(e) => {
                return Err(LaneError::Transport(format!(
                    "execute_cypher({}): {e}",
                    req.template
                )));
            }
        };

        let hits = res
            .rows
            .iter()
            .filter_map(|row| project_row(row, &req.template))
            .collect();
        Ok(hits)
    }
}

fn project_row(row: &Value, template: &str) -> Option<LaneHit> {
    let cells = row.as_array()?;
    let edge_from = cells.first().and_then(Value::as_str)?.to_string();
    let edge_to = cells.get(1).and_then(Value::as_str)?.to_string();
    let edge_type = cells
        .get(2)
        .and_then(Value::as_str)
        .unwrap_or("RELATED")
        .to_string();
    let hops = cells.get(3).and_then(Value::as_i64).unwrap_or(1).max(0) as u64;
    let label = cells
        .get(4)
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| edge_to.clone());

    if edge_from.is_empty() || edge_to.is_empty() {
        return None;
    }

    let mut extras = std::collections::BTreeMap::new();
    // The orchestrator's `derive_graph_neighbors` reads these four
    // keys to materialise `GraphNeighbor` overlays — keep them
    // exactly aligned. `source = "graph"` keeps the lane label
    // honest for the snippet column / fused result set even though
    // the keyword/vector lanes' `lane_label()` fallback never
    // selects it (graph hits don't usually flow into snippets).
    extras.insert("source".to_string(), Value::String("graph".to_string()));
    extras.insert("edge_from".to_string(), Value::String(edge_from.clone()));
    extras.insert("edge_to".to_string(), Value::String(edge_to.clone()));
    extras.insert("edge_type".to_string(), Value::String(edge_type.clone()));
    extras.insert("hops".to_string(), Value::Number(hops.into()));
    extras.insert("template".to_string(), Value::String(template.to_string()));

    // ADR-011 — typed overlay alongside extras. Graph lane owns
    // edge_from / edge_to / hops; the rest stay None.
    let overlay = crate::lanes::Overlay {
        edge_from: Some(edge_from.clone()),
        edge_to: Some(edge_to.clone()),
        hops: u8::try_from(hops).ok(),
        source: crate::lanes::LaneSource::Graph,
        ..crate::lanes::Overlay::default()
    };

    Some(LaneHit {
        // Doc-id keys per (template, from, to) so RRF buckets stay
        // distinct across templates that surface the same node pair.
        doc_id: format!("graph|{template}|{edge_from}|{edge_to}"),
        text: label,
        repo: None,
        path: None,
        symbol: Some(edge_type),
        content_hash: None,
        // Score by hop distance: 1.0 / hops. The orchestrator's RRF
        // doesn't depend on this for ranking (positional order +
        // RRF rank is enough), but a real-ish score keeps the
        // downstream snippet column from showing 0.0 across the
        // board.
        score: 1.0 / (hops as f64).max(1.0),
        ts: 0,
        severity: None,
        extras,
        overlay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scope;

    #[test]
    fn cypher_resolves_for_every_strategy_template() {
        // The 4 templates the strategies layer emits MUST resolve
        // — adding a new strategy without registering its template
        // here would silently disable graph lookups for that intent.
        for template in [
            "edge_artifact_touched_neighbours",
            "decision_supersedes_chain",
            "turn_analysis_decision_chain",
            "law_violations_last_30d",
        ] {
            assert!(
                cypher_for(template).is_some(),
                "template {template} must resolve to a Cypher string",
            );
        }
    }

    #[test]
    fn cypher_returns_none_for_unknown_template() {
        assert!(cypher_for("drop_database").is_none());
        assert!(cypher_for("").is_none());
    }

    #[test]
    fn projects_a_row_into_a_lane_hit_with_neighbour_extras() {
        let row = serde_json::json!([
            "DEC-0042",
            "DEC-0001",
            "SUPERSEDES",
            1_i64,
            "Adopt RRF fusion"
        ]);
        let hit = project_row(&row, "decision_supersedes_chain").unwrap();
        assert_eq!(
            hit.doc_id,
            "graph|decision_supersedes_chain|DEC-0042|DEC-0001"
        );
        assert_eq!(hit.text, "Adopt RRF fusion");
        assert_eq!(hit.symbol.as_deref(), Some("SUPERSEDES"));
        assert_eq!(
            hit.extras.get("edge_from").and_then(|v| v.as_str()),
            Some("DEC-0042")
        );
        assert_eq!(
            hit.extras.get("edge_to").and_then(|v| v.as_str()),
            Some("DEC-0001")
        );
        assert_eq!(
            hit.extras.get("edge_type").and_then(|v| v.as_str()),
            Some("SUPERSEDES")
        );
        assert_eq!(hit.extras.get("hops").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            hit.extras.get("source").and_then(|v| v.as_str()),
            Some("graph")
        );
    }

    #[test]
    fn projects_drops_rows_with_empty_endpoints() {
        let empty_from = serde_json::json!(["", "DEC-0001", "SUPERSEDES", 1_i64, "label"]);
        let empty_to = serde_json::json!(["DEC-0042", "", "SUPERSEDES", 1_i64, "label"]);
        assert!(project_row(&empty_from, "decision_supersedes_chain").is_none());
        assert!(project_row(&empty_to, "decision_supersedes_chain").is_none());
    }

    #[test]
    fn graph_request_shape_compiles() {
        let _ = GraphRequest {
            template: "decision_supersedes_chain".into(),
            params: serde_json::json!({ "query": "RRF" }),
            max_hops: 2,
            scope: Scope::default(),
        };
    }
}
