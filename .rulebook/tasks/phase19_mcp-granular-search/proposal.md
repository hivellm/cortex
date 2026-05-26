# Proposal: phase19_mcp-granular-search

Source: operator request 2026-05-26 + `docs/analysis/rework/glm5.1/execution-plan.md` Task 6.4 (MCP test gap).

## Why

The MCP server today exposes 13 tools but the search surface is
intentionally coarse:

- `cortex_query` fuses across every kind (Turn / ToolCall / Consolidation / Decision / etc.) — useful as a generic catch-all but the host cannot scope a query to one kind without post-filtering.
- `cortex_keyword_search` / `cortex_vector_search` / `cortex_graph_query` (phase11v) are raw per-backend pass-throughs — they bypass the fusion but expose no kind / repo / time / grain knobs beyond what the backend itself supports.
- `cortex_similar_sessions` (phase13g §2) is the only consolidation-aware tool, and it only does vector recall against `cortex_consolidations` — no BM25, no metadata filter, no lineage walk, no grain filter.
- `cortex_active_work` / `cortex_decision_chain` (phase13g §1, §3) cover two narrow operator-state slices.

The result: when an agent wants "all the `ToolCall` envelopes where `tool_name == "Bash"` AND `exit_code != 0` AND `repo == "cortex"` in the last 24h", or "every consolidation that mentions file `src/dispatcher.rs`", or "which laws fired during session X", the only path is `cortex_query` → manual filter in the host, which (a) bursts the 12 KB pre-thinking budget, (b) loses recall because the fusion drops candidates below the floor, and (c) makes consolidations (the highest-signal corpus the project produces) practically unreachable for structured questions.

Consolidations are the load-bearing corpus — they are the project's
distilled knowledge. The current MCP surface treats them as just
another kind in the fusion mix. Phase19 elevates them to first-class
queryable artefacts with dedicated tools for lineage + grain + entity
+ recency + cost.

## What Changes

Three groups of new MCP tools (16 in total, sized so each owns a
single SQL / Meili / Vectorizer round-trip):

### Group A — Envelope-shape granularity (5 tools)

1. **`cortex_events_by_kind`** — paged list of envelopes filtered by `kind` (Turn / ToolCall / AgentCall / Consolidation / Decision / Violation / KnowledgeNote / LearningNote / TopicCard) + optional `repo`, `session_id`, `since`/`until` window, `limit <= 50`. Reads from Meili kind-routed indexes (`cortex_tool_calls`, `cortex_turns`, `cortex_consolidations`, ...) so the response carries native shape, not a fusion projection.
2. **`cortex_session_timeline`** — every envelope for one `session_id` ordered by `ts asc`. Returns `{ts, kind, summary_or_title, event_id, deltas}` so the host can rebuild "what happened in session X" without a full archive walk.
3. **`cortex_tool_calls`** — granular ToolCall filter: `tool_name` (Bash / Read / Edit / WebFetch / ...), `exit_code` (eq / ne), `has_error` (bool), `repo`, `session_id`, `since`/`until`. Useful for "show me every failed Bash in repo X this week".
4. **`cortex_files_touched`** — given `session_id` or `(repo, since, until)`, return every file path that appeared in any `ToolCall.input.path` or `tool_call.payload.files_touched` extras, with per-file `(read_count, write_count, last_touched_ts)`.
5. **`cortex_topic_search`** — list every envelope tagged with a topic prefix (`tool:claude-code`, `kind:Bash`, `repo:cortex`). Reads `cortex_topic_cards` index + the per-envelope `topics: Vec<String>` extras the classifier stamps.

### Group B — Consolidation-first (6 tools)

6. **`cortex_consolidation_get`** — fetch one consolidation by `event_id` or `consolidation_id`. Returns the full payload: `summary`, `entities`, `relations`, `source_session_ids`, `cost`, `grain`, `repo`, `topic_id`. Today the host has to dig through `cortex_query` rows to assemble this.
7. **`cortex_consolidations_recent`** — chronological feed filterable by `repo`, `grain` in {session, topic, decision_trace}, `since`/`until`, `limit <= 30`. Drives the dashboard's Consolidations view without a `cortex_query` fusion call.
8. **`cortex_consolidations_by_entity`** — every consolidation mentioning an entity (file path, function name, decision id, repo, model). Reads `extras.entities` + `extras.relations` via Meili filter. Cross-session knowledge retrieval.
9. **`cortex_consolidations_search`** — pure hybrid search (BM25 + vector + RRF) scoped to `cortex_consolidations` only. No cross-kind fusion. Separate from `cortex_query` so the host can ask "what consolidations are similar to this prompt?" without diluting with tool_call hits.
10. **`cortex_consolidation_lineage`** — given a `consolidation_id`, return: `source_session_ids` (the sessions that fed it), `decisions[]` (ADRs cited), `files[]` (paths touched), `cost { model, cents, prompt_tokens, completion_tokens }`. Answers "why does this consolidation exist?".
11. **`cortex_consolidations_diff`** — given `since_ts`, return every consolidation accumulated after that point. Incremental sync; the host caches its own `last_seen_ts` and only pulls deltas.

### Group C — Governance + telemetry (5 tools)

12. **`cortex_law_violations`** — list every `PreToolUse` that the law-check denied: returns `(ts, session_id, tool_name, law_id, reason, originating_prompt)`. Reads from the violations index seeded by spec 13.
13. **`cortex_feedback_signals`** — list feedback rows (the phase14f surface) filterable by `helpful` (true/false), `intent`, `repo`, `since`/`until`. Returns `(query_id, intent, helpful, files_cited, free_text, implicit_score, ts)` so the host can audit pre-thinking quality.
14. **`cortex_decision_search`** — search decisions filterable by `status` in {proposed, accepted, superseded, deprecated, rejected}, `supersedes` (id), `superseded_by` (id), `tag`. Today `cortex_decision_chain` walks but does not search.
15. **`cortex_consolidation_costs`** — aggregate `(grain, model, day) -> cents + tokens + count` over a window. Operator-only.
16. **`cortex_query_explain`** — given a query string + intent, return per-lane raw hits + scores + the RRF fusion math. Debug helper for relevance tuning; complements `cortex_query` (which returns the fused answer without the inputs).

## Impact

- **Affected specs**:
  - `docs/specs/22-fine-grained-search.md` (extend the existing fine-grained search spec with the new tool surface — already covers phase11v's keyword/vector/graph trio, phase19 adds the 16-tool granular set).
  - `docs/specs/18-claude-code-plugin.md` § MCP tool list (extend the listing — phase14i shipped the timeout contract; phase19 adds the new entries).
  - `docs/specs/27-consolidation.md` (cross-reference the new consolidation-first tools).
- **Affected code**:
  - `crates/cortex-mcp-server/src/tools.rs` — 16 new `impl Tool` (registry size 13 -> 29).
  - `crates/cortex-mcp-server/src/lib.rs` — re-exports.
  - `crates/cortex-api/src/` — new HTTP routes backing each tool (`/v1/search/events`, `/v1/search/tool-calls`, `/v1/consolidations/{id}`, `/v1/consolidations/recent`, `/v1/consolidations/by-entity`, `/v1/consolidations/search`, `/v1/consolidations/{id}/lineage`, `/v1/consolidations/diff`, `/v1/laws/violations`, `/v1/feedback/list`, `/v1/decisions/search`, `/v1/consolidations/costs`, `/v1/query/explain`, `/v1/topic-cards/search`, `/v1/sessions/{id}/timeline`, `/v1/sessions/{id}/files-touched`).
  - `crates/cortex-storage/` — extends the metadata schema only when the lineage walker needs a new SQL join; otherwise read from existing tables.
  - Tests: `crates/cortex-mcp-server/tests/` adds one wiremock IT per tool (closes glm5.1 Task 6.4 in the same phase).
- **Breaking change**: NO. Every new tool is additive; existing tools unchanged.
- **User benefit**: The MCP host can answer structured questions about the corpus without burning the 12 KB pre-thinking budget on `cortex_query` post-filters. Consolidations become first-class queryable artefacts. Pre-thinking pipeline gains targeted retrieval verbs that don't need fusion noise.
