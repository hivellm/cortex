## 1. Layout helpers
- [ ] 1.1 Extend `cortex-storage::archive::ArchiveLayout` with `daily_partition()` and `monthly_partition()` returning the destination directories
- [ ] 1.2 Helper `fn enumerate_compactable(root, now, granularity) -> Vec<PartitionPlan>` returning all `(source_files, dest_file, granularity)` tuples eligible for rollup
- [ ] 1.3 Unit-test that the enumerator returns empty for partitions younger than the cutoff

## 2. Compactor
- [ ] 2.1 NEW `crates/cortex-retention/src/parquet_rollup.rs`
- [ ] 2.2 `compact_partition(plan: PartitionPlan)` reads all source files via `arrow`+`parquet`, concatenates record batches preserving schema, writes to `<dest>.tmp` with Zstd level 6
- [ ] 2.3 Atomic finalize: `fsync(tmp)` → `rename(tmp, dest)` → `unlink(sources)`
- [ ] 2.4 Crash-safe: on restart the compactor detects orphan `.tmp` files and deletes them
- [ ] 2.5 Per-partition row count assertion: sum(input rows) == output rows, otherwise abort and quarantine sources

## 3. 3-year drop with whitelist
- [ ] 3.1 `apply_three_year_drop(plan)` reads the monthly file, filters to whitelist (`pii_risk == "low"` OR `kind ∈ {decision, analysis, law_violation}`)
- [ ] 3.2 If filtered set is non-empty: write to `<year=>/preserved-<month>.parquet` then delete the original monthly
- [ ] 3.3 If empty: delete the monthly file outright
- [ ] 3.4 Records dropped are counted and reported in the sweep row

## 4. Corruption handling
- [ ] 4.1 Helper `quarantine(path, reason)` that moves to `events/_quarantine/<relpath>` and writes `<path>.reason`
- [ ] 4.2 On startup: walk archive once, move every `*.corrupted*` and `*.tmp` orphan into quarantine
- [ ] 4.3 Wrap every Parquet read with `try_open`; on `arrow::error::ArrowError::ParseError` quarantine the offending file and continue
- [ ] 4.4 Add `MetadataReader::is_quarantined(path)` so the query API skips quarantined paths

## 5. CLI / wiring
- [ ] 5.1 Subcommand `cortex-retention rollup [--time-travel RFC3339] [--dry-run] [--granularity hourly_to_daily|daily_to_monthly|three_year_drop|all]`
- [ ] 5.2 Default config in `cortex.toml` `[retention.parquet]` (`hourly_to_daily_days=90`, `daily_to_monthly_days=365`, `drop_after_days=1095`)
- [ ] 5.3 Re-uses the same advisory lock as 9a (one rollup at a time)

## 6. Spec / docs
- [ ] 6.1 Add §"Parquet rollup" to `docs/specs/19-retention.md`
- [ ] 6.2 CHANGELOG entry under `Added`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
