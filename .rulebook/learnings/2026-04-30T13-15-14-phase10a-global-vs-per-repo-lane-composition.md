# phase10a — global vs per-repo lane composition
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10a_query_lane_wiring
**Tags**: query-api, strategies, meili, vectorizer, scope, phase10a
The 2026-04-29 relevance audit caught `decision_lookup`, `law_check`, and `similar_problems` returning empty because the strategies layer was fanning out to per-repo `cortex-{slug}-{family}` indexes that frequently carried no data. The fix is to route those intents to the GLOBAL stores defined in `crates/cortex-storage/src/names.rs` (`cortex.decision.fp32`, `cortex.turn.fp32`/`pq`, `cortex_decisions`, `cortex_turns`, `cortex_laws`).

Two gotchas:
1. `cortex_laws` does NOT have `repo` as a filterable attribute — strip `scope.repo` in `law_check` before fan-out, otherwise Meili rejects the search with a 4xx.
2. When switching to global indexes that DO carry `repo` filterable (`cortex_turns`, `cortex_decisions`), the keyword lane MUST translate `scope.repo` into a `repo = '<slug>'` Meili filter — otherwise a repo-scoped query bleeds across other repos sharing the same global index.

Decision overlay separately needed `decision_title` + `rationale_excerpt` stamped into `LaneHit.extras` because the meili_lane projection sets `symbol = doc.kind` (the literal "decision") rather than the real ADR title; pre-fix every decision overlay rendered as the string "decision".