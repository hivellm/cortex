# 02 — Graph community detection (Leiden) — **HIGH**

## What graphify does

`cluster.py:cluster()` runs **Leiden** (`graspologic.partition.leiden`, Louvain fallback) over the whole graph to partition nodes into communities, then:
- **Oversized-community split** (`_split_communities`): any community > 25% of the graph is recursively re-Leidened so no "common utilities" super-cluster swallows everything.
- **Hub-percentile exclusion**: super-connectors (degree > 95th pct) are held out of partitioning and re-attached by majority vote of neighbors, so a utility/staging node doesn't fuse unrelated subsystems.
- Determinism: node order stabilized, `random_seed=42`.

Communities then power: `analyze.py` god-nodes & cross-community "surprises", the GRAPH_REPORT narrative, the `communities()` MCP tool, GraphRAG summaries (file 03), and a dedup signal (file 06).

## What Cortex does today

- The Nexus graph has rich edges (12 semantic kinds — `crates/cortex-workers/src/graph/extractors/` : calls, imports, defines, returns, supersedes, contradicts, emitted_by, about, answered_by, cites, mentions_file, relates_to — plus structural HAS_TURN/IN_REPO/TOUCHED/DEFINES/REMEMBERS) but **no community detection runs over it.** Verified: no `leiden`/`louvain`/`modularity` anywhere in `crates/` (grep, 2026-06-20).
- The only clustering Cortex does is **HDBSCAN over consolidation *embeddings*** (`crates/cortex-workers/src/consolidator/source/topic.rs`) — density clustering in vector space to group similar memories, **not** graph-topology community detection. Different input (vectors vs. edges), different output (topic groups vs. subsystem partition).

**Gap:** the graph is used as a *join/traversal* structure (neighbors, paths) but never as a *partition* structure. Cortex cannot answer "what are the major subsystems and how do they relate", cannot label nodes by community, and cannot use community membership as a signal elsewhere (dedup, ranking, summaries).

## Recommendation for Cortex

Add a **community-detection pass over the Nexus graph** as a new periodic worker (sibling to the stale-edge sweeper), writing a `community_id` (+ level for hierarchy) back onto nodes.

- **Where:** new module `crates/cortex-workers/src/graph/community.rs` + a bin/scheduler entry (mirror the sweeper/retention scheduler pattern). Runs on a cron grain (e.g. nightly) so it never blocks ingestion.
- **Algorithm:** Leiden. Options, in order of preference:
  1. A Rust Leiden/Louvain crate over a graph snapshot pulled from Nexus (project the edge list, run in-process, write `community_id` back via the existing `NodeOp` surface).
  2. If Nexus gains a community/GDS-style procedure, call it server-side. (Open an upstream issue — Nexus already had perf work this cycle.)
- **Port graphify's two guards** — oversized-community recursive split and hub-percentile exclusion — they are what make communities human-scale instead of one mega-cluster; cheap to replicate.
- **Scope edges:** weight structural noise down (e.g. exclude `emitted_by`/`mentions_file` hubs, or run on the code-semantic subgraph: calls/imports/defines/inherits) so communities reflect *architecture*, not session plumbing.
- **Expose:** a `cortex_graph_communities` MCP tool + a dashboard "subsystems" view (god nodes per community, cross-community edges as "surprises" — graphify's `analyze.py` is a direct template).

## Why it's high value

It is the keystone that unlocks files 03 (GraphRAG global queries / community summaries), 06 (community-aware dedup), and richer 08 ranking. Cortex already pays to build the richest part (the edges); community detection is the cheap transform that turns those edges into a navigable map.

## Effort / impact

- **Impact:** HIGH — new query class (architecture-level), and a reusable signal for 3 other improvements.
- **Effort:** MEDIUM — one worker + one MCP tool + a snapshot/writeback path; the algorithm is off-the-shelf. Main unknown is the Rust Leiden dependency vs. server-side support (decide via a short spike + an ADR).
- **Prereq:** none (edges already exist). **Pairs with:** 03, 06.
