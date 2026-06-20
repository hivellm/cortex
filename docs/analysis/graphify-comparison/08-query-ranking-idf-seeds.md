# 08 — Query ranking: IDF-weighted BFS seed selection — **MED**

## What graphify does

`serve.py:_score_nodes()` + `_pick_seeds()` pick *where to start* a graph query well:
- **Multi-tier match:** full-query exact > per-token exact > prefix > substring.
- **IDF weighting:** rare identifiers (`FooBarService`) outrank common ones (`error`, `handler`) — a node's score is discounted by how many nodes share its tokens.
- **Source-file bonus:** terms appearing in the node's `source_file` path get a boost.
- **Seed gating** (`_pick_seeds`): only nodes scoring > 80% of the top score become BFS seeds — so high-frequency noise nodes can't steal seed slots from a specific match.
- Diacritic-tolerant, with Chinese segmentation.

## What Cortex does today

Cortex's retrieval is **more sophisticated overall** on the fusion side:
- BM25 (Meili) + dense (Vectorizer) + graph (Nexus) lanes fused with **RRF** (`crates/cortex-api/src/search/fusion.rs`, strategies.rs), cross-project propagation, a **cross-encoder reranker** (BGE-reranker-v2-m3, phase17 `search/rerank/`), and a phantom-link verifier.
- BM25 inherently encodes IDF for the text lanes.

**Gap (narrow but real):** the **graph lane's seed selection** isn't IDF-gated the way graphify's is. When the orchestrator picks graph entry nodes from query terms, a common-token node can become a seed and pull in a generic neighborhood. graphify's "seed only if > 80% of top score" + per-token IDF is a cheap precision guard specifically for *graph traversal seeding*, distinct from the document-ranking IDF that BM25 already gives.

## Recommendation for Cortex

- In the **graph lane / strategies** (`crates/cortex-api/src/search/strategies.rs`), when resolving query terms → graph seed nodes, apply (a) per-token **IDF over node labels** (rare identifiers win) and (b) graphify's **80%-of-top seed gate** so only strong matches seed the BFS. Small, self-contained scoring tweak; measurable on the existing relevance eval harness (`crates/cortex-eval`, `crates/cortex-api/tests/relevance_eval_it.rs`).
- **Source-path bonus:** boost graph nodes whose `source_file` contains a query term — trivial and effective for "where is X" queries.
- Add `path(a,b)` / `compare(a,b)` MCP tools (graphify has them) if not present — "shortest path between two symbols" and "shared vs. divergent neighborhoods" are useful agent primitives the graph already supports.

## Effort / impact

- **Impact:** MED — better graph-lane precision; most of Cortex's ranking is already strong, so this is targeted polish, validated by the eval harness (gate like phase17's reranker).
- **Effort:** LOW — scoring function changes in one module + optional MCP tools; no new infra.
- **Pairs with:** 04 (confidence-weighted edges feed the same graph-lane scoring).
