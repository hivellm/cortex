## 1. Spike + decision
- [ ] 1.1 Spike: Rust Leiden/Louvain crate over a graph snapshot vs. server-side Nexus procedure; pick one
- [ ] 1.2 `rulebook_decision_create` ADR recording the algorithm/placement choice

## 2. Community detection worker
- [ ] 2.1 New `crates/cortex-workers/src/graph/community.rs`: snapshot the architecture subgraph (calls/imports/defines/inherits; down-weight session-plumbing edges) and run Leiden
- [ ] 2.2 Port oversized-community recursive split (community > ~25% of nodes → re-partition)
- [ ] 2.3 Port hub-percentile exclusion + neighbor-majority re-attachment
- [ ] 2.4 Write `community_id` + hierarchy `level` back onto nodes via the `NodeOp` surface (idempotent, deterministic seed)
- [ ] 2.5 Register a cron grain in the scheduler (nightly; never blocks ingestion)

## 3. Surface
- [ ] 3.1 `cortex_graph_communities` MCP tool (list communities + god nodes + cross-community edges)
- [ ] 3.2 Dashboard "subsystems" view (`crates/cortex-api/src/dashboard/graph.rs`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation (new spec `docs/specs/NN-graph-communities.md`; spec 07 community property; CHANGELOG)
- [ ] 4.2 Write tests (partition determinism, oversized-split, hub-exclusion units; MCP tool IT)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
