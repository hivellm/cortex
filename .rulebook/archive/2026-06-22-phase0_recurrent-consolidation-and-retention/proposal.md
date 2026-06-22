# phase0 — recurrent consolidation + retention/cleanup of stale data

Source: operator direction (2026-06-21) — "consolidate data more
recurrently, clean up old useless data". phase21 (data-classification)
deprioritized.

## Why

A review (2026-06-21) found that NO recurrent consolidation and NO
recurrent cleanup run today:

- **Consolidation:** indexes are ~empty (global `cortex_consolidations`
  and `cortex_topic_cards` do NOT exist; 42 stale per-repo docs). The
  trigger producer is off (`CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED`
  empty), there is no nightly cron, and there is no valid Anthropic key
  (the container's is 29 chars = placeholder). The trigger code exists
  (phase24: session_end / decision / topic).
- **Retention/cleanup:** the tooling exists (`cortex-ops`
  retention-archive-purge / meili-prune / turn-digest / tool-call-digest /
  rollup / retention-sweep / consolidation-prune / sweep-empty /
  pii-enforce) BUT cron schedules none (`next_runs` empty) → old raw data
  is never digested, orphan indexes are never pruned, and the graph
  carries garbage (13,037 nodes, 163 Turn with null `id` from legacy
  corruption).
- **Prerequisite:** live ingest is stopped (host adapter) — no new events
  to consolidate. Covered by `phase0_live-ingestion-staleness`.

Net effect: the system accumulates raw data and neither distils nor
prunes it — the opposite of a "living memory".

## What Changes

- **Recurrent consolidation cadence:** enable the trigger producer
  (config) + schedule the `nightly` (cron) + document the API-key
  requirement. Triggers: session_end (30 min idle), decision_landed,
  topic-threshold (phase24 §1.3 builder ready). Guard Opus cost behind a
  flag + estimate.
- **Recurrent retention/cleanup cadence:** schedule via cron the digest
  sweeps (turn/tool_call → compress old raw), rollup, and prune (Meili
  tier-prune + sweep-empty of orphan indexes). Define windows (e.g. digest
  > 30 d, prune cold-tier > 90 d); zero API cost (local pruning).
- **Corrupt-graph pruning:** remove null-id + legacy orphan nodes
  (deletion — requires explicit operator authorization).
- **Anti-recurrence watchdog:** alarm when `files_watched==0` / ingestion
  idle / no consolidation/sweep in N (sketched in
  `phase0_live-ingestion-staleness` §3).

## Impact
- Affected code: `docker-compose.yml` (flag + cron), `cortex-ops`
  schedule/retention, `crates/cortex-workers/src/{consolidator,retention,sweep}`,
  health/coverage alarms.
- Breaking change: NO (config + scheduling; graph deletion is
  corrupt-data repair).
- User benefit: memory that distils itself (recurrent consolidations /
  topic cards) and stays lean (old raw compressed, garbage pruned), with
  no manual intervention.
