# Proposal: phase10j_mcp_audit_capture

## Why

The 2026-04-30 MCP-surface review identified three gaps in the
Cortex MCP server. All three already have HTTP equivalents on
`cortex-api`; none are reachable from an agent context today.

1. **Audit envelope inspection.** The agent receives a
   `cortex_pre_thinking` bundle but cannot programmatically ask
   "what did each lane return for `query_id=01KQDX...`?". The
   `cortex-audit` slash command exists but lives in the
   conversation prompt — there is no MCP tool, so an agent
   running headless cannot debug retrieval quality.
2. **Captura ad-hoc.** The Cortex MCP is read-only. When the
   agent learns a fact mid-session, it captures via
   `rulebook_memory_save`. That writes to the Rulebook on-disk
   store, NOT the live Cortex lane that `cortex_query` reads.
   Result: knowledge captured during a session is invisible to
   the next pre-thinking bundle.
3. **Session replay.** 1672 turns + 181 conversations are
   indexed. The dashboard's `/v1/dashboard/conversations/{id}`
   exposes the ordered turns, but no MCP tool surfaces them.
   `intent=similar_problems` is broken (phase10a) and even after
   the fix it returns one row at a time, not a coherent thread.

The principle: **Rulebook MCP is source-of-truth for curated
disk artifacts; Cortex MCP is the live-lane window.** Cortex
MCP is missing two of the three live-lane surfaces (audit +
captura). Phase10j closes that.

## What Changes

1. NEW MCP tool `cortex_audit` — wraps the existing
   `/v1/audit/{query_id}` endpoint (or adds it if missing).
   Returns `{caller, intent, scope, lanes: [{name, hits,
   latency_ms, samples}], cache_hit, fail_open}` so the agent
   can pinpoint a missed retrieval to the responsible lane.
2. NEW MCP tool `cortex_capture_memory` — POSTs an envelope to
   `/v1/ingest` with `kind=memory`, the supplied body, and the
   caller-resolved `repo`/`topic`/`severity`. Returns the
   resolved `event_id` so the agent can cite it in a follow-up.
3. NEW MCP tool `cortex_session_replay` — wraps
   `/v1/dashboard/conversations/{session_id}` and returns the
   ordered turns + tool_call summaries. Supports `?max_turns=`
   so the bundle stays under context budget.
4. The three new tools live in
   `crates/cortex-mcp-server/src/tools/`. Wire them through
   the same auth + rate-limit hooks the existing three use.
5. Tests: unit tests against an in-memory `cortex-api` test
   double; integration test that exercises the full MCP→HTTP
   round-trip.

## Impact

- Affected specs: `docs/specs/11-query-api.md` §audit endpoint
  (clarify the contract), NEW
  `docs/specs/20-mcp-tool-surface.md` (canonical inventory),
  `docs/specs/12-pre-thinking-injection.md` §captura.
- Affected code: `crates/cortex-mcp-server/src/`,
  `crates/cortex-api/src/dashboard.rs` (mount
  `/v1/audit/{query_id}` if absent),
  `crates/cortex-api/src/types.rs` (audit response shape).
- Breaking change: NO. Pure additive surface.
- User benefit: an agent can debug retrieval, capture
  session-local knowledge into the live lane, and replay an
  earlier session without leaving the MCP transport.
