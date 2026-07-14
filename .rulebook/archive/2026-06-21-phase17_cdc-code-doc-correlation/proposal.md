# Proposal: phase17_cdc-code-doc-correlation

## Why

Cortex retrieval surfaces low-relevance results — the user (André, maintainer) has reported that "the data so far does not produce anything actually relevant." A 50-source academic survey (`docs/analysis/code-doc-correlation/`) attributes this to four addressable gaps: (1) no eval harness to measure regressions, (2) no cross-encoder reranker after spec-11 fusion, (3) no phantom-link verifier that confirms cited symbols exist, and (4) no lifecycle/recency weighting on ADR retrieval. Each gap is documented in the literature with concrete numbers (LLM trace F1 0.79 vs BM25 0.44, hybrid rerank +5–10% MRR, supersession demotion +10% recall@5 on accepted ADRs). Without these fixes, every subsequent Cortex feature inherits the relevance debt and the user-reported failure mode persists.

## What Changes

This umbrella task lands the four Tier-A recommendations from CDC-001 in dependency order:

- **P1 — Eval harness.** Reuses `phase14c_golden-set-eval-harness` (already pending). All subsequent CDC phases depend on its acceptance gate being green.
- **P2 — Cross-encoder reranker.** New module `crates/cortex-workers/src/rerank/` with `BgeRerankerV2M3` (or equivalent). Wired into the fusion lane after BM25+dense+graph fusion. Configurable via `cortex-config` (`reranker.enabled`, `reranker.model`, `reranker.top_k_input`, `reranker.timeout_ms`). Fail-open on timeout.
- **P3 — Phantom-link verifier.** New module `crates/cortex-workers/src/verify/symbols.rs` using Tree-sitter (Rust + Markdown initially). Verifies every cited `(path, symbol)` resolves; flags or filters unresolved per config.
- **P4 — Supersession + recency weighting on decision lookup.** New scoring function applied to ADR retrieval: `score' = base × lifecycle_weight × recency_decay`. Subsumed by `phase18_tlb-timeline-branching` Phase 2 once that lands; documented as superseded then.

Tier-B and Tier-C items (AST chunking, trace-link mining, provenance enforcement, dual embedders, back-translation training) are scoped out of this task; they will be follow-ups created after Phase 1 baseline is locked.

## Impact

- **Affected specs:** `docs/specs/29-eval.md` (extends via phase14c), new spec `docs/specs/37-retrieval-rerank.md`, new spec `docs/specs/28-phantom-link-verifier.md`, new spec `docs/specs/29-decision-supersession-weighting.md`.
- **Affected code:** `crates/cortex-workers/src/{rerank,verify,scoring}/`, `crates/cortex-config/src/config.rs` (new sections), `crates/cortex-api/src/http.rs` (audit emission), `crates/cortex-pre-thinking/src/bundle.rs` (consume verifier output).
- **Breaking change:** NO. All four features land behind feature flags with `enabled = true` defaults justified by eval gate.
- **User benefit:** Measurable retrieval quality lift (target MRR@10 +15% combined) + elimination of phantom-symbol citations + correct ADR ranking + a measurement substrate that prevents regressions on every future Cortex change.

## Source

`docs/analysis/code-doc-correlation/` (README, findings, gaps, execution-plan, references). Cross-references CDC-001 throughout.

## Dependencies

- Hard: `phase14c_golden-set-eval-harness` must reach the acceptance gate before P2/P3/P4 merge.
- Soft: `phase18_tlb-timeline-branching` Phase 2 (temporal classifier) will supersede CDC P4; coordinate to land P4 first as a stepping stone, then mark superseded.
