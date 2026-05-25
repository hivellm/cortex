# Cortex Gaps — Mapped to Literature Evidence

> **Analysis ID:** CDC-001 / Gaps
> **Date:** 2026-05-24
> **Scope:** Specific Cortex implementation gaps where code↔documentation correlation underperforms what the literature considers state of the art.

---

## Reading this document

Each gap is structured as:

- **Gap** — observable Cortex behavior that falls short.
- **Evidence (literature)** — paper or benchmark that documents the issue or the fix.
- **Evidence (Cortex)** — file path and/or behavior that demonstrates the gap.
- **Proposed fix** — concrete change.
- **Blast radius** — what depends on it.
- **Tier** — A (high ROI / low cost), B (high ROI / medium cost), C (research / long term).

---

## Gap 1 — No retrieval evaluation harness

- **Gap.** No way to measure whether a retrieval change improved or degraded relevance for real Cortex queries.
- **Evidence (literature).** CodeRAG-Bench, RepoQA — every reputable retrieval paper publishes gold sets and metrics. Without these, claims of improvement are unfalsifiable.
- **Evidence (Cortex).** No `tests/retrieval_eval_*` harness; no gold trace-link dataset in `docs/datasets/` or equivalent.
- **Proposed fix.** Build `crates/cortex-eval/` with: (a) a YAML/JSONL gold dataset of 50–100 `(query, expected_artifact_ids[])` tuples drawn from real session transcripts; (b) a runner that calls the live retrieval pipeline and computes MRR@10, Recall@{5,10,20}, F1@10; (c) a CI gate that fails on regression beyond ±2%.
- **Blast radius.** Foundational — every subsequent gap fix needs this to be honest.
- **Tier.** **A — must be first.**

---

## Gap 2 — No cross-encoder reranking after fusion

- **Gap.** Spec-11 fusion combines BM25 (Meili) + dense (Vectorizer) + graph (Nexus) but does not rerank top-K with a cross-encoder that jointly attends to query and candidate.
- **Evidence (literature).** Superlinked, NetApp Hybrid-RAG, "Hybrid Search 101": cross-encoder rerank on top-100 lifts MRR by 5–10% in production. Standard frontier pattern.
- **Evidence (Cortex).** Search `crates/cortex-workers/src/` for reranker calls — none beyond the linear fusion in spec-11.
- **Proposed fix.** Add a rerank stage after fusion. Model: BGE-reranker-v2-m3 (open, MIT-licensed, multilingual). Run on top-100 fused candidates. Configurable; default on for `cortex_query` intent.
- **Blast radius.** Adds 50–200ms per query. Need to make optional via config (`reranker.enabled`).
- **Tier.** **A.**

---

## Gap 3 — No phantom-link verifier

- **Gap.** Pre-thinking bundle and `cortex_query` results can cite symbols (functions, structs, file paths) that no longer exist or never existed.
- **Evidence (literature).** Arxiv 2506.16440 §error patterns documents this as a top hallucination class for LLM-driven traceability. Mechanical detection: AST parse + symbol existence check.
- **Evidence (Cortex).** Bundle and result envelopes carry `path` and `symbol` fields but no validation pass before emission.
- **Proposed fix.** Add a post-retrieval verifier that, for every cited `(path, symbol)`, confirms the file exists and (for `.rs`/`.ts`/`.py`) the symbol resolves via Tree-sitter. Mismatches are either dropped or flagged with `verified=false`. Cheap (<10ms per item with file-content cache).
- **Blast radius.** Pre-thinking bundle, `cortex_query` envelope, decision/snippet retrieval. Must work across Rust + Markdown + TOML/YAML.
- **Tier.** **A.**

---

## Gap 4 — No supersession/recency weighting on decision retrieval

- **Gap.** Decision retrieval over ADRs uses semantic similarity alone; `superseded` and `deprecated` ADRs can outrank `accepted` ones with similar text.
- **Evidence (literature).** AgenticAKM (arxiv 2602.04445) — explicit recommendation for lifecycle-aware ranking on ADR corpora.
- **Evidence (Cortex).** `cortex-historian` walks supersession chains **after** retrieval but the initial ranking does not penalize superseded items. ADR-016, for example, lives alongside any predecessors in the same embedding space.
- **Proposed fix.** Add a re-rank pass on decision lookup: `score' = score × lifecycle_weight × recency_decay`. Defaults: `accepted=1.0`, `superseded=0.2`, `deprecated=0.1`, `proposed=0.7`; `recency_decay = exp(-age_days/365)`.
- **Blast radius.** `cortex-historian`, `decision_lookup` intent, any tool relying on ADR retrieval.
- **Tier.** **A.**

---

## Gap 5 — Chunking not AST-aware

- **Gap.** Source-code chunking for embedding likely uses line/byte windows rather than AST top-level items.
- **Evidence (literature).** Arxiv 2510.06606 "Beyond More Context", arxiv 2510.08610 "Relative Positioning Chunking", arxiv 2605.04763 "How Does Chunking Affect…" — granularity dominates volume.
- **Evidence (Cortex).** Audit needed in `crates/cortex-workers/src/snippets/` and the embedding ingestion path. If chunking is not Tree-sitter-driven, this gap holds.
- **Proposed fix.** Implement AST-aware chunking for at least Rust (Tree-sitter-rust): chunk = top-level item (`fn`, `impl`, `struct`, `enum`, `trait`, `mod`) **plus its leading doc-comment block**. Markdown chunking: heading + body until next heading of same/higher level, plus breadcrumb path.
- **Blast radius.** Requires reindexing affected corpora. Large one-time cost; ongoing ingest cost ~unchanged.
- **Tier.** **B.**

---

## Gap 6 — No explicit code↔doc trace edges in Nexus

- **Gap.** Nexus stores `IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES` but the `DOCUMENTED_BY` edges are sparse — populated only by explicit references, not by inferred semantic links between an ADR and the code it constrains, or between a Markdown doc and the modules it describes.
- **Evidence (literature).** AgenticAKM multi-artifact grounding; arxiv 2602.02554 BatCoder bidirectional code↔doc back-translation; arxiv 2506.16440 LLM trace mining F1 0.80.
- **Evidence (Cortex).** Inspecting Nexus, `DOCUMENTED_BY` density is far below the obvious ground truth (every ADR references ≥1 module; only some have edges).
- **Proposed fix.** Offline LLM pass (Opus or Sonnet) over the corpus to mine candidate `traces_to` edges between ADRs/docs and code paths. Persist with `confidence` score. Re-run on commits to mined files or ADRs.
- **Blast radius.** Adds edges to Nexus; needs schema entry and a recurring job. Significant compute one-off (~hours for full HiveLLM corpus).
- **Tier.** **B.**

---

## Gap 7 — Bundle provenance not enforced at consumption

- **Gap.** Pre-thinking bundle attaches provenance (turn IDs, ADR IDs, paths) but downstream consumers (LLM prompts, MCP tools) do not enforce that every claim cited carries provenance.
- **Evidence (literature).** Microsoft Azure AI Foundry case study — 96% hallucination reduction with RAG + provenance enforcement + RLHF. FEWL (arxiv 2402.10412) — gold-free hallucination measurement.
- **Evidence (Cortex).** Bundle schema includes provenance fields; prompt templates do not require the LLM to cite them in output; no post-generation verifier checks that each claim has a source.
- **Proposed fix.** (a) Update prompt templates to require `[source: <id>]` after each substantive claim; (b) add a verifier that flags unsourced claims; (c) emit `cortex-audit` event recording grounding rate per turn.
- **Blast radius.** Prompt template changes affect every LLM-consuming surface. Should land behind a feature flag.
- **Tier.** **B.**

---

## Gap 8 — Single embedder for code and prose

- **Gap.** Code chunks and prose chunks likely share one embedding model. Code-specialized models (UniXcoder, CoCoSoDa, CodeCSE) outperform general embedders on code search; general embedders outperform code-specialized ones on prose.
- **Evidence (literature).** CodeBERT, GraphCodeBERT, UniXcoder, CoCoSoDa benchmarks. CoCoSoDa +13.3% MRR over CodeBERT on code search.
- **Evidence (Cortex).** `crates/cortex-workers/src/embeddings/` — audit which model is used and whether modality routing exists.
- **Proposed fix.** Route chunks by modality: code → code-specialized embedder; prose → general embedder. Maintain two vector indexes; fuse at query time.
- **Blast radius.** Doubles index storage; doubles ingest cost. Worth doing only after eval harness shows benefit.
- **Tier.** **C — measure first.**

---

## Gap 9 — No back-translation training pair generation

- **Gap.** Cortex has paired data (code + ADR + commit message + test) but does not exploit it for embedder fine-tuning.
- **Evidence (literature).** BatCoder (arxiv 2602.02554) — self-supervised bidirectional code↔doc via back-translation; significant gains when paired supervision is limited.
- **Evidence (Cortex).** No fine-tuning pipeline in the repo.
- **Proposed fix.** Generate (code, doc) pairs from the corpus, run BatCoder-style training on a small embedder. Defer until ≥500 high-quality pairs are available.
- **Blast radius.** New training pipeline; GPU-only. Significant complexity.
- **Tier.** **C.**

---

## Gap 10 — No multi-language code coverage (forward-looking)

- **Gap.** HiveLLM ecosystem spans Rust (Cortex, Nexus, Vectorizer), TypeScript (Rulebook, web surfaces), and Python (tooling, scripts). If Cortex indexes only Rust, cross-repo correlation suffers.
- **Evidence (literature).** CodeCSE (arxiv 2407.06360) — multilingual code+comment embeddings.
- **Evidence (Cortex).** Audit which languages the snippets/chunking lane covers.
- **Proposed fix.** Extend AST-aware chunking to TS and Python via Tree-sitter grammars; add to ingestion pipeline.
- **Blast radius.** Modest — Tree-sitter grammars exist and integrate cleanly.
- **Tier.** **B (once Rust pipeline is solid).**

---

## Summary table

| # | Gap | Tier | Estimated effort | Depends on |
|---|-----|------|------------------|------------|
| 1 | No eval harness | A | M (1–2 weeks) | — |
| 2 | No cross-encoder rerank | A | S (3–5 days) | #1 |
| 3 | No phantom-link verifier | A | S (3–5 days) | — |
| 4 | No supersession weighting | A | XS (1–2 days) | #1 |
| 5 | Chunking not AST-aware | B | M (1–2 weeks) | #1 |
| 6 | Missing code↔doc edges | B | L (2–4 weeks, mostly compute) | #1 |
| 7 | Provenance not enforced | B | M (1–2 weeks) | — |
| 8 | Single embedder | C | L | #1, #5 |
| 9 | Back-translation training | C | XL | #1, #5, #6 |
| 10 | Multi-language coverage | B | M | #5 |
