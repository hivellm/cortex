## 0. SUPERSEDED (2026-06-21)
- [x] 0.1 Superseded by the CLI-only resolution: the consolidator now summarises through the LOCAL logged-in `claude` CLI (no Anthropic API key), so the original premise (need a valid `ANTHROPIC_API_KEY` + Opus cost authorization) no longer applies. Done in `phase0_recurrent-consolidation-and-retention` §4 (commits 0701630 / 58398b0) + host daemon + autostart (`docs/consolidator-host-daemon.md`). The items below are retained for history only; none are actionable.

## 1. Gate (blocking precondition)
- [ ] 1.1 Confirm a valid `ANTHROPIC_API_KEY` is present in cortex-api + cortex-classifier-worker + cortex-consolidator (`key_len` plausible, not empty/placeholder)
- [ ] 1.2 Confirm operator authorization for the recurring Opus spend
- [ ] 1.3 Confirm live ingestion is flowing (depends on `phase0_live-ingestion-staleness` §2)

## 2. Enable the trigger producer
- [ ] 2.1 Set `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=1` in `docker-compose.yml` and recreate the classifier worker
- [ ] 2.2 Verify per-event consolidation triggers (session_end / decision / topic grains) are emitted

## 3. Confirm cadence runs for real
- [ ] 3.1 Verify `retention.consolidator_nightly` (02:00 UTC) and `retention.memory_consolidate` (weekly) execute real consolidations, not no-ops
- [ ] 3.2 Document the valid `ANTHROPIC_API_KEY` requirement in the deployment docs

## 4. Initial backfill + verification
- [ ] 4.1 Run the initial backfill with a cost estimate first (estimate, then apply)
- [ ] 4.2 Verify `similar_sessions` / `topic_search` / `consolidations_recent` populate
- [ ] 4.3 Verify the `health.watchdog` `consolidation_missing` / `consolidation_stale` alarms clear

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
