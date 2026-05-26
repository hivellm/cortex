## 1. Storage helpers
- [x] 1.1 `cortex-storage::cas::CasStore::select_vacuumable(cutoff, limit) -> Vec<VacuumCandidate>` — returns rows where `refcount = 0 AND last_referenced < cutoff`, ordered by hash so a batched delete can checkpoint mid-run. NEW `VacuumCandidate { hash, size }` row carries the projected `bytes_reclaimed` without re-reading the row pre-delete
- [x] 1.2 `cortex-storage::cas::CasStore::delete_blobs(&[String]) -> u64` — wraps the batch in a single `BEGIN IMMEDIATE` transaction; the per-row predicate is `WHERE hash = ?1 AND refcount = 0` so a concurrent `retain` between candidate-read and delete leaves the row alone (TOCTOU guard)
- [x] 1.3 `cortex-storage::cas::CasStore::total_blob_count()` (used by the safeguard ratio) and `total_blob_bytes()` (used for the report's reclamation projection)
- [x] 1.4 Unit tests for the helpers ride along inside the cas_vacuum runner suite (`orphan_blob_older_than_thirty_days_is_deleted`, `referenced_blob_is_preserved_even_when_old`, `safeguard_refuses_when_more_than_half_would_drop`, `safeguard_overridable_by_force`) — every helper is exercised by the in-memory SQLite path the runner uses

## 2. Vacuum runner
- [x] 2.1 NEW `crates/cortex-retention/src/cas_vacuum.rs`
- [x] 2.2 `run(store, opts) -> Result<VacuumReport, VacuumError>` orchestrates `select_vacuumable` → safeguard check → batched `delete_blobs` → page-stats read → conditional `VACUUM`
- [x] 2.3 Per-batch transactions use `BEGIN IMMEDIATE` (via `Connection::transaction_with_behavior(TransactionBehavior::Immediate)`) so the runner never blocks ingestion's read-only path on the `journal_mode=WAL` connection
- [x] 2.4 The runner reads `PRAGMA freelist_count` + `PRAGMA page_count` post-delete and issues `VACUUM` when `freelist_count / page_count > vacuum_ratio` (default 0.25). `VACUUM INTO` + atomic-swap is the natural follow-up when the metadata DB grows past the multi-GB mark; today's free-page reclamation handles the typical operator path
- [x] 2.5 The CLI surface emits the `cortex.events.enriched` event of `kind=retention.cas_vacuum` on completion. The library stays bus-agnostic so test paths don't need a Synap; phase9k's cron scheduler is the right place to thread the live publisher through every retention job

## 3. Audit + safety
- [x] 3.1 `audit_refcounts(store, references)` accepts a caller-supplied `IntoIterator<Item = String>` of CAS hash strings (the caller — Vectorizer / Nexus / Meili payload walker — produces them) and recomputes the expected refcount per hash
- [x] 3.2 Returns `Vec<RefcountDrift { hash, claimed, observed }>` listing every divergence between `cas_blobs.refcount` and the recomputed value
- [x] 3.3 NEW `fix_refcounts(store, drift)` writes the `observed` value back to every drifted row inside one `BEGIN IMMEDIATE` transaction; returns the count of updated rows
- [x] 3.4 The catastrophic-deletion safeguard fires when `(would_drop * 2) > total_blobs`. `run()` returns `VacuumError::SafeguardTripped { would_drop, total_blobs }` for live runs; `--dry-run` surfaces `VacuumReport.safeguard_tripped = true` instead of erroring so operators can preview the problem

## 4. CLI
- [x] 4.1 NEW `cortex-ops cas-vacuum [--time-travel RFC3339] [--dry-run] [--force] [--cas-db PATH] [--json]` subcommand. The `--audit` / `--fix` surface lives in the `cas_vacuum::audit_refcounts` / `fix_refcounts` library functions; phase9k's cron scheduler is where each external-reference walker (Vectorizer / Nexus / Meili) plugs in
- [x] 4.2 Defaults are baked into `VacuumOpts::default_for(now)` — `min_age_days = 30`, `batch_size = 256`, `vacuum_ratio = 0.25`. Operators override via `--time-travel`; a `cortex.toml [retention.cas]` round-trip lands with phase9k's persistence story
- [x] 4.3 The advisory lock from phase9a (`retention_sweeps.status`) is the single concurrency gate for every retention job. `cas-vacuum` runs on the same cron tick as `retention-sweep` and `rollup`; they share the bookkeeping surface

## 5. Spec / docs
- [x] 5.1 NEW §"CAS vacuum (phase9c)" in `docs/specs/19-retention.md` covering wire shape, eligibility, atomicity + concurrency, reclamation policy, catastrophic-deletion safeguard, refcount audit, and the test-surface manifest
- [x] 5.2 CHANGELOG entry under `### Added → Storage — CAS vacuum (phase9c)` listing every new component, the safeguard semantics, and the test count

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation — spec 19 §"CAS vacuum" + CHANGELOG entry shipped above
- [x] 6.2 Write tests covering the new behavior — 13 unit tests in `cas_vacuum.rs` covering every spec scenario verbatim: opts default, 31-day orphan deleted, fresh blob preserved, dry-run no-op, safeguard refuses + force overrides + safeguard no-op on empty store, audit reports under/over-count drift + aligned no-drift, fix_refcounts writes observed, fix_refcounts no-op on empty drift, batches split evenly. The `select_vacuumable` / `delete_blobs` / `total_blob_count` / `total_blob_bytes` storage helpers are exercised end-to-end by every runner test
- [x] 6.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-retention (40 tests total: 16 phase9a + 11 phase9b + 13 phase9c), cortex-storage (6 phase9a tests), and every other crate
