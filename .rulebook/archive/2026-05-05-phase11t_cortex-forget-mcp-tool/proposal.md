# Proposal: phase11t_cortex-forget-mcp-tool

## Why

`phase11o_vectorizer_demotion_api` shipped the hard-purge sink at
`crates/cortex-workers/src/pruner/purge.rs::forget` with a
`REQUIRED_CONFIRMATION_TOKEN = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE"`
contract that the operator-facing `/cortex forget <event_id>` MCP tool
is supposed to echo. The sink itself is fully implemented +
unit-tested (cascades Vectorizer `delete_vectors`, Meili
`delete-batch`, Nexus `DELETE node`, Parquet rewrite), but the MCP
descriptor + dispatcher binding that exposes it to operators still
needs to land in `cortex-mcp-server`. Without it, the cascade is
unreachable from the host and `phase11j_consolidation_tier §5.3`
stays partial.

This task closes the cross-task dependency cleanly so phase11j can
archive without an orphan blocker.

## What Changes

In `crates/cortex-mcp-server/src/tools.rs`:

1. New `ForgetTool` implementing the `Tool` trait with:
   - `name() -> "cortex.forget"`
   - `descriptor()` carrying input schema `{event_id: string,
     confirmation_token: string, dry_run: bool (default false)}`,
     plus a `confirmation` advisory in the descriptor metadata.
   - `call(input)` that resolves `vectorizer_collections` from the
     event's `kind` (consolidation ⇒ all three consolidation
     collections; raw kinds ⇒ the per-kind hot+warm+cold triple),
     instantiates the live Vectorizer + Meili + Nexus + Archive
     ops, and forwards to
     `cortex_workers::pruner::purge::forget`.

2. Register `ForgetTool` in `ToolRegistry::default_set()`.

3. Two unit tests inline:
   - `forget_tool_rejects_missing_confirmation_token`
   - `forget_tool_dry_run_does_not_call_purge_sink`

4. CHANGELOG entry under the existing phase11o block (additive — the
   tool is the operator-facing surface for the same sink).

5. Update `docs/specs/20-mcp-tool-surface.md` with the tool
   descriptor + cascade contract (or land that spec if missing).

## Impact

- Affected specs: `docs/specs/18-mcp-server.md` (new tool entry),
  `docs/specs/20-mcp-tool-surface.md` if/when it lands.
- Affected code: `crates/cortex-mcp-server/src/tools.rs`,
  `crates/cortex-mcp-server/src/lib.rs` (registry plumb).
- Breaking change: NO. Additive on the MCP surface; existing
  `query` / `pre_thinking` / `status` tools untouched.
- User benefit: the irreversible-purge cascade is reachable from
  the operator's MCP host; phase11j §5.3 closes structurally
  rather than as a partial.

## Blocked on

Nothing. `phase11o_vectorizer_demotion_api` shipped the sink + the
confirmation-token contract; this task is pure surface plumbing on
top of that.
