# Proposal: phase13g_mcp-grounding-tools

Source: 2026-05-19 manual consolidation review session — 5 system-generated consolidations confirmed quality is comparable to hand-written ones; pre-thinking integration gaps identified.

## Why

`cortex_query` (RRF fusion) + `cortex_keyword_search` / `_vector_search` / `_graph_query` (phase11v fine-grained) + `cortex_topic_*` + `cortex_forget` cover the **retrieval surface** but leave three grounding sources locked inside data the agent cannot reach explicitly:

1. **Rulebook task state** (active phase, blocked items, next checklist row). Today the agent re-walks `.rulebook/tasks/**` with grep on every session start. Pre-thinking does not surface this. Result: sessions reopen with no awareness of what is in-flight.
2. **Past similar work** (top consolidations matching the current intent). Pre-thinking fan-out already retrieves similar past turns via the vector lane, but consolidations are mixed in with raw turns under the same RRF score. The agent sees them but cannot say "show me 5 sessions that already tackled this kind of problem" with a deterministic budget.
3. **ADR supersession history** (decision X superseded by Y replaced by Z). The Decision nodes + `:SUPERSEDES` edges exist in Nexus from phase11k, but the only path to them is hand-rolled Cypher via `cortex_graph_query`. A focused tool returns the chain shape directly.

The 2026-05-19 consolidation review session showed the system produces quality consolidations (~1670c body avg, real file paths, line counts, version migrations, key decisions). But the **agent cannot do `cortex_similar_sessions("rework consolidator")` and get 3 deterministic hits** — it has to construct a fusion query and hope the relevance gate fires correctly.

Three tools close the loop:
- `cortex_active_work` — operator-facing tasks/phases snapshot, no LLM, pure SQL+FS scan
- `cortex_similar_sessions` — vector search restricted to `Kind::Consolidation` with confidence floor
- `cortex_decision_chain` — typed Nexus walk on `:SUPERSEDES` edges from one ADR id

Pre-thinking (spec 11 §pre_change_context) gains a "Recent operator state" section + a "Similar past sessions" section + an "ADR provenance" section, surfaced as additional context blocks alongside laws + snippets.

## What Changes

- New MCP tool `cortex_active_work` — returns `{ active_tasks: [{id, phase, status, next_unchecked_item, blocked_reason?}], in_progress_count, blocked_count, recent_archives: [{id, archived_at, title}] }`. Reads from `.rulebook/tasks/*/tasks.md` + `.metadata.json` + `.rulebook/archive/*`. Cached with mtime+TTL (60s). Filter by repo when scope.repo set.
- New MCP tool `cortex_similar_sessions` — vector search against `cortex-<repo>-consolidations` indexes (or all repos when scope omitted). Returns top-K consolidation hits with `{ consolidation_id, session_id, title, summary_markdown, source_event_count, occurred_at, score }`. Confidence floor 0.6 (mirrors topic-card threshold). K bounded `[1, 10]`, default 5.
- New MCP tool `cortex_decision_chain` — typed Nexus walk `MATCH path = (start:Decision { event_id: $id })-[:SUPERSEDES*0..16]-(end:Decision)` returning the ordered chain plus each ADR's `{id, slug, status, date, title}`. Walks both directions (predecessors + successors). Cycle-detection guard at 16 hops.
- Pre-thinking (`crates/cortex-pre-thinking/src/`) extended with three new section builders that call the tools above and render their output under existing `section_caps` byte budgets:
  - `## Active operator work` (cap 1200 bytes)
  - `## Similar past sessions` (cap 2000 bytes)
  - `## ADR provenance` (cap 800 bytes, only when query mentions an ADR id or a Decision-stamped envelope is in the fusion result)
- Each tool ships with `ToolDescriptor` registered in `cortex_mcp_server::tools::default_set()`; MCP `tools/list` count bumps `10 -> 13`.

## Impact

- Affected specs: `docs/specs/11-pre-thinking-context.md` (new section anchors), `docs/specs/22-fine-grained-search.md` (extend the tool registry table to 13).
- Affected code: `crates/cortex-api/src/active_work.rs` (new), `crates/cortex-api/src/similar_sessions.rs` (new), `crates/cortex-api/src/decision_chain.rs` (new), `crates/cortex-api/src/http.rs` (3 new routes), `crates/cortex-mcp-server/src/tools.rs` (3 new ToolDescriptor + invoke handlers + registry bump), `crates/cortex-pre-thinking/src/formatter.rs` (3 new section renderers + budget wiring).
- Breaking change: NO — additive only. Existing tools + endpoints unchanged.
- User benefit: pre-thinking surfaces "you have 4 tasks open including phase13a §5.2", "3 past sessions tackled rework analysis — cons-ses-xxx, cons-ses-yyy", "ADR-009 is superseded by ADR-014 (your current edit references the old one)". Closes the load-bearing grounding gap the 2026-05-19 review surfaced.
