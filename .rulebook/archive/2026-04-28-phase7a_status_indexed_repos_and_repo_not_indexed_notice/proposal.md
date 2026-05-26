# Proposal: phase7a_status_indexed_repos_and_repo_not_indexed_notice

Source: GitHub issue [hivellm/cortex#1](https://github.com/hivellm/cortex/issues/1)

## Why

`cortex_query` and `cortex_pre_thinking` cannot distinguish *no relevant
context* from *the requesting repo was never indexed*. Both surface as a
healthy empty result (`{ "results": {} }`) or a generic
`empty_bundle` soft-error, so callers assume Cortex was consulted when
it has not been. This breaks the spec-12 fail-open contract for
external repos: an agent operating on a fresh repo gets zero signal
that it needs to bootstrap, and the tool result reads as "Cortex has
nothing to say" instead of "Cortex has never seen this repo".

## What Changes

1. **`/v1/status`** — the existing `StatusBody` gains an
   `indexed_repos: Vec<String>` field, populated from the same
   keyword-lane snapshot the dashboard already uses to derive
   `repos_indexed`. Empty list when the daemon has no lane wired in.

2. **`QueryResponse`** — adds an optional `notice: Notice` field with
   a `code` discriminant. When the request carries `scope.repo` and the
   canonicalised slug does not appear in the indexed-repos snapshot,
   the orchestrator stamps `code: "repo_not_indexed"` plus a `hint`
   pointing at the bootstrap CLI. Existing behaviour
   (`results: {}`, `budget`, `debug`) is unchanged — the field is
   `skip_serializing_if Option::is_none`, so old clients are
   unaffected.

3. **Pre-thinking pipeline** — when the upstream `QueryResponse`
   carries `notice.code == "repo_not_indexed"`, the MCP shim emits
   `reason: "repo_not_indexed"` (not `empty_bundle`) and includes the
   resolved scope plus the same hint. `empty_bundle` stays for the
   case where the scope *is* indexed but no result fired.

4. **README** — adds a short "First-time indexing" section pointing at
   the bootstrap entry point (`cortex-bootstrap` binary / SDK) so the
   suggestion the response surfaces matches a documented path.

## Impact

- Affected specs: `docs/specs/11-query-api.md` (response schema gains
  `notice`), `docs/specs/18-mcp-tools.md` (status shape).
- Affected code: `crates/cortex-api/src/{http.rs,service.rs,types.rs,orchestrator.rs}`,
  `crates/cortex-api/src/lanes/memory.rs` (or wherever
  `MemoryKeywordLane` lives), `crates/cortex-mcp-server/src/tools.rs`,
  `crates/cortex-pre-thinking/src/pipeline.rs`.
- Breaking change: NO. New field is optional, additive on the wire.
- User benefit: agents using Cortex via MCP get a clear remediation
  path the moment they hit an unindexed repo, instead of silently
  succeeding with an empty bundle.
