# Proposal: phase9c_cas_vacuum

## Why

Spec 02 §"CAS (content-addressable store) for large blobs" promises a
weekly `vacuum` job that deletes blobs with `refcount = 0` and
`last_referenced > 30 days ago`, then `VACUUM`s the SQLite file. The
schema is in place; the job is not.

The CAS is where every >16 KB envelope payload lands (diffs, large tool
outputs, file snapshots). Without vacuum, every inline-overflow blob
ever produced sits in `cas_blobs` indefinitely. Worse, refcount
accounting only matters if something actually deletes when the count
hits zero — today nothing does, so the refcount column is decorative.

## What Changes

1. NEW subcommand `cortex-retention cas-vacuum` that:
   - opens the metadata DB,
   - selects `hash` rows where `refcount = 0 AND last_referenced < now - 30d`,
   - deletes them in batches of 256, rolled in per-batch transactions,
   - runs `VACUUM` (or `VACUUM INTO` then atomic swap on the metadata
     file when free pages > 25% of file size),
   - emits a `cortex.events.enriched` event of `kind="retention.cas_vacuum"`
     with `{ blobs_dropped, bytes_reclaimed, vacuum_ms }`.
2. Refcount-correctness audit pass (`--audit`): walks every Vectorizer /
   Nexus / Meili reference to a CAS hash and recomputes refcount;
   reports drift without mutating unless `--fix` is given.
3. Bookkeeping row in `retention_sweeps` with
   `tier_transitions_json.cas_vacuum`.
4. `--time-travel` flag matches 9a/9b.
5. A safety net: refuses to run if `(blobs_dropped / total_blobs) > 0.5`
   without `--force`, to avoid catastrophic deletion when an upstream
   refcount bug returns 0 for everything.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` §CAS,
  `docs/specs/19-retention.md` (add CAS vacuum section).
- Affected code: NEW `crates/cortex-retention/src/cas_vacuum.rs`,
  `crates/cortex-storage/src/cas.rs` (delete + audit helpers).
- Breaking change: NO. Pure cleanup behind the existing CAS contract.
- User benefit: bounded SQLite blob store; reclaims megabytes per
  bootstrap pass; surfaces refcount accounting bugs early.
