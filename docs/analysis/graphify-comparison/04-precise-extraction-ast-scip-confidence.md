# 04 — Precise extraction: deterministic AST + SCIP + confidence tiers — **HIGH**

## What graphify does

1. **Deterministic AST first, LLM second.** Code → tree-sitter (25+ langs, `ProcessPoolExecutor` to dodge the GIL) → `calls`/`imports`/`defines` edges with **EXTRACTED** confidence (1.0). Only prose/images/video reach an LLM (INFERRED edges, scored 0.55–0.95). Result: zero-token, repeatable, ground-truth code edges.
2. **SCIP ingestion** (`scip_ingest.py`): consumes language-server symbol indexes (Rust-analyzer, gopls, tsserver) as JSON. Two-pass — build a symbol→node-id index, then emit edges resolving each reference to its *exact* definition (same-document precedence, global fallback, `scip_external` stubs so edges never dangle). This is **precise** cross-file resolution, not name-matching heuristics.
3. **Confidence tagging on every edge:** `confidence ∈ {EXTRACTED, INFERRED, AMBIGUOUS}` + `confidence_score`. Agents and reports filter by trust; AMBIGUOUS edges get flagged for review.

## What Cortex does today

- **Tree-sitter, ~10 languages** (verified: `tree_sitter_{c,cpp,go,java,javascript,json,python,rust,toml,typescript}`). Code edges come from the semantic extractors (`crates/cortex-workers/src/graph/extractors/{calls,imports,defines,returns,inherits,...}.rs`) + the static analyzer pass in the graph worker.
- These edges are **heuristic name/scope resolution**, not symbol-table precise — a `calls` edge to `foo()` can't always disambiguate which `foo` across crates. (graphify hits the same wall *without* SCIP; with SCIP it's exact.)
- Edges carry a **provenance triple** (`source_event_id`, `analyzer_version`, `created_at_ms`) but **no confidence tier** — an AST-proven `defines` edge and an LLM-inferred `relates_to` edge are indistinguishable to a consumer by trust level.
- **No SCIP / LSIF / language-server ingestion** anywhere (grep: zero matches).

**Gap:** the graph lane is noisier than it needs to be. Consumers can't down-weight low-confidence edges, and cross-reference precision is bounded by tree-sitter heuristics even though Rust-analyzer (already in the dev loop) can emit exact references.

## Recommendation for Cortex

Two independent, additive changes — do the cheap one first:

### 4a. Edge confidence tiers (cheap, high ROI)
Add a `confidence` field (enum `Extracted | Inferred | Ambiguous`) + optional `confidence_score` to the edge/`NodeOp` model and stamp it in every extractor: deterministic tree-sitter edges → `Extracted`; analyzer-inferred or LLM-derived edges (`relates_to`, `about`, `answered_by`) → `Inferred` with a score. Then let the **graph lane in the orchestrator** (`crates/cortex-api/src/search/`) weight by confidence, and let the dashboard flag `Ambiguous`. This mirrors graphify's single most broadly useful idea and rides on the provenance plumbing that already exists.

### 4b. SCIP ingestion (bigger, precision win)
Add a `cortex-scip` ingestion path: run `rust-analyzer scip` (and `scip-typescript`, etc.) in bootstrap/CI, parse the SCIP index, and emit **precise** `calls`/`references`/`defines` edges (tagged `Extracted`, confidence 1.0), superseding the heuristic edges where SCIP covers the file. Port graphify's two-pass resolver + `scip_external` stub pattern so edges never dangle (Cortex's writer already soft-drops endpoint-missing edges — SCIP stubs would convert drops into resolvable anchors). Start Rust-only (highest value, the codebase is Rust), extend by language as SCIP indexers are available.

## Effort / impact

- **4a confidence tiers:** Impact MED-HIGH (better graph-lane precision + reviewable edges), Effort LOW (one field + per-extractor tagging + one weighting term). **Do this first.**
- **4b SCIP:** Impact HIGH (exact xrefs; best graph lane in the ecosystem), Effort MED-HIGH (new ingest path + per-language indexer orchestration in bootstrap/CI). ADR-worthy.
- **Note:** graphify's ProcessPool/GIL point is Python-specific — N/A for Rust (Cortex already parallelizes natively).
