# 45 — Graph communities (detection + read surface)

## Goal

Turn Cortex's rich Nexus graph edges into a navigable architecture map: detect
communities (subsystem clusters) over the graph, and expose them to agents
(MCP tool) and operators (dashboard) so questions like "what are the major
subsystems and how do they relate" have an answer, instead of only supporting
node-to-node traversal.

## Scope

- In: the community-detection algorithm (Leiden-style, in-process Rust over a
  Nexus snapshot), the idempotent writeback of `community_id` /
  `community_level` / `is_god_node` onto existing architecture nodes, and the
  read-side surface (MCP tool + dashboard endpoint) that queries those
  properties.
- Out: the periodic worker that actually snapshots the live graph and runs
  the writeback (phase27b §2.5) — blocked on the semantic graph projection
  being enabled in production (ADR-027; tracked as `phase29_graph-projection-
  unblock`). Out: a dedicated frontend GUI page rendering the subsystems view
  (the backend data endpoint is in scope; rendering it is a follow-up).

## Inputs / Outputs

**Detection (`crates/cortex-workers/src/graph/community.rs`, phase27b §2.1-2.4, already shipped):**
- Input: a `CommunityGraph` (in-memory, built from a Nexus snapshot of the
  architecture-bearing subgraph — code/semantic edges, session plumbing
  down-weighted) + a `CommunityConfig` (oversized-community split threshold,
  hub-percentile cutoff).
- Output: a `CommunityResult` (`HashMap<NodeId, NodeAssignment>`), where each
  `NodeAssignment` carries `community_id: u32`, `level: u32`, `is_hub: bool`.
- `community_node_ops(result, label_for) -> Vec<NodeOp>` maps that result to
  idempotent node-property writes (`ConflictPolicy::Match`, sets only
  `community_id`/`community_level`/`is_god_node`) — the same `NodeOp` surface
  every other graph writer uses (spec 07).

**Read surface (this spec, phase27b §3.1/§3.2):**
- `GET /v1/dashboard/graph/communities?level=<u32>&limit=<usize>` (cortex-api)
  — returns `{ communities: [{community_id, level, member_count, god_nodes:
  [{id, name}]}], cross_community_edges: [{from, from_community, to,
  to_community, relation}] }`. Always `200 OK`; an absent Nexus client, a
  Cypher error, or zero matching rows all resolve to empty arrays, never a
  5xx.
- MCP tool `cortex_graph_communities` (optional `level`, `limit`) proxies to
  the endpoint above.

## Design

The read surface runs two label-less Cypher passes against Nexus (precedented
elsewhere in this codebase — community membership is not scoped to one node
label):

1. **Members**: `MATCH (n) WHERE n.community_id IS NOT NULL [AND
   n.community_level = $level] RETURN n._id, n.name, n.community_id,
   n.community_level, n.is_god_node LIMIT $limit` — grouped client-side into
   one `CommunitySummary` per `community_id`, with `is_god_node = true` rows
   collected as that community's god nodes.
2. **Cross-community edges**: `MATCH (a)-[r]->(b) WHERE a.community_id IS NOT
   NULL AND b.community_id IS NOT NULL AND a.community_id <> b.community_id
   RETURN a._id, a.community_id, b._id, b.community_id, type(r) LIMIT
   $limit` — the "surprise" edges connecting otherwise-separate subsystems.

Both queries return zero rows today (2026-07) because the phase27b §2.5
writeback worker that would populate `community_id` on live nodes has not run
yet — it's gated on the semantic graph projection (`ADR-027`,
`phase29_graph-projection-unblock`). The handler treats "no Nexus client",
"Cypher error", and "zero rows" identically: an honest empty payload. This is
intentional — the read surface is complete and tested against both the
empty-graph reality of today and a synthetic non-empty graph, so no further
change is needed here once §2.5 ships; the data will simply start populating.

## Acceptance criteria

- [x] `communities()` handler returns `200 OK` with empty `communities` /
      `cross_community_edges` arrays when: no Nexus client is configured, the
      Cypher query errors, or zero nodes carry `community_id`.
- [x] `communities()` correctly groups members by `community_id`, separates
      god nodes, and surfaces cross-community edges given synthetic non-empty
      Nexus data (`crates/cortex-api/tests/graph_communities_it.rs`).
- [x] `cortex_graph_communities` MCP tool descriptor honestly states results
      are empty until the writeback worker is live; `call()` proxies `level`
      and `limit` as query params to the dashboard endpoint
      (`crates/cortex-mcp-server/src/tools.rs`).
- [x] Tool registry count and its two hardcoded-count assertions
      (`tools.rs`, `server.rs`, `transport_stdio.rs`) updated to include the
      new tool (37 → 38).
- [ ] §2.5's writeback worker actually runs against a live, non-empty
      architecture subgraph (tracked separately, gated on
      `phase29_graph-projection-unblock`).
- [ ] A dashboard GUI page renders this endpoint's data as a "subsystems"
      view (follow-up, not yet built).

## Open questions

1. **Frontend rendering.** No dedicated GUI page exists yet for the
   subsystems view — the backend endpoint is complete and tested, but a
   follow-up task is needed to actually render it (graph explorer overlay?
   dedicated view? TBD when picked up).
2. **Cross-community edge volume at scale.** Once §2.5 populates a
   production-size graph, the unbounded `LIMIT $limit` cross-community-edge
   query may need pagination or a stricter default cap — revisit once real
   data exists to measure against.

## References

- Spec 07 (graph writer) — Decision 9, the `community_id` /
  `community_level` / `is_god_node` node-property contract this spec reads.
- Spec 20 (MCP tool surface registry) — `cortex_graph_communities` entry.
- ADR-027 — semantic graph projection gating (nexus#12), the blocker for
  phase27b §2.5's live writeback.
- `docs/analysis/graphify-comparison/02-leiden-community-detection.md` — the
  source analysis this task chain originates from.
- `.rulebook/tasks/phase27b_graph-community-detection/` — proposal, tasks,
  and this spec's originating task.
