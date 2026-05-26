# Proposal: phase2_fulltext_routing_per_kind_topic

## Why

Spec-08 declares six Meilisearch indexes routed by event kind / classifier topic:
- `cortex-code` — `kind=artifact` with `topics ⊇ {code}`
- `cortex-docs` — `kind=artifact` with `topics ⊇ {doc}`
- `cortex-decisions` — `kind=decision`
- `cortex-governance` — `kind=law` / `kind=law_violation`
- `cortex-misc` — anything that does not fit the above
- `cortex-turns` — `kind=turn` / `kind=agent_call`

The 2026-04-27 reindex of 17 Hive repos shows the routing collapsed:

| Index | Docs | Expected |
|---|---|---|
| `cortex-code` | **0** | thousands (every Code-classified file) |
| `cortex-decisions` | **0** | dozens (every ADR-style file in `docs/decisions/`) |
| `cortex-docs` | 8 285 | only Doc-classified artifacts |
| `cortex-governance` | **0** | tens (every law / law_violation envelope) |
| `cortex-misc` | **0** | non-zero by construction (mismatches end up here) |
| `cortex-turns` | 1 715 | turn + agent_call |

Sample doc from `cortex-docs`:
```json
{
  "kind": "artifact",
  "topics": ["code", "yaml"],   // ← classified as code
  "path": "scripts/bench-config.yml",
  "repo": "Synap"
}
```

The doc carries `topics: ["code", ...]` but landed in `cortex-docs`. So the worker is indexing every artifact into a single index regardless of classification. Either:
- The worker reads `kind` only and maps `kind=artifact → cortex-docs` unconditionally, or
- The worker reads `topics` but the routing matrix is broken (`code` does not map to `cortex-code`), or
- The classifier ships `topics: []` for most events and the worker correctly defaults to `cortex-docs` — but the sample shows topics ARE present, ruling this out.

Net effect: keyword search has only two usable indexes out of six, the dashboard's Tools / Decisions / Laws views (when wired against fulltext) get no data, and the spec-08 contract is not met.

Source: 2026-04-27 reindex audit. `cortex-fulltext-worker` running, 12 580 docs total across only 2 of the 6 indexes.

## What Changes

- Build a routing function `route_to_index(envelope, classifier_output) -> &'static str` that owns the matrix above, with explicit ranking when topics overlap (e.g. `["code","doc"]` → `code` wins because the path tells us it's a source file).
- Worker calls the router once per event before constructing the Meili document; the result picks the index name (after applying the `cortex-` prefix from `CORTEX_FULLTEXT_INDEX_PREFIX`).
- Routing matrix:
  | Predicate | Index |
  |---|---|
  | `kind == decision` | `cortex-decisions` |
  | `kind == law \|\| kind == law_violation` | `cortex-governance` |
  | `kind == turn \|\| kind == agent_call` | `cortex-turns` |
  | `kind == artifact && topics ⊇ {code}` | `cortex-code` |
  | `kind == artifact && topics ⊇ {doc}` | `cortex-docs` |
  | otherwise | `cortex-misc` |
- Topic precedence: when `topics` contains both `code` and `doc`, prefer `code` if `path` ends with a known code extension (`.rs`, `.ts`, `.py`, `.go`, `.js`, `.tsx`, `.jsx`); otherwise `doc`. Captured as a small extension allowlist.
- Re-emit metric `cortex_fulltext_routed_total{index=...}` so the operator can confirm distribution post-fix.
- Backfill: drop the existing 6 indexes; re-run `cortex-bootstrap` so the worker recreates each with the right schema and routes the events correctly.

## Impact

- Affected specs: spec-08 (fulltext indexer routing matrix — currently underspecified; codify the matrix above).
- Affected code:
  - `crates/cortex-fulltext/src/routing.rs` (new module owning the matrix)
  - `crates/cortex-fulltext/src/worker.rs` (call the router; remove any hardcoded index name)
  - new metric counter
  - integration test seeding mixed events and asserting per-index distribution
- Breaking change: NO — the contract on the indexer surface is what spec-08 already declared; today's collapse is a regression vs spec.
- User benefit: 4 of 6 indexes become populated; downstream dashboards / lanes can finally filter by index correctly.
