## 1. Survey (no cost, no deletion)
- [x] 1.1 DONE (2026-06-21): surveyed the sweeps. `sweep-empty` dry-run → 7 empty orphan indexes (`cortex-cortex-governance` + 6 `*-consolidations`). Age scan: >1000 turns/code docs beyond 90d (Meili count capped at 1000) — a large un-digested/un-pruned backlog. KEY FINDING: `turn-digest`/`tool-call-digest` are LLM summarisers (cost API → blocked on the missing key); `sweep-empty`/`meili-prune`/`rollup`/`retention-sweep` are API-free.
- [x] 1.2 DONE: cadence gap confirmed — cron schedules NONE of the sweeps (`next_runs` empty); trigger producer flag empty; container `ANTHROPIC_API_KEY` is a 29-char placeholder.

## 2. Recurrent cleanup (retention) — CADENCE ALREADY EXISTS (diagnosis corrected)
> CORRECTION (2026-06-21): the cadence is NOT missing. `cortex-ops schedule list`
> shows 13 cron jobs registered + `enabled`, driven by the `retention_daemon`
> in cortex-api (seeds defaults + runs the tick loop). Most `last_status=success`
> with `next_run_at` set: rollup (04:00), meili_prune (05:30), sweep (03:00),
> consolidation_prune (03:00), turn_digest/tool_call_digest (weekly), pii_enforce,
> metadata_reap, cas_vacuum, sessions_backfill (hourly), consolidator_nightly
> (02:00). `rollup --dry-run` → files_in=0 (archive already compacted). The earlier
> "next_runs empty" was the dashboard endpoint returning empty, NOT missing jobs.
- [x] 2.1 Windows already defined by the daemon's default cron rows (digest >30d weekly, meili_prune >90d, rollup hourly>90d→daily>365d→monthly, tier sweep, etc.).
- [x] 2.2 Already scheduled + enabled (13 jobs, daemon executing). No action needed.
- [x] 2.3 Zero-loss cleanup applied: `sweep-empty --apply` dropped 7 empty orphan indexes (re-scan → 0). Rollup confirmed in-sync (0 pending).
- [ ] 2.4 Fix `retention.archive_purge` stuck on `last_status=lock_held` (the one sweep not running) — clear the stale lock / find why it never acquires.

## 3. Corrupt-graph pruning (deletion — requires explicit operator OK)
- [ ] 3.1 Identify + count the garbage (null-id nodes, edge-less orphans, legacy duplicates); preview without deleting
- [ ] 3.2 Prune after explicit operator authorization; re-verify count + integrity

## 4. Recurrent consolidation (costs Opus — requires key + cost authorization)
- [ ] 4.1 Enable the trigger producer (`CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=1`) in compose + recreate the classifier
- [ ] 4.2 Schedule the `nightly` consolidation in cron; document the valid `ANTHROPIC_API_KEY` requirement
- [ ] 4.3 Once ingest flows (phase0_live-ingestion-staleness) + a valid key + cost OK: run the initial backfill (estimate first) and verify `similar_sessions`/`topic_search`/`consolidations_recent` populate

## 5. Anti-recurrence watchdog
- [ ] 5.1 Coverage alarm when `files_watched==0` (non-empty mount) / ingestion has no POSTs in N min / no sweep or consolidation in N
- [ ] 5.2 Watchdog test

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation (consolidation+retention cadence architecture; CHANGELOG)
- [ ] 6.2 Write tests covering the new behavior (retention windows; scheduling; watchdog)
- [ ] 6.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
