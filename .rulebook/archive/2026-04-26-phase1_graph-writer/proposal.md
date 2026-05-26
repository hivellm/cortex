# Proposal: phase1_graph-writer

## Why

Hybrid retrieval needs the graph lane: from a seed node, expand 1–2 hops across `TOUCHED`, `LINKED_TO`, `SUPERSEDES`, `OF` edges to surface the full context around a file or decision. Without this, the query API is just vector + keyword. This task turns enriched events into Nexus nodes + edges idempotently.

## What Changes

- `NexusClient` with Bolt transport (HTTP fallback via flag) + retry + connection pool.
- Event-to-graph mapper: one `fn map(&EnrichedEvent) -> GraphPatch` with exhaustive match on `kind`.
- Cypher templates under `cortex-workers/cypher/*.cypher` (no string concat of user data).
- Schema bootstrap at startup: constraints + indexes for every label.
- Client-side patch coalescer (dedup nodes/edges within a micro-batch).
- Worker binary consuming `cortex.events.enriched`, publishing `cortex.events.graphed`.

## Impact

- **Affected specs:** [`docs/specs/07-graph-writer.md`](../../../docs/specs/07-graph-writer.md); unblocks 09 + 11.
- **Affected code:** new `cortex-graph/` crate, worker binary `cortex-graph-worker`, Cypher templates under `cortex-graph/cypher/`.
- **Breaking change:** NO — greenfield.
- **User benefit:** enables neighborhood-expansion retrieval and decision-register supersession graphs in the dashboard.

## Source

`docs/specs/07-graph-writer.md` · depends on specs 01 + 02 · PRD FR-7.
