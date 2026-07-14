# Proposal: phase0_retrieval-ranking-quality

Source: phase0_live-eval-gates-rerank-phantomlink (retrieval baseline,
commit adf6153, 2026-06-22).

## Why

The newly-captured live retrieval baseline isolates the relevance
problem precisely: against the curated golden set (`crates/cortex-eval/
tests/golden/retrieval.csv`, 10 queries), **recall@5 = 1.0 but
MRR@10 = 0.47**. Every genuinely-correct document IS indexed and IS
returned in the top 5 — but it is usually ranked ~2–3, not #1. So the
recurring complaint "Cortex retrieval doesn't return anything relevant"
is not a *data* gap (recall is perfect); it is a **ranking** gap: the
right answer is buried under less-relevant hits. This task fixes ranking
and proves the fix with the MRR number rising while recall holds.

## What Changes

Investigate and improve ranking across the real levers, each measured
against the golden baseline (recall@5 must stay ≥ 1.0; MRR@10 must rise):

1. **Weak vector lane (suspected root cause).** The classifier runs in
   Static mode (`CORTEX_CLASSIFIER_MODE=static`), which emits
   `summary: None` / `StaticFallback` (`crates/cortex-workers/src/
   classifier_worker/worker.rs`). The embedder then embeds raw JSON
   payloads instead of natural-language summaries, so dense similarity
   for NL queries is low and the keyword lane dominates with raw file
   paths. Make the embedding text natural-language (real summaries, or a
   descriptive projection) so the vector lane ranks NL queries well.
2. **Cross-encoder reranker (phase17, already shipped, currently off).**
   Stand up a TEI reranker endpoint, set `CORTEX_RERANKER_ENABLED=1` +
   `CORTEX_RERANKER_ENDPOINT`, and measure the MRR lift. Gate (phase17
   §2.7): MRR@10 ≥ +5% over baseline AND p95 latency increase ≤ 250 ms.
3. **RRF fusion + recency tuning.** `crates/cortex-api/src/search/
   fusion.rs` (`RRF_K = 60.0`, alpha = 70% positional / 30% native
   score; env `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K`) and
   `crates/cortex-api/config/relevance.toml` (per-intent recency λ).
   Tune only with the golden harness as the arbiter — no blind changes.
4. **Re-measure + record.** Re-run `cortex-eval --suite retrieval`
   against the live stack; record the new MRR@10 / recall@5 into
   `crates/cortex-eval/baselines/cdc-baseline-v1.json` and the knowledge
   base.

## Impact
- Affected specs: `docs/specs/11-query-api.md` (RRF/fusion),
  `docs/specs/37-retrieval-rerank.md` (reranker eval gate),
  `docs/specs/05-classifier.md` / `06-embedder.md` (summary→embedding).
- Affected code: `crates/cortex-api/src/search/{fusion,orchestrator,
  relevance_config}.rs`, `crates/cortex-workers/src/classifier_worker/`
  + embedder text projection, `config/relevance.toml`, compose
  (reranker TEI + env), `crates/cortex-eval/baselines/`.
- Breaking change: NO (ranking quality only).
- User benefit: the right answer ranks #1 — retrieval stops "returning
  nothing relevant" because the relevant hit is no longer buried.

## Measurement gate

Every lever is accepted ONLY if the golden harness confirms it:
MRR@10 strictly increases vs the 0.47 baseline and recall@5 stays 1.0.
A lever that does not move MRR (or regresses recall) is reverted.
