# Proposal: phase15h_graph-worker-offset-persistence

Source: 2026-06-09 graph-worker / Nexus meltdown incident (phase15c).

## Why

The graph-worker resolves its consumer offset from a SQLite metadata store
(`LiveSynapConsumer::with_persistent_offset` → `consumer_offset_lookup`).
But the `cortex-graph-worker` container has **no volume mount** for that DB
(`resolve_metadata_db_path` falls to `<home>/.cortex/metadata.sqlite`
*inside* the ephemeral container FS). So on every `docker compose up`
recreate the store is empty → the offset tracker seeds at **0** → the worker
re-consumes the **entire** `cortex.events.enriched` stream and re-MERGEs all
~12k nodes / ~13k edges (plus, with the projection on, anchors + semantic
edges) in one burst.

On 2026-06-09 that burst repeatedly pinned Nexus 2.3.2 at 100% CPU until
restarted (nexus#12 sustained-write stall) — even with the projection
disabled (structural-only). The worker ran fine for 15h before only because
it had reached the stream head and was trickling steady-state writes; the
recreates reset it to 0. Until the offset survives a recreate (and a cold
boot starts at the head rather than 0), the graph lane cannot be brought back
online without either a Nexus upgrade or whack-a-mole restarts.

## What Changes

- Mount a persistent volume for the worker's metadata DB and point
  `CORTEX_GRAPH_METADATA_DB` at it in `docker-compose.yml`, so the committed
  consumer offset survives recreates.
- Cold-boot policy: when no persisted offset exists, seed the tracker at the
  **current stream head** (a Synap "latest" query) instead of 0, so a fresh
  worker captures new events forward and leaves history to the
  `cortex-ops graph backfill` path — re-projecting the whole stream on first
  boot is never correct.
- Prefer Synap 0.12's durable consumer-group cursor (`synap_group`) over the
  local metadata store if the SDK now exposes it (removes the ephemeral-DB
  failure mode entirely).
- Operator runbook: how to seed the offset to head to recover the graph lane
  without re-processing history.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` (offset / resume).
- Affected code: `crates/cortex-workers/src/graph/worker.rs`
  (`LiveSynapConsumer`), `crates/cortex-workers/src/bin/graph-writer.rs`,
  `docker-compose.yml`.
- Breaking change: NO (changes resume semantics from re-process-all to
  resume-or-head).
- User benefit: recreating the graph-worker no longer melts Nexus; the graph
  lane survives restarts; recovery without a Nexus upgrade becomes possible.
