# Spec: LLM turn digest summarizer

## ADDED Requirements

### Requirement: Weekly per-topic digest of old turns

`cortex-retention turn-digest` MUST group turns whose `occurred_at <
now - digest_after_days` (default 30) by
`(repo, ISO_year_week, top_topic)` and produce one
`memory_type = "turn_digest"` Memory event per non-empty bucket whose
size is ≥ `min_bucket_size` (default 5).

The digest body MUST be a Sonnet-produced narrative of 200–400 tokens
referencing the bucket's `repo` and `year_week`.

#### Scenario: one digest per (repo, week, topic) bucket
Given 12 turns in repo "alpha" during ISO week 2026-W17 with topic "auth"
And `digest_after_days = 30`
When the runner is invoked at a date past the 30-day boundary
Then exactly one `:Memory{memory_type:'turn_digest'}` MUST be created
And it MUST be linked by `[:SUMMARIZES]` to all 12 source turns.

### Requirement: Source attribution

For every source turn in a digested bucket, the runner MUST:
- write a `(:Memory)-[:SUMMARIZES]->(:Turn)` edge in Nexus,
- set `payload.summarized_by = <digest_event_id>` on the source turn's
  Parquet row,
- record `source_event_ids` (full list) on the digest Memory node.

#### Scenario: source turns are addressable from the digest
Given a digest produced from 12 turns
When the dashboard renders the digest
Then `source_event_ids` MUST contain 12 ids
And every source turn MUST have a non-null `summarized_by`.

### Requirement: Idempotence

Re-running with the same inputs MUST NOT call Sonnet a second time for
buckets that already have a digest. The `--rebuild` flag MUST replace
the existing digest in place (same `event_id`, same Memory node) and
re-link `[:SUMMARIZES]` edges.

#### Scenario: second run is free
Given a successful previous run produced 100 digests
When the runner is re-invoked the next day with no new old turns
Then `classifier_spend.day` MUST NOT increase from this command
And the `retention_sweeps` row MUST report `buckets_done = 0`.

### Requirement: Cost ceiling

The runner MUST stop cleanly when the per-run budget
(`cortex.toml [retention.digest] max_usd_cents_per_run`) is exceeded.
Pending buckets MUST be reported in
`retention_sweeps.tier_transitions_json.turn_digest.buckets_pending`.

#### Scenario: budget exceeded mid-run
Given the budget is 100 ¢ and bucket 47 of 200 pushes spend over budget
When the runner observes the breach
Then it MUST persist the buckets it already finished
And it MUST exit with `buckets_done = 47`, `buckets_pending = 153`.

### Requirement: Demotion eligibility

Source turns belonging to a successfully digested bucket MUST be
considered cold-eligible by the 9a sweep regardless of age, because
the digest is now the authoritative compact representation.

#### Scenario: digested turns move to cold on the next sweep
Given a turn whose `payload.summarized_by` is set
When `cortex-retention sweep` runs against `cortex.turn.fp32` or `pq`
Then the turn MUST be demoted to `cortex.cold.binary`
And it MUST NOT linger in `pq`.
