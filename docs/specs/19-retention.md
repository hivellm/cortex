# Spec 19 — Retention Sweep

**Status:** 🟡 phase9a core shipped (in-memory ops); live Vectorizer
adapter follows in phase9b–9k.

## Why

Spec 02 §"Quantization & tier sweep" promised a daily sweep that
re-encodes Vectorizer records FP32 → PQ at 30 days and PQ → Binary
at 365 days. Without that sweep, Vectorizer FP32 collections grow
unboundedly and the cost projections in spec 02 are wrong by two
orders of magnitude after the first quarter.

This spec documents the contract phase9a implements; phase9b–9k
extend it with the parquet rollup compactor, CAS vacuum, PII
retention enforcement, LLM digest summariser, Meili archival
pruner, SQLite metadata reaper, auto-memory consolidator,
dashboard view, CI canary, and cron scheduler.

## Wire shape

### Command

```sh
cortex-ops retention-sweep \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--batch-size N] \
    [--metadata-db PATH] \
    [--json]
```

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Sweep completed; records demoted / dropped within ceiling. |
| 1 | Error-rate ceiling tripped, or hard failure (Vectorizer SDK error, bookkeeping write failure, etc.). |
| 2 | Another sweep is already in flight (concurrency lock). |

### Tier pairs

Default plan (built by `SweepPlan::default_for(now)`):

| kind | from | to | age_days |
|------|------|----|----------|
| `turn` | `fp32` | `pq` | 30 |
| `turn` | `pq` | `binary` | 365 |
| `tool_call` | `fp32` | `pq` | 30 |
| `tool_call` | `pq` | `binary` | 365 |
| `code_chunk` | `fp32` | `pq` | 30 |
| `code_chunk` | `pq` | `binary` | 365 |

Always-hot kinds (`decision`, `analysis`, `memory`, `law`) are
deliberately absent.

### Collection naming

- `cortex.<kind>.fp32` for fresh records.
- `cortex.<kind>.pq` for warm tier (≥ 30 d).
- `cortex.cold.binary` for the cold tier (≥ 365 d) — every kind
  shares the same Binary collection.

### Bookkeeping

Every invocation writes exactly one row to `retention_sweeps`:

| column | type | notes |
|--------|------|-------|
| `sweep_id` | TEXT | ULID. |
| `started_at` | TEXT | RFC-3339. |
| `finished_at` | TEXT | RFC-3339; `NULL` while running. |
| `records_demoted` | INTEGER | Total moves across all pairs. |
| `records_dropped` | INTEGER | Moves that failed re-encode/upsert. |
| `tier_transitions_json` | TEXT | `{"<kind>:<from>-><to>": <count>}` map. |
| `status` | TEXT | `running` / `success` / `failed` / `abandoned`. |

The `status` column is the concurrency lock: a second sweep with a
`running` row whose `started_at` is younger than the
abandon-grace window (1 h default) exits with code `2`. Older
`running` rows are auto-marked `abandoned` so the new sweep
proceeds.

### Idempotence

Every step is "re-encode → upsert dest → delete source". A re-run
short-circuits on `dest_has(event_id)` so the destination is never
double-written. A mid-flight crash leaves the source record in
place; the next sweep re-tries the same id from the source side
and finds the destination row already in place, then cleans the
source (also idempotent — second delete is a no-op).

### Error budget

Per-record drop rate above `max_error_rate` (default 5 %) fails
the sweep with `SweepError::ErrorRateExceeded`; the bookkeeping
row is still written with `status = 'failed'` so the dashboard
can surface the regression. Per-record errors below the ceiling
are tolerated and surface in `records_dropped`.

### Observability

The sweep emits one `cortex.events.enriched` event per moved
record with `kind = "retention.tier_transition"` and payload
`{ event_id, kind, from_tier, to_tier, reason }` so downstream
consumers (the dashboard's retention view, `phase9i`) can render
the timeline without re-reading the bookkeeping table.

## Test surface

phase9a ships 16 unit tests in `crates/cortex-retention/src/lib.rs`
covering:

- canonical collection naming (`collection_for`)
- `TierPair::pair_key` round-trip
- `TierPair::cutoff` arithmetic
- the default plan layout (6 pairs)
- `SweepReport::error_rate` empty + populated
- `tier_transitions_json` round-trip
- the FP32 → PQ at 31 d boundary scenario verbatim from this spec
- PQ → Binary at 366 d boundary
- under-threshold no-op
- idempotent re-run (mid-flight crash recovery)
- `--dry-run` observe-only behaviour
- ceiling-trip vs ceiling-allow drop-rate scenarios
- `new_sweep_id` ULID shape
- `Tier` serde round-trip

The bookkeeping helpers in `cortex-storage::MetadataStore` carry
their own round-trip tests (`start_retention_sweep` /
`finish_retention_sweep` / `list_recent_sweeps`).

## Parquet rollup (phase9b)

Spec 02 §"Event archive (Parquet)" promised a clean rollup
contract: hourly → daily at 90 d, daily → monthly at 365 d, drop
monthly at 3 y unless `pii_risk = "low"` or `kind ∈ {decision,
analysis, law_violation}`. Phase9b implements that contract.

> Despite the `.parquet` filename suffix, the on-disk format is
> **zstd-compressed line-delimited JSON**. The compactor
> concatenates source files line-by-line into a single destination,
> so the schema-stable contract spec 02 promised holds even though
> the encoding is NDJSON-on-zstd, not Apache Parquet.

### Wire shape

```sh
cortex-ops rollup \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--granularity all|hourly-to-daily|daily-to-monthly|three-year-drop] \
    [--archive-root PATH] \
    [--json]
```

### Granularities

| Granularity | Cutoff | Source layout | Destination |
|-------------|--------|---------------|-------------|
| `hourly_to_daily` | 90 d | `events/year=Y/month=M/day=D/hour=*/raw-*.parquet` | `events/year=Y/month=M/day=D/raw-daily.parquet` |
| `daily_to_monthly` | 365 d | `events/year=Y/month=M/day=*/raw-daily.parquet` | `events/year=Y/month=M/raw-monthly.parquet` |
| `three_year_drop` | 1 095 d | `events/year=Y/month=M/raw-monthly.parquet` | `events/year=Y/month=M/preserved.parquet` (only records passing the whitelist; otherwise the source is removed outright) |

### 3-year drop whitelist

Records survive when:
- `kind ∈ {decision, analysis, law_violation}` (always-preserved
  audit kinds), OR
- `redactions[]` carries `"pii_risk:low"` (string form) or
  `{"pii_risk": "low"}` (object form).

### Atomicity

Every compaction is **read → write `<dest>.tmp` → `sync_all` →
`rename` → `unlink sources`**:

- A crash between `sync_all` and `rename` leaves an orphan `.tmp`
  that the next run cleans up via `quarantine_pre_existing`.
- A crash between `rename` and `unlink sources` leaves the dest
  durable + sources intact; the next run re-attempts and the row-
  count assertion (`sources_rows == dest_rows`) catches the
  duplicate.
- Row-count mismatch returns `RollupError::RowMismatch` and removes
  the tmp; the sources stay alone for the operator to inspect.

### Corruption quarantine

Files matching `*.corrupted*`, orphan `*.tmp`, or any file that
fails the zstd decode get moved to
`events/_quarantine/<original-relpath>` with a sibling `.reason`
text file describing why. The query layer skips paths under
`_quarantine/` because the lane walker filters on `extension ==
"parquet"`. Quarantine is **best-effort**: filesystem failures log
at WARN and the run continues — losing a quarantine line is
preferable to crashing the rollup.

### Reporting

`RollupCounts { files_in, files_out, bytes_reclaimed, quarantined,
records_dropped, records_preserved }` is returned per granularity
plus a totals row. The CLI prints both; `--json` ships the same
shape for downstream tooling.

### Test surface

11 unit tests in `crates/cortex-retention/src/parquet_rollup.rs`:

- `granularity_default_cutoffs_match_spec`
- `enumerate_returns_empty_when_no_partitions_exist`
- `enumerate_skips_partitions_younger_than_cutoff`
- `enumerate_returns_91_day_old_day_for_hourly_to_daily`
- `compact_partition_merges_sources_atomically_and_unlinks`
- `three_year_drop_preserves_decisions_and_drops_high_pii_turns`
- `three_year_drop_removes_monthly_outright_when_nothing_passes`
- `quarantine_pre_existing_moves_corrupted_and_tmp_files`
- `rollup_counts_merge_accumulates_every_field`
- `record_passes_whitelist_recognises_each_audit_kind`
- `granularity_serde_round_trips_via_snake_case`
