# Proposal: phase2_scope_repo_resolution

## Why

Every `/v1/query` response on 2026-04-27 returned `scope_resolved.repos: []`, even when the request originated inside the Cortex repo and `cwd` was set on the adapter side. Probe:

```
$body = @{ intent="free_search"; query="…"; …}
…
"scope_resolved.repos: $($r.scope_resolved.repos -join ',')"
→ scope_resolved.repos:    (empty)
```

Implication: **per-repo filtering is broken**. The orchestrator can't restrict hits to "this repo I'm working in", which is the central use case for pre-thinking enrichment. A query made while editing `cortex-adapter-claude-code` should not surface a top hit from another repo, but today the lanes have no scope filter to apply.

Two failure points are likely:

1. The adapter's pre-thinking caller doesn't populate `scope` when constructing `QueryRequest` (`cortex-pre-thinking::pipeline::run` builds the request with `scope: derived.scope`, but the derivation `scope::derive` may be returning an empty scope when `cwd` is set to a repo without a `cortex.toml`).
2. The orchestrator's `scope_resolved` echo at `cortex-api/src/types.rs::QueryResponse.scope_resolved` is only built from the request — when the request has empty `scope.repos`, the response echoes empty.

The fix is a) ensure `derive_scope` always emits a `repo` when `cwd` is inside a git repo (even without `cortex.toml`) by walking ancestors for `.git`, b) propagate the resolved repo into every lane's filter so hits are repo-scoped, c) echo the canonicalised value back in `scope_resolved.repos`.

## What Changes

- `cortex-pre-thinking::scope::derive` already walks ancestors for `.git` (per the doc-comment) but appears to skip emitting a `repo` entry — fix the derivation to always set `scope.repos = [<git-root-basename>]` when a `.git` ancestor exists.
- The orchestrator's strategy layer (`cortex-api/src/strategies.rs`) propagates `scope.repos` into each lane's `filter` so the keyword lane / vector lane / graph lane all filter by repo.
- `QueryResponse.scope_resolved.repos` echoes the canonical repo id used by the lanes — never empty when a repo was resolved.
- `cortex-bootstrap` writes a deterministic repo id (per `cortex.toml.cortex.id`, falling back to git-root basename) so every component agrees on the same string for the same repo.

## Impact

- Affected specs: spec-12 (scope derivation), spec-11 (response echo + filter pass-through).
- Affected code:
  - `crates/cortex-pre-thinking/src/scope.rs`
  - `crates/cortex-api/src/strategies.rs`
  - `crates/cortex-api/src/types.rs` (response echo logic)
- Breaking change: NO (additive — empty scope is still accepted, just no longer the silent default)
- User benefit: pre-thinking bundles become repo-scoped — when working in Cortex, the model sees Cortex turns / decisions / laws, not a global mash from every captured repo.

## Source

2026-04-27 audit; `scope_resolved.repos` empty across 12 probes.
