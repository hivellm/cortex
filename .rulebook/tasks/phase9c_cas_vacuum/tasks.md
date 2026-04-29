## 1. Storage helpers
- [ ] 1.1 `cortex-storage::cas` — `select_vacuumable(now, age_days, limit) -> Vec<BlobRef>`
- [ ] 1.2 `cortex-storage::cas` — `delete_blobs(hashes: &[String]) -> usize` in a single tx
- [ ] 1.3 `cortex-storage::cas` — `total_blob_count()`, `total_blob_bytes()`
- [ ] 1.4 Unit tests for both helpers (in-memory SQLite)

## 2. Vacuum runner
- [ ] 2.1 NEW `crates/cortex-retention/src/cas_vacuum.rs`
- [ ] 2.2 `run(opts: VacuumOpts) -> VacuumReport` orchestrating select → delete-in-batches → fsync → VACUUM
- [ ] 2.3 Per-batch tx with `BEGIN IMMEDIATE` to avoid SQLITE_BUSY against ingestion
- [ ] 2.4 Decide between `VACUUM` and `VACUUM INTO` based on `freelist_count / page_count > 0.25`
- [ ] 2.5 Emit a Synap `retention.cas_vacuum` event on completion

## 3. Audit + safety
- [ ] 3.1 `audit_refcounts()` walks Vectorizer/Nexus/Meili payloads, collects every referenced CAS hash, compares against `cas_blobs.refcount`
- [ ] 3.2 Returns `Vec<RefcountDrift { hash, claimed, observed }>`
- [ ] 3.3 `--fix` corrects the column inside one tx
- [ ] 3.4 `run()` aborts unless `--force` when `(would_drop / total_blobs) > 0.5`

## 4. CLI
- [ ] 4.1 `cortex-retention cas-vacuum [--audit [--fix]] [--time-travel RFC3339] [--dry-run] [--force]`
- [ ] 4.2 Reads `cortex.toml` `[retention.cas]` (`min_age_days = 30`, `batch = 256`)
- [ ] 4.3 Reuses 9a's advisory lock keyed on `("cas-vacuum")`

## 5. Spec / docs
- [ ] 5.1 Add §"CAS vacuum" to `docs/specs/19-retention.md`
- [ ] 5.2 CHANGELOG entry under `Added`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
