//! Phase27c §1.1 — live community source.
//!
//! Reads the graph communities the phase27b writeback stamped onto
//! Nexus nodes (`community_id` / `community_level` / `is_god_node`)
//! and shapes them into [`CommunityInput`]s for
//! `Orchestrator::run_community`. Two label-less Cypher passes (same
//! shapes as the phase27b §3.1 dashboard endpoint): one over member
//! nodes, one over cross-community edges. Until the phase27b §2.5
//! writeback worker runs live (gated on the semantic projection —
//! ADR-027), both queries return zero rows and `fetch` yields an
//! empty vec, which the grain treats as a benign no-op.
//!
//! The row → input grouping is a pure function
//! ([`group_into_inputs`]) so the multi-level partition semantics
//! (§1.3 — one input per `(community_id, level)`) are fully
//! unit-testable without a live Nexus.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::consolidator::producer::community::{
    CommunityCrossEdge, CommunityInput, CommunityMember,
};
use crate::graph::nexus_client::LiveNexusClient;

use super::SourceError;

/// Row cap per Cypher pass — same default the phase27b dashboard
/// endpoint uses. A partition bigger than this gets clipped; the
/// producer's per-community MIN_COMMUNITY_SIZE gate and the inline
/// source-id cap keep the payload bounded regardless.
pub const COMMUNITY_FETCH_LIMIT: usize = 20_000;

/// One member row as projected out of the members Cypher pass.
/// Public so tests (and the grain's in-memory fetcher) can build
/// snapshots without a live Nexus.
#[derive(Debug, Clone)]
pub struct MemberRow {
    /// Node id (`_id`).
    pub id: String,
    /// Node label.
    pub label: String,
    /// Display name (falls back to `id`).
    pub name: String,
    /// Partition id.
    pub community_id: u32,
    /// Leiden hierarchy level.
    pub level: u32,
    /// God-node flag.
    pub is_god_node: bool,
}

/// One cross-community edge row from the edges Cypher pass.
#[derive(Debug, Clone)]
pub struct EdgeRow {
    /// Source node id.
    pub from: String,
    /// Source node's community.
    pub from_community: u32,
    /// Source node's level (cross edges only pair same-level nodes).
    pub level: u32,
    /// Target node id.
    pub to: String,
    /// Target node's community.
    pub to_community: u32,
    /// Relationship type.
    pub relation: String,
}

/// Pure grouping: rows → one [`CommunityInput`] per
/// `(level, community_id)`, members sorted by node id, cross edges
/// attached to their FROM community. Deterministic output order
/// (level asc, community_id asc) so re-runs derive identical
/// consolidation ids.
pub fn group_into_inputs(
    members: Vec<MemberRow>,
    edges: Vec<EdgeRow>,
    repo: &str,
    snapshot_ms: i64,
) -> Vec<CommunityInput> {
    let mut by_key: BTreeMap<(u32, u32), Vec<CommunityMember>> = BTreeMap::new();
    for m in members {
        by_key
            .entry((m.level, m.community_id))
            .or_default()
            .push(CommunityMember {
                id: m.id,
                label: m.label,
                name: m.name,
                is_god_node: m.is_god_node,
            });
    }
    let mut edges_by_key: BTreeMap<(u32, u32), Vec<CommunityCrossEdge>> = BTreeMap::new();
    for e in edges {
        edges_by_key
            .entry((e.level, e.from_community))
            .or_default()
            .push(CommunityCrossEdge {
                from: e.from,
                to: e.to,
                relation: e.relation,
                other_community: e.to_community,
            });
    }
    by_key
        .into_iter()
        .map(|((level, community_id), mut members)| {
            members.sort_by(|a, b| a.id.cmp(&b.id));
            let mut cross_edges = edges_by_key
                .remove(&(level, community_id))
                .unwrap_or_default();
            cross_edges.sort_by(|a, b| (a.from.as_str(), a.to.as_str()).cmp(&(&b.from, &b.to)));
            CommunityInput {
                community_id,
                level,
                repo: repo.to_string(),
                members,
                cross_edges,
                snapshot_ms,
            }
        })
        .collect()
}

/// Live source backed by the graph worker's [`LiveNexusClient`].
#[derive(Clone)]
pub struct LiveCommunitySource {
    client: Arc<LiveNexusClient>,
    limit: usize,
}

impl LiveCommunitySource {
    /// Build a live source over `client`.
    pub fn new(client: Arc<LiveNexusClient>) -> Self {
        Self {
            client,
            limit: COMMUNITY_FETCH_LIMIT,
        }
    }

    /// Override the per-pass row cap.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// Snapshot every community currently stamped on the graph.
    /// `repo` tags the resulting inputs (the community properties
    /// themselves are not repo-scoped — the projection is
    /// single-repo in practice today); `snapshot_ms` stamps the
    /// inputs' temporal anchor.
    pub async fn fetch(
        &self,
        repo: &str,
        snapshot_ms: i64,
    ) -> Result<Vec<CommunityInput>, SourceError> {
        let limit = self.limit;
        let members_cypher = format!(
            "MATCH (n) WHERE n.community_id IS NOT NULL \
             RETURN n._id AS id, labels(n) AS labels, n.name AS name, \
                    n.community_id AS community_id, n.community_level AS level, \
                    n.is_god_node AS is_god_node \
             LIMIT {limit}"
        );
        let members_res = self
            .client
            .execute_with_retry(&members_cypher, None)
            .await
            .map_err(|e| SourceError::Storage(format!("community members query: {e}")))?;

        let mut members = Vec::new();
        for row in &members_res.rows {
            let Some(cells) = row.as_array() else {
                continue;
            };
            let id = match cells.first().and_then(|c| c.as_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let label = cells
                .get(1)
                .and_then(|c| c.as_array())
                .and_then(|l| l.first())
                .and_then(|c| c.as_str())
                .unwrap_or("Node")
                .to_string();
            let name = cells
                .get(2)
                .and_then(|c| c.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&id)
                .to_string();
            let Some(community_id) = cells.get(3).and_then(|c| c.as_u64()) else {
                continue;
            };
            let level = cells.get(4).and_then(|c| c.as_u64()).unwrap_or(0);
            let is_god_node = cells.get(5).and_then(|c| c.as_bool()).unwrap_or(false);
            members.push(MemberRow {
                id,
                label,
                name,
                community_id: u32::try_from(community_id).unwrap_or(u32::MAX),
                level: u32::try_from(level).unwrap_or(0),
                is_god_node,
            });
        }

        let edges_cypher = format!(
            "MATCH (a)-[r]->(b) \
             WHERE a.community_id IS NOT NULL AND b.community_id IS NOT NULL \
               AND a.community_id <> b.community_id \
               AND a.community_level = b.community_level \
             RETURN a._id AS from_id, a.community_id AS from_community, \
                    a.community_level AS level, b._id AS to_id, \
                    b.community_id AS to_community, type(r) AS relation \
             LIMIT {limit}"
        );
        let edges_res = self
            .client
            .execute_with_retry(&edges_cypher, None)
            .await
            .map_err(|e| SourceError::Storage(format!("community edges query: {e}")))?;

        let mut edges = Vec::new();
        for row in &edges_res.rows {
            let Some(cells) = row.as_array() else {
                continue;
            };
            let (Some(from), Some(from_community), Some(to), Some(to_community)) = (
                cells.first().and_then(|c| c.as_str()),
                cells.get(1).and_then(|c| c.as_u64()),
                cells.get(3).and_then(|c| c.as_str()),
                cells.get(4).and_then(|c| c.as_u64()),
            ) else {
                continue;
            };
            let level = cells.get(2).and_then(|c| c.as_u64()).unwrap_or(0);
            let relation = cells
                .get(5)
                .and_then(|c| c.as_str())
                .unwrap_or("RELATED")
                .to_string();
            edges.push(EdgeRow {
                from: from.to_string(),
                from_community: u32::try_from(from_community).unwrap_or(u32::MAX),
                level: u32::try_from(level).unwrap_or(0),
                to: to.to_string(),
                to_community: u32::try_from(to_community).unwrap_or(u32::MAX),
                relation,
            });
        }

        Ok(group_into_inputs(members, edges, repo, snapshot_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, community: u32, level: u32, god: bool) -> MemberRow {
        MemberRow {
            id: id.into(),
            label: "Symbol".into(),
            name: format!("name-{id}"),
            community_id: community,
            level,
            is_god_node: god,
        }
    }

    #[test]
    fn group_into_inputs_splits_by_community_and_level() {
        // §1.3 — the same community_id at two levels is two inputs.
        let members = vec![
            member("a", 1, 0, false),
            member("b", 1, 0, true),
            member("c", 2, 0, false),
            member("d", 1, 1, false),
        ];
        let inputs = group_into_inputs(members, Vec::new(), "cortex", 500);
        assert_eq!(inputs.len(), 3);
        // Deterministic order: (level 0, c1), (level 0, c2), (level 1, c1).
        assert_eq!((inputs[0].level, inputs[0].community_id), (0, 1));
        assert_eq!((inputs[1].level, inputs[1].community_id), (0, 2));
        assert_eq!((inputs[2].level, inputs[2].community_id), (1, 1));
        assert_eq!(inputs[0].members.len(), 2);
        assert!(inputs[0].members.iter().any(|m| m.is_god_node));
        assert!(inputs.iter().all(|i| i.repo == "cortex"));
        assert!(inputs.iter().all(|i| i.snapshot_ms == 500));
    }

    #[test]
    fn group_into_inputs_attaches_cross_edges_to_from_community() {
        let members = vec![member("a", 1, 0, false), member("b", 2, 0, false)];
        let edges = vec![EdgeRow {
            from: "a".into(),
            from_community: 1,
            level: 0,
            to: "b".into(),
            to_community: 2,
            relation: "CALLS".into(),
        }];
        let inputs = group_into_inputs(members, edges, "cortex", 0);
        assert_eq!(inputs.len(), 2);
        let c1 = &inputs[0];
        assert_eq!(c1.community_id, 1);
        assert_eq!(c1.cross_edges.len(), 1);
        assert_eq!(c1.cross_edges[0].other_community, 2);
        assert_eq!(c1.cross_edges[0].relation, "CALLS");
        let c2 = &inputs[1];
        assert!(
            c2.cross_edges.is_empty(),
            "edge belongs to its FROM community only"
        );
    }

    #[test]
    fn group_into_inputs_empty_rows_yield_empty_vec() {
        // The realistic live state today (projection gated off).
        let inputs = group_into_inputs(Vec::new(), Vec::new(), "cortex", 0);
        assert!(inputs.is_empty());
    }

    #[test]
    fn group_into_inputs_is_deterministic_members_sorted() {
        let members = vec![
            member("z", 1, 0, false),
            member("a", 1, 0, false),
            member("m", 1, 0, false),
        ];
        let inputs = group_into_inputs(members, Vec::new(), "cortex", 0);
        let ids: Vec<&str> = inputs[0].members.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "m", "z"]);
    }
}
