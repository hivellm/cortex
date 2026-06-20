# Proposal: phase27b_graph-community-detection

Source: docs/analysis/graphify-comparison/ (R2, file 02)

## Why

Cortex's Nexus graph has rich edges (12 semantic kinds + structural) but
**no community detection ever runs over it** — verified: no
`leiden`/`louvain`/`modularity` anywhere in `crates/` (the only
clustering is HDBSCAN over consolidation *embeddings*, not graph
topology). The graph is used only for traversal/joins, never as a
partition. Consequently Cortex cannot answer architecture-level questions
("what are the major subsystems and how do they relate"), cannot label
nodes by subsystem, and cannot use community membership as a signal
elsewhere. graphify treats Leiden community detection as core, and it is
the keystone that unlocks community summaries (phase27c), community-aware
dedup, and richer graph ranking. Cortex already pays to build the
richest part (the edges); community detection is the cheap transform that
turns them into a navigable map.

## What Changes

- New periodic worker (sibling to the stale-edge sweeper / retention
  scheduler) that snapshots the graph and runs **Leiden** community
  detection, writing `community_id` (+ hierarchy `level`) back onto nodes
  via the existing `NodeOp` surface. Runs on a cron grain so it never
  blocks ingestion.
- Port graphify's two guards: **oversized-community recursive split**
  (any community > ~25% of nodes is re-partitioned) and **hub-percentile
  exclusion** (super-connectors held out, re-attached by neighbor
  majority vote) — these keep communities human-scale.
- Scope the partition to the architecture-bearing subgraph (code/semantic
  edges: calls/imports/defines/inherits; down-weight session plumbing
  like `emitted_by`/`mentions_file`).
- Expose a `cortex_graph_communities` MCP tool + a dashboard "subsystems"
  view (god nodes per community, cross-community "surprise" edges —
  graphify's `analyze.py` is the template).
- ADR for the algorithm choice (in-process Rust Leiden over a snapshot
  vs. server-side Nexus procedure) after a short spike.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` (community property),
  new spec `docs/specs/NN-graph-communities.md`.
- Affected code: new `crates/cortex-workers/src/graph/community.rs` + a
  bin/scheduler entry; `graph/projection.rs` / writer for `community_id`
  writeback; `crates/cortex-mcp-server/src/tools.rs`;
  `crates/cortex-api/src/dashboard/graph.rs`.
- Breaking change: NO (additive node property + new worker/tool).
- User benefit: a new query class (architecture/subsystem map,
  onboarding) and a reusable community signal for phase27c + dedup.
- Prereq: none. Pairs with / unblocks: phase27c, community-aware dedup.
