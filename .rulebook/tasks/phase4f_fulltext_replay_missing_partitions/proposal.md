# Proposal: phase4f_fulltext_replay_missing_partitions

## Why

`phase4a_fulltext_fanout_parity_and_stale_meili_cleanup` shipped the
boot-time stale-sweep, the `list_indexes` / `delete_index` MeiliClient
surface, and the `is_canonical_index_name` invariant guard. The fan-out
gap that triggered the original audit is **closed** today (verified by
re-probe on 2026-04-28: every populated repo has its
`cortex-{slug}-{family}` indexes present), but the replay-missing
defense in proposal §2 was carved out — the worker still relies on the
`cortex.events.enriched` Synap stream catching every event in real
time. There is no automatic recovery if the worker:

- crashes before catching up to the bootstrap that seeded a new repo;
- runs against a Synap that pruned the stream before it caught up;
- starts cold against an archive-only deployment (no live Synap).

A replay-missing-repos path closes that hole as defense-in-depth.

## What Changes

- A `boot_replay::replay_missing_partitions(client, archive_root)`
  routine added to `cortex-fulltext`:
  1. Calls `list_indexes` + `routing::is_canonical_index_name` to
     compute the set of `(repo_slug, family)` partitions Meili
     currently knows.
  2. Walks `$CORTEX_ARCHIVE_ROOT/events/**/*.parquet` (zstd-NDJSON)
     and computes the set of `(repo_slug, family)` partitions
     present in the archive — same path / topic routing the worker
     uses live.
  3. For every partition present in the archive but missing from
     Meili, replays the matching events through the existing
     `MeiliFulltextIndexer` upsert path. Idempotent because Meili
     keys on `id = doc_id` derived from `content_hash`.
- The replay runs **after** the stale-sweep but **before** the
  worker pool starts pulling from Synap, so a fresh boot lands in
  a consistent state regardless of stream age.
- Per-partition metrics: `cortex_fulltext_replay_events_total{repo, family}`.
- Off by default — gated on `CORTEX_FULLTEXT_REPLAY_MISSING=1` so a
  hot-path restart does not trigger a multi-minute archive scan.

## Impact

- Affected specs: spec-08 (`Boot-time stale-index sweep` section
  gains a sibling `Boot-time replay-missing` section).
- Affected code:
  - `crates/cortex-fulltext/src/boot_replay.rs` (new)
  - `crates/cortex-fulltext/src/main.rs` (gated call after sweep)
  - tests against `MemoryMeiliClient` + a temp archive directory
- Breaking change: NO. Behaviour is opt-in via env flag.
- Depends on: phase4a (stale-sweep + list_indexes already shipped).
- User benefit: keyword search recovers automatically after a
  worker outage that spans a Synap stream rotation; cold-stack
  archive-only deployments get a populated keyword lane on first
  boot.

## Source

- Carved out of `phase4a_fulltext_fanout_parity_and_stale_meili_cleanup`
  proposal §2 to honour the no-orphan protocol after the fan-out gap
  was confirmed closed by the phase4a re-probe.
