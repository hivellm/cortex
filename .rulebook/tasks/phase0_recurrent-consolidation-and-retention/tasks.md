## 1. Survey (no cost, no deletion)
- [x] 1.1 DONE (2026-06-21): surveyed the sweeps. `sweep-empty` dry-run → 7 empty orphan indexes (`cortex-cortex-governance` + 6 `*-consolidations`). Age scan: >1000 turns/code docs beyond 90d (Meili count capped at 1000) — a large un-digested/un-pruned backlog. KEY FINDING: `turn-digest`/`tool-call-digest` are LLM summarisers (cost API → blocked on the missing key); `sweep-empty`/`meili-prune`/`rollup`/`retention-sweep` are API-free.
- [x] 1.2 DONE: cadence gap confirmed — cron schedules NONE of the sweeps (`next_runs` empty); trigger producer flag empty; container `ANTHROPIC_API_KEY` is a 29-char placeholder.

## 2. Recurrent cleanup (retention — local, no API)
- [ ] 2.1 Define retention windows (turn/tool_call digest > N days; cold-tier prune > M days; sweep-empty orphan indexes) in config
- [ ] 2.2 Schedule the sweeps in cron (nightly) via `cortex-ops schedule`; confirm `next_runs` populated
- [~] 2.3 First apply pass — STARTED with the zero-loss step: `sweep-empty --apply` dropped 7 empty orphan indexes (re-scan → 0 candidates). REMAINING (alter/lose data → need window confirmation): `meili-prune` blanks turn/tool_call bodies >90d (keeps doc+summary), `rollup` merges old parquets, `retention-sweep` re-encodes vectors; `*-digest --purge-originals` deletes raw after digesting (also needs the API key).

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
