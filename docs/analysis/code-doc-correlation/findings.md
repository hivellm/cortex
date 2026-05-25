# Findings — Code↔Documentation Correlation Literature Survey

> **Analysis ID:** CDC-001 / Findings
> **Date:** 2026-05-24
> **Method:** 28 papers + benchmarks reviewed across software traceability, code retrieval, RAG, ADR management, and hybrid search literature.

---

## 1. The Three Eras of Traceability Link Recovery

Software-engineering research on linking documentation to code spans 25+ years. Three distinct eras, each defined by the dominant technique and its ceiling:

| Era | Years | Dominant technique | Typical recall | F1 ceiling | Compute cost |
|-----|-------|--------------------|----------------|------------|--------------|
| Bag-of-words | 2002–2015 | VSM, LSI, LDA | 30–55% | 0.40–0.55 | Low |
| Neural embedding | 2018–2022 | CodeBERT, GraphCodeBERT, UniXcoder | 55–75% | 0.55–0.70 | Medium |
| LLM-assisted | 2024–2026 | One-to-many LLM matching + IR hybrid | 75–85% | **0.79–0.80** | High |

**Reference numbers** (arxiv 2506.16440, Crawl4AI dataset, 2026):
- Claude 3.5 Sonnet: F1 **0.794**
- GPT-4o: F1 ~0.77
- o3-mini: F1 ~0.74
- BM25: F1 0.441
- TF-IDF: F1 0.542
- CodeBERT: F1 0.362

LLMs **precision >87%**, **recall 47–75%** — conservative, miss links, rarely fabricate. Errors are concentrated in three patterns:

1. Phantom links — citing a symbol that does not exist.
2. Naming-assumption errors — semantic confusion from look-alike identifiers.
3. Multi-step chain breakdown — endpoint accuracy stays high but intermediate node accuracy collapses to 13–80%.

---

## 2. Seven Consolidated Principles

### Principle 1 — Hybrid retrieval beats any single modality

Across CodeRAG-Bench, NetApp Hybrid-RAG, Superlinked, and academic IR studies: BM25 + dense + cross-encoder reranking outperforms any single lane by **15–30% recall**.

BM25 alone wins on:
- Exact identifiers (`cortex_pre_thinking::scope::derive`)
- Error codes, version strings, ADR IDs (`ADR-016`)
- Rare terms with zero semantic signal

Dense alone wins on:
- Paraphrased intent ("how does Cortex decide what to retrieve?" → `scope_derive`)
- Cross-modal queries (prose ↔ code)

Cross-encoder rerank (BGE-reranker-v2-m3, Cohere Rerank) adds 5–10% MRR on top of fusion by jointly attending to (query, candidate) pairs. Cost: ~50–200ms for top-100.

### Principle 2 — LLM trace mining works offline, not online

LLM one-to-many matching (paper LLM-traceability, arxiv 2506.16440) achieves F1 0.80 but takes minutes per repository. The correct deployment pattern is:

1. Mine candidate trace links offline with an LLM pass over the corpus.
2. Persist links as graph edges with confidence scores.
3. Serve online via fast graph + IR fusion.
4. Re-mine on corpus drift (commits, new ADRs).

Online LLM-only retrieval is uneconomical and slow. Offline mining + online graph walking captures most of the gain.

### Principle 3 — Granularity and order beat volume

Paper "Beyond More Context" (arxiv 2510.06606): chunk **granularity and ordering** affect completion quality more than total context size. RepoQA (arxiv 2406.06025) corroborates.

Best chunking unit for code: **AST item of top level** (function/struct/impl block + leading docstring/comment block). Bad: fixed token windows that cut across function boundaries.

For docs: paragraph + nearest heading + section path (breadcrumb). Bad: fixed-byte windows.

### Principle 4 — Diverse multi-source corpora outperform single canonical stores

CodeRAG-Bench tested 5 source types (competition solutions, tutorials, library docs, StackOverflow, GitHub). Significant gains over GPT-4 baseline came from **mixing** sources, not from any single store being canonical.

Implication for Cortex: indexing only code or only ADRs misses gains. The current mix (turns, decisions, laws, snippets) is the right shape; the question is fusion quality.

### Principle 5 — Graph + vector + lexical is the frontier

AgenticAKM (arxiv 2602.04445) and NetApp Hybrid-RAG: vector retrieval ignores explicit relations; graph retrieval ignores fuzzy semantic similarity. Fusion of both is required.

Cortex's spec-11 fusion lane (vector + keyword + graph via Nexus) **matches the state of the art**. The architectural question is settled. The implementation questions are: (a) fusion weights, (b) rerank, (c) edge quality in the graph.

### Principle 6 — ADRs need supersession-aware ranking

AgenticAKM identifies a specific failure mode: vector retrieval over ADR corpora returns `superseded` decisions because their text is still semantically close to the query. Fix: rank by `recency × lifecycle_weight` where `lifecycle_weight(accepted)=1.0`, `lifecycle_weight(superseded)=0.2`, `lifecycle_weight(deprecated)=0.1`.

Cortex's `cortex-historian` agent walks supersession chains correctly **after** retrieval, but the initial retrieval does not penalize superseded items. This causes superseded ADRs to occupy ranking slots that accepted ones should hold.

### Principle 7 — Phantom-link detection is free hallucination reduction

Paper LLM-traceability §error patterns: a large fraction of LLM false positives are citations to symbols that do not exist (renamed, removed, never existed). Detection is mechanical: parse the cited path, AST-walk the file, check the symbol exists. Tree-sitter is the standard tool.

Cost: negligible (single AST parse per cited symbol). Benefit: eliminates an entire hallucination class without retraining or prompt engineering.

---

## 3. Code-Specific Embedding Models (When to Use Which)

| Model | Strength | Weakness | When to use |
|-------|----------|----------|-------------|
| CodeBERT (2020) | Cheap, well-studied | Plain transformer, no structure awareness | Baseline only |
| GraphCodeBERT (2020) | Uses dataflow | Heavier; needs DFG extraction | When dataflow matters (security, refactoring) |
| UniXcoder (2022) | Unified code+comment+NL | Bigger | Cross-modal queries (prose ↔ code) |
| CoCoSoDa (2022) | Contrastive multi-modal | Less ecosystem | Best MRR in benchmark (vs CodeBERT +13%, UniXcoder +5.9%) |
| CodeCSE (2024) | Multilingual, simple | Newer, less benchmarked | When Cortex indexes multiple languages |
| BGE-M3 / BGE-Code | General-purpose, well-supported | Not code-pretrained as deeply | Pragmatic default; works for code+prose |

For Cortex (currently Rust-heavy, with prose ADRs and Markdown docs), **BGE-M3 or BGE-Code as embedder + BGE-reranker-v2-m3 as reranker** is the pragmatic choice. Specialization (UniXcoder, CoCoSoDa) makes sense only after eval harness exists.

---

## 4. Hallucination Reduction with RAG + Guardrails

Three mechanisms documented in the literature, stackable:

1. **RAG with strict grounding** — instruction "answer only from retrieved context" + "say 'I don't know' if unsure". Reductions of 60–80% baseline hallucination rates.
2. **Multi-layered quality control** — secondary verification pass (e.g., a separate model checks each claim against sources). Reductions to <0.5% reported with expert verification.
3. **Provenance enforcement** — every claim must carry a source ID; claims without provenance are dropped or flagged. Combined with RAG+RLHF: 96% reduction (Microsoft Community Hub case study).

For Cortex: the pre-thinking bundle already attaches provenance (turn IDs, ADR IDs, snippet paths). The gap is enforcement at consumption — downstream tools need to refuse to act on un-provenanced claims.

---

## 5. Evaluation Harnesses

No retrieval system improves without measurement. Two reference benchmarks:

- **CodeRAG-Bench** (arxiv code-rag-bench.github.io) — programming tasks + heterogeneous retrieval sources, oracle vs. retrieved comparison. Format: `(query, gold_documents, expected_completion)`.
- **RepoQA** (arxiv 2406.06025) — long-context repository understanding. Format: needle-in-haystack questions about specific functions in large repos.

For Cortex, the minimum viable harness is:
- 50–100 gold (query, expected_artifact_set) tuples drawn from real session transcripts.
- Metrics: MRR@10, Recall@5, Recall@20, F1@10.
- CI integration: regression gate on retrieval changes.

Without this, every fusion-weight tweak or reranker swap is guesswork.

---

## 6. What the Literature Says NOT to Do

1. **Pure dense retrieval for code** — fails silently on identifiers and error codes. BM25/Meili must remain in the fusion.
2. **Trust LLM trace links without verifier** — multi-step chain accuracy drops to 13–80%. Verify every cited symbol.
3. **Fixed-token-window chunking ignoring AST** — fragments functions, loses structural anchors.
4. **Replace IR with LLM** — IR + LLM is Pareto-optimal vs LLM alone in cost and recall.
5. **Optimize fusion weights without an eval harness** — local maxima abound; without ground truth you cannot tell which weights help.
6. **Index ADRs without lifecycle weighting** — superseded decisions outrank accepted ones on semantic similarity alone.

---

## 7. Specific Numbers Worth Memorizing

| Metric | Value | Source |
|--------|-------|--------|
| LLM-traceability F1 (Claude 3.5 Sonnet) | 0.794 | arxiv 2506.16440 |
| BM25 F1 on same dataset | 0.441 | arxiv 2506.16440 |
| CodeBERT F1 on same dataset | 0.362 | arxiv 2506.16440 |
| Hybrid (BM25 + dense + rerank) recall lift | +15–30% | Superlinked, NetApp |
| Cross-encoder rerank latency (top-100) | 50–200ms | BGE-reranker-v2-m3 |
| Hallucination reduction (RAG + RLHF + guardrails) | 96% | Microsoft Azure AI Foundry |
| LLM multi-step chain intermediate accuracy | 13–80% | arxiv 2506.16440 |
| CoCoSoDa MRR lift over CodeBERT | +13.3% | arxiv 2204.03293 |

These are the load-bearing numbers when justifying Cortex retrieval changes.
