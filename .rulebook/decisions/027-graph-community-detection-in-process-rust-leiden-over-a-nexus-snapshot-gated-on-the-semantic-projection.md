# 27. Graph community detection: in-process Rust Leiden over a Nexus snapshot, gated on the semantic projection

**Status**: proposed
**Date**: 2026-06-21
**Related Tasks**: phase27b_graph-community-detection, phase27c_graphrag-community-summaries, phase25_nexus-graph-write-performance

## Context

phase27b wants Leiden community detection over the architecture subgraph (CALLS/IMPORTS/DEFINES/INHERITS) to produce a subsystem map. Spike (2026-06-21) found two decisive facts: (1) the live Nexus graph has ZERO architecture/semantic edges — `MATCH ()-[r]->() RETURN type(r)` returns only structural kinds (IN_REPO 8078, HAS_TOOL_CALL 2982, TOUCHED 1677, HAS_TURN 1611, EMITTED_BY 366, ABOUT 28, REMEMBERS 5). The semantic edges come from `graph/projection.rs`, which is gated OFF in prod (CORTEX_GRAPH_PROJECTION_ENABLED=false, nexus#12), itself blocked on Nexus write performance (phase25). (2) Nexus is a custom graph DB with no built-in community-detection procedure, and there is no mature pure-Rust Leiden crate to depend on.

## Decision

Run community detection IN-PROCESS in Rust over a Nexus snapshot (pull architecture edges via Cypher, build an in-memory adjacency, partition, write `community_id` + `level` back via the existing NodeOp surface) — NOT as a server-side Nexus procedure. Implement the algorithm in-process (no external Leiden crate dependency); start from modularity-optimizing Louvain with the Leiden refinement step for well-connected communities, plus graphify's two human-scale guards (oversized-community recursive split at ~25%, hub-percentile exclusion + neighbor-majority re-attachment). The algorithm + guards are unit-testable offline against synthetic graphs with known partitions. LIVE value is GATED on the semantic projection being enabled (nexus#12 → phase25): until then the architecture subgraph is empty, so the worker has nothing meaningful to partition in prod. Build + unit-test now; wire the cron + writeback + MCP tool + dashboard; live-verify once the projection is on.

## Alternatives Considered

- Server-side Nexus community-detection procedure — ruled out: Nexus has no such procedure/plugin surface.
- Depend on an external Rust Leiden/Louvain crate — ruled out: no mature, well-tested crate; in-process keeps determinism + control over the two human-scale guards.
- Partition the STRUCTURAL graph (IN_REPO/TOUCHED/HAS_TURN) that exists live today — ruled out: those edges encode session plumbing + repo membership, not architecture; communities over them are meaningless for the subsystem-map use case.
- Run HDBSCAN over node embeddings (the existing consolidation clustering) — ruled out: that clusters by semantic similarity, not graph topology; phase27b specifically wants topology-based subsystems.

## Consequences

Additive node property (`community_id`/`level`) + a new nightly worker + an MCP tool + a dashboard view; no breaking change. The algorithm core is deterministic (fixed seed) and offline-unit-testable, so phase27b can be implemented + tested now. BUT phase27b (and phase27c community summaries, which build on it) deliver NO live value until the semantic projection is enabled — which is blocked on Nexus write performance (phase25/nexus#12). This makes phase25 the de-facto unblocker for the whole phase27b/c graph-intelligence track. Recommend sequencing phase25 (or enabling the projection) before investing in the full phase27b worker, OR building phase27b algorithm+tests now and accepting live verification is deferred to when the projection lands.
