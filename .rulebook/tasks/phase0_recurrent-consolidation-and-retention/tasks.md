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
- [x] 3.2 DONE (2026-06-21): operator authorized scope A+B. `DETACH DELETE` with single-anchor predicates (dry-run count before each): 4 label-less nodes (`size(labels(n))=0`) + 1280 unkeyed Artifacts (`n.natural_key IS NULL`) = 1284 nodes removed. Integrity re-verified: total nodes 13037→11753 (−1284 exact), edges 14749→13465, label-less now 0, all 6998 remaining Artifacts keyed (`natural_key IS NOT NULL`). Orphans (level C) intentionally KEPT — live ingestion idle since 2026-06-20, orphans may be nodes awaiting edge-building.

## 4. Recurrent consolidation (costs Opus — requires key + cost authorization)
- [ ] 4.1 BLOCKED (2026-06-21): `ANTHROPIC_API_KEY` is empty in all containers (verified `key_len=0` on cortex-api + cortex-classifier-worker) and operator authorization for Opus spend is pending. Arming `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=1` without a valid key would emit triggers the consolidator cannot process (API calls fail). Operator chose to gate §4 on a valid key. Tracked in follow-up rulebook task `phase0_consolidation-enablement-on-valid-key` (status: blocked).
- [ ] 4.2 BLOCKED (2026-06-21): scheduling already exists — `retention.consolidator_nightly` cron is registered + enabled (02:00 UTC, last_status=success as a no-op). The remaining work (document the valid `ANTHROPIC_API_KEY` requirement + verify real consolidation) is gated on the key. Same follow-up task as §4.1.
- [ ] 4.3 BLOCKED (2026-06-21): the backfill spends Opus and needs a valid key + cost OK + live ingest flowing (phase0_live-ingestion-staleness §2). Same follow-up task as §4.1.

## 5. Anti-recurrence watchdog
- [x] 5.1 DONE (2026-06-21): new `cortex-ops watchdog` command (`watchdog.rs`) — pure `evaluate()` over `WatchdogSignals` raises alarms: `archive_watcher_blind` (Critical, watcher healthy but `files_watched==0`), `archive_watcher_unreachable` (Warn), `ingest_stale` (Warn, no emitter flush in N s, default 3600), `sweep_missing`/`sweep_stale` (Warn, `retention_sweeps` recency, default 90000 s), `consolidation_missing`/`consolidation_stale` (Warn, pruner-status `last_run_ts`, default 172800 s). Severity = max(alarms); exit 0/1/2 mirrors `CoverageSeverity`. Seeded as cron `health.watchdog` (`*/15 * * * *`, `cortex-ops watchdog --json`, enabled) so silent failures surface as non-success `last_status`. Live smoke vs the real watcher: files_watched=202 ok, ingest fresh, correctly raised `consolidation_missing` (Warn, exit 1) — consolidation genuinely isn't running (empty key, §4).
- [x] 5.2 DONE (2026-06-21): 8 unit tests on `evaluate()` (all-fresh ok; blind critical; unreachable warn precedence; stale ingest; missing sweep+consolidation; stale sweep; critical outranks warn; rfc3339 parse) + scheduler `seed_defaults` test extended (13→14 jobs, asserts the watchdog row schedule+command). All green.

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 DONE (2026-06-21): CHANGELOG — Added (coverage watchdog + cadence) + Fixed (archive_purge phantom lock_held + graph prune). Spec `docs/specs/19-retention.md` extended with two sections: "`--before` relative-duration shorthand" and "Coverage watchdog" (wire shape, signals table, exit codes, cadence, test surface).
- [x] 6.2 DONE (2026-06-21): tests cover retention windows (`parse_cutoff`/`parse_relative_duration`, 5), scheduling (`seed_defaults` 14-job + watchdog row assertions), and the watchdog evaluator (8). All in-tree `#[cfg(test)]`.
- [x] 6.3 DONE (2026-06-21): `cargo check --workspace` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` = 3137 passed / 0 failed (fixed 2 job-count assertions: `seed_defaults` 13→14 and `retention_daemon::spawn_seeds_defaults_when_metadata_empty` 13→14; fixed ADR-016 audit gate by routing the watcher-URL env read through `cortex_config::Config`).
