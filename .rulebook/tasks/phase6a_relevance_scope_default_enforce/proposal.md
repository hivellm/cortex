# Proposal: phase6a_relevance_scope_default_enforce

## Why

`POST /v1/query` accepts requests with `scope.repo = None` and silently routes to the `cortex-unknown-{family}` slug — empty index, empty collection, empty graph. Hits: `0`. Operators see "Cortex returned nothing" and assume broken; reality is route-to-nowhere.

The pre-thinking pipeline derives `scope.repo` from `cwd` ([crates/cortex-pre-thinking/src/scope.rs:95-111](../../../crates/cortex-pre-thinking/src/scope.rs)), so MCP `cortex_pre_thinking` callers are protected. But every **direct** caller of `/v1/query` — MCP `cortex_query` ([crates/cortex-mcp-server/src/tools.rs](../../../crates/cortex-mcp-server/src/tools.rs)), the dashboard search bar, the GUI ([gui/src/views/](../../../gui/src/views/)) — sends the user's literal `QueryRequest` with no `cwd`, hits the unknown slug, gets nothing.

This is the highest-leverage 1-day fix in [docs/analysis/relevance/](../../../docs/analysis/relevance/) (R1 step 1, closes F-003).

Source: `docs/analysis/relevance/01-findings.md` §F-003; `crates/cortex-api/src/strategies.rs:19-29` (`repo_scoped` + `UNKNOWN_REPO_SLUG`); `crates/cortex-api/src/types.rs:60-74` (`Scope.repo: Option<String>`); `crates/cortex-mcp-server/src/tools.rs` (callers without scope); `crates/cortex-api/src/service.rs` (where to enforce).

## What Changes

### Server-side scope resolution in `service.rs`
Order of resolution (first hit wins):
1. `request.scope.repo` (explicit) — round-trip unchanged.
2. `x-cortex-repo` header — set by MCP server / dashboard / pre-thinking pipeline.
3. Caller hint header `x-cortex-cwd` → repo-slug derivation via `cortex_storage::names::slug_for_repo`.
4. `422 Unprocessable Entity` with spec-11 `reason: "scope_repo_required"` — no implicit fallback to `cortex-unknown-*`.

### Caller updates
- **MCP server** ([crates/cortex-mcp-server/src/tools.rs](../../../crates/cortex-mcp-server/src/tools.rs)): `cortex_query` tool MUST inject `x-cortex-repo` from the MCP context's `cwd` before calling `/v1/query`. Same change as the pre-thinking pipeline already does for `scope`, but at the HTTP boundary.
- **Dashboard** ([gui/src/lib/api.ts](../../../gui/src/lib/api.ts)): every `/v1/query`-shaped call MUST include the active repo filter as `x-cortex-repo`. The Sidebar already tracks `filters.repo`; pipe it through.
- **Pre-thinking pipeline**: continues to set `request.scope.repo` directly — no change required.

### Audit envelope
Add `scope_resolution: "explicit" | "header" | "cwd" | "rejected"` to the audit envelope so the dashboard's `/v1/dashboard/violations` / future `query_audit` view can flag misconfigured callers.

### Backward compatibility
This is a **breaking change** for any caller currently relying on the `unknown` slug fallback. Audit shows zero such callers in production (the slug is empty across all backends), but we add a one-week deprecation window: a `CORTEX_ALLOW_UNKNOWN_SCOPE=1` escape hatch logs a `tracing::warn!` and falls back to today's behaviour. Removed in `phase6e` (the harness gate) once the audit shows zero hits.

## Impact

- Affected specs: [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) (add `422 scope_repo_required`); [`docs/specs/18-mcp-server.md`](../../../docs/specs/18-mcp-server.md) (header injection contract).
- Affected code: `crates/cortex-api/src/service.rs` (resolver); `crates/cortex-api/src/types.rs` (audit field); `crates/cortex-api/src/audit.rs` (record `scope_resolution`); `crates/cortex-mcp-server/src/tools.rs` (header injection); `gui/src/lib/api.ts` (pass active repo as header).
- Breaking change: YES — but gated behind `CORTEX_ALLOW_UNKNOWN_SCOPE=1` for one week.
- Depends on: nothing.
- User benefit: every direct `/v1/query` call (MCP, dashboard, GUI) now hits a real repo collection instead of `cortex-unknown-*`. Closes F-003 — the largest single relevance uplift identified in the analysis.
