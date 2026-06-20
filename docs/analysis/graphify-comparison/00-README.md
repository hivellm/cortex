# Analysis — graphify → Cortex improvement opportunities

**Source:** [`safishamsi/graphify`](https://github.com/safishamsi/graphify) (Python, MIT, ~70k★) — an AI-agent skill that turns any folder (code, SQL schemas, scripts, docs, papers, images, videos) into a single queryable knowledge graph.
**Subject:** Cortex (Rust; Vectorizer dense + Meilisearch BM25 + Nexus graph + Synap bus).
**Date:** 2026-06-20.
**Method:** Deep-read of graphify's source + docs (ARCHITECTURE, how-it-works, node-summaries RFC, ~20 modules) cross-referenced against Cortex's current code, verified by grep (see each file for `file:line` citations).

> Per [`.claude/rules/consult-analysis-before-implementing.md`](../../../.claude/rules/consult-analysis-before-implementing.md): read this index + the relevant numbered file before implementing any item it covers, and check `rulebook_knowledge_list` for `analysis:graphify-comparison`.

## Why graphify is worth studying

Cortex and graphify solve the **same core problem** — give an AI agent a structured, queryable memory of a codebase instead of re-reading raw files — but from opposite ends:

- **Cortex** is a long-running *service mesh* (event bus → workers → 3 stores → fused retrieval) optimized for **live session capture** + multi-repo + governance. Its graph is event-derived (sessions/turns/tool-calls/decisions + 12 semantic code edges).
- **graphify** is a *batch extractor* optimized for **one-shot corpus → graph → GraphRAG queries**. Its graph is content-derived (AST + LLM semantics + DB introspection) with Leiden communities on top.

graphify is ahead on the **graph-as-knowledge-product** dimension: community structure, GraphRAG global queries, precise extraction, cross-domain unification, confidence tagging. Those are exactly the areas Cortex under-invests in. This analysis extracts the transferable ideas — none require abandoning Cortex's architecture; most are additive workers or query lanes.

## Gap matrix (the headline)

| # | Capability | graphify | Cortex today | Gap | File |
|---|-----------|----------|--------------|-----|------|
| 02 | Graph community detection (Leiden) | ✅ core | ❌ (HDBSCAN on *consolidation embeddings* only, not graph topology) | **HIGH** | [02](02-leiden-community-detection.md) |
| 03 | GraphRAG global queries + community summaries | ✅ | ⚠️ consolidations/topic-cards are embedding/event-driven, not graph-community-derived | **HIGH** | [03](03-graphrag-queries-vs-consolidations.md) |
| 04 | Precise extraction (deterministic AST + SCIP) + confidence tags | ✅ SCIP, EXTRACTED/INFERRED/AMBIGUOUS | ⚠️ tree-sitter heuristic edges (10 langs), no SCIP, provenance but no confidence tier | **HIGH** | [04](04-precise-extraction-ast-scip-confidence.md) |
| 05 | Cross-domain graph (code + DB schema + infra) | ✅ pg/cargo introspect | ❌ no schema/infra ingestion | **MED-HIGH** | [05](05-cross-domain-graph-schema-infra.md) |
| 06 | Community-aware entity dedup (MinHash + entropy gate + community boost) | ✅ | ⚠️ MinHash exists in *memory consolidation* only, no graph entity dedup | **MED** | [06](06-entity-dedup-community-aware.md) |
| 07 | Incremental impact (affected-node BFS) + content-hash semantic cache | ✅ | ⚠️ producer checkpoints exist; no graph-affected re-analysis | **MED** | [07](07-incremental-and-caching.md) |
| 08 | Query ranking: IDF-weighted BFS seed selection | ✅ | ⚠️ RRF fusion + reranker, but no IDF seed gating on graph | **MED** | [08](08-query-ranking-idf-seeds.md) |
| 09 | Multi-modal ingestion + query-log learning | ✅ image/video/papers | ⚠️ no multimodal; feedback signals exist | **LOW-MED** | [09](09-multimodal-and-feedback-learning.md) |

## Reading order

1. **[01](01-graphify-architecture-reference.md)** — graphify reference (skim once; the "what it is").
2. **High-value gaps first:** [02](02-leiden-community-detection.md) → [03](03-graphrag-queries-vs-consolidations.md) → [04](04-precise-extraction-ast-scip-confidence.md).
3. **[10](10-recommendations-roadmap.md)** — the prioritized, Cortex-specific roadmap with crate/spec touch-points, effort, and impact. **Start here if you only read one file after this index.**

## One-paragraph recommendation

Cortex already has the hard parts graphify lacks (durable event bus, multi-store fusion, reranker, governance, live capture). The highest-leverage adoptions are **(1) Leiden community detection over the Nexus graph + community summaries as a new consolidation grain** (turns the graph from a join-table into a queryable knowledge product and unlocks GraphRAG *global* questions like "what are the major subsystems and how do they relate"), and **(2) precise extraction — SCIP ingestion + edge confidence tiers** (today's tree-sitter `calls`/`imports` edges are heuristic and unscored, so the graph lane is noisy). Everything else (cross-domain schema nodes, community-aware dedup, affected-node incrementality) is incremental polish on top.
