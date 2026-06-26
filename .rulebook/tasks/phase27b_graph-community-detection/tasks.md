## 0. Spike finding (2026-06-21) — read before §2
The live Nexus graph has NO architecture edges (CALLS/IMPORTS/DEFINES) — only structural kinds (IN_REPO 8078, HAS_TOOL_CALL 2982, TOUCHED 1677, HAS_TURN 1611, EMITTED_BY 366, ABOUT 28, REMEMBERS 5). The semantic edges come from `graph/projection.rs`, gated OFF in prod (`CORTEX_GRAPH_PROJECTION_ENABLED=false`, nexus#12 → phase25). So the partition INPUT does not exist live until the projection is enabled. The algorithm + guards are offline-unit-testable now; live value (and §2.4 writeback / §2.5 cron / §3 surface verification) is GATED on the projection. See ADR-027.

## 1. Spike + decision
- [x] 1.1 DONE: spiked the inputs (architecture edges absent live — projection-gated) + the placement options. No mature Rust Leiden crate; Nexus has no community-detection procedure → in-process Rust over a Nexus snapshot.
- [x] 1.2 DONE: ADR-027 (`graph-community-detection-in-process-rust-leiden-over-a-nexus-snapshot-gated-on-the-semantic-projection`) records the algorithm/placement choice + the projection gating (phase25 is the de-facto unblocker for the 27b/27c track).

## 2. Community detection worker
> STATUS (2026-06-21): a first cut of `community.rs` (Louvain + the two guards) was attempted but the implementation was incorrect — 2 unit tests failed and 3 hung (non-terminating Louvain/​split loop), which would wedge the `cargo test` gate. It was removed (uncommitted) to keep the suite green. §2 needs a CORRECT from-scratch implementation in a focused session (the algorithm must converge deterministically — pin a pass cap + a strictly-decreasing modularity guard + a recursion-depth cap on the oversized-split). Also note ADR-027: this whole worker has NO live value until the semantic projection is enabled (nexus#12 → phase25), so phase25 is the de-facto unblocker.
- [x] 2.1 New `crates/cortex-workers/src/graph/community.rs`: in-memory `CommunityGraph` + hierarchical Louvain `detect_communities()` — deterministic (fixed iteration order + stable tie-breaks) and provably terminating (hard caps: `max_local_move_passes`, modularity-gain epsilon, `max_levels`). NOTE: the live Nexus snapshot of the architecture subgraph is §2.4-adjacent and gated on the semantic projection (ADR-027); the algorithm core operates on a pure in-memory graph and is fully offline-unit-tested. Tests: two_cliques→2, determinism, K20 termination, empty/singleton. check + clippy clean.
- [x] 2.2 Oversized-community recursive split (> `oversized_fraction` 0.25 of nodes → re-partition the induced subgraph, reindexed; depth-capped `max_split_depth`; stops when a re-partition makes no progress). Test `oversized_community_is_split`.
- [x] 2.3 Hub-percentile exclusion (top 1% by degree, `hub_percentile` 0.99) + neighbor-majority re-attachment (ties → lowest community id); hubs marked `is_hub`. Test `hub_is_excluded_then_reattached_to_neighbor_majority`.
- [x] 2.4 Writeback mapper `community_node_ops(result, label_for) -> Vec<NodeOp>` in community.rs: idempotent (`NodeOp::with_identity` → `ConflictPolicy::Match`, sets only `community_id`/`community_level`/`is_god_node`), deterministic (emitted sorted by node id; ids from the deterministic detect pass). Nodes with an unresolved label are omitted — the actual Nexus push is the existing graph writer's job (GraphPatch), invoked by the §2.5 worker against the live architecture snapshot, which is gated on the semantic projection (ADR-027). Test `community_node_ops_are_idempotent_sorted_and_skip_unresolved`. check + clippy clean.
- [ ] 2.5 ⏸ blocked: the worker body must snapshot the architecture subgraph from Nexus (Cypher), which is empty until the semantic projection is enabled (ADR-027 → phase25/nexus#12) — a nightly cron over an empty subgraph is a no-op with no verifiable behavior. Register the cron grain (nightly; never blocks ingestion) + worker (snapshot → detect_communities → community_node_ops → GraphPatch push) once the projection lands.

## 3. Surface
- [ ] 3.1 `cortex_graph_communities` MCP tool (list communities + god nodes + cross-community edges)
- [ ] 3.2 Dashboard "subsystems" view (`crates/cortex-api/src/dashboard/graph.rs`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation (new spec `docs/specs/NN-graph-communities.md`; spec 07 community property; CHANGELOG)
- [ ] 4.2 Write tests (partition determinism, oversized-split, hub-exclusion units; MCP tool IT)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
