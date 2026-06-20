# Proposal: phase27c_graphrag-community-summaries

Source: docs/analysis/graphify-comparison/ (R3 file 03, R6 file 06)

## Why

Once graph communities exist (phase27b), Cortex can serve the GraphRAG
**global** query class it cannot answer today — "what are the subsystems
and how do they relate", architecture/onboarding questions that no single
chunk answers. Cortex's current summaries (consolidations, topic-cards)
are organized by embedding-similarity topic and event grain, **not** by
the graph's own community partition, so the topological view is missing.
This task adds (a) community summaries as a new consolidation grain + a
global query route, and (b) community-aware entity dedup — both are
graph-community-derived and naturally belong together. graphify uses the
Leiden community as both a summarization unit and a dedup tiebreaker
(same-community boost separates homonyms like "User" in auth vs.
shipping without embeddings).

## What Changes

- **Community summaries (R3):** new `Community` consolidation grain whose
  source selector = the nodes/edges/god-nodes of one graph community;
  reuses the existing consolidation envelope + storage. Hierarchy from
  Leiden levels (coarse subsystems → fine modules). Budgeted return
  (top-N summaries within the pre-thinking byte budget).
- **Global query route (R3):** the orchestrator detects
  architecture-level intent and routes to a map-reduce over community
  summaries instead of the per-chunk fusion lane.
- **Community-aware dedup (R6):** a graph entity-resolution pass reusing
  the existing MinHash util (today only in
  `crates/cortex-cli/src/ops/memory_consolidate.rs`): entropy gate →
  MinHash/LSH blocking → Jaro-Winkler verify → **same-community boost** →
  union-find merge with survivor-id preference. Optional LLM tiebreaker
  behind a flag + the existing daily budget tracker.

## Impact

- Affected specs: spec 12 (consolidation grains — add `Community`),
  spec 11 (global query route), `docs/specs/07-graph-writer.md` (dedup
  merge semantics).
- Affected code: `crates/cortex-workers/src/consolidator/source/` (new
  community source), `crates/cortex-api/src/search/` (intent route +
  map-reduce), a shared MinHash util + `graph/community.rs` dedup pass.
- Breaking change: NO (new grain, new route, additive dedup pass).
- User benefit: architecture/onboarding answers; cleaner graph (merged
  duplicate entities, homonyms disambiguated by community).
- Prereq: **phase27b** (communities). Distinct from embedding-topic
  consolidations — keep both axes.
