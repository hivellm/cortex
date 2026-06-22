# Proposal: phase0_consolidation-enablement-on-valid-key

## Why

`phase0_recurrent-consolidation-and-retention` §4 was blocked: the
recurrent consolidation pipeline spends Opus via the Anthropic API and
requires a valid `ANTHROPIC_API_KEY` plus explicit cost authorization.
At the time of that task the key was **empty** in every container
(`key_len=0` on cortex-api and cortex-classifier-worker), so arming the
trigger producer would only emit consolidation triggers the consolidator
could not process (API calls fail), and the backfill could not run. The
operator chose to defer §4 rather than arm a broken pipeline. This
follow-up captures the deferred §4 work so it is not orphaned; it becomes
actionable the moment a valid key is injected and the recurring Opus
spend is authorized.

## What Changes

- Enable the consolidator trigger producer
  (`CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=1`) in `docker-compose.yml`
  and recreate the classifier worker so per-event consolidation triggers
  (session_end / decision / topic grains) start flowing.
- Confirm the nightly + weekly consolidation cron rows
  (`retention.consolidator_nightly` 02:00 UTC, `retention.memory_consolidate`
  weekly — already seeded + enabled) run for real, not as no-ops; document
  the valid `ANTHROPIC_API_KEY` requirement in the deployment docs.
- Run an initial backfill with a cost estimate first, then verify the
  consolidation read surface populates (`similar_sessions`, `topic_search`,
  `consolidations_recent`) and the `health.watchdog`
  `consolidation_missing` / `consolidation_stale` alarms clear.

## Impact

- Affected specs: docs/specs/19-retention.md (consolidation cadence)
- Affected code: docker-compose.yml; consolidator trigger producer; deployment docs
- Breaking change: NO
- User benefit: recurrent memory consolidation actually runs, populating the similar-sessions / topic-search / consolidations-recent surfaces and clearing the watchdog alarm.

## Gate

Do not start until: (1) a valid `ANTHROPIC_API_KEY` is present in the
containers, (2) the operator authorizes the recurring Opus spend, and
(3) live ingestion is flowing (see `phase0_live-ingestion-staleness`).
