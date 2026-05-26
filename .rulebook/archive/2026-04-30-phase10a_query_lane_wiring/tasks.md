## 1. Decision lane
- [x] 1.1 In `crates/cortex-api/src/strategies.rs`, add `decision_lookup` to the lane-set table: vectorizer (`cortex.decision.fp32`) + meili (`cortex_decisions`) + nexus (`:Decision`).
- [x] 1.2 `decisions` lane returns `{ id, title, rationale_excerpt, status, repo, occurred_at }` (excerpt: first 1 KiB of the rationale body, not the title).
- [x] 1.3 Orchestrator fuses the three lanes via the existing RRF blend with the same weights `pre_change_context` uses.

## 2. Law lane
- [x] 2.1 Add `law_check` to the lane-set table: meili (`cortex_laws` + `cortex_violations`) + nexus (`:Law`).
- [x] 2.2 `laws` lane returns `{ law_id, title, severity, body_excerpt, applies, violations_7d }`.
- [x] 2.3 Always include the body excerpt — the dashboard row shows `body_excerpt` is populated, the lane just isn't projecting it.

## 3. Turn lane
- [x] 3.1 Add `similar_problems` to the lane-set table: vectorizer (`cortex.turn.fp32`/`pq`) + meili (`cortex_turns`).
- [x] 3.2 `similar_turns` lane returns `{ event_id, session_id, occurred_at, repo, snippet }`.
- [x] 3.3 Honor `scope.repo` so a repo-scoped query does not bleed across sessions.

## 4. Tests + harness
- [x] 4.1 Direct unit tests in `crates/cortex-api/src/strategies.rs` and `orchestrator.rs` for each new intent's lane composition.
- [x] 4.2 Re-run `cargo run -p cortex-cli --bin cortex-relevance-eval` against a fresh `cortex-api` and assert recall@10 > 0 for `law_check`, `decision_lookup`, `similar_problems`.
- [x] 4.3 Update `tests/relevance/queries.toml` to lowercase repos consistently (so the harness omits zero buckets).

## 5. Spec / docs
- [x] 5.1 Update `docs/specs/11-query-api.md` §"Lane composition per intent" with the new mapping.
- [x] 5.2 Cross-link from `docs/specs/13-laws-dsl.md` §retrieval and `docs/specs/16-dashboard.md`.

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation
- [x] 6.2 Write tests covering the new behavior
- [x] 6.3 Run tests and confirm they pass
