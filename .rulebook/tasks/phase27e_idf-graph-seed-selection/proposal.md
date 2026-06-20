# Proposal: phase27e_idf-graph-seed-selection

Source: docs/analysis/graphify-comparison/ (R7, file 08)

## Why

Cortex's retrieval fusion is strong (RRF over BM25 + dense + graph, plus
a cross-encoder reranker), but the **graph lane's seed selection** — how
query terms map to graph entry nodes for traversal — is not IDF-gated.
A common-token node (e.g. `error`, `handler`) can become a BFS seed and
pull in a generic neighborhood, crowding out a specific match
(`FooBarService`). graphify's serve layer fixes exactly this with
per-token IDF weighting plus a "seed only if the node scores above 80% of
the top score" gate, so high-frequency noise can't steal seed slots. This
is a small, self-contained scoring change, distinct from the
document-ranking IDF that BM25 already provides, and measurable on the
existing relevance eval harness.

## What Changes

- In the graph lane / strategies
  (`crates/cortex-api/src/search/strategies.rs`), when resolving query
  terms to graph seed nodes: (a) weight candidate nodes by **per-token
  IDF over node labels** (rare identifiers win), and (b) apply graphify's
  **80%-of-top seed gate** so only strong matches seed the BFS.
- Add a **source-path bonus**: boost graph nodes whose `source_file`
  contains a query term (helps "where is X" queries).
- Add `path(a, b)` and `compare(a, b)` MCP tools (shortest path between
  two symbols; shared vs. divergent neighborhoods) — primitives the graph
  already supports and graphify exposes.
- Validate on the relevance eval harness (`crates/cortex-eval`,
  `crates/cortex-api/tests/relevance_eval_it.rs`); gate like the phase17
  reranker.

## Impact

- Affected specs: spec 11 (graph lane seed selection).
- Affected code: `crates/cortex-api/src/search/strategies.rs`;
  `crates/cortex-mcp-server/src/tools.rs` (path/compare tools);
  eval fixtures.
- Breaking change: NO (scoring tweak + additive MCP tools).
- User benefit: more precise graph-lane results (specific matches beat
  generic hubs); two useful agent query primitives.
- Prereq: none. Pairs with phase27a (confidence-weighted edges feed the
  same scoring).
