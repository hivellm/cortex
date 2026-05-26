# Spec: consolidation tier

## ADDED Requirements

### Requirement: Consolidation envelope is a first-class Kind

The system SHALL expose a `Kind::Consolidation` variant carrying a
`ConsolidationPayload` that summarises a set of source events and
MUST round-trip through the existing ingestion + indexing pipeline
without schema-shape changes outside the new variant + payload.

#### Scenario: serde round-trip preserves every field

Given a `ConsolidationPayload` with non-empty `source_event_ids`, `takeaways`, and `outcome_distribution`
When the envelope is serialised and deserialised via `serde_json`
Then every field MUST be preserved byte-for-byte
And `validate_event` MUST return `Ok` for the round-tripped envelope

#### Scenario: schema_stem maps to canonical filename

Given a `Kind::Consolidation` envelope
When `Kind::schema_stem()` is called
Then it MUST return `"consolidation"`

#### Scenario: source_event_count clamps the inline list

Given a consolidation summarising 10 000 raw events
When the producer emits the envelope
Then `source_event_count` MUST equal 10 000
And `source_event_ids` MUST contain at most 256 entries (the inline cap)
And the dropped IDs MUST be retrievable from the Parquet archive via the consolidation's temporal_span

### Requirement: Three grains share one envelope shape

The producer SHALL emit consolidations with `grain ∈ {Session, Topic, DecisionTrace}`. The `scope` field
MUST discriminate against the grain.

#### Scenario: session grain carries session_id

Given a session consolidation
When the envelope is emitted
Then `grain == Session`
And `scope.session_id` MUST be a non-empty ULID
And `temporal_span.start <= occurred_at_min(source_event_ids)`
And `temporal_span.end >= occurred_at_max(source_event_ids)`

#### Scenario: decision-trace grain links to a Decision

Given a decision-trace consolidation produced by walking from a `Kind::Decision` envelope
When the envelope is emitted
Then `grain == DecisionTrace`
And `scope.decision_id` MUST equal the source decision's `decision_id`
And `parent_event_id` MUST equal the source decision's `event_id`

#### Scenario: topic grain carries a topic label and HDBSCAN cluster signature

Given a topic consolidation produced from an HDBSCAN cluster of ≥ 3 sessions
When the envelope is emitted
Then `grain == Topic`
And `scope.topic` MUST be a non-empty string
And `tags` MUST include the dominant repo slug
And `source_event_ids` MUST include at least one event per source session in the cluster

### Requirement: Consolidations are routed to the new family

`Kind::Consolidation` SHALL route to the new `consolidations` family
on every backend.

#### Scenario: fulltext routing emits to global + per-repo Meili indexes

Given a Consolidation envelope with `repos=["cortex"]`
When the fulltext worker indexes it
Then a document MUST appear in `cortex_consolidations` (global index)
And a document MUST appear in `cortex-cortex-consolidations` (per-repo index)

#### Scenario: embedder routing emits to consolidation collection

Given the same envelope
When the embedder worker indexes it
Then chunks MUST land in `cortex.consolidation.fp32` (hot tier) when age ≤ 7 days
And chunks MUST migrate to `cortex.consolidation.pq` (warm tier) on the standard age schedule

### Requirement: Pre-thinking prefers consolidations over raw

The pre-thinking renderer SHALL replace the raw "Past sessions"
section with a "Consolidated context" section when ≥ 1 consolidation
matches the query scope.

#### Scenario: consolidated context renders top-3

Given a query with 5 matching consolidations
When the pre-thinking renderer produces the bundle
Then the bundle MUST contain a "Consolidated context" section
And that section MUST contain exactly 3 lines (top-3 by similarity)
And EACH line MUST be ≤ 200 bytes
And the section MUST stay under the 32 KiB budget

#### Scenario: fallback to raw when zero consolidations

Given a query with 0 matching consolidations and ≥ 1 matching raw turn
When the pre-thinking renderer produces the bundle
Then the bundle MUST contain the raw "Past sessions" section (per phase 11i §4.1)
And MUST NOT contain a "Consolidated context" section

### Requirement: Pruning never destroys evidence

The pruner SHALL demote raw events to lower tiers when a referencing
consolidation exists, and MUST NOT remove the underlying Parquet
archive entry except via explicit hard-purge paths.

#### Scenario: 90-day-old event with active consolidation demotes to cold

Given a raw turn with `occurred_at` 91 days ago
And a Consolidation envelope listing that turn in `source_event_ids`
When the pruner runs
Then the turn MUST be removed from `cortex.turn.fp32` and `cortex.turn.pq`
And MUST be present in `cortex.cold.binary` with reduced fields
And the Parquet archive entry MUST be unchanged

#### Scenario: pruner refuses to drop an event referenced by a recent consolidation

Given a raw turn with `occurred_at` 400 days ago
And a Consolidation envelope (occurred_at 5 days ago) listing that turn in `source_event_ids`
When the pruner runs
Then the turn MUST remain in the cold tier
And MUST NOT be dropped from indexes
And the pruner MUST log a `held_by_active_consolidation` reason

#### Scenario: hard purge requires confirmation token

Given a `cortex_forget` MCP call without a confirmation token
When the request reaches the pruner
Then the pruner MUST refuse the operation
And MUST return an error indicating the token is required

### Requirement: Fidelity gate prevents hallucinated takeaways

CI SHALL run `consolidation_fidelity_it` sampling 50 raw → consolidation
pairs; the test MUST assert every `takeaways[]` entry has ≥ 1
supporting `source_event_id`. Threshold: ≥ 90 % supported on Haiku
consolidations, ≥ 98 % on Opus.

#### Scenario: fidelity IT fails when threshold is missed

Given a fixture set where 15 of 50 sampled Haiku takeaways have no supporting source_event_id
When `cargo test -p cortex-consolidator --test consolidation_fidelity_it` runs with `CORTEX_FIDELITY_IT=1`
Then the test MUST fail
And the failure message MUST include the offending takeaway texts and consolidation IDs

#### Scenario: fidelity IT passes at threshold

Given a fixture set where 47 of 50 sampled Haiku takeaways are supported
When the IT runs
Then the test MUST pass (94 % > 90 % threshold)

### Requirement: Cost telemetry surfaces in coverage health

`cortex-consolidator` SHALL emit per-grain cost metrics that the
`/v1/health/coverage` endpoint surfaces under a new `consolidator`
block.

#### Scenario: monthly burn metric appears in health coverage

Given the consolidator has run for 30 days emitting 1 000 Haiku + 50 Opus consolidations
When the operator calls `GET /v1/health/coverage`
Then the response MUST contain a `consolidator` block
And the block MUST include `monthly_cost_usd`, `consolidations_per_grain`, `last_run_ts`
And `monthly_cost_usd` MUST equal the sum of per-call telemetry within ±1 %
