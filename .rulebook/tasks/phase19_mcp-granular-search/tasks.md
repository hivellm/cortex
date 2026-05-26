## 1. Group A — Envelope-shape granularity (5 tools)
- [x] 1.1 `cortex_events_by_kind`: `cortex-api` HTTP route `POST /v1/search/events` accepting `{kind, repo?, session_id?, since?, until?, limit<=50}`; reads from the Meili kind-routed indexes; returns the native envelope shape (no fusion projection). MCP tool wraps it. — Handler shipped in `crates/cortex-api/src/search/events_by_kind.rs` with `kind_to_index()` mapping 10 kinds (turn/tool_call/consolidation/decision/analysis/memory/law/knowledge/learning/topic_card) to canonical Meili index uids. Accepts both snake_case + PascalCase. Filter builder assembles `repo = "..." AND session_id = "..." AND occurred_at_ms >= N AND occurred_at_ms <= N` with quote-escaping. Sort hardcoded `occurred_at_ms:desc` (newest-first). Route mounted in `http.rs`. MCP tool `EventsByKindTool` in `tools.rs` wraps via `proxy_search`. Registry 13 → 14. Tests: 7 unit (kind_to_index coverage + reject + build_filter empty/clauses/timestamps/escape + clamp_limit + parse_rfc3339) + 4 wiremock IT (`tests/events_by_kind_it.rs`: happy path / bad_input / api_unreachable / descriptor enum pin) = 11 new tests, all green.
- [ ] 1.2 `cortex_session_timeline`: HTTP `GET /v1/sessions/{session_id}/timeline?limit=<=200`; reads from `cortex_events.archive_loader` ordered by `occurred_at asc`; returns `{ts, kind, summary_or_title, event_id, deltas}`. MCP tool wraps it.
- [ ] 1.3 `cortex_tool_calls`: HTTP `POST /v1/search/tool-calls` accepting `{tool_name?, exit_code_eq?, exit_code_ne?, has_error?, repo?, session_id?, since?, until?, limit<=50}`; reads from `cortex_tool_calls` Meili index with native filter. MCP tool wraps it.
- [ ] 1.4 `cortex_files_touched`: HTTP `GET /v1/sessions/{session_id}/files-touched` OR `POST /v1/search/files-touched {repo, since, until}`; aggregates `(read_count, write_count, last_touched_ts)` per path from ToolCall envelopes. MCP tool wraps it.
- [ ] 1.5 `cortex_topic_search`: HTTP `POST /v1/topic-cards/search {topic_prefix, repo?, limit<=30}`; reads `cortex_topic_cards` index + the per-envelope `topics: Vec<String>` extras for the cross-kind hit list. MCP tool wraps it.

## 2. Group B — Consolidation-first (6 tools)
- [ ] 2.1 `cortex_consolidation_get`: HTTP `GET /v1/consolidations/{id}` returning `{summary, entities, relations, source_session_ids, cost, grain, repo, topic_id, occurred_at}`. MCP tool wraps it.
- [ ] 2.2 `cortex_consolidations_recent`: HTTP `GET /v1/consolidations/recent?repo&grain&since&until&limit<=30`. MCP tool wraps it.
- [ ] 2.3 `cortex_consolidations_by_entity`: HTTP `POST /v1/consolidations/by-entity {entity: {kind: "file"|"function"|"decision_id"|"repo"|"model", value}, limit<=30}`; reads `extras.entities` + `extras.relations` via Meili filter. MCP tool wraps it.
- [ ] 2.4 `cortex_consolidations_search`: HTTP `POST /v1/consolidations/search {query, k<=20, intent_hint?}`; hybrid BM25 + vector + RRF scoped to `cortex_consolidations` only. MCP tool wraps it.
- [ ] 2.5 `cortex_consolidation_lineage`: HTTP `GET /v1/consolidations/{id}/lineage` returning `{source_session_ids, decisions[], files[], cost{model,cents,prompt_tokens,completion_tokens}}`. Read joins `consolidations` + `decisions` + per-session tool-call file extracts. MCP tool wraps it.
- [ ] 2.6 `cortex_consolidations_diff`: HTTP `GET /v1/consolidations/diff?since_ts=<ms>&limit<=200`; reads from the `cortex_consolidations` Meili index ordered by `accumulated_at asc`. MCP tool wraps it.

## 3. Group C — Governance + telemetry (5 tools)
- [ ] 3.1 `cortex_law_violations`: HTTP `POST /v1/laws/violations {repo?, session_id?, law_id?, since?, until?, limit<=50}`; returns `(ts, session_id, tool_name, law_id, reason, originating_prompt)`. MCP tool wraps it.
- [ ] 3.2 `cortex_feedback_signals`: HTTP `POST /v1/feedback/list {helpful?, intent?, repo?, since?, until?, limit<=50}`; reads from the `pre_thinking_feedback` SQLite table (phase14f). MCP tool wraps it.
- [ ] 3.3 `cortex_decision_search`: HTTP `POST /v1/decisions/search {status?, supersedes?, superseded_by?, tag?, limit<=50}`; reads from `cortex_decisions` Meili index. MCP tool wraps it.
- [ ] 3.4 `cortex_consolidation_costs`: HTTP `POST /v1/consolidations/costs {since, until, group_by: ["grain", "model", "day"]}`; aggregates from the `grain_costs` SQL table. MCP tool wraps it.
- [ ] 3.5 `cortex_query_explain`: HTTP `POST /v1/query/explain {query, intent?, scope?}`; returns `{per_lane_hits[], fusion_math{rrf_k, weights, drops}, final_envelope}`. MCP tool wraps it.

## 4. Registry + spec
- [ ] 4.1 `ToolRegistry::default_set()` updated from 13 -> 29 tools. `MCP_REGISTRY_SIZE` test in `crates/cortex-mcp-server/tests/end_to_end.rs` + the descriptor count assertion in `hook_drift.rs` bumped.
- [ ] 4.2 `lib.rs` re-exports every new `*Tool` struct.
- [ ] 4.3 `docs/specs/22-fine-grained-search.md` extended with a "Phase19 granular tool surface" section + the wire-shape + error-taxonomy table per group.
- [ ] 4.4 `docs/specs/18-claude-code-plugin.md` MCP tool list updated with the 16 new entries.
- [ ] 4.5 `docs/specs/27-consolidation.md` cross-references the new consolidation-first tools.

## 5. Tests (closes glm5.1 Task 6.4 in the same phase)
- [ ] 5.1 One wiremock IT per new tool in `crates/cortex-mcp-server/tests/` covering: happy path, invalid-input, upstream 5xx (returns `api_http_error`), upstream timeout (returns `tool_timeout` per phase14i), budget overflow (returns `budget_exceeded` per phase11c).
- [ ] 5.2 Per-route `cortex-api` unit tests cover the SQL/Meili filter shapes + the 4xx error paths (missing param, malformed payload, unknown enum).
- [ ] 5.3 `cortex-eval` golden-set: add a `mcp_search` suite that drives a 10-row fixture per granular tool and asserts the recall floor (`recall@5 >= 0.5` for entity / topic / file-touched lookups).
- [ ] 5.4 Workspace smoke: `cargo check --workspace && cargo clippy -p cortex-mcp-server -p cortex-api -- -D warnings && cargo test -p cortex-mcp-server -p cortex-api` clean.

## 6. Tail (mandatory)
- [ ] 6.1 Update `docs/specs/22-fine-grained-search.md` + `docs/specs/27-consolidation.md` + `docs/specs/18-claude-code-plugin.md` + `CHANGELOG.md`.
- [ ] 6.2 `rulebook_learn_capture` with title "MCP granular search verbs land — consolidations promoted to first-class corpus".

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation
- [ ] 99.2 Write tests covering the new behavior
- [ ] 99.3 Run tests and confirm they pass
