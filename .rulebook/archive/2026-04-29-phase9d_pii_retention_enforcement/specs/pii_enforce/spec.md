# Spec: PII retention enforcement

## ADDED Requirements

### Requirement: High-risk drop at 30 days

For every event where `payload.pii_risk = "high"` and
`occurred_at < now - 30d`, the runner MUST:
- decrement the CAS refcount for `payload.body_ref` if present,
- rewrite the Parquet row with `payload.body = null` and
  `payload.redacted = "pii_high_30d"`,
- delete the matching record from every Vectorizer collection
  (`fp32`, `pq`, `cold.binary`),
- delete the matching Meili document.

The cross-store sequence MUST be: Parquet → Vectorizer → Meili → CAS.
A partial run MUST NOT leave the public surface (Vectorizer + Meili)
holding raw PII.

#### Scenario: 31-day-old high-risk event is fully redacted
Given an event with `pii_risk="high"` and `occurred_at = now - 31d`
When `cortex-retention pii-enforce` runs
Then the Parquet row MUST have `body=null` and `redacted="pii_high_30d"`
And the Vectorizer record MUST be absent in every tier
And the Meili document MUST be absent
And the CAS refcount for the body MUST be decremented.

### Requirement: Medium-risk re-summarization at 90 days

For every event where `payload.pii_risk = "medium"` and
`occurred_at < now - 90d`, the runner MUST replace `payload.body` with a
classifier-produced summary capped at 512 tokens, set
`payload.redacted = "pii_medium_90d"`, re-embed, re-index in Meili, and
decrement the CAS refcount on the original body.

#### Scenario: 91-day medium-risk event keeps a summary, loses the raw body
Given an event with `pii_risk="medium"` at `now - 91d` whose body is 4 KB
When the medium path runs
Then `payload.body` length MUST be ≤ 512 tokens
And `payload.redacted` MUST equal `"pii_medium_90d"`
And the Vectorizer vector MUST have been recomputed from the new body
And the Meili document MUST contain the summary, not the original.

### Requirement: Null-tier safety net

Events with `payload.pii_risk = null` and `occurred_at < now - 90d` MUST
be treated as medium-risk. A `cortex.warnings` event MUST be emitted
per record so classifier coverage gaps are auditable.

#### Scenario: legacy untagged event is summarized, not retained
Given an event with `pii_risk=null` at `now - 100d`
When `cortex-retention pii-enforce` runs
Then the medium path MUST execute on the record
And a `cortex.warnings` event MUST be emitted for it.

### Requirement: Idempotence

Re-running the enforcement against an already-enforced event MUST be a
no-op. The match predicate MUST exclude rows where `payload.redacted`
is already set.

#### Scenario: re-running does not re-summarize
Given an event whose `payload.redacted = "pii_medium_90d"`
When the runner sees it again
Then no classifier call MUST happen for that event
And the Vectorizer vector MUST NOT be touched.
