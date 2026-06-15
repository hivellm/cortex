# Proposal: phase15c_graph-projection-live-coverage-backfill

Source: phase15b §4.3 (live smoke that could not green in-session).

## Why

phase15b wired the 12-kind semantic-edge projection into the live graph
worker (`worker.rs` batch step 3b) and shipped the `cortex-ops
doctor-graph-coverage` doctor (§4.1/§4.2). The projection now runs on every
NEW enriched event. But §4.3 ("backfill against the current Cortex graph;
doctor reports all 12 kinds present") could not be completed live for two
structural reasons discovered during phase15b:

1. Synap does not re-serve drained history. The graph consumer seeds its
   offset tracker from an ephemeral metadata store (no volume mount), so a
   recreated worker starts at offset 0 — but `StreamManager::consume(room, 0)`
   returns nothing for the already-drained `cortex.events.enriched` room. The
   worker sits idle at the tail; no historical replay occurs (also why there
   is no nexus#12 flood risk on restart).
2. The archive stores RAW envelopes, not enriched events. `cortex-ops graph
   backfill` rebuilds an `EnrichedEvent` with a `StaticFallback` classifier,
   so only the payload-driven extractors (SUPERSEDES / CONTRADICTS /
   EMITTED_BY / ANSWERED_BY / CITES-body-regex) emit edges. The 7
   classifier-driven kinds (CALLS / IMPORTS / DEFINES / RETURNS / ABOUT /
   MENTIONS_FILE / RELATES_TO) need real classifier output, which the archive
   does not retain.

Therefore "all 12 kinds present" is only reachable by re-running the
classifier over historical envelopes and writing the projected edges live —
a heavy, sustained-write operation gated on nexus#12 (edge-UNWIND
throughput), the same gate phase25 is parked on.

## What Changes

- Add a bounded live-write mode to `cortex-ops graph backfill` (`--apply` +
  `--limit`) that projects archived envelopes through the real `GraphWriter`
  instead of counting only — so payload-driven kinds can be backfilled into
  the live graph without a full Synap replay.
- Add a classifier-replay path: re-enrich a bounded window of archived
  envelopes through the live classifier (inline or via the classifier
  worker) so the 7 classifier-driven kinds populate.
- Run `cortex-ops doctor-graph-coverage` against the populated graph and
  confirm all 12 kinds clear the §4.2 floor; capture the output as the
  acceptance artifact.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` § Semantic-edge projection.
- Affected code: `crates/cortex-cli/src/bin/cortex-ops/graph_cmd.rs`
  (`graph_backfill` live-write mode); a thin classifier-replay helper in
  `cortex-workers`.
- Breaking change: NO.
- User benefit: graph lane returns non-empty multi-hop results across all 12
  edge kinds, closing the phase15b §4.3 acceptance gap.
