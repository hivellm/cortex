# Proposal: phase10c_bootstrap_dedup

## Why

The 2026-04-29 audit shows the lane carries:

- 26 decisions for **2 ADRs on disk** (~13× over-count)
- 37 laws for **12 rule files on disk** (~3× over-count)
- 33 analyses for ~4 source dirs (probable ≥3× over-count)

Inspecting the rows confirms the same content is re-published
under different ULIDs (`01KQ8F7BQ7N8...`, `01KQA7AE9AN6...`,
`01KQAC1KNF1V...` for the same "Bypass vectorizer-sdk" decision).
Each `cortex-bootstrap` run re-emits every file as if it were new
because the walker keys events on `(repo, path, run_id)` instead
of `(repo, path, content_hash)`.

The duplicates rot every downstream surface: the dashboard counts
3× decisions, RRF fusion crowds the top-10 with near-identical
rows, the relevance harness MRR drops, and the dedup bookkeeping
in `retention_sweeps` / `metadata_reap` fights itself.

## What Changes

1. Walker computes `content_hash = sha256(body_after_redaction)`
   for every file before publishing.
2. Before emitting a `cortex.events.bootstrap` envelope, the
   walker queries the metadata DB
   (`bootstrap_seen(repo, path, content_hash)`) — if the row
   already exists with the same hash, skip; if hash changed,
   emit + update; if missing, emit + insert.
3. New `bootstrap_seen` table in the metadata schema:
   `(repo TEXT, path TEXT, content_hash TEXT, last_run_id TEXT,
   last_emitted_at TEXT, PRIMARY KEY(repo, path))`.
4. One-shot CLI `cortex-ops bootstrap dedup` that walks the
   existing lane, groups by `content_hash`, keeps the newest
   ULID, and removes the duplicates from Vectorizer + Meili +
   Nexus + Parquet (the latter via a `compact_partition` sweep).
5. Bootstrap pre-flight check: if the new `bootstrap_seen`
   table is empty AND the existing lane has > 2× the disk file
   count for `:Decision` / `:Law` / `:Analysis`, the walker
   surfaces a warning suggesting `bootstrap dedup` before
   re-running.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md`
  §"Metadata store" (new `bootstrap_seen` table),
  `docs/specs/09-bootstrap-cli.md` §dedup.
- Affected code: `crates/cortex-cli/src/bootstrap/runner.rs`,
  `crates/cortex-cli/src/bootstrap/walker.rs`,
  `crates/cortex-storage/schemas/sqlite/schema.sql`,
  `crates/cortex-storage/src/metadata.rs` (new helpers).
- Breaking change: NO. Deduper is opt-in for existing data;
  the walker dedup path is invisible to consumers.
- User benefit: the dashboard reflects on-disk reality (12 laws
  not 37); RRF fusion stops crowding the top-10 with duplicates;
  retention sweeps process each artifact exactly once.
