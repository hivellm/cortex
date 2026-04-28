## 1. Lane projection — kind-aware text precedence
- [ ] 1.1 Replace the fixed `summary > title > body` chain in `MeiliKeywordLane::project` (`crates/cortex-api/src/meili_lane.rs`) with a kind-aware match: artifact / law_violation prefer `body > summary > title`; decisions / analyses / memories keep `summary > title > body`; turns / tool_calls / agent_calls use `summary > body > title`; default falls back to today's chain
- [ ] 1.2 Add a `tracing::debug!` when the projected body exceeds `cortex_fulltext::OVERSIZE_BODY_BYTES` so oversized artifact bodies are visible without forcing per-snippet clamping in the lane

## 2. Worker-side guard — surface empty-doc writes
- [ ] 2.1 In `crates/cortex-fulltext/src/builders.rs`, emit a `tracing::warn!` near the `select_body` step when `chosen.body`, `summary`, AND `title` are all empty — keeps the write path unchanged but flags the upstream bug

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation — append the kind-aware projection table to `docs/specs/08-fulltext-indexer.md` and to `docs/specs/11-query-api.md` (snippet `text` field semantics); add `F-009 — Meili artifact projection prefers path over body` to `docs/analysis/relevance/01-findings.md`
- [ ] 3.2 Write tests covering the new behavior — `cortex-api` unit on `MeiliKeywordLane::project` with one fixture per kind asserting the chosen field; regression test pinning that `kind=artifact` with non-empty `body` projects `body` to `text`, NOT the path
- [ ] 3.3 Run tests and confirm they pass — `cargo test -p cortex-api -p cortex-fulltext` green with zero warnings; live smoke (manual): `cortex_query free_search "JWT refresh"` against `scope.repo=cortex` returns `vectorizer_lane.rs` in the top 5
