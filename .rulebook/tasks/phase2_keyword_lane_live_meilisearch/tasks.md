## 1. Live keyword lane impl
- [ ] 1.1 Add `meilisearch-sdk` (or `reqwest` with typed bodies) to the workspace
- [ ] 1.2 New module `cortex-fulltext::keyword_lane` exposing `MeiliKeywordLane { client, index_alias }`
- [ ] 1.3 Implement `KeywordLane::search` translating `KeywordRequest` to Meili `/indexes/{index}/search` with `q`, `filter`, `limit`
- [ ] 1.4 Map Meili hits to `LaneHit` with `source = "keyword"` and the real `_rankingScore`
- [ ] 1.5 Connection-error path returns `LaneError::Transport`, not a panic — fail-open behaviour preserved upstream

## 2. Boot-time wiring in cortex-api
- [ ] 2.1 Read `MEILI_URL` / `CORTEX_MEILI_URL` env at startup; require an API key from `MEILI_MASTER_KEY` when present
- [ ] 2.2 Probe the server with a `health` GET before binding; on success swap `MemoryKeywordLane` for `MeiliKeywordLane`
- [ ] 2.3 On probe failure, log a warn with the URL + reason and keep `MemoryKeywordLane` as the fallback (preserve dev workflow)
- [ ] 2.4 `archive_loader.rs` seeds the live Meili index when the live lane is active (one-time + periodic), so the existing `~/.cortex/archive/` content is queryable

## 3. Source-label fix
- [ ] 3.1 Stop labelling keyword-derived hits as `source = "vector"` — verified misleading by 2026-04-27 audit
- [ ] 3.2 Add a debug assert in `Orchestrator::run` that every hit in the keyword lane's contribution carries `source = "keyword"`

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (extend spec-08 with a `## Read path` section)
- [ ] 4.2 Write tests: unit tests against a `wiremock` Meili double, integration test driving the orchestrator
- [ ] 4.3 Run tests and confirm they pass
