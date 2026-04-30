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

## Meili archival pruner (phase9f)

Meili holds the full body of every turn / tool_call indefinitely.
Phase9f blanks those bodies after 90 d, caps `summary` at 4 KiB,
and stamps `pruned: true` + `pruned_at`. The document is **never**
deleted — the keyword lane still surfaces it on a `summary` match.

### Wire shape

```sh
cortex-ops meili-prune \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--rebuild] \
    [--batch-size N] \
    [--json]
```

### Eligibility

A document enters the pruner when **all** hold:

- `index ∈ {cortex_turns, cortex_tool_calls}`
- `occurred_at < now - prune_after_days` (default 90 d)
- `pruned == false` (idempotence guard); `--rebuild` accepts
  already-pruned docs

### Summary cap

Summaries longer than `summary_cap_bytes` (default 4 096) get
truncated with the canonical `…` (3-byte UTF-8 ellipsis) so the
post-prune length is exactly `cap_bytes`. The cap walker rounds
back to a UTF-8 char boundary so multibyte codepoints never get
sliced mid-character.

### `MeiliBackend` trait surface

```rust
#[async_trait]
pub trait MeiliBackend: Send + Sync {
    async fn enumerate_prunable(&self, index, cutoff, accept_pruned, batch_size) -> Result<Vec<MeiliDoc>>;
    async fn update_documents(&self, index, ops: &[PruneOp]) -> Result<()>;
}
```

Production wires the live Meili SDK; the production
`enumerate_prunable` impl translates the call into the
`filter = "occurred_at < <cutoff> AND pruned != true"` Meili
query. `update_documents` ships the partial-update payload and
awaits the task to terminal state.

The trait deliberately omits `delete_documents` — pruning never
deletes; the keyword lane still surfaces pruned rows on summary
matches.

### Test surface (Meili prune, phase9f)

16 unit tests in `crates/cortex-retention/src/meili_prune.rs`:

- `plan_default_uses_spec_thresholds`
- `cap_summary_returns_unchanged_when_under_cap`
- `cap_summary_truncates_with_ellipsis_marker`
- `cap_summary_rounds_to_char_boundary`
- `cap_summary_handles_cap_smaller_than_ellipsis`
- `enumerate_excludes_fresh_documents`
- `enumerate_excludes_already_pruned_unless_accept`
- `run_prunes_91_day_old_documents_in_each_index`
- `re_run_after_commit_is_a_no_op`
- `rebuild_re_prunes_already_pruned_docs`
- `dry_run_records_skipped_without_calling_update`
- `oversize_summary_is_capped_with_ellipsis`
- `enumerate_failure_propagates_to_runner`
- `update_failure_propagates_to_runner`
- `report_meili_prune_json_round_trips`
- `batch_size_splits_large_runs_into_chunks`

## Metadata reaping (phase9g)

The metadata DB (`~/.cortex/metadata.sqlite`) collects three classes
of rows that grow without bound today: `bootstrap_jobs` (one row per
successful run), `sessions` (one row per Claude Code / agent
session), and `classifier_spend` (one row per UTC day). Plus two
operational logs that live next to the DB (`hook-invocations.log`,
`hook-errors.log`).

Phase9g consolidates the rows we don't need at high resolution into
parallel rollup tables and rotates the logs so the operator's home
directory is bounded.

### CLI

```
cortex-ops metadata-reap \
    [--time-travel <RFC3339>] \
    [--dry-run] \
    [--target all|bootstrap_jobs|sessions|classifier_spend|logs] \
    [--metadata-db PATH] \
    [--log-dir PATH] \
    [--skip-logs] \
    [--json]
```

### Rollups

| Source                | Trigger                                                    | Sink                       | Bucket key                          |
|-----------------------|------------------------------------------------------------|----------------------------|-------------------------------------|
| `bootstrap_jobs`      | `status='success' AND finished_at < now - 30 d`            | `bootstrap_jobs_daily`     | `(day, repo_path)`                  |
| `sessions`            | `started_at < now - 365 d`                                 | `sessions_monthly`         | `(year_month, tool, repo)`          |
| `classifier_spend`    | `day < (now - 365 d)`                                      | `classifier_spend_monthly` | `year_month`                        |

`bootstrap_jobs.status='failed'` rows are NEVER rolled — they stay
raw for full-detail debugging. Sessions whose `repo` is NULL collapse
into the empty-string `repo` bucket so the SQLite primary key
constraint stays satisfied.

Each target runs inside its own `BEGIN IMMEDIATE` transaction
(aggregate UPSERT + DELETE). Idempotence: re-running with no new
aged rows is a no-op (zero deletions, zero rolled rows added).

### Vacuum

After every rollup, the runner reads `PRAGMA freelist_count` and
`PRAGMA page_count`. When `freelist / page_count > 0.25`, the runner
issues `VACUUM` to reclaim disk. The wall-clock duration surfaces as
`ReapReport.vacuum_ms`.

### Hook log rotation

`~/.cortex/hook-invocations.log` and `~/.cortex/hook-errors.log` are
rotated to `<name>.<YYYY-MM-DD>.gz` when they exceed 5 MB or 7 days
of age. Rotation is rename-first (`mv live → staged → gzip`) so a
concurrent appender holding `O_APPEND` continues writing into the
moved-aside file until it reopens — those bytes ARE captured in the
gzip output. The 8 most recent rotations per file are retained;
older rotations are unlinked.

### Bookkeeping

`metadata-reap` writes one `retention_sweeps` row per invocation
(same machinery as phase9a) with `records_demoted = sum(collapsed
across all targets)` and `tier_transitions_json` carrying the full
breakdown plus log-rotation counters. The dashboard's retention
view (phase9i) renders the row alongside every other sweep.

### Read-side awareness

`cortex_storage::union_read_bootstrap_jobs`, `union_read_sessions`,
and `union_read_classifier_spend` transparently union the raw rows
still living in the source table with the rolled summary tables.
Dashboard queries that span >30 days for bootstrap or >365 days for
sessions / spend MUST go through the union helper so the totals
stay continuous after the reaper runs.

### Test surface (metadata reap, phase9g)

Unit tests in `crates/cortex-retention/src/metadata_reap.rs`:

- `plan_default_uses_spec_thresholds`
- `bootstrap_success_row_thirty_one_days_old_collapses`
- `bootstrap_failed_row_is_preserved`
- `bootstrap_multiple_runs_same_day_aggregate_into_one_bucket`
- `sessions_year_old_rows_collapse_to_monthly`
- `sessions_with_null_repo_collapse_to_empty_string_bucket`
- `classifier_spend_year_old_rows_collapse_to_monthly`
- `re_run_with_no_aged_rows_is_a_noop`
- `dry_run_records_counters_without_mutating`
- `target_filter_runs_only_one_rollup`
- `rerun_with_already_rolled_bucket_increments_existing_row`
- `report_metadata_reap_json_round_trips`
- `vacuum_runs_when_freelist_ratio_high`

Unit tests in `crates/cortex-cli/src/ops/log_rotate.rs`:

- `opts_default_uses_spec_thresholds`
- `missing_file_is_a_noop`
- `empty_file_is_a_noop`
- `fresh_small_file_does_not_rotate`
- `six_megabyte_file_triggers_rotation`
- `day_suffix_collisions_use_a_monotonic_counter`
- `keeps_only_n_most_recent_rotations`
- `old_file_past_age_threshold_rotates_even_when_small`

Read-side awareness test in `crates/cortex-storage/src/metadata.rs`:

- `union_read_returns_identical_totals_before_and_after_rollup`

## Scheduler (phase9k)

The retention pipeline is dangerous and high-frequency: nightly
sweeps, weekly vacuums, ad-hoc digests. Phase9k owns the cron
machinery so a fresh install runs the full pipeline without any
external scheduler.

### Registry table

`cron_jobs` lives in the SQLite metadata DB:

| column          | type    | role                                                                |
|-----------------|---------|---------------------------------------------------------------------|
| `name`          | TEXT PK | Stable identifier (`retention.sweep`, `retention.metadata_reap`, …).|
| `schedule`      | TEXT    | 5-field cron expression (`m h dom mon dow`), UTC.                    |
| `command`       | TEXT    | Shell command line the scheduler spawns.                             |
| `enabled`       | INT     | `1` to fire, `0` to skip.                                            |
| `last_run_at`   | TEXT    | RFC-3339 of the last run start.                                      |
| `last_status`   | TEXT    | `success` / `failed` / `lock_held`.                                  |
| `next_run_at`   | TEXT    | Next firing — recomputed from `schedule` after every run.            |
| `last_error`    | TEXT    | First line of stderr when `last_status='failed'`.                    |
| `last_stdout`   | TEXT    | Tail of the last child's stdout (capped at 64 KB).                   |
| `last_stderr`   | TEXT    | Tail of the last child's stderr (capped at 64 KB).                   |
| `failure_streak`| INT     | Consecutive failures; reset to 0 on success.                         |
| `last_warning_at` | TEXT  | Stamp from the most recent `schedule.repeated_failure` warning.      |

`apply_phase9k_schema(conn)` ensures the table exists at every
`cortex-ops` boot.

### Defaults

`seed_defaults` (called by the daemon on startup) inserts eight
rows via `INSERT OR IGNORE`:

| name                          | schedule       | command                                  | enabled |
|-------------------------------|----------------|------------------------------------------|---------|
| `retention.sweep`             | `0 3 * * *`    | `cortex-ops retention-sweep`             | yes     |
| `retention.rollup`            | `0 4 * * *`    | `cortex-ops rollup`                      | yes     |
| `retention.cas_vacuum`        | `30 4 * * 1`   | `cortex-ops cas-vacuum --force`          | yes     |
| `retention.pii_enforce`       | `0 5 * * *`    | `cortex-ops pii-enforce`                 | yes     |
| `retention.turn_digest`       | `0 6 * * 0`    | `cortex-ops turn-digest --budget-cents 500` | yes  |
| `retention.meili_prune`       | `30 5 * * *`   | `cortex-ops meili-prune`                 | yes     |
| `retention.metadata_reap`     | `45 5 * * *`   | `cortex-ops metadata-reap`               | yes     |
| `retention.memory_consolidate`| `0 7 * * 0`    | `cortex-ops memory-consolidate --apply`  | no      |

Operators who disable a job retain that setting across restarts —
seeding is idempotent.

### Scheduler loop

**Phase10k — daemon ownership.** The tick loop runs inside
`cortex-api` as a `tokio::spawn` task next to the silent-drop
watcher. On boot the daemon calls
`cortex_retention::scheduler::seed_defaults` once (idempotent — an
operator who disabled a row keeps the setting across restarts) and
then ticks every 30 s. The opt-out env var
`CORTEX_RETENTION_DAEMON=disabled` keeps the daemon idle so an
operator can drive sweeps externally (CI, rolling upgrades). See
`crates/cortex-api/src/retention_daemon.rs` for the wire-up.

The scheduler ticks at most every 30 s. Each tick:

1. Calls `select_due_cron_jobs(now)` — `enabled=1 AND
   next_run_at <= now`, sorted by `next_run_at` ASC then `name` ASC.
2. For every due job, acquires a per-name in-process semaphore so
   concurrent ticks (or a `run-now` racing the timer) serialise.
3. Calls the registered [`Runner`] — production wires
   `ProcessRunner` (spawns `cmd /C ...` on Windows, `sh -c ...`
   elsewhere); tests use `MemoryRunner`.
4. Captures the tail of stdout / stderr (capped at 64 KB each)
   and translates the exit code:
   - `0` → `success`
   - `2` → `lock_held` (the underlying retention subcommand's
     advisory lock is the authority — the scheduler defers).
   - any other → `failed`
5. Writes `last_run_at`, `last_status`, `last_error`,
   `last_stdout`, `last_stderr`, `failure_streak`, and the new
   `next_run_at` (re-derived from `schedule`).

### CLI surface

```
cortex-ops schedule list                          # table
cortex-ops schedule show <name>                   # full row + stdout/stderr tail
cortex-ops schedule enable <name>
cortex-ops schedule disable <name>
cortex-ops schedule set <name> "<5-field cron>"   # validates + recomputes next_run_at
cortex-ops schedule run-now <name>                # bypasses timer, honours advisory lock
cortex-ops schedule seed-defaults                 # idempotent
cortex-ops schedule tick [--time-travel RFC3339]  # one tick (in-memory runner)
```

`run-now` exits with code 2 (`lock_held`) when another run is
already in flight against the underlying `retention_sweeps.status`
row — operators get a clear "lock held" message instead of a
double-execution.

### Repeated-failure observability

When a job's `failure_streak` reaches 2, the scheduler queues a
`RepeatedFailureWarning { name, recent_failures, last_error }`.
The same job will not raise a second warning within a 24 h
window — `last_warning_at` is the dedup pivot. The phase9i
dashboard banner (spec
[16-dashboard.md §"Retention view"](16-dashboard.md))
surfaces these warnings without further work.

### Test surface (scheduler, phase9k)

11 unit tests in `crates/cortex-retention/src/scheduler.rs`
(moved from `cortex-cli` in phase10k so `cortex-api` can spawn the
tick loop without taking a circular dependency on the CLI crate):

- `parse_schedule_accepts_five_six_and_seven_field_forms`
- `next_after_advances_for_daily_schedule`
- `seed_defaults_inserts_eight_jobs_idempotently`
- `tick_runs_due_job_and_advances_next_run`
- `tick_skips_disabled_jobs`
- `run_now_records_outcome_and_advances_next_run`
- `run_now_propagates_lock_held_status`
- `two_consecutive_failures_emit_repeated_failure_warning`
- `third_consecutive_failure_does_not_double_warn`
- `semaphore_serialises_runs_for_same_name`
- `trail_capped_returns_tail_only_when_exceeding_cap`

## CI canary (phase9j)

The retention pipeline is dangerous: it deletes data, rewrites
archives, and (in production) calls Sonnet at the user's expense.
Phase9j adds a deterministic regression-detection canary that
runs on every PR touching the retention surface plus a nightly
schedule.

### Harness

[`crates/cortex-retention/tests/canary.rs`](../../crates/cortex-retention/tests/canary.rs)
hosts two integration tests:

- `synthetic_corpus_distribution_matches_spec` — asserts the
  corpus mix (600 turns / 250 tool_calls / 50 decisions / 50
  analyses / 50 memory) and PII distribution (60 % null / 25 %
  low / 10 % medium / 5 % high) are exact.
- `retention_canary_full_pipeline` — drives every retention
  stage end-to-end, asserts the post-state, and re-runs the full
  pipeline to verify idempotence.

The corpus generator lives next to the test at
[`tests/support/synth_corpus.rs`](../../crates/cortex-retention/tests/support/synth_corpus.rs)
and is pure-Rust (no I/O, no RNG) so a CI re-run produces
byte-identical envelopes.

### Stages exercised

The harness drives every retention library entrypoint with
`--time-travel = now` (anchored at `2026-04-29T18:00:00Z` so
boundaries fire deterministically):

1. `run_sweep` — tier transitions FP32 → PQ at 30 d, PQ → Binary at 365 d.
2. `quarantine_pre_existing` + `enumerate_compactable` +
   `compact_partition` + `apply_three_year_drop` — archive rollup.
3. `run_enforcement` — PII high-30d / medium-90d / null-safety-90d.
4. `run_turn_digest` — bounded by `max_usd_cents_per_run = 5`.
5. `run_meili_prune` — body blanking + summary cap.
6. `metadata_reap::run` — bootstrap_jobs / sessions / classifier_spend rollups.
7. `cas_vacuum::run` — orphan blob reclamation (`--force` so the
   seeded 100-orphan cohort drops in one pass).

### Storage assertions

- FP32 collections contain zero records older than 30 d.
- PQ collections contain zero records older than 365 d.
- Cold binary contains every record that started >365 d old plus
  every record demoted from PQ during the sweep.
- Archive: `_quarantine/` contains the planted `.corrupted`
  artifact; no `.tmp` orphans; no `.corrupted` files outside
  `_quarantine/`.
- Meili: zero docs older than 90 d remain unpruned (re-enumerating
  after `commit_updates` returns an empty set).
- SQLite: zero `bootstrap_jobs` success rows older than 30 d;
  `bootstrap_jobs_daily` populated.
- CAS: every seeded orphan is gone after the vacuum.
- PII: every high-cohort target has a rewrite stamped
  `pii_high_30d` with `body=None`; every medium-cohort target
  has a rewrite stamped `pii_medium_90d` with the synthesized
  summary.

### Bounded LLM cost

The canary caps the per-run LLM budget at 5 ¢ via
`DigestPlan::max_usd_cents_per_run = 5`. The reference merger
charges 0 ¢ per call (deterministic in-process), so the cap is
honoured even if a regression accidentally widens the bucket
set.

### Idempotence

After the first pass the canary:

- snapshots every redacted event_id and rebuilds PII targets with
  `redacted: Some(_)` so the matcher's idempotence guard fires;
- pre-populates the digest backend's `existing` map from
  `persisted()` so `lookup_existing` short-circuits;
- calls `MemoryMeiliBackend::commit_updates` so the
  `already_pruned` flag is sticky.

It then drives every stage a second time and asserts:

- zero records demoted,
- zero docs pruned,
- zero blobs vacuumed,
- zero new digests produced,
- zero classifier cents spent,
- zero metadata rows collapsed,
- zero PII rewrites applied.

### CI workflow

[`.github/workflows/retention-canary.yml`](../../.github/workflows/retention-canary.yml)
runs the canary on every PR touching `crates/cortex-retention/`,
`crates/cortex-storage/`, `crates/cortex-classifier/`, or
`crates/cortex-workers/`, plus a nightly cron at 04:00 UTC. A
failing canary fails the workflow and uploads the cargo-test log
as an artifact.

## Observability (phase9i)

Every sweeper stamps one `retention_sweeps` row per invocation and
emits one `cortex.events.enriched` event of `kind=retention.*`. The
dashboard's Retention tab (spec
[16-dashboard.md §"Retention view"](16-dashboard.md))
subscribes to both surfaces:

- `GET /v1/retention/sweeps?limit=N&since=RFC3339` — recent rows
  with the per-stage JSON breakdown surfaced as a `stages` object
  the GUI projects into one card per sweep type.
- `GET /v1/retention/state` — `archive_bytes` bucketed by age,
  `cas` totals, `next_runs` (all `"never"` until phase9k publishes
  a cron schedule).
- `GET /v1/dashboard/timeline/stream` — SSE; the GUI filters for
  events whose `kind` starts with `retention.` and renders the
  last 100 in a live log strip.

Together these close the observability loop on Phase 9: a sweeper
that fails twice in a row surfaces a red banner in the Retention
tab without anyone needing to grep `retention_sweeps`.

## Auto-memory consolidator (phase9h)

Claude Code's auto-memory writes one Markdown file per memory under
`~/.claude/projects/<project-slug>/memory/*.md` plus a top-level
`MEMORY.md` index. The index is loaded into every Claude Code session
context and is truncated past line 200 — so the more entries the
auto-memory accumulates, the more frequently a human needs to prune
them. Phase9h treats that directory like any other Cortex memory
store: embed every entry, cluster near-duplicates, ask a merge agent
to produce one denser entry per cluster, archive the originals,
regenerate the index.

### CLI

```
cortex-ops memory-consolidate \
    [--project <slug>] \
    [--threshold 0.78] \
    [--drift-floor 0.6] \
    [--max-clusters N] \
    [--apply] \
    [--memory-dir PATH] \
    [--json]
```

Default mode is preview-only: the run reads the directory, prints
the cluster plan, and exits without touching any file. `--apply`
must be supplied to mutate the filesystem.

### Project slug

The slug derives from the working tree path the same way Claude
Code does: every `:`, `/`, or `\\` becomes a single `-`. So
`e:\HiveLLM\Cortex` → `e--HiveLLM-Cortex`, matching
`~/.claude/projects/e--HiveLLM-Cortex/memory/` exactly.

### Discovery

The walker reads every `*.md` other than `MEMORY.md` (and anything
inside `_archive/`). Files whose YAML frontmatter is missing or
invalid are surfaced as warnings — they're left in place but never
clustered.

### Clustering

Each surviving file's body is embedded once and clusters form by
greedy cosine grouping inside each `type` bucket: a file attaches
to the highest-similarity existing cluster whose representative is
≥ `threshold` (default 0.78), otherwise it starts a new cluster of
size 1. The matcher NEVER mixes types — `feedback` and `project`
entries with byte-identical bodies stay in separate clusters.

### Sonnet merge + drift guard

Clusters of size ≥ 2 go to the merger, which produces one merged
[`MergedMemory`](../../crates/cortex-cli/src/ops/memory_consolidate.rs)
preserving every concrete instruction from the inputs. The
orchestrator then re-embeds the merged body and compares it to
every source body; if any source-to-merged cosine drops below
`drift_floor` (default 0.6), the merge is rejected and the cluster
remains intact. The cluster surfaces as `SkippedDrift` in the
report.

The prompt template is
[`CONSOLIDATE_AUTO_MEMORY_V1`](../../crates/cortex-classifier/src/prompt.rs)
and ships with the [`cortex-classifier`](../../crates/cortex-classifier/prompts/consolidate_auto_memory.v1.txt)
crate.

### Apply step

When `--apply` is supplied for a successful merge:

1. Source files move into `memory/_archive/<RFC3339>/<original>` —
   never deleted, always preserved for replay.
2. The merged body lands at `memory/consolidated_<short-hash>.md`,
   where `<short-hash>` is the first 8 hex chars of SHA-256 over
   the rendered file (frontmatter + body).
3. `MEMORY.md` is regenerated from the surviving files'
   frontmatter — one line per entry, capped at 150 chars, no
   YAML frontmatter on the index itself.

### Idempotence

Re-running `--apply` immediately after a successful run finds zero
clusters of size ≥ 2 (every survivor sits in its own
`Singleton` cluster) and exits without writing anything new.

### Test surface (memory consolidator, phase9h)

17 unit tests in `crates/cortex-cli/src/ops/memory_consolidate.rs`:

- `slug_replaces_drive_colon_with_double_dash_and_separator_with_single`
- `slug_strips_trailing_separators`
- `parse_memory_body_extracts_frontmatter_and_body`
- `parse_memory_body_strips_quotes_around_values`
- `parse_memory_body_rejects_missing_close_marker`
- `parse_memory_body_rejects_unknown_type`
- `read_memory_dir_skips_index_and_collects_warnings`
- `hashing_embedder_returns_unit_norm_vectors`
- `cosine_self_similarity_is_one`
- `cluster_groups_near_duplicates_within_same_type`
- `cluster_never_mixes_types`
- `dry_run_leaves_directory_untouched`
- `apply_archives_originals_and_writes_consolidated`
- `re_run_after_apply_finds_no_clusters`
- `drifted_merge_is_rejected_and_originals_remain`
- `render_index_caps_each_line_at_150_chars`
- `render_memory_body_round_trips_through_parser`
