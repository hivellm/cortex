## 1. Schema
- [x] 1.1 Add `bootstrap_seen(repo TEXT, path TEXT, content_hash TEXT, last_run_id TEXT, last_emitted_at TEXT, PRIMARY KEY(repo, path))` to `crates/cortex-storage/schemas/sqlite/schema.sql`
- [x] 1.2 Migration helper `apply_phase10c_schema(conn)` (idempotent CREATE IF NOT EXISTS)
- [x] 1.3 `MetadataStore` helpers: `bootstrap_seen_lookup`, `bootstrap_seen_upsert`, `bootstrap_seen_count`

## 2. Walker dedup
- [x] 2.1 In `crates/cortex-cli/src/bootstrap/walker.rs`, compute `content_hash` per file before publishing
- [x] 2.2 Before emit: lookup `bootstrap_seen(repo, path)` — when the hash is unchanged, suppress the publish and only refresh `last_run_id`; when the hash changed, emit + upsert; when the row is absent, emit + insert
- [x] 2.3 Surface a per-repo suppressed-count in the runner report

## 3. Pre-flight warning
- [x] 3.1 If `bootstrap_seen` is empty AND existing lane has > 2× disk file counts for `:Decision`/`:Law`/`:Analysis`, log a warning and emit a `cortex.warnings` event (`kind=bootstrap.likely_duplicates`)
- [x] 3.2 The warning includes the suggested `cortex-ops bootstrap dedup` command

## 4. One-shot dedup CLI
- [x] 4.1 NEW `cortex-ops bootstrap dedup [--repo NAME] [--dry-run] [--apply]` (default dry-run)
- [x] 4.2 Walks the lane, groups by `content_hash`, keeps the newest ULID, deletes the rest from Vectorizer + Meili + Nexus
- [x] 4.3 The Parquet archive's older copies stay (the archive is append-only); they fall off via the existing 9b/9d sweepers
- [x] 4.4 Final report: `(decisions_dropped, laws_dropped, analyses_dropped, vectors_dropped, meili_docs_dropped, nexus_nodes_dropped)`

## 5. Tests
- [x] 5.1 Walker re-run with no file changes → zero new emits
- [x] 5.2 Walker re-run after editing one file → one new emit, that file only, hash updated
- [x] 5.3 Dedup CLI dry-run on a synthetic lane with 3× duplicates → reports the right counts, no mutation
- [x] 5.4 Dedup CLI `--apply` on the same → idempotent (second pass reports zero)

## 6. Spec / docs
- [x] 6.1 Update `docs/specs/02-storage-layout.md` §"Metadata store" with the new table
- [x] 6.2 Update `docs/specs/09-bootstrap-cli.md` §"Dedup" with the walker behavior + one-shot CLI

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
