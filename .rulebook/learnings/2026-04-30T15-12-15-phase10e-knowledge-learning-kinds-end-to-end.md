# phase10e — knowledge/learning kinds end-to-end
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10e_knowledge_learnings_walker
**Tags**: knowledge, learnings, kind, walker, phase10e, rulebook
The 2026-04-29 audit found 60 high-signal entries on disk (`.rulebook/knowledge/**` + `.rulebook/learnings/**`) that no Cortex lane indexed — they were lumped under `FileClass::Memory` and routed to the catch-all `cortex_memories` index, so the orchestrator could not surface them distinctly.

Phase10e wires the full pipeline:
1. **Kind enum** — added `Kind::Knowledge` + `Kind::Learning` to `cortex-core`. Every match site needed updating (~15 sites across classifier/statics, workers/embedder/routing, workers/fulltext/builders × 4 arms, workers/fulltext/routing, workers/graph/mapper × 2, workers/bin/graph-backfill, cli/ops/doctor, cli/bootstrap/estimate, cli/bootstrap/emitter, cli/bootstrap/walker × 2). Mechanical but exhaustive — Rust's exhaustive matching catches everything via `cargo check`.
2. **Payloads** — new `KnowledgePayload { knowledge_id, title, category, body, source_path, tags }` + `LearningPayload { learning_id, title, body, related_task, source_path, tags }`. `category` discriminates pattern/anti-pattern from the path.
3. **Storage** — global `cortex.knowledge.fp32` + `cortex.learning.fp32` collections (single-tier; small + dense + high-signal corpus, demoting to PQ would lose precision); Meili indexes `cortex_knowledge` + `cortex_learnings` with bundled v1 settings JSON.
4. **Walker** — split canonical globs: `RULEBOOK_KNOWLEDGE_GLOBS` + `RULEBOOK_LEARNING_GLOBS` route to `FileClass::Knowledge`/`Learning` BEFORE the memory fallback. Emitter has `emit_knowledge_imported` + `emit_learning_imported` with synthetic `<repo>:<stem>` ids for idempotent re-walks.
5. **Workers** — embedder per-repo `knowledge` + `learnings` family; fulltext routing same; graph mapper rides alongside Memory (`emit_memory`) with dedicated `:Knowledge`/`:Learning` labels for the dashboard's graph view colour-coding.

Live-backend Nexus changes (`(:Session)-[:LEARNED]->(:Learning)` and `(:Knowledge)-[:RELATES_TO]->(:Decision)`) are deferred to phase10e-follow — those edges require richer relation-emission logic in the graph mapper. Today the structural mapping covers the canonical Session→Knowledge/Learning skeleton via `OWNS`, and the classifier-entities pass already supports cross-references.

Critical detail: every walker re-classification of an already-classified path needs corresponding test updates. The pre-existing `classify_picks_up_canonical_rulebook_layout_without_cortex_toml` test asserted Knowledge/Learning resolved to `FileClass::Memory`; updated to assert the new dedicated variants.