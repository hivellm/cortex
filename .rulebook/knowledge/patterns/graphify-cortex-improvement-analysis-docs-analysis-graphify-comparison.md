# graphify → Cortex improvement analysis (docs/analysis/graphify-comparison)

**Category**: architecture
**Tags**: analysis:graphify-comparison, graphrag, knowledge-graph, leiden, scip, community-detection, nexus, retrieval

## Description

Analysis of safishamsi/graphify (Python knowledge-graph builder for AI agents, ~70k★) vs Cortex, with 11 numbered topic files in docs/analysis/graphify-comparison/. Cortex owns the infra graphify lacks (Synap event bus, Vectorizer+Meili+Nexus multi-store fusion, BGE reranker, phantom-link verifier, governance, live capture); graphify owns the graph-as-knowledge-product layer Cortex under-builds. Grep-verified gaps in Cortex (2026-06-20): NO graph community detection (HDBSCAN exists but only over consolidation embeddings in consolidator/source/topic.rs, not graph topology); NO SCIP/precise xref (tree-sitter heuristic edges, 10 langs); NO DB-schema/infra ingestion; NO multimodal; edges have provenance triple but NO confidence tier; MinHash exists only in cortex-cli memory_consolidate, not graph entity dedup; feedback_record/signals exist (query learning partially covered). Prioritized roadmap R1–R11: Tier-1 = R1 edge confidence tiers (Extracted/Inferred/Ambiguous, LOW effort) + R2 Leiden community detection over Nexus (MED, keystone); Tier-2 = R3 community summaries as new consolidation grain + GraphRAG global query route, R4 SCIP ingestion (rust-analyzer scip) for precise calls/refs; Tier-3 = R5 cargo workspace topology, R6 community-aware dedup, R7 IDF-gated graph seed selection, R8 affected-node summary invalidation.

## Example

Read docs/analysis/graphify-comparison/00-README.md (index + gap matrix) then 10-recommendations-roadmap.md (R1-R11 with effort/impact/touch-points). High-value files: 02 (Leiden), 03 (GraphRAG global), 04 (SCIP + confidence tiers).

## When to Use

Before implementing any graph-knowledge improvement to Cortex: community detection, GraphRAG global queries, community/architecture summaries, SCIP/precise extraction, edge confidence, cross-domain (schema/infra) graphing, graph entity dedup, or graph-lane query ranking.
