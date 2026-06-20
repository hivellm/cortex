# 10 — Recommendations & roadmap

Prioritized, Cortex-specific. Each item: impact, effort, prereqs, concrete touch-points. Ordered for dependency + ROI. None require changing Cortex's core architecture (event bus → workers → 3 stores → fused retrieval); all are additive workers, ingestion sources, or scoring tweaks.

## Tiered list

### Tier 1 — do first (high ROI, unlocks the rest)

**R1. Edge confidence tiers** (file 04a) — **Impact MED-HIGH · Effort LOW · prereq none**
Add `confidence ∈ {Extracted, Inferred, Ambiguous}` (+score) to the edge/`NodeOp` model; tag deterministic tree-sitter edges `Extracted`, analyzer/LLM edges `Inferred`; weight the graph lane by it; flag `Ambiguous` in the dashboard.
Touch: `crates/cortex-workers/src/graph/extractors/*`, the `NodeOp`/edge model, `crates/cortex-api/src/search/strategies.rs`. *Cheapest high-value item; ride the existing provenance plumbing.*

**R2. Leiden community detection over the Nexus graph** (file 02) — **Impact HIGH · Effort MED · prereq none**
Nightly worker: snapshot graph → Leiden (oversized-split + hub-exclusion) → write `community_id`/level back. Expose `cortex_graph_communities` MCP tool + dashboard "subsystems" view (god nodes, cross-community surprises).
Touch: new `crates/cortex-workers/src/graph/community.rs` + scheduler entry; MCP tool in `cortex-mcp-server`; ADR for Rust-Leiden-vs-server-side (short spike first). *Keystone — unlocks R3, R6, sharper R5/R8.*

### Tier 2 — high value, needs Tier 1

**R3. Community summaries (new consolidation grain) + global query route** (file 03) — **Impact HIGH · Effort MED · prereq R2**
New `Community` consolidation grain (source = community nodes/edges); hierarchy from Leiden levels; orchestrator routes architecture-level intent to map-reduce over community summaries; budgeted return. ADR alongside DEC-005.
Touch: `crates/cortex-workers/src/consolidator/source/`, `crates/cortex-api/src/search/` (intent route). *The payoff of R2 — answers "what are the subsystems / how do they relate".*

**R4. SCIP ingestion (Rust first)** (file 04b) — **Impact HIGH · Effort MED-HIGH · prereq none (independent of R2)**
Run `rust-analyzer scip` in bootstrap/CI; parse; emit precise `calls`/`references`/`defines` (confidence `Extracted`), superseding heuristic edges where covered; two-pass resolver + `scip_external` stubs. Extend by language later. ADR-worthy.
Touch: new `cortex-scip` ingest path + bootstrap/CI orchestration. *Best-in-class graph precision; pairs with R1.*

### Tier 3 — incremental polish

**R5. Cargo workspace topology source** (file 05) — **Impact MED · Effort LOW · prereq none**
`Cargo.toml` members/deps → `Crate` nodes + `crate_depends_on` edges; enables dependency-cycle queries on Cortex's own 14-crate DAG. *Good standalone first slice of cross-domain.*

**R6. Community-aware graph entity dedup** (file 06) — **Impact MED · Effort MED · prereq R2 (for the boost)**
Generalize the existing MinHash (`crates/cortex-cli/src/ops/memory_consolidate.rs`) into a shared util; entropy gate → MinHash/LSH → Jaro-Winkler → same-community boost → union-find; run in the R2 worker.

**R7. IDF-gated graph seed selection + path/compare tools** (file 08) — **Impact MED · Effort LOW · prereq none**
Per-token IDF over node labels + 80%-of-top seed gate in the graph lane; source-path bonus; add `path(a,b)`/`compare(a,b)` MCP tools. Validate on `crates/cortex-eval` (gate like phase17 reranker).

**R8. Affected-node-scoped summary invalidation** (file 07) — **Impact MED · Effort LOW-MED · prereq R3 to fully shine**
BFS affected set on file change → scope which community summaries / topic-cards go stale (sharper than count/age triggers); ensure rename-safe content-hash summary cache.

### Tier 4 — defer / conditional

**R9. DB-schema + infra ingestion** (file 05) — gate on a real DB/infra consumer; LOW priority for current scope (compose-service topology is a cheap exception worth ~R5-level effort).
**R10. Multi-modal ingestion** (file 09a) — defer unless Cortex's scope expands beyond code+docs+sessions.
**R11. Passive query telemetry** (file 09b) — LOW effort; extend existing `feedback_*` tables to mine coverage gaps + promotion candidates. Quick win whenever convenient.

## Suggested sequencing

```
Phase A (foundation):     R1 confidence tiers → R2 Leiden communities
Phase B (the payoff):     R3 community summaries + global queries   (∥ R4 SCIP, independent)
Phase C (polish):         R5 cargo topology, R6 dedup, R7 IDF seeds, R8 affected-invalidation
Phase D (conditional):    R9 schema/infra, R10 multimodal, R11 passive telemetry
```

## What NOT to copy from graphify

- **NetworkX/JSON single-file graph + batch one-shot model** — Cortex's event-bus + Nexus + multi-store fusion is deliberately more capable for live, multi-repo, governed use. Keep it.
- **ProcessPool/GIL workarounds** — Python-specific; Cortex parallelizes natively.
- **Re-implementing fusion/reranking** — Cortex is already ahead (RRF + cross-encoder reranker + phantom-link verifier). graphify's value is upstream (graph structure/extraction), not in retrieval ranking.

## Net assessment

Cortex owns the hard infrastructure graphify lacks; graphify owns the graph-as-knowledge-product layer Cortex under-builds. The two Tier-1 items (**R1 confidence tiers**, **R2 Leiden communities**) are low/medium effort, unlock the high-impact Tier-2 GraphRAG capabilities (**R3**, **R4**), and make Cortex's already-rich edge set finally pay off as a navigable, summarizable map rather than a traversal-only join table.
