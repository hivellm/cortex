# Proposal: phase27a_graph-edge-confidence-tiers

Source: docs/analysis/graphify-comparison/ (R1, file 04a)

## Why

Today every edge written to the Nexus graph carries a provenance triple
(`source_event_id`, `analyzer_version`, `created_at_ms`) but **no
confidence level**. A deterministic, AST-proven `defines` edge and an
LLM-inferred `relates_to`/`about` edge are indistinguishable to any
consumer by trustworthiness. graphify tags every edge
EXTRACTED / INFERRED / AMBIGUOUS with a score, which lets the graph lane
down-weight weak edges and lets reviewers flag uncertain ones. This is
the single cheapest high-value idea from the graphify analysis — it
rides on plumbing that already exists and makes the graph lane measurably
less noisy.

## What Changes

- Add a `confidence` enum (`Extracted | Inferred | Ambiguous`) plus an
  optional `confidence_score: f32` to the edge / `NodeOp` edge model in
  `cortex-workers`.
- Stamp it in every extractor under
  `crates/cortex-workers/src/graph/extractors/`: deterministic
  tree-sitter edges (`defines`, `imports`, `calls`, `returns`,
  `inherits`) → `Extracted` (1.0); analyzer/LLM-derived edges
  (`relates_to`, `about`, `answered_by`, `mentions_file`) → `Inferred`
  with a rubric score; reserve `Ambiguous` for low-confidence matches.
- Persist the field through the graph writer to Nexus (additive node/edge
  property; back-compat — absent = unknown).
- Weight the graph lane in the query orchestrator
  (`crates/cortex-api/src/search/strategies.rs`) by confidence, and
  surface `Ambiguous` edges in the dashboard graph view.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` (edge schema +
  confidence), spec 11 (graph lane weighting).
- Affected code: `crates/cortex-workers/src/graph/extractors/*`, the
  edge/`NodeOp` model + `graph/projection.rs`, the graph writer,
  `crates/cortex-api/src/search/strategies.rs`,
  `crates/cortex-api/src/dashboard/graph.rs`.
- Breaking change: NO (additive property; consumers tolerate absence).
- User benefit: less noisy graph lane (weak edges ranked down),
  reviewable provenance (filter/flag by trust), and a reusable signal for
  later graph work (community dedup, GraphRAG).
