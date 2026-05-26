# Proposal: phase4e_graph_symbol_backfill_runbook

## Why

`phase4c_graph_richer_edges_defines` shipped the schema, mapper, Cypher
templates, and unit/integration tests for `Symbol` nodes and the
`DEFINES` edge. The actual replay of the existing event archive against
the live `cortex-graph-worker` — to populate `Symbol` + `DEFINES` for
the artifacts already in Nexus — is an **operational** step that
requires a live Nexus and a worker run, not a code change. It belongs
in an operations runbook, not a code PR.

phase4c §4.1–§4.3 originally tried to bundle the replay + spot-check
into the same PR. The rulebook tail enforces "no orphan items," so
this task carries those steps to closure on a real cluster.

## What Changes

- A runbook script `scripts/backfill-graph-symbols.sh` that:
  1. Confirms `CORTEX_NEXUS_URL` reachability via the `ensure_schema`
     bootstrap (fails fast on a missing constraint or connection).
  2. Runs `cortex-graph-backfill` against the configured archive root
     (idempotent under MERGE; no duplicate Symbol or DEFINES rows).
  3. Verifies the post-replay graph with two Cypher probes:
     - `MATCH (s:Symbol)-[:DEFINES]->(a:Artifact) RETURN count(s) AS sym, count(DISTINCT a) AS art` — both columns MUST be > 0.
     - `MATCH (s:Symbol {name: "PreThinkingTool"})-[:DEFINES]->(a:Artifact) RETURN a.repo, a.path` — MUST return `Cortex` + `crates/cortex-mcp-server/src/tools.rs`.

- The runbook is a one-shot operations command — no recurring schedule.
  Re-running it after subsequent worker upgrades stays safe because of
  Nexus MERGE semantics.

## Impact

- Affected specs: none (schema + mapper land in phase4c).
- Affected code: `scripts/backfill-graph-symbols.sh` (new). No
  changes to `crates/cortex-graph/*` — the worker is already
  symbol-aware.
- Breaking change: NO.
- Depends on: phase4c (must be merged + workers must run the new
  binary before this task fires).
- User benefit: closes the audit gap from 2026-04-27 22:36 UTC where
  Nexus knew "which file is in which repo" but not "where is
  `PreThinkingTool` defined" against the existing graph.

## Source

- Carved out of `phase4c_graph_richer_edges_defines` items 4.1–4.3 to
  honour the no-orphan protocol.
