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
