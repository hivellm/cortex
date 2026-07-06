use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use nexus_sdk::{NexusClient, Value as NexusValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    clip, collect_lane_hits, normalize_repo, symbol_to_kind, title_from_hit, DashboardState,
};
use crate::lanes::MemoryKeywordLane;

/// Graph payload mirroring `MOCK.graph` — explicit `nodes` + `edges`
/// arrays the SPA's inline SVG renderer consumes directly.
#[derive(Debug, Clone, Serialize)]
pub struct GraphPayload {
    /// Nodes laid out in canvas-space.
    pub nodes: Vec<GraphNode>,
    /// Directed edges between node ids.
    pub edges: Vec<GraphEdge>,
}

/// One graph node.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    /// Node id (`session-...`, `turn-...`, `tool_call-...`, etc.).
    pub id: String,
    /// Display label.
    pub label: String,
    /// X coordinate in the SVG viewBox (0..820).
    pub x: i32,
    /// Y coordinate in the SVG viewBox (0..400).
    pub y: i32,
    /// `session` | `turn` | `tool_call` | `decision` | `law` | `violation` | `analysis` | `artifact`.
    pub kind: String,
}

/// One graph edge.
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Edge label (e.g. `INVOKED`, `WROTE`, `OBSERVED_IN`).
    pub label: String,
    /// Phase27a §3.2 — edge-confidence tier (`extracted` / `inferred`
    /// / `ambiguous`) read off the Nexus edge property. Lets the SPA
    /// style low-confidence edges distinctly (e.g. dash + amber for
    /// `ambiguous`) so a reviewer can spot inferred links at a glance.
    /// `None` when the edge predates phase27a or the synthetic
    /// lane-fallback path produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

/// Query params for `/v1/dashboard/graph`.
#[derive(Debug, Default, Deserialize)]
pub struct GraphQuery {
    /// Restrict to one session — when set, the Cypher MATCH anchors
    /// at `Session {session_id: $sid}`. When unset, the handler
    /// returns the most-recently-active subgraph capped at `limit`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Restrict to one or more repos. Each `repo=<name>` query param
    /// is appended; the filter passes when the artifact's owning
    /// Repo matches ANY of the listed repos. Other kinds (Session /
    /// Decision / Memory / Law / Analysis) are kept regardless so
    /// the cross-project knowledge spine stays visible even when
    /// the user is drilling into a single repo's artifacts.
    #[serde(default)]
    pub repo: Vec<String>,
    /// Cap the total node count. Defaults to 200, max 50,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub(super) async fn graph(
    State(state): State<DashboardState>,
    Query(params): Query<GraphQuery>,
) -> Response {
    // The cap is intentionally generous: the explorer's "show me the
    // whole panorama" mode needs to walk every Repo / Session / Turn /
    // ToolCall / Artifact at once. Sigma WebGL renders 30k+ nodes at
    // 60fps; the network round-trip at this size is ~200 KB gzipped.
    // The `unwrap_or` default stays small so unauthenticated probes
    // don't accidentally pull the full graph.
    let limit = params.limit.unwrap_or(200).clamp(1, 50_000);
    let repo_filter: Vec<String> = params
        .repo
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| normalize_repo(s))
        .collect();

    // Live path: when a Nexus client is configured, run a real
    // Cypher MATCH and convert the returned rows into the GraphPayload
    // shape the GUI consumes. On any failure (transport, schema, empty
    // result), fall through to the synthetic-from-lane fallback so a
    // dev environment without a populated Nexus still renders
    // something useful.
    if let Some(nx) = state.nexus.as_ref() {
        match query_nexus_graph(
            nx.as_ref(),
            params.session_id.as_deref(),
            &repo_filter,
            limit,
        )
        .await
        {
            Ok(payload) if !payload.nodes.is_empty() => {
                return (StatusCode::OK, Json(payload)).into_response();
            }
            Ok(_) => {
                tracing::debug!("nexus returned an empty graph; falling back to lane synthesis");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "nexus graph query failed; falling back to lane synthesis"
                );
            }
        }
    }

    let payload = synthesize_graph_from_lane(&state.lane, limit);
    (StatusCode::OK, Json(payload)).into_response()
}

/// Run several Cypher MATCH passes against Nexus and assemble a
/// [`GraphPayload`].
///
/// Nexus 1.15 returns nodes as flat `{_nexus_id, ...properties}`
/// objects (no nested `properties` field, no `labels` array) and
/// returns relationships as `{_nexus_id, type}` (no `start`/`end`).
/// We therefore pull each label and edge type with explicit
/// `RETURN`-projected columns rather than parsing whole-node objects,
/// which means this code already works for the seven node labels and
/// three edge types the writer produces today (`Session`, `Turn`,
/// `Artifact`, `Repo`, `Memory`, `LawViolation`, `Decision` /
/// `IN_REPO`, `HAS_TURN`, `REMEMBERS`). New labels just need a fresh
/// pass below; new edges only need a fresh `MATCH ... RETURN`.
async fn query_nexus_graph(
    client: &NexusClient,
    session_id: Option<&str>,
    repo_filter: &[String],
    limit: usize,
) -> anyhow::Result<GraphPayload> {
    let mut nodes_by_id: std::collections::HashMap<String, GraphNode> =
        std::collections::HashMap::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_edges: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Build a HashSet for O(1) membership; the loop below short-
    // circuits the whole-Cypher round-trip when the filter rejects.
    let repo_set: std::collections::HashSet<&str> =
        repo_filter.iter().map(String::as_str).collect();

    // Edge-first sampling. The previous node-first strategy pulled
    // each label independently (10 Sessions, 18 Turns, 18 Memories…)
    // and then filtered edges to those whose endpoints happened to
    // overlap. With independent random samples the overlap was
    // near-zero — a fresh DB returned 98 nodes and 23 edges, mostly
    // because Sessions and their Turns weren't co-sampled.
    //
    // The fix: each MATCH returns BOTH endpoints' id + display label
    // in one round-trip, and we materialize the nodes inline from
    // the edge rows. Every edge we keep is guaranteed to have its
    // endpoints rendered.
    //
    // Tuple: (from_label, from_id_prop, from_label_prop, from_kind,
    //         rel, to_label, to_id_prop, to_label_prop, to_kind)
    // `*_label_prop` resolves the human-readable display name for
    // each endpoint. The mapper writes `name` on every node kind
    // (Turn = user message, Decision = title, Session/Memory = best-
    // effort short label). Defaulting to `id` made the canvas a wall
    // of ULIDs — `name` lets the explorer show "fixes" / "docs:
    // move..." instead of "01KQ8534N93Y1MA28...".
    #[allow(clippy::type_complexity)]
    let edge_specs: &[(&str, &str, &str, &str, &str, &str, &str, &str, &str)] = &[
        (
            "Session", "id", "name", "session", "HAS_TURN", "Turn", "id", "name", "turn",
        ),
        (
            "Session",
            "id",
            "name",
            "session",
            "REMEMBERS",
            "Memory",
            "id",
            "name",
            "memory",
        ),
        (
            "Turn",
            "id",
            "name",
            "turn",
            "HAS_TOOL_CALL",
            "ToolCall",
            "id",
            "name",
            "tool_call",
        ),
        // Session→ToolCall is the mapper's fallback anchor for tool_call
        // events without a `parent_event_id` (bootstrap envelopes ship
        // without one — see `cortex-graph/src/mapper.rs::emit_tool_call`).
        // Without this row the dashboard misses every backfilled
        // HAS_TOOL_CALL because the canonical `(:Turn)-[:HAS_TOOL_CALL]->`
        // pattern matches zero on bootstrap data.
        (
            "Session",
            "id",
            "name",
            "session",
            "HAS_TOOL_CALL",
            "ToolCall",
            "id",
            "name",
            "tool_call",
        ),
        (
            "Turn",
            "id",
            "name",
            "turn",
            "HAS_AGENT_CALL",
            "AgentCall",
            "id",
            "name",
            "agent_call",
        ),
        (
            "Session",
            "id",
            "name",
            "session",
            "HAS_AGENT_CALL",
            "AgentCall",
            "id",
            "name",
            "agent_call",
        ),
        // Phase11l §8 — Artifact identity moved from the synthetic
        // `natural_key` property to Nexus 2.1's reserved `_id` slot.
        // The dashboard reads through `a._id` so the graph view
        // colour-coding stays accurate post-migration. The legacy
        // `natural_key` property still ships on the node for one
        // soft-fallback release; this read path follows the
        // canonical identity.
        (
            "ToolCall",
            "id",
            "name",
            "tool_call",
            "TOUCHED",
            "Artifact",
            "_id",
            "name",
            "artifact",
        ),
        (
            "Artifact", "_id", "name", "artifact", "IN_REPO", "Repo", "name", "name", "repo",
        ),
        (
            "LawViolation",
            "id",
            "name",
            "violation",
            "OBSERVED_IN",
            "Turn",
            "id",
            "name",
            "turn",
        ),
        (
            "LawViolation",
            "id",
            "name",
            "violation",
            "OF",
            "Law",
            "id",
            "name",
            "law",
        ),
        (
            "Decision",
            "id",
            "name",
            "decision",
            "SUPERSEDES",
            "Decision",
            "id",
            "name",
            "decision",
        ),
    ];

    // Per-relation budget. Each MATCH gets the full node-count cap so
    // the rarest relationships land first and the densest one
    // (IN_REPO, ~28k rows on Cortex) is allowed to fill the rest of
    // the budget. The early-break on `nodes_by_id.len() >= limit`
    // inside the row loop bounds total memory regardless of
    // per-relation supply. The integer is inlined into the query
    // string because the Nexus Cypher dialect ignores `LIMIT $param`.
    let per_rel_limit = limit;

    for (from_label, from_id_p, from_lbl_p, from_kind, rel, to_label, to_id_p, to_lbl_p, to_kind) in
        edge_specs
    {
        if nodes_by_id.len() >= limit {
            break;
        }
        // Session-anchored mode: restrict the edge's "from" side to
        // descendants of the requested Session. Cheaper than a full
        // 3-hop path because each rel-spec already names the depth.
        // Phase27a §3.2 — every variant also projects `r.confidence`
        // (cell 4) so the edge payload can carry the tier. Edges
        // without the property surface it as null → `None`.
        let cy = match session_id {
            Some(_) if *from_label == "Session" => format!(
                "MATCH (a:Session {{ session_id: $sid }})-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl, \
                        r.confidence AS conf \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "Turn" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(a:Turn)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl, \
                        r.confidence AS conf \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "ToolCall" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(:Turn)-[:HAS_TOOL_CALL]->(a:ToolCall)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl, \
                        r.confidence AS conf \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "LawViolation" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(t:Turn)<-[:OBSERVED_IN]-(a:LawViolation)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl, \
                        r.confidence AS conf \
                 LIMIT {per_rel_limit}"
            ),
            // Decision/SUPERSEDES and Artifact/IN_REPO are not
            // session-scoped — skip when filtering by session.
            Some(_) => continue,
            None => format!(
                "MATCH (a:{from_label})-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl, \
                        r.confidence AS conf \
                 LIMIT {per_rel_limit}"
            ),
        };

        let mut params: std::collections::HashMap<String, NexusValue> =
            std::collections::HashMap::new();
        if let Some(sid) = session_id {
            params.insert("sid".to_string(), NexusValue::String(sid.to_string()));
        }

        let res = match client.execute_cypher(&cy, Some(params)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    rel,
                    error = %e,
                    "nexus edge pull failed; skipping this relationship"
                );
                continue;
            }
        };

        for row in &res.rows {
            let cells = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            let from_id = cell_str(cells.first()).unwrap_or_default();
            let to_id = cell_str(cells.get(2)).unwrap_or_default();
            if from_id.is_empty() || to_id.is_empty() {
                continue;
            }
            let from_lbl = cell_str(cells.get(1))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| from_id.clone());
            let to_lbl = cell_str(cells.get(3))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| to_id.clone());

            // Repo-filter slice. The filter is applied at the edge-row
            // level so the per-relationship Cypher stays a generic
            // pattern — IN_REPO edges drop unless the destination
            // Repo is in the allow-list, and TOUCHED edges drop
            // unless the artifact's `repo|path|hash` natural key
            // starts with one of the filtered repos. Other edge types
            // (HAS_TURN, REMEMBERS, OBSERVED_IN, etc.) are
            // project-agnostic and pass through so the cross-project
            // session-tree spine stays visible even in a single-repo
            // drill-down.
            if !repo_set.is_empty() {
                if *rel == "IN_REPO" {
                    if !repo_set.contains(normalize_repo(&to_id).as_str()) {
                        continue;
                    }
                } else if *rel == "TOUCHED" {
                    let artifact_repo = to_id.split('|').next().unwrap_or("");
                    if !repo_set.contains(normalize_repo(artifact_repo).as_str()) {
                        continue;
                    }
                }
            }

            nodes_by_id
                .entry(from_id.clone())
                .or_insert_with(|| GraphNode {
                    id: from_id.clone(),
                    label: clip(&from_lbl, 64),
                    x: 0,
                    y: 0,
                    kind: (*from_kind).to_string(),
                });
            nodes_by_id
                .entry(to_id.clone())
                .or_insert_with(|| GraphNode {
                    id: to_id.clone(),
                    label: clip(&to_lbl, 64),
                    x: 0,
                    y: 0,
                    kind: (*to_kind).to_string(),
                });

            // Phase27a §3.2 — cell 4 carries the edge confidence tier
            // (null for pre-phase27a edges → None).
            let confidence = cell_str(cells.get(4)).filter(|s| !s.is_empty());
            let key = format!("{from_id}|{rel}|{to_id}");
            if seen_edges.insert(key) {
                edges.push(GraphEdge {
                    from: from_id,
                    to: to_id,
                    label: (*rel).to_string(),
                    confidence,
                });
            }
            if nodes_by_id.len() >= limit {
                break;
            }
        }
    }

    // Top-up pass — surface isolated nodes (Decisions without a
    // SUPERSEDES, Laws without a violation, etc.) so the graph
    // doesn't show 100% session-tree and hide the rest of the
    // domain. Skipped when session-anchored: in that mode we want
    // a focused subgraph, not a domain dump.
    if session_id.is_none() && nodes_by_id.len() < limit {
        // Same `name`-as-label convention as the edge_specs above —
        // the mapper writes a human-readable `name` on every kind.
        let label_specs: &[(&str, &str, &str, &str)] = &[
            ("Decision", "id", "name", "decision"),
            ("Law", "id", "name", "law"),
            ("Analysis", "id", "name", "analysis"),
            ("Repo", "name", "name", "repo"),
            ("Memory", "id", "name", "memory"),
        ];
        let remaining = limit - nodes_by_id.len();
        let per_label = (remaining / label_specs.len()).max(3);

        for (cypher_label, id_prop, label_prop, gui_kind) in label_specs {
            if nodes_by_id.len() >= limit {
                break;
            }
            let cy = format!(
                "MATCH (n:{cypher_label}) \
                 RETURN n.{id_prop} AS id, n.{label_prop} AS label \
                 LIMIT {per_label}"
            );
            let res = match client
                .execute_cypher(
                    &cy,
                    Some(std::collections::HashMap::<String, NexusValue>::new()),
                )
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in &res.rows {
                let cells = match row.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                let id = cell_str(cells.first()).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                // Skip Repo nodes outside the repo filter (when set).
                // Other kinds pass through — Decisions / Laws /
                // Memories cross repo boundaries and dropping them
                // would gut the cross-project knowledge spine.
                if !repo_set.is_empty()
                    && *gui_kind == "repo"
                    && !repo_set.contains(normalize_repo(&id).as_str())
                {
                    continue;
                }
                let label = cell_str(cells.get(1))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| id.clone());
                nodes_by_id.entry(id.clone()).or_insert_with(|| GraphNode {
                    id,
                    label: clip(&label, 64),
                    x: 0,
                    y: 0,
                    kind: (*gui_kind).to_string(),
                });
                if nodes_by_id.len() >= limit {
                    break;
                }
            }
        }
    }

    let nodes: Vec<GraphNode> = nodes_by_id.into_values().collect();
    Ok(GraphPayload { nodes, edges })
}

// ---------------------------------------------------------------------------
// Phase27b §3.1/§3.2 — community surface
//
// Reads the `community_id` / `community_level` / `is_god_node` node
// properties the phase27b §2.4 writeback mapper (`community_node_ops`
// in `cortex-workers`) stamps onto architecture nodes, once the §2.5
// worker (blocked on the semantic graph projection — ADR-027) actually
// runs. Until then no node carries these properties, so both queries
// below return zero rows and this handler answers an honest empty
// payload instead of an error — never a client-visible failure just
// because the upstream projection isn't live yet.
//
// §3.2 (dashboard "subsystems" view) reuses this exact endpoint as its
// data source rather than standing up a duplicate query — the shape
// (communities + god nodes + cross-community edges) is the same data
// a "subsystems" view needs; a dedicated frontend rendering of it is a
// follow-up, not part of this backend slice.
// ---------------------------------------------------------------------------

/// One detected community and its members, as surfaced by
/// `GET /v1/dashboard/graph/communities`.
#[derive(Debug, Clone, Serialize)]
pub struct CommunitySummary {
    /// Community id stamped by the phase27b Leiden/Louvain worker.
    pub community_id: u32,
    /// Hierarchy level the community was last updated at (0 = base level).
    pub level: u32,
    /// Total number of member nodes (including god nodes).
    pub member_count: usize,
    /// God nodes (hub-percentile-excluded, re-attached by majority
    /// vote — see `is_god_node` in `cortex-workers::graph::community`)
    /// that landed in this community.
    pub god_nodes: Vec<CommunityGodNode>,
}

/// One god node within a [`CommunitySummary`].
#[derive(Debug, Clone, Serialize)]
pub struct CommunityGodNode {
    /// Node identity (`_id`, Nexus 2.1 reserved slot).
    pub id: String,
    /// Human-readable display name (falls back to `id` when absent).
    pub name: String,
}

/// A "surprise" edge connecting two nodes in different communities.
#[derive(Debug, Clone, Serialize)]
pub struct CrossCommunityEdge {
    /// Source node id.
    pub from: String,
    /// Source node's community id.
    pub from_community: u32,
    /// Target node id.
    pub to: String,
    /// Target node's community id.
    pub to_community: u32,
    /// Edge relationship type (`type(r)` in Cypher).
    pub relation: String,
}

/// Response payload for `GET /v1/dashboard/graph/communities`.
#[derive(Debug, Clone, Serialize)]
pub struct CommunitiesPayload {
    /// Every detected community, sorted by `community_id` ascending.
    pub communities: Vec<CommunitySummary>,
    /// Cross-community edges ("surprises"), sorted for determinism.
    pub cross_community_edges: Vec<CrossCommunityEdge>,
}

/// Query params for `/v1/dashboard/graph/communities`.
#[derive(Debug, Default, Deserialize)]
pub struct CommunitiesQuery {
    /// Restrict to one hierarchy level. Unset returns every level.
    #[serde(default)]
    pub level: Option<u32>,
    /// Cap the number of member rows / edge rows pulled per query.
    /// Defaults to 500, max 20,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub(super) async fn communities(
    State(state): State<DashboardState>,
    Query(params): Query<CommunitiesQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(500).clamp(1, 20_000);

    if let Some(nx) = state.nexus.as_ref() {
        match query_nexus_communities(nx.as_ref(), params.level, limit).await {
            Ok(payload) => return (StatusCode::OK, Json(payload)).into_response(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "nexus community query failed; returning empty result"
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(CommunitiesPayload {
            communities: Vec::new(),
            cross_community_edges: Vec::new(),
        }),
    )
        .into_response()
}

/// Run the two Cypher passes behind `communities()`: one gathering
/// every node carrying `community_id` (grouped into
/// [`CommunitySummary`] rows with their god nodes), one gathering
/// edges whose endpoints sit in different communities.
///
/// Label-less `MATCH (n)` / `MATCH (a)-[r]->(b)` Cypher is already
/// precedented in this codebase (`graph/indexer.rs`,
/// `lanes/nexus_graph_lane.rs`) — community membership is not scoped
/// to one Cypher label, so neither query anchors on one.
async fn query_nexus_communities(
    client: &NexusClient,
    level: Option<u32>,
    limit: usize,
) -> anyhow::Result<CommunitiesPayload> {
    let node_level_clause = level
        .map(|l| format!(" AND n.community_level = {l}"))
        .unwrap_or_default();
    let members_cypher = format!(
        "MATCH (n) WHERE n.community_id IS NOT NULL{node_level_clause} \
         RETURN n._id AS id, n.name AS name, n.community_id AS community_id, \
                n.community_level AS level, n.is_god_node AS is_god_node \
         LIMIT {limit}"
    );
    let members_res = client
        .execute_cypher(
            &members_cypher,
            Some(std::collections::HashMap::<String, NexusValue>::new()),
        )
        .await?;

    let mut by_community: std::collections::BTreeMap<u32, CommunitySummary> =
        std::collections::BTreeMap::new();
    for row in &members_res.rows {
        let cells = match row.as_array() {
            Some(a) => a,
            None => continue,
        };
        let id = match cell_str(cells.first()) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let Some(community_id) = cell_u32(cells.get(2)) else {
            continue;
        };
        let name = cell_str(cells.get(1))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.clone());
        let node_level = cell_u32(cells.get(3)).unwrap_or(0);
        let is_god_node = cell_bool(cells.get(4)).unwrap_or(false);

        let entry = by_community
            .entry(community_id)
            .or_insert_with(|| CommunitySummary {
                community_id,
                level: node_level,
                member_count: 0,
                god_nodes: Vec::new(),
            });
        entry.member_count += 1;
        if is_god_node {
            entry.god_nodes.push(CommunityGodNode { id, name });
        }
    }
    for summary in by_community.values_mut() {
        summary.god_nodes.sort_by(|a, b| a.id.cmp(&b.id));
    }

    let edge_level_clause = level
        .map(|l| format!(" AND a.community_level = {l} AND b.community_level = {l}"))
        .unwrap_or_default();
    let edges_cypher = format!(
        "MATCH (a)-[r]->(b) WHERE a.community_id IS NOT NULL \
         AND b.community_id IS NOT NULL AND a.community_id <> b.community_id{edge_level_clause} \
         RETURN a._id AS from_id, a.community_id AS from_community, \
                b._id AS to_id, b.community_id AS to_community, type(r) AS rel_type \
         LIMIT {limit}"
    );
    let edges_res = client
        .execute_cypher(
            &edges_cypher,
            Some(std::collections::HashMap::<String, NexusValue>::new()),
        )
        .await?;

    let mut cross_community_edges: Vec<CrossCommunityEdge> = Vec::new();
    for row in &edges_res.rows {
        let cells = match row.as_array() {
            Some(a) => a,
            None => continue,
        };
        let from = match cell_str(cells.first()) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let to = match cell_str(cells.get(2)) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let (Some(from_community), Some(to_community)) =
            (cell_u32(cells.get(1)), cell_u32(cells.get(3)))
        else {
            continue;
        };
        let relation = cell_str(cells.get(4)).unwrap_or_default();
        cross_community_edges.push(CrossCommunityEdge {
            from,
            from_community,
            to,
            to_community,
            relation,
        });
    }
    cross_community_edges
        .sort_by(|a, b| (&a.from, &a.to, &a.relation).cmp(&(&b.from, &b.to, &b.relation)));

    Ok(CommunitiesPayload {
        communities: by_community.into_values().collect(),
        cross_community_edges,
    })
}

/// Pull a row cell as `u32`, accepting a JSON number or a numeric
/// string. Returns `None` for null / missing / non-numeric cells.
fn cell_u32(v: Option<&serde_json::Value>) -> Option<u32> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok();
    }
    v.as_str()?.parse::<u32>().ok()
}

/// Pull a row cell as `bool`, accepting a JSON bool or the strings
/// `"true"` / `"false"`. Returns `None` for null / missing / other
/// cells (the caller defaults to `false`).
fn cell_bool(v: Option<&serde_json::Value>) -> Option<bool> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(b) = v.as_bool() {
        return Some(b);
    }
    match v.as_str() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

/// Pull a row cell as `String`, accepting either a JSON string or a
/// non-null scalar (number / bool) that we stringify. Returns `None`
/// for null / missing cells so the caller can substitute a fallback.
fn cell_str(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    Some(v.to_string())
}

/// Accumulator that dedups Nexus nodes/edges by id while turning
/// untyped `serde_json::Value` cells into typed [`GraphNode`] /
/// [`GraphEdge`]. Nexus returns nodes with shape
/// `{ "labels": [..], "properties": {..}, "_id": .. }` and edges
/// with shape `{ "type": "...", "start": .., "end": .., ... }` —
/// both surface here as `Value::Object`.
#[allow(dead_code)]
#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    seen_nodes: std::collections::HashSet<String>,
    seen_edges: std::collections::HashSet<String>,
}

#[allow(dead_code)]
impl GraphBuilder {
    fn add_node(&mut self, cell: Option<&Value>, default_kind: &str) {
        let obj = match cell.and_then(|v| v.as_object()) {
            Some(o) => o,
            None => return,
        };
        let props = obj
            .get("properties")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let id = match props
            .get("session_id")
            .or_else(|| props.get("turn_id"))
            .or_else(|| props.get("tool_call_id"))
            .or_else(|| props.get("event_id"))
            .or_else(|| obj.get("_id"))
            .and_then(|v| v.as_str().map(String::from).or_else(|| Some(v.to_string())))
        {
            Some(s) if !s.is_empty() => s,
            _ => return,
        };
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let kind = obj
            .get("labels")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(label_to_kind)
            .unwrap_or_else(|| default_kind.to_string());
        let label = node_label(&kind, &props, &id);
        // x/y are unused by the Cytoscape renderer (cose layout owns
        // positioning) but kept in the shape so older clients still
        // round-trip the response.
        self.nodes.push(GraphNode {
            id,
            label,
            x: 0,
            y: 0,
            kind,
        });
    }

    fn add_edge(&mut self, cell: Option<&Value>) {
        let obj = match cell.and_then(|v| v.as_object()) {
            Some(o) => o,
            None => return,
        };
        let from = obj.get("start").and_then(|v| v.as_str()).map(String::from);
        let to = obj.get("end").and_then(|v| v.as_str()).map(String::from);
        let rel = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("REL")
            .to_string();
        let (from, to) = match (from, to) {
            (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => (a, b),
            _ => return,
        };
        let key = format!("{from}|{rel}|{to}");
        if !self.seen_edges.insert(key) {
            return;
        }
        self.edges.push(GraphEdge {
            from,
            to,
            label: rel,
            confidence: None,
        });
    }

    fn into_payload(mut self, limit: usize) -> GraphPayload {
        if self.nodes.len() > limit {
            self.nodes.truncate(limit);
            let kept: std::collections::HashSet<&String> =
                self.nodes.iter().map(|n| &n.id).collect();
            self.edges
                .retain(|e| kept.contains(&e.from) && kept.contains(&e.to));
        }
        GraphPayload {
            nodes: self.nodes,
            edges: self.edges,
        }
    }
}

/// Map a Nexus node label (`Session`, `Turn`, `ToolCall`, …) to the
/// kebab kind the GUI consumes (`session`, `turn`, `tool_call`, …).
#[allow(dead_code)]
fn label_to_kind(label: &str) -> String {
    match label {
        "Session" => "session".to_string(),
        "Turn" => "turn".to_string(),
        "ToolCall" => "tool_call".to_string(),
        "AgentCall" => "agent_call".to_string(),
        "Decision" => "decision".to_string(),
        "Law" => "law".to_string(),
        "LawViolation" => "violation".to_string(),
        "Analysis" => "analysis".to_string(),
        "Artifact" => "artifact".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

/// Pick a human-readable label for a node based on the kind + props.
#[allow(dead_code)]
fn node_label(kind: &str, props: &serde_json::Map<String, Value>, fallback_id: &str) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| props.get(*k).and_then(|v| v.as_str()))
            .map(|s| clip(s, 48))
    };
    // The graph writer (cortex-graph mapper) now stamps a `name`
    // prop on every node — it's the single human-readable label
    // every kind ends up carrying. Prefer it; fall back to the
    // payload-specific fields only when `name` is absent (older
    // nodes written before the labelling change).
    if let Some(name) = props.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            return clip(name, 96);
        }
    }
    match kind {
        "session" => pick(&["title", "session_id"]).unwrap_or_else(|| "Session".to_string()),
        "turn" => pick(&["user_message", "text"]).unwrap_or_else(|| "Turn".to_string()),
        "tool_call" => pick(&["tool_name"])
            .map(|s| format!("[{s}]"))
            .unwrap_or_else(|| "ToolCall".to_string()),
        "agent_call" => pick(&["agent_type"])
            .map(|s| format!("Task: {s}"))
            .unwrap_or_else(|| "AgentCall".to_string()),
        "decision" => pick(&["title", "decision_id"]).unwrap_or_else(|| "Decision".to_string()),
        "analysis" => pick(&["title", "analysis_id"]).unwrap_or_else(|| "Analysis".to_string()),
        "law" => pick(&["title", "law_id"]).unwrap_or_else(|| "Law".to_string()),
        "violation" => pick(&["message", "law_id"]).unwrap_or_else(|| "Violation".to_string()),
        "artifact" => pick(&["path"]).unwrap_or_else(|| fallback_id.to_string()),
        _ => fallback_id.to_string(),
    }
}

/// Original lane-only graph synthesis — used as the fallback when no
/// Nexus client is configured or the live query fails.
fn synthesize_graph_from_lane(lane: &MemoryKeywordLane, limit: usize) -> GraphPayload {
    let hits = collect_lane_hits(lane);

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let session_id = "session-active";
    nodes.push(GraphNode {
        id: session_id.to_string(),
        label: "Session".to_string(),
        x: 60,
        y: 200,
        kind: "session".to_string(),
    });

    let mut turns_seen: std::collections::BTreeMap<String, (i32, i32)> =
        std::collections::BTreeMap::new();
    let mut tool_calls_seen = 0i32;
    let mut decisions_seen = 0i32;
    let mut analyses_seen = 0i32;
    let mut violations_seen = 0i32;
    let cap = limit.saturating_sub(1).min(60);

    for (idx, h) in hits.iter().enumerate().take(cap) {
        let kind = symbol_to_kind(h.symbol.as_deref());
        let label = title_from_hit(h);

        match kind {
            "turn" => {
                let id = format!("turn-{idx}");
                let y = 80 + (turns_seen.len() as i32) * 60;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 220,
                    y,
                    kind: kind.to_string(),
                });
                edges.push(GraphEdge {
                    from: session_id.to_string(),
                    to: id.clone(),
                    label: "CONTAINS".to_string(),
                    confidence: None,
                });
                turns_seen.insert(id, (220, y));
            }
            "tool_call" => {
                let id = format!("tool_call-{idx}");
                let y = 80 + tool_calls_seen * 50;
                tool_calls_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 24),
                    x: 420,
                    y,
                    kind: kind.to_string(),
                });
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: parent,
                    to: id,
                    label: "INVOKED".to_string(),
                    confidence: None,
                });
            }
            "decision" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("decision-{raw_id}");
                let y = 80 + decisions_seen * 60;
                decisions_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 720,
                    y,
                    kind: kind.to_string(),
                });
                // Anchor the decision under the most recent turn —
                // when no turn is around, hang it directly off the
                // session so the canvas stays connected.
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: parent,
                    to: id,
                    label: "REFERENCES".to_string(),
                    confidence: None,
                });
            }
            "analysis" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("analysis-{raw_id}");
                let y = 220 + analyses_seen * 60;
                analyses_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 720,
                    y,
                    kind: kind.to_string(),
                });
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: parent,
                    to: id,
                    label: "PRODUCED".to_string(),
                    confidence: None,
                });
            }
            "law_violation" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("violation-{raw_id}");
                let y = 320 + violations_seen * 50;
                violations_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 28),
                    x: 540,
                    y,
                    kind: "violation".to_string(),
                });
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: id.clone(),
                    to: parent,
                    label: "OBSERVED_IN".to_string(),
                    confidence: None,
                });
            }
            _ => {}
        }
    }

    GraphPayload { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_payload_carries_confidence_when_present_and_omits_when_none() {
        // Phase27a §3.2 — an ambiguous edge surfaces its tier so the
        // SPA can style it; a confidence-less edge keeps the lean
        // shape (field skipped) for back-compat with existing clients.
        let ambiguous = GraphEdge {
            from: "a".into(),
            to: "b".into(),
            label: "RELATED_TO".into(),
            confidence: Some("ambiguous".into()),
        };
        let plain = GraphEdge {
            from: "a".into(),
            to: "b".into(),
            label: "HAS_TURN".into(),
            confidence: None,
        };
        let amb_json = serde_json::to_value(&ambiguous).unwrap();
        assert_eq!(
            amb_json.get("confidence").and_then(|v| v.as_str()),
            Some("ambiguous"),
            "ambiguous edge must expose its tier to the graph view"
        );
        let plain_json = serde_json::to_value(&plain).unwrap();
        assert!(
            plain_json.get("confidence").is_none(),
            "confidence-less edge must omit the field, not emit null"
        );
    }

    #[test]
    fn cell_str_reads_confidence_cell_and_treats_null_as_absent() {
        // The edge-pull loop reads the tier from cell 4 via cell_str;
        // a null cell (pre-phase27a edge) yields None.
        let row = serde_json::json!(["f", "from", "t", "to", "inferred"]);
        let cells = row.as_array().unwrap();
        assert_eq!(cell_str(cells.get(4)).as_deref(), Some("inferred"));
        let null_row = serde_json::json!(["f", "from", "t", "to", null]);
        let null_cells = null_row.as_array().unwrap();
        assert!(cell_str(null_cells.get(4)).is_none());
    }
}
