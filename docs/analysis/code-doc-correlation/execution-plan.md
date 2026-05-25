# Execution Plan — Code↔Documentation Correlation

> **Analysis ID:** CDC-001 / Execution Plan
> **Date:** 2026-05-24
> **Scope:** Concrete sequence of tasks to close the gaps identified in [gaps.md](gaps.md), grounded in the principles in [findings.md](findings.md).
> **Status:** Draft — pending user approval before promotion to Rulebook tasks.

---

## Sequencing rationale

The plan is sequenced by **measurability gate**: nothing else ships before the eval harness, because every other change needs the harness to prove it helps. After the harness, Tier A items are independent and can run in parallel. Tier B depends on at least one Tier A success and a reindex window. Tier C is research-grade and deferred.

---

## Phase 1 — Measurement foundation (must precede everything)

### Step 1.1 — Build retrieval eval harness

**Goal.** A reproducible way to score retrieval quality on real Cortex queries.

**Deliverables.**
- New crate `crates/cortex-eval/` with:
  - Gold dataset at `crates/cortex-eval/data/gold_v1.jsonl` containing 50–100 entries:
    ```
    {
      "query_id": "...",
      "query": "...",
      "intent": "decision_lookup | similar_problems | scope_pack | ...",
      "expected_artifact_ids": ["adr-016", "crates/cortex-config/src/config.rs", "..."],
      "expected_rank_top_k": 5,
      "notes": "..."
    }
    ```
  - Runner binary `cortex-eval-run` that hits the live retrieval pipeline and emits per-query and aggregate metrics.
  - Metrics: MRR@10, Recall@{5,10,20}, F1@10, latency p50/p95.
- CI integration: `cargo run -p cortex-eval --bin cortex-eval-run -- --baseline baseline.json` exits non-zero on >2% regression in MRR@10.
- Documentation: `crates/cortex-eval/README.md` describing schema, how to add entries, and how to read reports.

**Source material for gold entries.** Real session transcripts in Cortex's own turn corpus, filtered to queries with clear ground truth. Bias toward queries that have already failed (the user-reported "no relevant data" cases).

**Acceptance criteria.**
- Harness runs end-to-end against a clean local Cortex stack.
- Reports written to `target/cortex-eval/<run_id>/report.json` with both summary and per-query breakdown.
- Baseline locked in `crates/cortex-eval/baseline.json` against current main.

**Effort.** 1–2 weeks. **Tier.** A. **Depends on.** —.

---

## Phase 2 — Tier A improvements (run in parallel after Phase 1)

These three can land independently. All are gated by Phase 1's harness showing positive deltas before merge.

### Step 2.1 — Cross-encoder reranker after spec-11 fusion

**Goal.** Lift MRR@10 by 5–10% with a top-100 rerank pass.

**Deliverables.**
- New module `crates/cortex-workers/src/rerank/` with:
  - Trait `Reranker` (`score(query, candidates) -> Vec<f32>`).
  - Implementation `BgeRerankerV2M3` calling a local or remote inference endpoint.
  - Wiring in the fusion lane: rerank top-100 after RRF, return top-K.
- Config schema additions in `crates/cortex-config/`:
  - `reranker.enabled: bool` (default true)
  - `reranker.model: string`
  - `reranker.top_k_input: usize` (default 100)
  - `reranker.endpoint: string`
  - `reranker.timeout_ms: u64` (default 500)
- Fail-open: on timeout or error, return pre-rerank order. Emit `cortex-audit` event with `reranker.fallback=true`.

**Acceptance criteria.**
- Eval harness shows MRR@10 ≥ +5% over Phase 1 baseline.
- p95 latency increase ≤ 250ms.
- Fallback path covered by integration test.

**Effort.** 3–5 days. **Tier.** A. **Depends on.** 1.1.

### Step 2.2 — Phantom-link verifier

**Goal.** Eliminate citations to non-existent symbols.

**Deliverables.**
- New module `crates/cortex-workers/src/verify/symbols.rs` exposing:
  ```
  fn verify_symbol(path: &Path, symbol: &str) -> SymbolVerdict { Verified, NotFound, Unsupported }
  ```
- Tree-sitter integration for at minimum Rust and Markdown. TS/Python optional, return `Unsupported` until grammars are added.
- Post-retrieval pass over `cortex_query` envelopes and pre-thinking bundles: for every cited `(path, symbol)`, attach `verified: bool`. Filter or flag according to config.
- Config additions:
  - `verify.symbols.enabled: bool` (default true)
  - `verify.symbols.action: "filter" | "flag"` (default `"flag"` initially, then `"filter"` after observation period)

**Acceptance criteria.**
- Unit tests covering present symbols, renamed symbols, deleted files, unsupported language.
- Eval harness shows hallucination rate (cited symbols that fail verification) drops to ≤1% on regression suite.
- Audit event `phantom_link_dropped` emitted with counts per turn.

**Effort.** 3–5 days. **Tier.** A. **Depends on.** —.

### Step 2.3 — Supersession + recency weighting on decision lookup

**Goal.** Stop returning superseded ADRs ahead of accepted ones.

**Deliverables.**
- New scoring function in `cortex-historian`'s retrieval path (and any other ADR-touching lane):
  ```
  score' = base_score × lifecycle_weight × recency_decay
  lifecycle_weight: accepted=1.0, proposed=0.7, superseded=0.2, deprecated=0.1
  recency_decay: exp(-age_days / 365)
  ```
- Config additions allowing per-deployment tuning of the four lifecycle weights and the decay constant.
- Cortex-audit event capturing pre/post-rerank ordering for the top-10 ADR candidates per `decision_lookup` query.

**Acceptance criteria.**
- Eval harness `decision_lookup` subset shows Recall@5 ≥ +10% on cases where the gold ADR is in `accepted` state.
- No regression on cases where the gold ADR is in `superseded` state and the query is historical.
- Audit events queryable from `cortex-audit` for debugging.

**Effort.** 1–2 days. **Tier.** A. **Depends on.** 1.1.

---

## Phase 3 — Tier B improvements (sequenced after Phase 2 results)

Order matters. Step 3.1 unlocks reindexing-dependent gains; 3.2 and 3.3 follow.

### Step 3.1 — AST-aware chunking (Rust + Markdown first)

**Goal.** Replace line/token chunking with AST-item chunking.

**Deliverables.**
- New module `crates/cortex-workers/src/chunking/` with:
  - `RustChunker` (Tree-sitter-rust): chunk = `(item_kind, item_name, source_text, leading_doc_comment, file_path, byte_range)`.
  - `MarkdownChunker` (Tree-sitter-markdown): chunk = `(heading_path[], body_text, source_path, byte_range)`.
- Migration script `scripts/reindex_v2.sh` that re-ingests affected corpora.
- Versioned index name (`cortex-snippets-v2`) so rollback is possible.

**Acceptance criteria.**
- Eval harness shows Recall@5 ≥ +10% on code-citation queries after reindex.
- Latency neutral or better.
- Rollback path documented (point Meili/Vectorizer aliases back to v1).

**Effort.** 1–2 weeks. **Tier.** B. **Depends on.** 1.1, 2.1.

### Step 3.2 — Code↔doc trace edges in Nexus (offline LLM mining)

**Goal.** Densify Nexus `DOCUMENTED_BY` and add `TRACES_TO` edges between ADRs/docs and code paths.

**Deliverables.**
- New job `cortex-ops mine-trace-links` driving an LLM (Opus/Sonnet) over the corpus.
- Output: candidate edges with `confidence ∈ [0,1]`. Persist edges with `confidence >= 0.7` automatically; queue `0.4–0.7` for review; drop `<0.4`.
- Recurring schedule: re-mine when source artifacts change.

**Acceptance criteria.**
- Eval harness shows F1@10 ≥ +8% on queries where ground truth is "ADR X → code path Y".
- Audit log records every edge with provenance (which LLM, which prompt version, which timestamp).
- Estimated one-off cost ≤ $200 for the full HiveLLM corpus (record actuals).

**Effort.** 2–4 weeks (mostly compute and review). **Tier.** B. **Depends on.** 1.1, 3.1.

### Step 3.3 — Provenance enforcement at consumption

**Goal.** Every claim in an LLM-facing surface carries a source ID; unsourced claims are flagged.

**Deliverables.**
- Updated prompt templates for `cortex_query`, `pre_thinking`, and MCP tool handlers requiring `[source: <id>]` after substantive claims.
- New verifier `crates/cortex-workers/src/verify/grounding.rs` that parses LLM output, extracts citations, and computes a grounding rate per turn.
- `cortex-audit` event `grounding_rate` per turn.
- Feature flag `grounding.enforce: bool` (default false → true after 2 weeks of observation).

**Acceptance criteria.**
- Grounding rate ≥ 0.85 on the regression eval set.
- No measurable degradation in user-facing answer quality (manual review of 20 sampled turns).

**Effort.** 1–2 weeks. **Tier.** B. **Depends on.** 2.2.

### Step 3.4 — Multi-language AST chunking (TS, Python)

**Goal.** Extend AST chunking to the rest of the HiveLLM ecosystem.

**Deliverables.**
- `TypeScriptChunker` and `PythonChunker` mirroring `RustChunker` semantics.
- Reindex of relevant repos.

**Acceptance criteria.**
- Eval harness with cross-repo queries shows positive deltas on TS/Python citations.
- Existing Rust performance unchanged.

**Effort.** 1–2 weeks. **Tier.** B. **Depends on.** 3.1.

---

## Phase 4 — Tier C (research / deferred)

### Step 4.1 — Modality-routed dual embedder

**Goal.** Code-specialized embedder for code, general for prose; fused at query time.

**Trigger condition.** Eval harness shows that 30%+ of regression cases involve cross-modal queries where the single embedder underperforms.

**Effort.** Research + 2–3 weeks build. **Tier.** C.

### Step 4.2 — BatCoder-style back-translation fine-tune

**Goal.** Fine-tune embedder on Cortex's own (code, doc) pairs.

**Trigger condition.** ≥500 high-quality pairs validated; Tier A and B exhausted.

**Effort.** Significant — GPU pipeline + training infra. **Tier.** C.

---

## Cross-cutting concerns

### Knowledge capture

After each step, run:
- `rulebook_knowledge_add pattern` — record what worked (e.g., "BGE-reranker-v2-m3 lifts MRR@10 by 7% on Cortex eval set").
- `rulebook_knowledge_add anti-pattern` — record what did not work (e.g., "Removing BM25 from the fusion dropped recall by 20%").
- `rulebook_learn_capture` — implementation insights that do not belong in code.
- `rulebook_decision_create` — ADR for any architecturally significant choice (reranker model, chunking strategy).

### Memory persistence

Save to `rulebook_memory_save`:
- Final eval-harness baseline metrics (so the next session can compare).
- Any model selection rationale (reranker, embedder).
- Cost actuals from Step 3.2 LLM mining.

### Audit and rollback

- Every step adds a `cortex-audit` event class. Document each in `docs/audit-events.md`.
- Every reindex uses versioned index names. Rollback = pointer swap.

---

## Next concrete action

If approved, the first task to create is:

```
rulebook_task_create
  title: "Build retrieval eval harness (CDC-001 Phase 1.1)"
  scope: "crates/cortex-eval/"
  proposal_source: "docs/analysis/code-doc-correlation/"
```

All subsequent steps depend on this. Until the harness exists, retrieval changes are unmeasurable, and the project should not ship them.
