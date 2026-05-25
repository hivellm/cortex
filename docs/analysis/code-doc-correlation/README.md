# Code↔Documentation Correlation Analysis — Executive Summary

> **Analysis ID:** CDC-001
> **Date:** 2026-05-24
> **Scope:** How Cortex should correlate documentation (ADRs, prose docs, comments, READMEs) with source code to improve retrieval grounding and reduce hallucination
> **Status:** Complete (research phase)
> **Source:** Academic literature survey (28 papers, 2002–2026) + Cortex codebase gap analysis

---

## 1. Executive Summary

Cortex already implements hybrid retrieval (Meili + dense vectors + Nexus graph) and a pre-thinking bundle that fuses laws, decisions, similar turns, and snippets. The question is **how to raise the *relevance* of what comes back** — the user-reported failure mode is "data exists but query doesn't surface what it should."

Academic literature converges on seven principles:

1. **Hybrid retrieval (BM25 + dense + rerank) beats any single modality** by 15–30% recall — BM25 is non-negotiable for identifiers, error codes, ADR IDs (zero semantic signal in dense embeddings).
2. **LLM-based traceability (F1 79–80%) crushes classical IR (F1 36–54%)** but precision-biased (recall 47–75%) — best used to mine candidate links offline, not online.
3. **Granularity and order of chunks matter more than volume** — AST-aware chunking beats line/token chunking.
4. **Multi-source corpora (docs + code + SO + tutorials + ADRs) outperform single canonical datastores** even with GPT-4-class generators.
5. **Graph + vector + lexical fusion** closes the gap left by any one alone (AgenticAKM, NetApp Hybrid-RAG). Cortex's spec-11 fusion lane is the right architecture.
6. **ADR retrieval needs supersession-aware ranking** — recency + lifecycle state weighting prevents surfacing dead decisions.
7. **Phantom-link detection (verify cited symbol exists in AST)** eliminates a documented hallucination class essentially for free.

**Key finding:** Cortex's architecture is correct. The relevance gap is in **(a)** absence of an eval harness to measure regressions, **(b)** missing cross-encoder rerank after fusion, **(c)** chunking strategy, and **(d)** no explicit code↔doc trace edges in Nexus.

---

## 2. Cortex Cognitive Gaps Addressed

| Gap | How code↔doc correlation closes it |
|---|---|
| User asks "why X?" and gets stale code without the ADR that motivated it | ADR↔code trace edges + supersession weighting |
| Pre-thinking bundle cites function that no longer exists | Phantom-link verifier against current AST |
| Retrieval misses a symbol because the query uses prose phrasing | BM25 + dense + cross-encoder rerank covers both lanes |
| Top-K dense recall ranks `superseded` ADR above `accepted` one | Recency + lifecycle re-ranker |
| LLM cited the right area but wrong specific function | AST-aware chunking (chunk = item-of-top-level, not N lines) |
| Bundle has snippets but no traceable provenance | Multi-artifact bundle with per-item provenance schema |
| No way to know if a change made retrieval better or worse | Eval harness with gold trace links (CodeRAG-Bench format) |

---

## 3. Document Map

- **[findings.md](findings.md)** — Full literature synthesis: the three eras of traceability, seven consolidated principles, numbers and citations.
- **[execution-plan.md](execution-plan.md)** — Tier A/B/C recommendations with concrete owners, files, and acceptance criteria.
- **[gaps.md](gaps.md)** — Specific Cortex gaps mapped to literature evidence and proposed fixes.
- **[references.md](references.md)** — Annotated bibliography (28 sources).

---

## 4. Recommendation in One Line

Before any new feature: **build the eval harness first**. Without measurement, every retrieval change is faith. Then ship cross-encoder rerank + phantom-link verifier (Tier A) — highest ROI, lowest blast radius.
