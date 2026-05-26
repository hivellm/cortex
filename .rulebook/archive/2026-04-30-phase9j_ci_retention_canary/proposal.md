# Proposal: phase9j_ci_retention_canary

## Why

The retention pipeline (9a–9h) is dangerous: it deletes data, rewrites
Parquet, calls Sonnet at the user's expense. A regression that
demotes too aggressively, drops a non-PII record, or mangles an
idempotence guard is hard to spot manually because the failure mode is
"data quietly disappears". The Phase 8f synthetic e2e canary set the
template for catching pipeline regressions; this task adds a parallel
canary for retention.

## What Changes

1. NEW integration test target `tests/retention_canary.rs` (probably as
   a binary in `crates/cortex-retention/tests/`) that:
   - boots the local docker-compose stack (Vectorizer + Nexus + Meili
     + Synap), reusing `crates/cortex-bootstrap/tests/runner.rs` setup,
   - ingests a synthetic 1k-event corpus across known boundary dates
     (now, now-15d, now-31d, now-91d, now-366d, now-1100d) with a
     mix of `pii_risk` tags,
   - drives every retention subcommand with `--time-travel` set to
     `now+1s` so each boundary fires deterministically,
   - asserts the post-state across all five storage layers.
2. Assertions:
   - FP32 collections contain 0 records >30 d old.
   - PQ collections contain 0 records >365 d old.
   - Cold binary contains every >365 d record except whitelist drops.
   - Parquet archive: hourly files >90 d are gone, daily files exist,
     monthly files exist for >365 d, no `*.tmp` orphans.
   - `_quarantine/` contains the planted `.corrupted` artifact.
   - Meili: docs >90 d have `pruned=true`, `body=""`.
   - SQLite: `bootstrap_jobs` rows >30 d are gone, `_daily` rolled up.
   - `cas_blobs`: orphan blobs are gone after vacuum.
   - PII-high records have `body=null`, PII-medium have a summary.
3. NEW workflow `.github/workflows/retention-canary.yml` that runs the
   canary on every PR touching `crates/cortex-retention/`,
   `crates/cortex-storage/`, `cortex-fulltext/`, `cortex-classifier/`,
   `cortex-graph/`, plus a nightly schedule.
4. Cost-bounded: the test passes a fixed `max_usd_cents_per_run = 5`
   to 9e so the LLM step exercises one bucket but doesn't break the
   bank.

## Impact

- Affected specs: `docs/specs/19-retention.md` §Tests,
  `docs/specs/03-local-stack.md` §CI.
- Affected code: NEW `crates/cortex-retention/tests/canary.rs`,
  NEW `.github/workflows/retention-canary.yml`, helper
  `tests/support/synth_corpus.rs`.
- Breaking change: NO. Pure test surface.
- User benefit: catches regressions in the retention pipeline before
  they hit a real archive; documents expected behavior end-to-end.
