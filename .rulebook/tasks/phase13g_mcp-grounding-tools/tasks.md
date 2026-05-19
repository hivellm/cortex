## 1. `cortex_active_work` — operator-state snapshot
- [ ] 1.1 New module `crates/cortex-api/src/active_work.rs`. Walks `.rulebook/tasks/*/{.metadata.json,tasks.md}` + `.rulebook/archive/*` under `CORTEX_WORKSPACE_ROOT`. Wrapped in mtime+TTL (60s) cache mirroring `tasks_loader`.
- [ ] 1.2 Response shape: `{ active_tasks: [TaskRow], in_progress_count, blocked_count, recent_archives: [ArchiveRow] }`. `TaskRow = { id, phase, status, next_unchecked_item: Option<String>, blocked_reason: Option<String> }`. `next_unchecked_item` reads the first `- [ ]` from `tasks.md` (lowest section).
- [ ] 1.3 Route `GET /v1/dashboard/active-work` on `cortex-api` returning JSON. Query param `?repo=<slug>` filters by `proposal.md` `Affected code` first path segment.
- [ ] 1.4 MCP tool `cortex_active_work` in `crates/cortex-mcp-server/src/tools.rs`. Descriptor + handler POSTs to the `/v1/dashboard/active-work` route, applies `MCP_RESPONSE_HARD_CAP` budget (truncates `next_unchecked_item` to 256 chars, caps `active_tasks` at 50).
- [ ] 1.5 Unit tests: 6 (cache HIT, cache MISS-after-mtime-tick, repo-filter, blocked-detection from `metadata.status="blocked"`, next-unchecked extraction with multi-section `tasks.md`, MCP budget cap).

## 2. `cortex_similar_sessions` — consolidation vector search
- [ ] 2.1 New module `crates/cortex-api/src/similar_sessions.rs`. Inputs: `{ query: String, repo: Option<String>, k: u32 (default 5, max 10), confidence_floor: f32 (default 0.6) }`. Outputs: top-K `ConsolidationHit { consolidation_id, session_id, title, summary_markdown, source_event_count, occurred_at, score }`.
- [ ] 2.2 Embeds `query` via the same path the embedder uses for ingestion (`cortex_workers::embedder::vectorizer_client::LiveVectorizerClient::embed_query`), then searches `cortex-<repo>-consolidations` Vectorizer collection (or all consolidation collections when repo unset) via `vectorizer-sdk::search`. Applies the confidence floor.
- [ ] 2.3 Route `POST /v1/search/similar-sessions` on `cortex-api`. Body matches the input shape; response is `Vec<ConsolidationHit>`.
- [ ] 2.4 MCP tool `cortex_similar_sessions` registered alongside the existing `cortex_vector_search`. Cypher gate not applicable (no graph access). Cap K at 10 server-side.
- [ ] 2.5 Unit tests: 5 (empty corpus returns empty Vec, repo filter scopes to one index, confidence floor drops sub-0.6 hits, K cap clamps requests > 10, score is reciprocal-rank-of-distance not raw cosine).

## 3. `cortex_decision_chain` — ADR supersession walker
- [ ] 3.1 New module `crates/cortex-api/src/decision_chain.rs`. Input: `{ event_id: String, max_hops: u32 (default 16) }`. Output: `{ chain: Vec<DecisionNode>, walked_predecessors: u32, walked_successors: u32 }`. `DecisionNode = { event_id, slug, status, date, title, supersedes: Option<event_id>, superseded_by: Option<event_id> }`.
- [ ] 3.2 Cypher walks both directions via `nexus-graph-sdk`: `MATCH (d:Decision { event_id: $id })<-[:SUPERSEDES*0..16]-(pred:Decision)` for predecessors and `(d)-[:SUPERSEDES*0..16]->(succ:Decision)` for successors. Merges results into a single chronological vector by `date`. Cycle-detection: when a node already appears in the result set, the walker stops that branch.
- [ ] 3.3 Route `GET /v1/search/decision-chain?event_id=<id>&max_hops=<n>` on `cortex-api`. Returns the merged chain.
- [ ] 3.4 MCP tool `cortex_decision_chain`. Handler enforces `max_hops <= 16`; rejects `event_id` not matching `[0-9A-Z]{26}` (ULID regex).
- [ ] 3.5 Unit tests: 6 (no-supersession case returns single-node chain, linear chain A->B->C produces 3-node ordered result, fork A<-B, A<-C produces both predecessors, cycle guard breaks loop A->B->A at hop 2, max_hops cap stops walk early, invalid event_id returns 400).

## 4. Pre-thinking integration
- [ ] 4.1 `crates/cortex-pre-thinking/src/formatter.rs` — add three section renderers: `render_active_work` (cap 1200 bytes), `render_similar_sessions` (cap 2000 bytes), `render_adr_provenance` (cap 800 bytes, conditional).
- [ ] 4.2 New `section_caps::ACTIVE_WORK_BYTES = 1_200`, `SIMILAR_SESSIONS_BYTES = 2_000`, `ADR_PROVENANCE_BYTES = 800`. Sum stays under the existing pre-thinking 12 KB ceiling.
- [ ] 4.3 The orchestrator queries `cortex_active_work` unconditionally, `cortex_similar_sessions` when the query has > 16 chars (avoid noise on bare tool pings), and `cortex_decision_chain` only when an ADR event_id pattern is present in the fusion result OR in the query string.
- [ ] 4.4 Render order: laws (existing, top) → active work → similar sessions → ADR provenance → consolidated context (existing) → past turns (existing) → snippets (existing). Active work and similar sessions outrank past raw turns because they carry richer signal per byte.
- [ ] 4.5 Unit tests: 6 (active-work section budget cap respected, similar-sessions section omitted on bare query, ADR provenance triggers on ULID pattern, render order preserved, total bundle stays under 12 KB ceiling, missing-tool error degrades gracefully — section omitted, others continue).

## 5. Specs + docs
- [ ] 5.1 Update `docs/specs/11-pre-thinking-context.md` § Sections — add the three new sections + their caps.
- [ ] 5.2 Update `docs/specs/22-fine-grained-search.md` — extend tool table to 13, document the three new request/response shapes + error taxonomy entries.
- [ ] 5.3 Update `CHANGELOG.md` Added entry covering the three tools + the pre-thinking integration.

## 6. Tail (mandatory)
- [ ] 6.1 Update or create documentation covering the implementation.
- [ ] 6.2 Write tests covering the new behavior.
- [ ] 6.3 Run tests and confirm they pass.
- [ ] 6.4 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
