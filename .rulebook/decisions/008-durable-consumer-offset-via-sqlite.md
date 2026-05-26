# 8. Durable consumer offset via SQLite (graph + fulltext workers)

**Status**: proposed
**Date**: 2026-05-03
**Related Tasks**: phase11s_pipeline_drainage_recovery

## Context

The 2026-05-03 phase11p reindex showed the graph worker dropping
events across container restarts: only 8 of 20 sibling repos
landed `:Artifact` nodes in Nexus, while Meili (which uses
spec-08's separate consumer that DOES persist offsets via Meili's
task uid system) carried full coverage. The root cause was the
TODO at [`crates/cortex-workers/src/graph/worker.rs:12-16`]:

> Synap 0.11 has no durable consumer-group surface in the SDK; the
> worker tracks the next offset locally and dedupes by event_id
> in-memory.

The "track ephemerally" path defaults to "latest" on every restart,
so the rebuild window between (a) a worker exiting and (b) a
worker booting is silently lost. The Cypher-template hotfix train
during phase 11p triggered three back-to-back graph-worker
rebuilds; each restart dropped a window.

The live stack already runs Synap server 0.12.0 (per
`/health`). The proposal hoped phase 11s could simply wire
durable consumer-group support and retire the TODO. But auditing
the upstream `synap-sdk-0.12.0/src/queue.rs` showed the API only
exposes `queue::ack(queue_name, message_id)` — there is no
durable consumer-group surface on the stream API the workers
consume from. The §2.2 happy path is therefore not viable today.

Three implementation paths the proposal considered:

1. **Wait for upstream Synap to ship consumer groups.** Defer the
   fix and accept silent drainage in the meantime.
2. **Persist `(consumer_id, stream) → offset` to local SQLite.**
   Use the existing `MetadataStore` (already in process, already
   persistent across restarts, already shared with the classifier
   worker's spend ledger) as the durable backing for the
   in-process `OffsetTracker`. On boot, seed the tracker; on every
   successful `ack`, write through to SQLite.
3. **Persist `last_event_id` to a sidecar Synap stream
   (`cortex.consumer.offsets`).** Self-host an offset stream
   inside Synap; consume from latest on boot.
4. **Add Redis to the stack as a durable cursor.** Treat consumer
   offsets as a separate concern with their own backing.

## Decision

Ship path #2 — persist offsets to the SQLite `MetadataStore` under
a new `consumer_offsets` table keyed on `(consumer_id, stream)`.
The boot path reads the row and seeds `OffsetTracker` at
`last_offset + 1`; every successful `ack` writes through via
`consumer_offset_upsert` (MAX-semantics — out-of-order acks never
roll the cursor back). A separate `consumer_offset_set` primitive
supports the `cortex-ops graph replay --since=<offset>` rewind
without the MAX guard.

The graph-writer bin builds the consumer via the new
`LiveSynapConsumer::with_persistent_offset(handle, metadata,
consumer_id, stream)` constructor; legacy ephemeral mode stays
available via `LiveSynapConsumer::new` for tests.

## Alternatives Considered

### Wait for upstream Synap consumer groups

Rejected. The 2026-05-03 reindex showed silent drainage already
costs days of operator effort to recover; waiting for the upstream
adds one or more release cycles before the fix lands. The §2
proposal explicitly called out the pipe-jacking pattern (block on
upstream API change while production silently bleeds) as the
anti-pattern this ADR replaces.

### Sidecar Synap offset stream

Rejected. A self-hosted offset stream inside Synap solves the
durability problem but trades it for a second cold-start path: on
restart the worker has to consume from `cortex.consumer.offsets`
to learn where it left off, and the offset stream itself has the
same "from latest" default unless we durably persist its cursor
(turtles all the way down). Worse, the offset stream becomes a
synchronisation hotspot — every worker writes to it, so a Synap
hiccup affects every consumer simultaneously.

### Redis sidecar

Rejected. Adding Redis to the stack triples the operator surface
(ports, auth, monitoring, retention) for a problem that fits
inside SQLite cleanly. The SQLite store is already in-process,
already persistent, already on the operator runbook for the
classifier spend ledger. Reuse beats add.

### Persist last_event_id only (no offset)

Considered as a stricter contract. `event_id` is a ULID and orders
events by mint time, but Synap's `consume(room, Some(offset),
Some(max))` API takes `offset`, not `event_id`. Resuming by
`event_id` would require a `find_offset_by_event_id` lookup the
SDK does not expose. Stamping both (`last_offset` AND
`last_event_id`) keeps the ledger forward-compatible without
constraining today's resume path.

## Consequences

**Positive:**
- Restart no longer drops events. The `cortex-graph-worker`
  container can be rebuilt at will (Cypher hotfix, image rotation,
  scheduled restart) and resume cleanly from the persisted offset.
- The replay primitive (`consumer_offset_set` →
  `cortex-ops graph replay --since=<offset>`) gives operators a
  surgical recovery tool when a known event window was lost from
  causes outside the ack path (e.g. an operator manually deleted a
  Nexus label).
- The same ledger pattern extends to the fulltext worker (today
  uses Meili's task UID — already durable but separate) and any
  future stream consumer. Pin the (consumer_id, stream) shape so
  later workers inherit the discipline.
- Schema migration is additive (`CREATE TABLE IF NOT EXISTS`); no
  data migration, no breaking change.

**Negative / tradeoffs:**
- The ack path now writes to SQLite on every event. Throughput
  cost is bounded by the WAL-mode write cycle (~µs per row); even
  at 10k ev/s the embedder workload sits well under SQLite's
  measured 100k writes/s ceiling on commodity SSDs. Documented in
  `docs/cortex/pipeline-drainage-runbook.md`.
- A persistence write failure is logged but not propagated — the
  in-process tracker still advances, so the runtime stays correct
  for THIS event but a subsequent restart may re-process it. The
  alternative (propagate the failure and crash) is worse: a
  transient SQLite lock would crash the worker every iteration.
  Documented in [`worker.rs::ack`].
- The MAX-on-upsert semantics mean operators cannot rewind via
  `consumer_offset_upsert`. The §2.4 replay path uses
  `consumer_offset_set` which has no MAX; tests pin both contracts
  separately so a refactor can't accidentally apply MAX to the
  rewind primitive.
- The fix is workaround-shaped — it shadows a missing upstream
  feature. When Synap ships a durable consumer-group surface in a
  future release, the SQLite path stays useful (single source of
  truth for the dashboard's lag metrics) but the worker can switch
  the read path to the SDK at zero cost. A follow-up ADR will
  capture the migration when that ships.
