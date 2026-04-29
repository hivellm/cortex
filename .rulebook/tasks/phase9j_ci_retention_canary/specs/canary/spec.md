# Spec: Retention CI canary

## ADDED Requirements

### Requirement: End-to-end retention canary

The canary MUST ingest a deterministic synthetic corpus, drive every
`cortex-retention` subcommand with `--time-travel`, and assert the
post-state across Vectorizer, Nexus, Meili, SQLite, CAS, and the
Parquet archive.

The canary MUST run on every PR that modifies
`crates/cortex-retention/`, `crates/cortex-storage/`,
`crates/cortex-fulltext/`, `crates/cortex-classifier/`, or
`crates/cortex-graph/`, plus a nightly schedule.

#### Scenario: regression in tier-transition fails the canary
Given a code change that causes the sweeper to leave records in FP32 past 30 d
When the canary runs
Then assertion 4.1 MUST fail
And the workflow MUST exit non-zero
And the failing run MUST upload the SQLite + Parquet artifacts.

### Requirement: Bounded LLM cost

The canary MUST cap the per-run LLM budget at 5 ¢ via 9e's
`--max-usd-cents-per-run`. A canary run MUST NOT spend more than this
on classifier calls regardless of bucket count.

### Requirement: Idempotence assertion

After the first full pass the canary MUST re-run every subcommand and
assert that:
- zero records are demoted,
- zero documents are pruned,
- zero blobs are vacuumed,
- zero classifier calls are issued,
- every `retention_sweeps` row written by the second pass has its
  per-stage counters at 0.

#### Scenario: idempotent second pass
Given a successful first canary pass
When every retention subcommand runs a second time
Then `classifier_spend.day` MUST NOT increase
And every assertion in step 4 MUST still hold
And the second-pass `retention_sweeps` rows MUST report zero work.

### Requirement: Quarantine surfacing

The canary MUST plant a single `.corrupted` Parquet artifact before
running the rollup, and MUST assert that after the rollup the file
exists under `events/_quarantine/` with a `.reason` sibling.
