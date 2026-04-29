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

### Test surface (parquet rollup)

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

## CAS vacuum (phase9c)

Spec 02 §"CAS (content-addressable store) for large blobs"
promised a weekly vacuum job. Phase9c implements that contract.

### Wire shape

```sh
cortex-ops cas-vacuum \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--force] \
    [--cas-db PATH] \
    [--json]
```

### Eligibility

A blob is vacuum-eligible when **`refcount == 0` AND
`last_referenced < now - min_age_days`** (default `min_age_days = 30`,
configurable via `cortex.toml [retention.cas]`).

### Atomicity + concurrency

- Per-batch transactions (256 rows at a time) keep `SQLITE_BUSY`
  from blocking ingestion for more than a couple ms at once.
- Each batch uses `BEGIN IMMEDIATE` so a concurrent ingestion path
  can still serve `retain` / `release` calls without holding an
  exclusive lock.
- The `DELETE FROM cas_blobs WHERE hash = ? AND refcount = 0`
  predicate guards against a TOCTOU: if a concurrent path bumped
  the refcount between the candidate read and the delete, the row
  stays.

### Reclamation

Post-delete, the runner reads `PRAGMA freelist_count` and
`PRAGMA page_count`. When `freelist_count / page_count >
vacuum_ratio` (default `0.25`), the runner issues `VACUUM` to
reclaim disk. The wall-clock duration surfaces as
`VacuumReport.vacuum_ms` so dashboards can flag pathological runs.

### Catastrophic-deletion safeguard

The runner refuses when the candidate set covers more than 50 %
of total blobs unless `--force` is passed. A live run errors with
`VacuumError::SafeguardTripped`; a dry run still produces the
report with `safeguard_tripped: true` so operators can preview the
problem without erroring out.

### Refcount audit

`audit_refcounts(store, references)` recomputes the expected
refcount for every hash by counting external references the
caller supplies, then compares against `cas_blobs.refcount`.
Returns `Vec<RefcountDrift { hash, claimed, observed }>`.
`fix_refcounts(store, drift)` writes the observed values back
inside one `BEGIN IMMEDIATE` transaction.

The CLI surface is library-only at phase9c — operators that want
the audit pass call into `cortex_retention::cas_vacuum` from
their own bin or wire it through phase9k's cron scheduler. The
audit shape is stable; the integration with each external
reference source (Vectorizer / Nexus / Meili payload walkers)
lands as those crates expose their CAS-reference iterators.

### Test surface (CAS vacuum)

13 unit tests in `crates/cortex-retention/src/cas_vacuum.rs`:

- `opts_default_uses_thirty_day_cutoff`
- `orphan_blob_older_than_thirty_days_is_deleted`
- `referenced_blob_is_preserved_even_when_old`
- `dry_run_records_counts_but_does_not_delete`
- `safeguard_refuses_when_more_than_half_would_drop`
- `safeguard_overridable_by_force`
- `safeguard_does_not_trip_on_empty_store`
- `audit_refcounts_reports_under_count_drift`
- `audit_refcounts_reports_over_count_drift`
- `audit_refcounts_returns_empty_when_aligned`
- `fix_refcounts_writes_observed_to_store`
- `fix_refcounts_no_op_on_empty_drift`
- `batches_split_candidates_evenly`

## PII retention enforcement (phase9d)

Spec 01 §"PII tiers" defines three classes; phase9d enforces them.
Records with `pii_risk = null` AND age ≥ 90 d enter the medium
path automatically (defaulting to `low` would silently retain
unclassified PII forever — the safety net closes that gap).

### Cohort matrix

| Cohort | Match | Action | Redaction tag |
|--------|-------|--------|---------------|
| `High30d` | `pii_risk = "high"` AND `age >= 30 d` | Parquet body blanked, Vectorizer + Meili docs purged, CAS refcount decremented | `pii_high_30d` |
| `Medium90d` | `pii_risk = "medium"` AND `age >= 90 d` | Body re-summarized to ≤512 tokens, re-embedded, re-indexed; CAS refcount decremented | `pii_medium_90d` |
| `NullSafety90d` | `pii_risk = null` AND `age >= 90 d` | Same as `Medium90d`; emits a `cortex.warnings` event for classifier-coverage audit | `pii_medium_90d` |

Records with `pii_risk = "low"` are never redacted. Records whose
`payload.redacted` is already set short-circuit the matcher
(idempotence guard).

### Cross-store ordering

The library mandates the order so a partial run never leaves the
public surface holding raw PII:

- **High path**: Parquet rewrite → Vectorizer delete → Meili
  delete → CAS decrement.
- **Medium / null-safety path**: summarize → re-embed → re-index
  → Parquet rewrite → CAS decrement. The re-embed / re-index run
  BEFORE the Parquet rewrite so the public surface is never
  without the new summary.

A failure mid-flight rolls FORWARD on the next sweep — re-runs
converge because `classify` filters out already-redacted rows
and the medium path re-summarizes from the existing body if it
still exists.

### `PiiBackend` trait surface

The runner is library-only; production wires the live storage
clients (Vectorizer SDK, Meili HTTP, CAS store, classifier
client) through the `PiiBackend` trait. Tests use
`MemoryPiiBackend` for in-memory round-trips.

```rust
#[async_trait]
pub trait PiiBackend: Send + Sync {
    async fn rewrite_row(&self, event_id, kind, new_body, redaction_tag) -> Result<...>;
    async fn delete_vector(&self, event_id, kind) -> Result<...>;
    async fn delete_meili(&self, event_id, kind) -> Result<...>;
    async fn decrement_cas(&self, body_ref) -> Result<...>;
    async fn summarize(&self, original) -> Result<String>;
    async fn reembed_and_upsert(&self, event_id, kind, summary) -> Result<String>;
    async fn reindex_meili(&self, event_id, kind, summary) -> Result<...>;
    async fn emit_warning(&self, event_id, message) -> Result<...>;
}
```

### CLI surface

```sh
cortex-ops pii-enforce \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--cohort high|medium|null] \
    [--json]
```

Today's CLI is a synthetic preview against a built-in cohort
suite (one record per cohort + a fresh no-op + an already-
redacted idempotence guard) so operators can verify the matcher
contract before phase9k wires the production backend.

### Test surface (PII enforcement, phase9d)

16 unit tests in `crates/cortex-retention/src/pii_enforce.rs`:

- `pii_risk_round_trips_via_serde`
- `classify_high_at_31_days_picks_high_cohort`
- `classify_medium_at_91_days_picks_medium_cohort`
- `classify_null_at_91_days_falls_back_to_medium_safety`
- `classify_low_is_never_redacted`
- `classify_under_threshold_is_left_alone`
- `classify_already_redacted_record_is_idempotent`
- `cohort_redaction_tag_matches_spec`
- `high_path_runs_in_parquet_vector_meili_cas_order`
- `medium_path_summarises_re_embeds_and_re_indexes`
- `null_safety_path_emits_warning_and_runs_medium`
- `dry_run_records_outcomes_but_does_not_mutate`
- `cohort_filter_skips_other_cohorts`
- `already_redacted_target_is_skipped`
- `high_path_records_error_when_vector_delete_fails`
- `report_cohort_counts_json_round_trips`

## LLM turn digest summarizer (phase9e)

Phase9a–9d shrink storage record-by-record. They keep one row per
turn forever — a repo with 10 000 daily turns over a year yields
3.6 M `:Turn` nodes plus 3.6 M Vectorizer vectors plus 3.6 M Meili
docs, most of which is noisy back-and-forth nobody will ever query
individually. Phase9e produces a dense weekly digest per `(repo,
ISO_year_week, top_topic)` so the long tail becomes a small set of
queryable narratives.

### Cohort matrix

Turns enter the digest pipeline when **all** of the following hold:

- `occurred_at < now - digest_after_days` (default 30 d)
- `payload.summarized_by` is `None` (idempotence guard)
- The bucket size ≥ `min_bucket_size` (default 5)

Single-turn weeks fall below the threshold and are left for the
phase9b parquet rollup to consolidate.

### Bucket key

`(repo, year_week, top_topic)` — `year_week` uses the ISO 8601
label (`YYYY-Www`) so weeks span midnight Sunday→Sunday across
timezones. `top_topic` falls back to `"other"` for untagged turns.

### `DigestBackend` trait surface

The orchestrator is library-only; production wires the live
classifier (Sonnet via `cortex-classifier`) + embedder + Nexus
writer + Parquet rewriter. Tests use `MemoryDigestBackend` for
in-memory round-trips with one-shot failure injection.

```rust
#[async_trait]
pub trait DigestBackend: Send + Sync {
    async fn lookup_existing(&self, repo, year_week, top_topic) -> Result<Option<String>>;
    async fn summarize(&self, bucket) -> Result<DigestResult>;
    async fn persist_digest(&self, bucket, digest) -> Result<String>;
    async fn tag_source_turns(&self, digest_event_id, event_ids) -> Result<()>;
}
```

`persist_digest` does the entire write fan-out in one call:
`cortex.events.enriched` emit + embed + Nexus `:Memory` insert +
`(:Memory)-[:SUMMARIZES]->(:Turn)` edges. Returns the digest event
id so the orchestrator's follow-up `tag_source_turns` call sets
`payload.summarized_by` on every source turn's Parquet row.

### Cost ceiling

`max_usd_cents_per_run` (default 500) caps the per-run spend.
The orchestrator checks the running total before each
`lookup_existing` call so a budget cut-off resumes from the same
point on the next run. Buckets the cut-off cut surface in
`buckets_pending` for the dashboard's retention view.

### Idempotence

`run_turn_digest` calls `lookup_existing(repo, year_week, top_topic)`
before the classifier; an existing digest short-circuits unless
`--rebuild` is on. A re-run with no new old turns reports
`buckets_done = 0`, `already_digested = N`, `usd_cents = 0`.

### CLI surface

```sh
cortex-ops turn-digest \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--rebuild] \
    [--budget-cents N] \
    [--json]
```

Today's CLI runs against a synthetic 16-target preview suite
covering two topics in one week so operators can verify the
bucketize + budget + persist contract before phase9k wires the
production walker.

### Test surface (turn digest, phase9e)

14 unit tests in `crates/cortex-retention/src/turn_digest.rs`:

- `iso_year_week_uses_rfc_label`
- `bucket_key_is_pipe_joined`
- `bucketize_groups_old_turns_by_repo_week_topic`
- `bucketize_filters_below_min_size`
- `bucketize_excludes_fresh_turns`
- `bucketize_excludes_already_digested_turns`
- `run_persists_one_digest_per_bucket_in_call_order`
- `idempotent_re_run_does_not_call_summarize`
- `rebuild_flag_re_summarises_existing_buckets`
- `dry_run_records_pending_without_calling_classifier`
- `budget_ceiling_stops_run_cleanly`
- `per_bucket_failure_records_error_and_continues`
- `report_turn_digest_json_round_trips`
- `plan_default_uses_spec_thresholds`
