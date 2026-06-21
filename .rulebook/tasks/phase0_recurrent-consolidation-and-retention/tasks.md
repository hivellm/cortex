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
- [x] 2.4 DONE (2026-06-21): root-caused — NOT a lock. The cron row runs `retention-archive-purge --before 365d`, but the binary only parsed RFC-3339, so `365d` failed (`premature end of input`) → exit 2 → the run loop's `Some(2) => "lock_held"` mislabel. Fix: `parse_cutoff` in `retention_archive_purge.rs` now accepts both RFC-3339 and relative shorthand (`Nd`/`Nw`/`Nh`), resolving `now - dur`. Verified: `--before 365d --dry-run` → exit 0, cutoff = now−365d (was exit 2). +5 unit tests. (Note: exit 2 == `lock_held` is a real contract for `rollup`/`retention_sweep` running-row advisory-lock conflicts; the conflation with generic exit-2 errors is pre-existing and out of scope here.)

## 3. Corrupt-graph pruning (deletion — requires explicit operator OK)
- [x] 3.1 DONE (2026-06-21): cataloged live (read-only). Graph = 13037 nodes / 14749 edges. `n.id IS NULL` is NOT a corruption signal — Artifact/Turn/ToolCall/Repo key on `natural_key`+`_nexus_id`, not `id`. Real garbage populations:
      - **4 label-less nodes** (`_nexus_id` only, no label/props, degree 1) — dangling edge endpoints; highest-confidence corruption.
      - **1280 Artifacts without `natural_key`** (and no `id`) — unkeyed, unreachable by content-addressable lookup, can't dedupe/match; 126 also orphan, 1154 still carry edges.
      - **6119 orphan Artifacts** (no edges at all) — relationally dead; 5993 keyed + 126 unkeyed.
      - No `natural_key` duplicates (dedup is sound). SAFETY CAVEAT for §3.2: the Nexus planner returns a FULL label scan (all 8278 Artifacts) whenever the WHERE leads with `n.id IS NULL OR n.id=''` on the unindexed `:Artifact(id)` pair — silently widening any DELETE. Deletion queries MUST use single-anchor predicates that verified correct (`n.natural_key IS NULL`, `NOT (n)-[]-()`), never `n.id IS NULL OR …`.
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
