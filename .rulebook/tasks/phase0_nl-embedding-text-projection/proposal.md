# Proposal: phase0_nl-embedding-text-projection

Source: phase0_retrieval-ranking-quality §2 (2026-06-22).

## Why

The retrieval ranking diagnosis (phase0_retrieval-ranking-quality §1.3)
found the vector lane's dense scores are systematically ~half the keyword
scores, so the semantically-correct doc loses RRF fusion despite being
retrieved. Root cause: the classifier runs in Static mode
(`CORTEX_CLASSIFIER_MODE=static`) and emits `summary: None` /
`StaticFallback`, so the embedder embeds raw JSON payloads instead of
natural-language summaries — weak similarity for NL queries.

The cross-encoder reranker (phase0_retrieval-ranking-quality §3) already
lifted MRR@10 0.47 → 0.64, so this is no longer blocking the ranking
gate. But fixing the embedding text is the deeper fix: it would raise the
vector lane's own scores (helping pre-rerank fusion AND giving the
reranker cleaner candidate text), and likely recover the recall@5 dip
(1.0 → 0.80) the aggressive reranker introduced.

## What Changes

- Make the embedder embed a natural-language projection of each event
  (a descriptive rendering of the payload, or real classifier summaries)
  instead of raw JSON. Deterministic projection preferred (no LLM
  dependency) so it works with the classifier in Static mode.
- Re-embed / re-index the affected corpus (Vectorizer + the dense lane).
- Re-run `cortex-eval --suite retrieval` (fusion-only AND reranked);
  confirm the vector lane's pre-rerank MRR rises and recall@5 returns to
  1.0, and record the deltas.

## Impact
- Affected specs: `docs/specs/06-embedder.md`, `docs/specs/05-classifier.md`.
- Affected code: `crates/cortex-workers/src/embedder/**` (text projection),
  possibly `classifier_worker` summary path; a corpus re-index run.
- Breaking change: NO (retrieval quality only).
- User benefit: stronger semantic retrieval independent of the reranker;
  recovers the recall dip; cleaner reranker input.

## Note

Requires a corpus re-embed/re-index window (heavy). Sequenced after the
reranker win (already shipped) because the reranker already meets the
ranking gate; this is the deeper quality improvement.
