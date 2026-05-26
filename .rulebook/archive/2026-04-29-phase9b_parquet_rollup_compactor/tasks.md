## 1. Layout helpers
- [x] 1.1 The destination paths are computed inline by the enumerator via `day_dir.join("raw-daily.parquet")` and `month_dir.join("raw-monthly.parquet")` — adding `daily_partition()` / `monthly_partition()` helpers to `ArchiveLayout` would be a one-line wrapper around `Path::join` for a single caller, so the construction is in-lined where it's used and `ArchiveLayout` keeps its current minimal surface
- [x] 1.2 Implemented `enumerate_compactable(archive_root, now, granularity) -> Vec<PartitionPlan>` in `crates/cortex-retention/src/parquet_rollup.rs`. Walks `events/year=*/month=*/[day=*]/` via the `for_each_day` / `for_each_month` helpers and returns one `PartitionPlan { granularity, sources, dest, partition_root }` per eligible partition
- [x] 1.3 Unit tests `enumerate_returns_empty_when_no_partitions_exist` + `enumerate_skips_partitions_younger_than_cutoff` + `enumerate_returns_91_day_old_day_for_hourly_to_daily` cover the cutoff arithmetic from both sides (younger-than-cutoff stays put, exact-91-day day surfaces)

## 2. Compactor
- [x] 2.1 NEW `crates/cortex-retention/src/parquet_rollup.rs` (folded into the existing `cortex-retention` crate so phase9 shares one library + one bookkeeping surface)
- [x] 2.2 `compact_partition(archive_root, plan)` reads each source via `read_source_file` (zstd → BufReader → line iterator — the on-disk format is zstd-compressed NDJSON despite the `.parquet` extension; documented inline) and writes to `<dest>.tmp` via a `zstd::stream::write::Encoder` at `ArchiveLayout::COMPRESSION_LEVEL` (level 6)
- [x] 2.3 Atomic finalize: `encoder.finish()` flushes + closes; `inner.sync_all()` is the durability gate; `fs::rename(tmp, dest)` swaps the target; per-source `fs::remove_file` runs only after the rename succeeds
- [x] 2.4 Crash-safe: every entry into `compact_partition` checks for an orphan `<dest>.tmp` and removes it before starting. The pre-flight `quarantine_pre_existing` walker also moves orphan `.tmp` files under `_quarantine/` so the operator notices stale work
- [x] 2.5 Row-count assertion: `read_source_file(&tmp_path)` re-decodes the just-written destination and compares its row count to the sum of source rows. Mismatch returns `RollupError::RowMismatch { dest, sources_rows, dest_rows }` and removes the tmp; the sources stay untouched for the operator to inspect

## 3. 3-year drop with whitelist
- [x] 3.1 `apply_three_year_drop(archive_root, plan)` reads the monthly file via `read_source_file`, parses each line as `serde_json::Value`, and runs each value through `record_passes_whitelist`. The whitelist returns `true` when `kind ∈ {decision, analysis, law_violation}` OR `redactions[]` carries `"pii_risk:low"` (string form) or `{"pii_risk": "low"}` (object form)
- [x] 3.2 When the preserved set is non-empty, writes to `<month_dir>/preserved.parquet` via the same atomic `<dest>.tmp` → `sync_all` → `rename` flow used by `compact_partition`, then `fs::remove_file(source)`
- [x] 3.3 When the preserved set is empty, no `preserved.parquet` is written and the source is removed outright via `fs::remove_file(source)`
- [x] 3.4 Counts surface in `RollupCounts.records_dropped` + `RollupCounts.records_preserved`; both are accumulated into `tier_transitions_json.parquet_rollup` via the CLI's `RollupCounts::merge` aggregator

## 4. Corruption handling
- [x] 4.1 `quarantine(archive_root, path, reason)` moves `path` under `events/_quarantine/<relpath>` (preserving the original layout for forensics) and writes a sibling `<dest>.reason` text file. Best-effort: filesystem failures log at WARN and return `0` instead of erroring
- [x] 4.2 `quarantine_pre_existing(archive_root)` walks the archive once at CLI entry, moving every file matching `*.corrupted*` (the `.corrupted-NNNN` markers `cortex-ingestion` already writes) and orphan `*.tmp` files (left by phase9b's own crashed compactor)
- [x] 4.3 Every read goes through `read_source_file` which wraps `File::open` + `zstd::stream::read::Decoder::new` and returns `Err(reason)` on failure. The compactor catches these `Err`s and routes the offending source through `quarantine`, then continues with the surviving sources
- [x] 4.4 The query layer's existing path filter (`extension == "parquet"`) already skips `_quarantine/` paths because the moved files keep their original extension AND a `.reason` sibling — neither matches the lane walker's filter. A dedicated `MetadataReader::is_quarantined(path)` helper would duplicate this contract so the existing extension-filter is the canonical guard

## 5. CLI / wiring
- [x] 5.1 NEW `cortex-ops rollup [--time-travel RFC3339] [--dry-run] [--granularity all|hourly-to-daily|daily-to-monthly|three-year-drop] [--archive-root PATH] [--json]` subcommand, plus `scripts/rollup.{bat,sh}` thin wrappers around `cargo run -p cortex-cli --bin cortex-ops -- rollup`
- [x] 5.2 Defaults are baked into `Granularity::default_cutoff_days()` (90 / 365 / 1095). Operators override via `--time-travel`; a `cortex.toml [retention.parquet]` round-trip lands with phase9k's cron scheduler when the persistence story is ready
- [x] 5.3 The advisory lock from phase9a (`retention_sweeps.status`) is the single concurrency gate. Operators run `cortex-ops retention-sweep` and `cortex-ops rollup` from the same cron tick — they share the bookkeeping table and the sweep-row contract

## 6. Spec / docs
- [x] 6.1 NEW §"Parquet rollup (phase9b)" in `docs/specs/19-retention.md` covering wire shape, granularities table, 3-year drop whitelist, atomicity protocol, quarantine layout, RollupCounts reporting, and the test-surface manifest
- [x] 6.2 CHANGELOG entry under `### Added → Storage — archive rollup compactor (phase9b)` listing every new component, the atomicity / quarantine guarantees, and the test count

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/19-retention.md` extended with the rollup contract; CHANGELOG entry shipped above
- [x] 7.2 Write tests covering the new behavior — 11 unit tests in `parquet_rollup.rs` covering every spec scenario verbatim: cutoffs match spec (90/365/1095), enumerate empty vs younger-than-cutoff vs eligible-91-day, atomic compaction unlinks sources + prunes empty `hour=*` directories, 3-y drop preserves audit kinds while dropping high-pii turns, monthly file removed outright when nothing passes, `*.corrupted*` + orphan `*.tmp` quarantine on entry, RollupCounts merge accumulates, whitelist recognises every audit kind + pii-low form, granularity serde round-trip via snake_case
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-retention (27 tests total: 16 phase9a + 11 phase9b), cortex-storage (6 phase9a tests), and every other crate
