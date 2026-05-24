# 25 — Event identity index

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 02, 06, 07, 08 · **ADR:** [ADR-012](../../.rulebook/decisions/012-eventidentity-cross-backend-join-key-sqlite-identityindex.md)

## Goal

Single indexed lookup `event_id → (nexus_id, vec_id, meili_id, archive_partition)` so cross-backend operators (`forget`, `dedup`, `doctor`, retention) collapse from per-call N-way fan-out to one SQLite row read.

## Why

Pre-phase13d every operator that touched the five backends re-derived the join per call:

- `cortex doctor consistency` over 100k events ran four parallel per-backend probes — total wall-clock in minutes.
- `cortex admin forget` hardcoded the per-backend cascade. When Synap was added as a fifth backend (phase11i) the cascade was not updated; a missed backend silently orphaned the row.
- `cortex-embedder-worker` dedup re-queried Vectorizer by `dedup_key` on every chunk instead of reading the stored `vec_id` from a side table.

A persisted typed join table closes the loop: every projection stamps its native id on insert; every cross-backend consumer reads the row once.

## Scope

**In:**

- New SQLite table `event_identity` indexed on `event_id` (PK) + three partial UNIQUE indexes on `nexus_id` / `vec_id` / `meili_id`.
- Per-projection write-back: embedder, fulltext indexer, graph mapper, archive writer each stamp their native id after a successful per-backend insert.
- New `IdentityIndex` trait + `SqliteIdentityIndex` impl in `cortex-storage`.
- New `cortex-ops doctor-identity-coverage` subcommand (indexed scan, exit code `2` on any backend NULL).
- `cortex admin forget` cascade drops the identity row after every per-backend leg succeeds.

**Out:**

- Live `exists(backend, id)` per-row probe inside the doctor (requires backend connectivity; lands behind `--live` after §3 reaches steady-state coverage).
- Identity-driven dispatch inside `admin forget` (the cascade still uses archive-derived kind for collection resolution; per-backend dispatch from identity rows lands alongside the doctor's `--live` flag).
- Backfill sweep for pre-phase13d events (manual one-shot per `cortex-ops identity-backfill` lands in phase13e).

## Data model

### Table DDL

```sql
CREATE TABLE IF NOT EXISTS event_identity (
    event_id          TEXT PRIMARY KEY,
    nexus_id          TEXT,
    vec_id            TEXT,
    meili_id          TEXT,
    archive_partition TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_nexus
    ON event_identity (nexus_id) WHERE nexus_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_vec
    ON event_identity (vec_id) WHERE vec_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_meili
    ON event_identity (meili_id) WHERE meili_id IS NOT NULL;
```

### Why no `synap_id` column

Synap is the source-of-truth event stream. Its id IS the row's `event_id`. A separate column would duplicate the PK.

### Why `archive_partition` is NOT UNIQUE

Multiple envelopes share the same hour-bucket parquet partition file (`events/year=2026/month=05/day=24/hour=10/raw-00000.parquet`). A UNIQUE constraint would reject the second event landing in the same partition.

### Native id semantics

| Backend       | Native value          | Source contract |
|---------------|-----------------------|------------------|
| `Vectorizer`  | server-assigned UUID  | first chunk's `UpsertedChunk.server_id` (per-event 1:1; many-chunk events collapse to a representative) |
| `Meili`       | `event_id` itself     | `Document::id` per spec-08 §Index keys (live envelopes) |
| `Nexus`       | `event_id` itself     | spec-07 §Node keys — `MERGE (n { event_id: $id })` |
| `Archive`     | partition path        | returned by `ArchiveWriter::write` |

## Write-back contract

| Projection                                                 | Trigger                                  | Backend       | Native value        |
|------------------------------------------------------------|------------------------------------------|---------------|---------------------|
| `cortex_workers::embedder::Worker::handle_message`         | after `publish_success`                  | `Vectorizer`  | first chunk's `server_id` |
| `cortex_workers::fulltext::Worker::handle_batch`           | after `publish_report` (Ok)              | `Meili`       | `event_id`          |
| `cortex_workers::graph::Worker::handle_batch`              | after `write_patches`(Ok) + publish      | `Nexus`       | `event_id`          |
| `cortex_workers::ingestion::router::process_event`         | after `archive.write` returns OK         | `Archive`     | returned partition path |

Each call site is best-effort: a poisoned mutex or SQLite error logs at WARN but does NOT undo the per-backend write. A missed stamp surfaces in the doctor as a coverage gap (the structural invariant the doctor relies on). Each Worker carries an optional `metadata: Option<Arc<Mutex<MetadataStore>>>` handle; `None` keeps the legacy pre-phase13d worker path running unchanged for callers that have not wired the metadata DB.

## Consumer paths

### `cortex-ops doctor-identity-coverage`

Walks `event_identity` once and reports per-backend coverage gaps. One indexed `COUNT(*)` + four `COUNT(*) WHERE <col> IS NULL` + four sampled `SELECT event_id … LIMIT sample` queries. Exit code `2` when any backend column has at least one NULL row. Budget: < 10 s for 100 k rows (measured 2.86 s end-to-end including 100 k row seed on the Windows test machine). Live `exists(backend, id)` per-row probe lands behind a future `--live` flag.

### `cortex admin forget`

`handle_forget(req, …, identity)`:

1. Validate confirmation token.
2. Resolve kind from the archive (legacy path; identity-driven dispatch is a follow-up).
3. Run `cortex_workers::pruner::purge::forget` cascade.
4. ON SUCCESS: drop the identity row via `IdentityIndex::delete(event_id)`.

A cascade failure (`ForgetError::Cascade`) returns before the delete, preserving the row so a retried forget still sees the full backend set. Dry-run returns before the delete by design — a dry-run must not mutate state.

## Migration

`cortex_storage::identity::apply_phase13d_schema(&Connection)` is called from `MetadataStore::migrate` at every open. Idempotent (`CREATE TABLE IF NOT EXISTS`). Pre-phase13d databases pick up the table on first MetadataStore boot. Existing rows from pre-phase13d events have no identity rows — the doctor reports them as `rows_total = 0` until a backfill sweep populates retroactively (phase13e §1).

## Failure modes

| Symptom                                          | Doctor output                            | Likely cause |
|--------------------------------------------------|------------------------------------------|--------------|
| `nexus_missing > 0`                              | sample lists orphan event_ids            | graph worker stamp path skipped (poisoned mutex, mid-batch panic, worker rebuilt without `with_metadata`) |
| `vec_missing > 0`                                | sample lists orphan event_ids            | embedder worker stamp path skipped; OR event's kind routes around the embedder (e.g. `LawViolation`) — by design, NOT a bug |
| `meili_missing > 0`                              | sample lists orphan event_ids            | fulltext worker stamp path skipped |
| `archive_missing > 0`                            | sample lists orphan event_ids            | ingestion router rebuilt without `with_metadata` |
| `failed: true` + every counter `> 0`             | `metadata_db` line surfaces resolved path | metadata DB itself is empty (cold boot before any envelope landed); not a regression |

## Open items

1. `identity-backfill` sweep for pre-phase13d archive (phase13e §1).
2. Live `exists(backend, id)` probe behind `--live` once §3 stable in production.
3. Identity-driven dispatch in `admin forget` (replaces archive-derived kind).
4. `kind` of write-back: today every event stamps every applicable backend; events that legitimately skip a backend (e.g. `LawViolation` → no Vectorizer) need a typed exemption table so the doctor does not flag them.
