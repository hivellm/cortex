# 07 — Incremental updates + content-hash caching — **MED**

## What graphify does

- **Manifest-driven incremental:** if `manifest.json` + `graph.json` exist, `detect_incremental()` returns `(new, unchanged, deleted)`; only changed files are re-extracted; `build_merge(prune_sources=deleted)` updates in place; **manifest written only on success** (crash-safe).
- **Content-hash semantic cache:** LLM extractions keyed by file SHA256; only uncached files hit the LLM. **Rename-safe** — a moved file reuses its cached result with `source_file` rewritten (no re-LLM on refactors).
- **Affected-node impact** (`affected.py`): `affected_nodes(graph, seed, depth, relations)` = BFS over `calls/references/imports/inherits/...` to find what a change *touches*, driving `--update` re-analysis scope.

## What Cortex does today

- **Producer checkpoints** (`producer_checkpoints` table, ADR-010; `crates/cortex-workers/src/producer/`): bootstrap/claude-archive/topic-cards/consolidator resume from a durable cursor — strong incremental *ingestion* (don't re-emit seen events).
- **Bootstrap walker** has size/skip filters and per-repo progress checkpoints.
- **Content-addressed storage** exists for envelopes (content hashes on events).
- **But:** no **affected-node graph impact analysis** (change file X → which graph nodes/summaries are now stale?), and consolidations/topic-cards re-synthesize on **event-count / age / impact triggers**, not on a precise "this edit touched these nodes" set.

**Gap:** Cortex is incremental at the *event/ingestion* layer but not at the *graph-impact* layer. A small code change can either over-trigger (re-synthesize broadly) or under-trigger (miss a dependent summary), because there's no BFS-of-affected-nodes to scope re-analysis.

## Recommendation for Cortex

- **Affected-node scoping for re-synthesis:** when a file's symbols change, BFS the graph (`calls`/`imports`/`defines`/`inherits`) to the affected node set, and use it to scope which **community summaries / topic-cards** to mark stale — instead of (or in addition to) the current count/age triggers. graphify's `affected.py` is a direct template; Cortex already has the edges and the staleness machinery (topic-card rewrite triggers, spec 12).
- **Rename-safe summary cache:** ensure a moved/renamed file reuses prior LLM syntheses keyed by content hash with path rewritten — adopt graphify's "update `source_file` in the cached entry" so refactors don't re-pay LLM cost. Check the consolidator's cache keys honor this.

## Effort / impact

- **Impact:** MED — sharper, cheaper re-synthesis; fewer stale summaries after edits. Compounds once community summaries (03) exist (precise invalidation).
- **Effort:** LOW-MED — reuses existing edges + staleness triggers; mainly an affected-set BFS + wiring it into the rewrite-trigger decision.
- **Note:** Cortex's event-bus incrementality is already *stronger* than graphify's file-manifest approach for the capture path — this is specifically about **graph-impact-scoped summary invalidation**, the one place graphify is ahead.
