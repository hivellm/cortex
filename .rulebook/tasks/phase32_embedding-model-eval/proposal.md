# Proposal: phase32_embedding-model-eval

## Why

Source: docs/analysis/cortex-platform-2026-07/README.md (see
execution-plan.md, "Better retrieval" section, `phase32_embedding-model-eval`).

The 2026-07-05 platform re-scope analysis and the archived
`phase26e_retrieval-quality-remediation` task converge on the same
finding: Cortex's embedding model (nomic-embed-text, 384-dim) caps raw
cosine similarity at roughly 0.42-0.45 for the benchmark query "event
classification system" (repo `cortex`, measured via `/v1/query`
per-source score), and this ceiling holds regardless of pipeline or data
hygiene. phase26e's ~90% corpus dedup/purge pass (a one-time transition
to content-independent stable vector ids that took the `cortex` repo
from ~169k to ~15.7k vectors) shrank the corpus measurably but did not
move the top-1 score past this ceiling (phase26e §1.4). This is already
captured in the knowledge base as the pattern
`separate-pipeline-hygiene-gains-from-embedding-model-ceiling-when-benchmarking-retrieval-quality`.
Further pipeline-quality work (NL summary quality, chunking, dedup) has
diminishing returns until the embedding model itself is evaluated and,
if warranted, replaced.

This task is deferred behind `phase28_retrieval-eval-gate-live` — both
per the execution plan's own dependency table and because any
embedding-model comparison run against the current placeholder/10-row
golden set would not be trustworthy. `phase28_retrieval-eval-gate-live`
is responsible for producing the real golden set (50-100 realistic
queries with ground-truth bundles); this task consumes that output
rather than duplicating it.

## What Changes

- Shortlist 2-3 candidate embedding models compatible with the
  Vectorizer SDK's supported backends. Vectorizer owns the actual
  embedding model, not Cortex (spec 06 "Decisions" §1: "Client, not
  model" — Cortex requests a model from Vectorizer, it does not run
  one), so any migration is ultimately a Vectorizer-side model change
  coordinated from Cortex's embedder config. The 2026-07-05 execution
  plan names starting candidates: BGE-small-en-v1.5 (384d, faster), a
  BAAI embedding variant (higher-dimensional, may clear the cosine
  ceiling by dimensionality alone), and a custom fine-tuned variant if
  Cortex-specific training signal exists. Given a large fraction of the
  corpus is source-code chunks (Tree-sitter symbol-level chunking per
  spec 06), prioritize candidates with code-aware training. Record
  licensing and self-hosting-vs-hosted-API latency/cost profile for
  each.
- Once `phase28_retrieval-eval-gate-live` has produced a real
  (non-placeholder) golden set, benchmark each candidate against it:
  recall@5, MRR@10, p50/p95 embedding latency, and per-embedding cost
  (hosted API) or resource cost (self-hosted).
- Re-run the exact benchmark query the 2026-07-05 analysis and phase26e
  used ("event classification system", repo `cortex`, per-source score
  via `/v1/query`) against each candidate, specifically to test whether
  it raises the raw-cosine ceiling itself — not just whether the
  fused/reranked score changes.
- Produce a recommendation: "stay on nomic-embed-text" (no candidate
  clears the ceiling meaningfully net of migration cost) or "migrate to
  X" with a concrete migration plan (full corpus re-embed, dual-write /
  cutover strategy, rollback plan). Record the recommendation as an ADR.

## Impact

- Affected specs: `docs/specs/06-embedder.md` ("Open questions" §1,
  per-collection/model choice — this task is the first
  retrieval-quality pass that question was deferred to), new spec delta
  `retrieval` (this task).
- Affected code: `crates/cortex-workers/src/embedder/` (embedding
  client + config — `EmbedderConfig::vector_dim`, env
  `CORTEX_EMBEDDER_DIM`; note production currently runs dim 384 for
  nomic-embed-text even though the in-code `Default` documents 768 as
  the FastEmbed baseline — confirm which is authoritative for the live
  deployment before benchmarking), and cross-repo, the Vectorizer
  service's model configuration. Requires a full corpus re-embed if a
  migration is recommended and approved.
- Breaking change: NO — this task is evaluation/recommendation only; an
  approved migration is its own follow-up implementation task.
- User benefit: settles whether the current retrieval-quality ceiling
  is fixable by a model change before investing further in
  pipeline-hygiene work that phase26e already showed cannot clear it
  alone.
