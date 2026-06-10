# 25. Reranker model: BGE-reranker-v2-m3 via TEI /rerank, fail-open with 500ms budget

**Status**: proposed
**Date**: 2026-06-10
**Related Tasks**: phase17_cdc-code-doc-correlation

## Context

Phase17 P2 adds a second-stage cross-encoder to the spec-11 fusion lane. The fused BM25+dense+graph candidate list ranks well on lexical/structural signals but mis-orders semantically-close hits; a cross-encoder scoring (query, text) pairs jointly fixes this at the cost of one HTTP round-trip per query. We needed to pick a model, a serving protocol, and a failure policy.

## Decision

Use BGE-reranker-v2-m3 served by Text Embeddings Inference (TEI), called as POST {endpoint}/rerank with {query, texts, return_text:false}. The orchestrator sends the top-100 fused candidates (top_k_input=100), enforces a 500ms client-side timeout, and is strictly fail-open: any HTTP/decode/timeout error preserves the pre-rerank fusion order and emits a cortex_audit `reranker.fallback` event. The implementation hides the model behind the `Reranker` trait (crates/cortex-workers/src/rerank/) so the model/server can be swapped without touching the orchestrator. enabled=true by default but the lane only activates when an endpoint is configured and the reranker is injected.

## Alternatives Considered

- Cohere/Voyage hosted rerank APIs — rejected: external dependency + per-query cost + data egress for a local-first memory system
- ColBERT late-interaction — rejected: requires index-side token embeddings, heavier integration than a stateless scorer
- LLM-as-reranker (small instruct model) — rejected: 10-100x latency for marginal gain at top-100 scale
- No reranker (fusion order only) — rejected: phase17 analysis identified semantic mis-ordering as a measurable retrieval gap

## Consequences

Pros: multilingual (m3) matches the pt-BR + English corpus; TEI is already the embedding-serving stack so ops surface stays uniform; trait-shaped seam keeps the model swappable; fail-open means reranker downtime never breaks retrieval. Cons: adds up to 500ms p95 to the query path when the endpoint is slow (gate: ≤ +250ms observed); fallback events must be monitored or a silently-down reranker degrades quality back to fusion order; acceptance gate (MRR@10 ≥ +5%) is still pending — blocked on golden CSV event IDs from a live run (phase17 §2.7).
