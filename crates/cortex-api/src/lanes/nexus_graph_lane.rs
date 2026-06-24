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
        // Phase27a §3.1 — every strategy template also returns the
        // edge's confidence tier + score (cells 5/6) so `project_row`
        // can down-weight `Inferred`/`Ambiguous` edges in the fused
        // native-score term. Edges written before phase27a carry
        // neither property → Cypher returns null → weight 1.0 (no
        // down-weighting, back-compatible).
        "edge_artifact_touched_neighbours" => Some(
            "MATCH (a:Artifact)<-[r:TOUCHED]-(s) \
             WHERE a.path CONTAINS $q OR a.natural_key CONTAINS $q \
             RETURN s.id AS edge_from, a.path AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(a.path, a.natural_key) AS label, \
                    r.confidence AS confidence, r.confidence_score AS confidence_score \
             LIMIT 50",
        ),
        "decision_supersedes_chain" => Some(
            "MATCH (d:Decision)-[r:SUPERSEDES]->(prev:Decision) \
             WHERE d.id CONTAINS $q OR d.title CONTAINS $q \
                OR prev.id CONTAINS $q OR prev.title CONTAINS $q \
             RETURN d.id AS edge_from, prev.id AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(d.title, d.id) AS label, \
                    r.confidence AS confidence, r.confidence_score AS confidence_score \
             LIMIT 50",
        ),
        "turn_analysis_decision_chain" => Some(
            "MATCH (t:Turn)-[r1:OBSERVED_IN]->(an:Analysis) \
             WHERE t.id CONTAINS $q OR an.id CONTAINS $q OR an.title CONTAINS $q \
             OPTIONAL MATCH (an)-[r2:PRODUCED_DECISION]->(d:Decision) \
             RETURN t.id AS edge_from, an.id AS edge_to, type(r1) AS edge_type, 1 AS hops, \
                    coalesce(an.title, an.id) AS label, \
                    r1.confidence AS confidence, r1.confidence_score AS confidence_score \
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
                    coalesce(l.title, l.id) AS label, \
                    r.confidence AS confidence, r.confidence_score AS confidence_score \
             LIMIT 50",
        ),
        // Phase18 §5.2 — cross-project reference edges. Invoked
        // directly by the orchestrator's propagation helper (not via
        // the strategies layer) so the `$q` param carries the active
        // project's canonical branch id (`<project>:main`). Returns
        // the edge's version constraint and bitemporal validity window
        // so the propagation helper can classify them via the same
        // temporal wedge used for regular hits.
        "cross_project_ref" => Some(
            // Confidence rides cells 5/6 like every other template;
            // this edge's own extras (version_constraint, valid_from,
            // valid_to) shift to cells 7/8/9 so `project_row` reads
            // confidence at the same offset for every template.
            "MATCH (a:Branch)-[r:CROSS_PROJECT_REF]->(b:Branch) \
             WHERE a.id CONTAINS $q \
             RETURN a.id AS edge_from, b.id AS edge_to, type(r) AS edge_type, 1 AS hops, \
                    coalesce(b.id, a.id) AS label, \
                    r.confidence AS confidence, r.confidence_score AS confidence_score, \
                    r.version_constraint AS version_constraint, \
                    r.valid_from AS valid_from, r.valid_to AS valid_to \
             LIMIT 50",
        ),
        _ => None,
    }
}

/// Phase21 §5.4 — map a template name to the primary node alias used
/// in its WHERE clause. The ACL injection appends predicates on this
/// alias so the Bell-LaPadula level + compartment check applies to the
/// "content" node (the one whose classification the principal must hold
/// clearance for). Returns `None` for `cross_project_ref` (structural
/// meta-edge; class filtering is not applicable there).
fn primary_node_alias(template: &str) -> Option<&'static str> {
    match template {
        "edge_artifact_touched_neighbours" => Some("a"),
        "decision_supersedes_chain" => Some("d"),
        "turn_analysis_decision_chain" => Some("an"),
        "law_violations_last_30d" => Some("v"),
        _ => None,
    }
}

/// Phase21 §5.4 — build the ACL-augmented Cypher for a template when
/// an ACL context is present. Inserts the Bell-LaPadula WHERE fragment
/// (inline literals — the ACL filter is generated as a static string
/// fragment and is not a user-supplied value, so inline is the correct
/// form here) immediately before the `RETURN` keyword by replacing the
/// first occurrence of ` RETURN `. Returns the base template unchanged
/// (as `String`) when ACL is `None` or the template has no registered
/// primary alias (structural templates like `cross_project_ref`).
fn cypher_with_acl(
    base: &'static str,
    template: &str,
    acl: Option<&crate::lanes::AclContext>,
) -> String {
    let acl = match acl {
        Some(a) => a,
        None => return base.to_string(),
    };
    let alias = match primary_node_alias(template) {
        Some(a) => a,
        None => return base.to_string(),
    };
    let fragment =
        cortex_workers::acl::filter::nexus_acl_where_for_alias(
            acl.clearance_level,
            &acl.compartment_grants,
            alias,
        );
    // Insert before ` RETURN ` to extend the existing WHERE clause.
    // Every template has exactly one RETURN; if the pattern is absent
    // the base Cypher is returned unchanged (safe degradation).
    base.replacen(" RETURN ", &format!(" AND {fragment} RETURN "), 1)
}

#[async_trait]
impl GraphLane for NexusGraphLane {
    async fn query(&self, req: &GraphRequest) -> Result<Vec<LaneHit>, LaneError> {
        let base = cypher_for(&req.template).ok_or_else(|| {
            LaneError::Rejected(format!(
                "unknown graph template: {} (whitelist: edge_artifact_touched_neighbours, \
                 decision_supersedes_chain, turn_analysis_decision_chain, law_violations_last_30d, \
                 cross_project_ref)",
                req.template
            ))
        })?;
        let cy_owned = cypher_with_acl(base, &req.template, req.acl.as_ref());
        let cy = cy_owned.as_str();

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

/// Phase27a §3.1 — map an edge-confidence tier (+ optional explicit
/// score) to a retrieval weight in `(0.0, 1.0]`. Mirrors
/// `cortex_workers::graph::EdgeConfidence::default_score`
/// (`extracted → 1.0`, `inferred → 0.7`, `ambiguous → 0.4`). An
/// explicit `confidence_score` property wins when present + finite +
/// positive; otherwise the tier string decides; an edge carrying
/// neither (everything written before phase27a) yields `1.0` so the
/// down-weighting is fully back-compatible.
fn confidence_weight(tier: Option<&str>, score: Option<f64>) -> f64 {
    if let Some(s) = score {
        if s.is_finite() && s > 0.0 {
            return s.clamp(0.0, 1.0);
        }
    }
    match tier.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("extracted") => 1.0,
        Some("inferred") => 0.7,
        Some("ambiguous") => 0.4,
        _ => 1.0,
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

    // Phase27a §3.1 — cells 5/6 carry the edge's confidence tier +
    // score for every template (see `cypher_for`). A row that
    // predates the RETURN change, or an edge without the property,
    // surfaces these as absent → `confidence_weight` returns 1.0.
    let confidence = cells.get(5).and_then(Value::as_str).map(String::from);
    let confidence_score = cells.get(6).and_then(Value::as_f64);
    let weight = confidence_weight(confidence.as_deref(), confidence_score);

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

    // Phase27a §3.1 — round-trip the confidence tier + the realised
    // retrieval weight for debug / dashboard surfacing (§3.2). Only
    // stamped when the edge actually carries a tier so pre-phase27a
    // edges keep the leaner extras shape.
    if let Some(tier) = &confidence {
        extras.insert("confidence".to_string(), Value::String(tier.clone()));
    }
    if let Some(s) = confidence_score {
        if let Some(n) = serde_json::Number::from_f64(s) {
            extras.insert("confidence_score".to_string(), Value::Number(n));
        }
    }

    // Phase18 §5.2 — cross_project_ref edges carry three extra cells
    // (cells 5/6/7: version_constraint, valid_from, valid_to). Read
    // them only for this template so a 5-cell row from any other
    // template is unaffected. `source_project` is derived from the
    // target branch id by splitting on ':' and taking the prefix
    // (e.g. "nexus:main" → "nexus").
    if template == "cross_project_ref" {
        // Phase27a §3.1 — these shifted from cells 5/6/7 to 7/8/9 when
        // confidence took cells 5/6 across every template.
        if let Some(vc) = cells.get(7).and_then(Value::as_str) {
            extras.insert(
                "version_constraint".to_string(),
                Value::String(vc.to_string()),
            );
        }
        if let Some(vf) = cells.get(8).and_then(Value::as_str) {
            extras.insert("valid_from".to_string(), Value::String(vf.to_string()));
        }
        if let Some(vt) = cells.get(9).and_then(Value::as_str) {
            extras.insert("valid_to".to_string(), Value::String(vt.to_string()));
        }
        // Derive source_project from edge_to ("project:branch" → "project").
        let source_project = edge_to
            .split_once(':')
            .map(|(proj, _)| proj)
            .unwrap_or(edge_to.as_str())
            .to_string();
        extras.insert("source_project".to_string(), Value::String(source_project));
    }

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
        // Score by hop distance (`1.0 / hops`) down-weighted by edge
        // confidence (Phase27a §3.1): an `Inferred` (×0.7) or
        // `Ambiguous` (×0.4) edge contributes proportionally less to
        // the fused native-score term than a direct `Extracted` edge
        // at the same hop distance. The fusion blend reads this via
        // `LaneHit::normalized_score`, so the down-weighting shifts
        // fused order whenever two graph edges differ only in
        // confidence. Edges without a confidence property keep weight
        // 1.0 (back-compatible with everything written pre-phase27a).
        score: (1.0 / (hops as f64).max(1.0)) * weight,
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

    // ---------------- Phase27a §3.1 — confidence weighting ----------------

    #[test]
    fn confidence_weight_maps_tiers_to_default_scores() {
        // Mirrors EdgeConfidence::default_score in cortex-workers.
        assert_eq!(confidence_weight(Some("extracted"), None), 1.0);
        assert_eq!(confidence_weight(Some("inferred"), None), 0.7);
        assert_eq!(confidence_weight(Some("ambiguous"), None), 0.4);
        // Case / whitespace tolerant.
        assert_eq!(confidence_weight(Some("  Inferred "), None), 0.7);
        // Unknown tier or absent property → no down-weighting.
        assert_eq!(confidence_weight(Some("bogus"), None), 1.0);
        assert_eq!(confidence_weight(None, None), 1.0);
    }

    #[test]
    fn confidence_weight_prefers_explicit_score_when_present() {
        // An explicit, finite, positive confidence_score wins over
        // the tier mapping and is clamped onto (0,1].
        assert_eq!(confidence_weight(Some("extracted"), Some(0.55)), 0.55);
        assert_eq!(confidence_weight(Some("inferred"), Some(1.5)), 1.0);
        // Non-positive / non-finite scores fall back to the tier.
        assert_eq!(confidence_weight(Some("inferred"), Some(0.0)), 0.7);
        assert_eq!(confidence_weight(Some("ambiguous"), Some(f64::NAN)), 0.4);
    }

    #[test]
    fn project_row_downweights_inferred_edge_score() {
        // Two single-hop edges that differ only in confidence: the
        // Extracted edge keeps the full 1.0 hop score, the Inferred
        // one is multiplied by 0.7 so it contributes less to the
        // fused native-score term.
        let extracted = serde_json::json!([
            "S1", "src/lib.rs", "TOUCHED", 1_i64, "src/lib.rs", "extracted", 1.0
        ]);
        let inferred = serde_json::json!([
            "S2", "about", "RELATES_TO", 1_i64, "about", "inferred", 0.7
        ]);
        let he = project_row(&extracted, "edge_artifact_touched_neighbours").unwrap();
        let hi = project_row(&inferred, "edge_artifact_touched_neighbours").unwrap();
        assert!((he.score - 1.0).abs() < 1e-9, "extracted keeps full score");
        assert!((hi.score - 0.7).abs() < 1e-9, "inferred down-weighted ×0.7");
        assert!(hi.score < he.score);
        // Tier is round-tripped into extras for §3.2 surfacing.
        assert_eq!(
            hi.extras.get("confidence").and_then(|v| v.as_str()),
            Some("inferred")
        );
    }

    #[test]
    fn project_row_without_confidence_keeps_full_hop_score() {
        // A 5-cell row (pre-phase27a edge, or an edge missing the
        // property) must keep weight 1.0 — no down-weighting.
        let row = serde_json::json!(["A", "B", "SUPERSEDES", 1_i64, "label"]);
        let hit = project_row(&row, "decision_supersedes_chain").unwrap();
        assert!((hit.score - 1.0).abs() < 1e-9);
        assert!(
            !hit.extras.contains_key("confidence"),
            "no confidence extra stamped for an edge without the property"
        );
    }

    #[test]
    fn cross_project_ref_reads_extras_after_confidence_cells() {
        // cross_project_ref now returns confidence at cells 5/6 and
        // its own extras at 7/8/9. Confirm both the down-weight and
        // the shifted extra reads.
        let row = serde_json::json!([
            "cortex:main",
            "nexus:main",
            "CROSS_PROJECT_REF",
            1_i64,
            "nexus:main",
            "ambiguous",
            0.4,
            "^2.1",
            "2026-01-01",
            "2026-12-31"
        ]);
        let hit = project_row(&row, "cross_project_ref").unwrap();
        assert!((hit.score - 0.4).abs() < 1e-9, "ambiguous ×0.4");
        assert_eq!(
            hit.extras.get("version_constraint").and_then(|v| v.as_str()),
            Some("^2.1")
        );
        assert_eq!(
            hit.extras.get("valid_from").and_then(|v| v.as_str()),
            Some("2026-01-01")
        );
        assert_eq!(
            hit.extras.get("valid_to").and_then(|v| v.as_str()),
            Some("2026-12-31")
        );
        assert_eq!(
            hit.extras.get("source_project").and_then(|v| v.as_str()),
            Some("nexus")
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
            acl: None,
        };
    }

    // Phase18 §5.2 — the cross_project_ref template is invoked
    // directly by the orchestrator's propagation helper (not via the
    // strategies layer), so it is NOT included in the
    // `cypher_resolves_for_every_strategy_template` list above. This
    // test confirms the template resolves independently so a rename
    // in `cypher_for` is caught at compile-time.
    #[test]
    fn cross_project_ref_template_resolves() {
        assert!(
            cypher_for("cross_project_ref").is_some(),
            "cross_project_ref must resolve to a Cypher string",
        );
    }

    // ── Phase21 §5.4 — cypher_with_acl / primary_node_alias ─────────

    #[test]
    fn primary_node_alias_maps_each_template() {
        assert_eq!(primary_node_alias("edge_artifact_touched_neighbours"), Some("a"));
        assert_eq!(primary_node_alias("decision_supersedes_chain"), Some("d"));
        assert_eq!(primary_node_alias("turn_analysis_decision_chain"), Some("an"));
        assert_eq!(primary_node_alias("law_violations_last_30d"), Some("v"));
        // Structural template — no content-node ACL filter.
        assert_eq!(primary_node_alias("cross_project_ref"), None);
        // Unknown — no alias.
        assert_eq!(primary_node_alias("unknown_template"), None);
    }

    #[test]
    fn cypher_with_acl_injects_before_return_keyword() {
        use crate::lanes::AclContext;
        let acl = AclContext {
            clearance_level: 1,
            compartment_grants: vec!["hr".to_string()],
        };
        let base = cypher_for("decision_supersedes_chain").unwrap();
        let cy = cypher_with_acl(base, "decision_supersedes_chain", Some(&acl));
        // Level clause uses alias 'd'.
        assert!(
            cy.contains("d.class_level IS NULL OR d.class_level <= 1"),
            "level clause missing: {cy}"
        );
        // Compartment clause uses alias 'd'.
        assert!(
            cy.contains("d.class_compartments"),
            "compartment clause missing: {cy}"
        );
        // Injection is before RETURN, not after.
        let inject_pos = cy.find("d.class_level").unwrap();
        let return_pos = cy.find("RETURN").unwrap();
        assert!(inject_pos < return_pos, "injection must precede RETURN");
    }

    #[test]
    fn cypher_with_acl_wildcard_emits_level_clause_only() {
        use crate::lanes::AclContext;
        let acl = AclContext {
            clearance_level: 3,
            compartment_grants: vec!["*".to_string()],
        };
        let base = cypher_for("law_violations_last_30d").unwrap();
        let cy = cypher_with_acl(base, "law_violations_last_30d", Some(&acl));
        assert!(
            cy.contains("v.class_level IS NULL OR v.class_level <= 3"),
            "level clause missing: {cy}"
        );
        assert!(
            !cy.contains("class_compartments"),
            "wildcard must omit compartment clause: {cy}"
        );
    }

    #[test]
    fn cypher_with_no_acl_returns_base_unchanged() {
        let base = cypher_for("edge_artifact_touched_neighbours").unwrap();
        let cy = cypher_with_acl(base, "edge_artifact_touched_neighbours", None);
        assert_eq!(cy, base, "no ACL must return base cypher verbatim");
    }

    #[test]
    fn cypher_with_acl_cross_project_ref_unchanged() {
        use crate::lanes::AclContext;
        let acl = AclContext {
            clearance_level: 2,
            compartment_grants: vec!["financial".to_string()],
        };
        let base = cypher_for("cross_project_ref").unwrap();
        let cy = cypher_with_acl(base, "cross_project_ref", Some(&acl));
        // cross_project_ref has no primary alias → base returned unchanged.
        assert_eq!(cy, base, "structural template must not be ACL-filtered");
    }
}
