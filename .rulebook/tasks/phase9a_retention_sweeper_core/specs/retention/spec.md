# Spec: Retention sweeper core

## ADDED Requirements

### Requirement: Daily tier-transition sweep

The Cortex stack SHALL provide an idempotent sweep job that demotes
Vectorizer records across quantization tiers based on age.

The sweep MUST process the pairs:
- `cortex.turn.fp32` → `cortex.turn.pq` at `occurred_at < now - 30d`
- `cortex.tool_call.fp32` → `cortex.tool_call.pq` at `occurred_at < now - 30d`
- `cortex.code_chunk.fp32` → `cortex.code_chunk.pq` at `occurred_at < now - 30d`
- `cortex.{turn,tool_call,code_chunk}.pq` → `cortex.cold.binary` at `occurred_at < now - 365d`

Records in always-hot collections (`decision`, `analysis`, `memory`, `law`)
MUST NOT be demoted by this sweep.

#### Scenario: FP32 record at 31 days moves to PQ
Given a turn record in `cortex.turn.fp32` with `occurred_at = now - 31d`
When `cortex-retention sweep` runs at `now`
Then the record MUST exist in `cortex.turn.pq` with the same `event_id`
And MUST NOT exist in `cortex.turn.fp32`
And a `retention.tier_transition` event MUST be emitted with
`from_tier="fp32"`, `to_tier="pq"`.

#### Scenario: idempotent re-run is a no-op
Given a sweep has already moved a record from FP32 to PQ
When the sweep runs a second time within the same window
Then no Vectorizer mutation MUST happen for that record
And the resulting `retention_sweeps` row MUST have `records_demoted = 0`.

#### Scenario: --time-travel drives the cutoff deterministically
Given `--time-travel 2030-01-01T00:00:00Z` is supplied
When the sweep evaluates each record
Then the `now` reference MUST be 2030-01-01 regardless of system clock.

### Requirement: Bookkeeping in retention_sweeps

Every invocation MUST write exactly one row to `retention_sweeps`. The row
MUST contain `sweep_id` (ULID), `started_at`, `finished_at`,
`records_demoted`, `records_dropped`, and `tier_transitions_json` with a
per-(source,destination) count breakdown.

#### Scenario: crashed sweep leaves a marker
Given a sweep is killed mid-flight
When the next sweep starts
Then it MUST mark the orphan row `status='abandoned'`
And it MUST proceed with its own fresh `sweep_id`.

### Requirement: Concurrency safety

A second `cortex-retention sweep` invocation that overlaps an in-progress
sweep MUST exit with code 2 and MUST NOT touch Vectorizer.

The lock MUST be released within 60 s of the holding process dying.

### Requirement: Observability

The sweeper MUST emit `cortex.events.enriched` events with `kind =
"retention.tier_transition"` and one event per moved record. Each event
MUST carry `event_id`, `from_tier`, `to_tier`, `reason`, and the originating
collection name.
