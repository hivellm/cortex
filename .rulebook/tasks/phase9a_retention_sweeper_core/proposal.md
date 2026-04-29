# Proposal: phase9a_retention_sweeper_core

## Why

Spec 02 §"Quantization & tier sweep" promises a daily sweep that re-encodes
Vectorizer records FP32 → PQ at 30 days and PQ → Binary at 365 days, and the
SQLite schema declares a `retention_sweeps` table to bookkeep it. None of
this is implemented today. Every embedded turn, tool call and code chunk
sits in the hot FP32 collections forever, so:

- Vectorizer FP32 collections grow unboundedly (estimated ~50 k → multi-M
  vectors over a year on a single repo).
- The cost projections in `docs/specs/02-storage-layout.md` are wrong by
  two orders of magnitude after the first quarter.
- There is no production data path that exercises the warm/cold tiers, so
  defects in those code paths surface only when the disk is full.

A core retention sweeper closes the loop and is the load-bearing dependency
for every other Phase 9 task (9b parquet rollup, 9d PII enforcement, 9e
LLM digest, 9k scheduler).

## What Changes

1. NEW binary `cortex-retention` (or a `retention sweep` subcommand on
   `cortex-ops`) that runs one sweep pass and exits, suitable for cron.
2. Sweep walks each tier-aware collection
   (`cortex.turn.{fp32,pq}`, `cortex.tool_call.{fp32,pq}`,
   `cortex.code_chunk.{fp32,pq}`) using the Vectorizer SDK.
3. For each batch of records whose `occurred_at` crosses the boundary:
   - re-encode with target quantization (PQ at 30 d, Binary at 365 d),
   - upsert into the destination collection by `event_id`,
   - delete from the source collection,
   - emit a `cortex.retention.tier_transition` event on the bus
     (`from_tier`, `to_tier`, `event_id`, `kind`, `reason`).
4. `--time-travel <RFC3339>` flag overrides "now" so tests can drive the
   30-day / 365-day boundaries deterministically.
5. Bookkeeping row inserted into `retention_sweeps`
   (`sweep_id` = ULID, counters per tier transition, `tier_transitions_json`
   = full per-collection breakdown).
6. Idempotent: re-running the same sweep is a no-op (Vectorizer upsert by
   `event_id` already handles this; sweeper checks
   `WHERE NOT EXISTS in destination` before delete).
7. Concurrency-safe: uses an advisory lock row in the metadata DB
   (`retention_sweeps.status='running'`) so two cron firings do not stomp
   on each other.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` (clarify sweep
  contract), NEW `docs/specs/19-retention.md`.
- Affected code: NEW `crates/cortex-retention/`,
  `crates/cortex-storage/src/metadata.rs` (sweep insert helpers),
  `crates/cortex-ops/` (subcommand wire-up if chosen).
- Breaking change: NO. Pure additive on the storage layer.
- User benefit: bounded vectorizer growth, predictable cost curve,
  unblocks every other Phase 9 task.
