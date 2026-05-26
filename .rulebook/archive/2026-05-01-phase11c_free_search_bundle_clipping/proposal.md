# Proposal: phase11c_free_search_bundle_clipping

## Why

The MCP tool `mcp__cortex__cortex_query` with `intent = "free_search"` returns a bundle that exceeds the MCP transport's per-tool-result token cap. Verified live on 2026-04-30 — a single `free_search` call for `"classifier-worker pipeline Meilisearch indexing"` produced a 180,567-character single-line response that the MCP server rejected with:

```
Error: result (180.567 characters across 1 line) exceeds maximum allowed tokens.
Output has been saved to ...tool-results/mcp-cortex-cortex_query-*.txt
```

Effect: `free_search` is unusable through the MCP surface. Every call dumps to a side-file and forces the agent into a manual-slicing recovery loop, which defeats the purpose of the intent (it is the catch-all "give me everything you know about X" surface and is the most common one for ad-hoc queries).

Root cause: the other intents (`pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`) all run through the section-cap budget pipeline that clips per-section to fit `budget_bytes`. `free_search` does not — it bypasses or under-applies the same clipping. The `cortex-api` `/v1/query` response includes the full per-lane hits + snippets concatenated, and at scale (15 indexed repos, 7.6k cortex events) the result trivially blows past 30k tokens.

The MCP transport already has a built-in cap (visible in the error: ~30k chars hard limit for a single result). The `query` tool itself accepts a `budget_ms` parameter but no `budget_bytes` analogue at the MCP surface, and the underlying `/v1/query` response shape doesn't honour a byte cap on the snippet text.

## What Changes

1. **Apply the section-cap budget to `free_search`.** The same budget pipeline that `pre_change_context` uses (per-section caps with deterministic clipping) must run on the `free_search` path. Simplest fix: route every `free_search` response through `cortex_pre_thinking::pipeline::run` (or its formatter) with a default `budget_bytes = 24 * 1024` matching the pre-thinking path.
2. **Expose `budget_bytes` on the MCP query schema.** Add an optional `budget_bytes` parameter to the MCP `cortex_query` tool (default `32768`) so callers can tighten or loosen the cap. Plumb it to `/v1/query` body.
3. **Snippet body cap.** Per-snippet text should be clipped to `<=` ~1 KiB inside the bundle so a single large file doesn't eat the whole budget. The existing `formatter.rs` per-section clipping already does this for `pre_change_context`; `free_search` must reuse it.
4. **Hit-count cap with overflow note.** When the result set has more than `limit` hits but the byte budget would exceed cap, the formatter should truncate the list and append a single line `<!-- N more results clipped -->` so the agent knows there's more to ask for.
5. **MCP server-side guard.** As a belt-and-braces measure, the `cortex-mcp-server` adapter for `cortex_query` should re-check the response size before emitting and, on overflow, return a structured error describing the cap rather than letting the transport reject the call after the fact.

## Impact

- Affected code:
  - [crates/cortex-api/src/](crates/cortex-api/src/) — the `free_search` branch of `/v1/query` (likely in `mcp.rs` / `strategies.rs` / `query_rewrite.rs`).
  - [crates/cortex-pre-thinking/src/formatter.rs](crates/cortex-pre-thinking/src/formatter.rs), [crates/cortex-pre-thinking/src/budget.rs](crates/cortex-pre-thinking/src/budget.rs) — confirm they run on `free_search`.
  - [crates/cortex-mcp-server/src/tools.rs](crates/cortex-mcp-server/src/tools.rs) — schema + plumbing for `budget_bytes`.
- Breaking change: NO — adding a parameter with a default; existing callers see clipped output (which is what they need anyway).
- User benefit: `free_search` returns usable results through MCP. No more side-file dumps, no manual slicing.

## Source

- Live MCP transcript (2026-04-30): `Error: result (180.567 characters across 1 line) exceeds maximum allowed tokens. Output has been saved to ...tool-results/...`.
- Working comparison: `pre_change_context` returned 104 ms / 5 snippets / well under cap on the same probe — confirming the budget pipeline works when invoked.
