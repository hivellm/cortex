# Retention daemon: 6 independent gaps surfaced as one "tudo never" dashboard
**Source**: manual
**Date**: 2026-05-05
**Related Task**: phase11v_retention-daemon-recovery
**Tags**: retention, cron-scheduler, dashboard-bugs, env-resolution, cron-dow, phase11v
# Retention daemon — 6 independent gaps observed as "every sweep card says never"

**Date observed**: 2026-05-05
**Symptom**: Operator opens `127.0.0.1:5173 → Retention`. Every sweep card reports `last run: never`. `Bytes reclaimed last 30 d: 0`. `cas/blobs: 0 B`. `consolidator_nightly` and `consolidation_prune` cards do not exist at all.

The reflex reading is "retention is broken". Reality: 6 of the 7 daily sweeps had run successfully on the previous night. The dashboard was wrong. The retention pipeline was 80% green; the remaining 20% was three subtle bugs hiding behind one observable.

## The six gaps, each independent

1. **Hardcoded "never" in the dashboard handler.**
   `crates/cortex-api/src/dashboard.rs::retention_state` iterated over a static slug list and stamped `next_run: "never"` on every entry. The test next to it (`dashboard.rs::tests::retention_state_*`) FROZE the bug as a contract: `// Per-sweep next_runs all "never" until phase9k.` phase9k landed `retention_daemon` proper, but the dashboard cut never made the same crossing. Result: the only signal a normal operator looks at lied for months.

2. **Wrong env-var name in `consolidation-prune`.**
   `cortex-ops consolidation-prune` reads `CORTEX_EMBEDDER_VECTORIZER_URL`, then falls through to literal `http://127.0.0.1:17001`. Compose-driven boots export the unprefixed `CORTEX_VECTORIZER_URL`. The literal cannot resolve inside the container — it's the host-side mapped port. The same mistake repeats with the Meili key (`CORTEX_FULLTEXT_MEILI_KEY` vs the compose-exported `_API_KEY`). Both names exist in operator scripts that predate the unprefixed form, so a precedence walk (`UNPREFIXED → embedder-prefixed → loopback`) is the safe fix.

3. **`seed_defaults` is INSERT-only, never UPSERT.**
   When the default for `retention.memory_consolidate.enabled` flipped `false → true` in code, every pre-existing `cron_jobs` row stayed at the old value. `INSERT OR IGNORE` never revisits it. There is no migration step and no reconcile pass — operators effectively had to wipe the cron table to pick up the new default.

4. **`next_after()` returned the same instant when `from` matched a slot.**
   The `cron 0.15` crate REJECTS `0` in the day-of-week field (it expects `MON`-`SUN` or `1`-`7`). Our `parse_schedule` adapter passed the bare `0` through unchanged. Result: the schedule failed to parse, `next_after` returned `None`, the daemon's `unwrap_or_else(|| now.to_rfc3339())` fallback set `next_run_at = now`. Every tick: due → run → next=now → due again. `turn_digest` LLM-cost-bearing pipeline ran every 30 s instead of weekly.

5. **`cortex_consolidations` Meili index lazily created.**
   This one *isn't* a bug: the index gets created on first document insert by Meili itself, and the prune handler returns `Ok(vec![])` on `NOT_FOUND`. Worth recording because it WAS the next suspect — easy to misread as "missing schema".

6. **`retention_sweeps` bookkeeping table never received rows.**
   Only two of nine sweep handlers (`retention_sweep` and `rollup`) wrote a row. The other seven completed successfully against the live backend, stamped `last_status='success'` on `cron_jobs`, and emitted nothing into the canonical `retention_sweeps` table. The dashboard's `Bytes reclaimed last 30 d` panel reads `retention_sweeps` and rendered `0` forever.

## The one root cause that connects them

**Each sweep was implemented as a self-contained CLI with its own dashboard story, then bolted on to the cron scheduler later.** No shared "I am running as a sweep" wrapper exists, so each handler picks its own env-var names, its own bookkeeping pattern, its own error-recovery posture. Under that, the dashboard handler took its own shortcut. Six independent gaps all live in the same gap: "what does it mean to BE a sweep" was never codified.

## Process insights

- **Dashboard hardcodes are a tax on debugging.** Every minute spent staring at a hardcoded `"never"` is a minute the operator is mining the wrong vein. Frozen-as-test workarounds (`// hardcoded until phase9k`) do real damage when phase9k slips.
- **`seed_defaults` should always reconcile.** Default-driven config in a long-lived DB needs an UPSERT pass that respects operator intent (`failure_streak > 0 || last_warning_at IS NOT NULL` are the operator-intent signals to honour).
- **Schedule parsers MUST normalise DOW.** Standard cron uses `0`-`6` Sun-Sat. The `cron` crate uses three-letter strings. Translate at the adapter boundary; never trust upstream syntax to round-trip.
- **`next_*()` helpers MUST return strictly future timestamps.** Iteration must skip the current slot when `from == slot`. Add a property test across 365 days × every shipped schedule to lock the contract.
- **Bookkeeping tables get one row per invocation, ALWAYS.** Even on the failure path. The dashboard reads from there, not from the cron-jobs table.

## Where to look next

- `crates/cortex-api/src/dashboard.rs::retention_state` — live-read of `cron_jobs`.
- `crates/cortex-cli/src/bin/cortex-ops.rs::consolidation_prune` — env precedence walk.
- `crates/cortex-workers/src/retention/scheduler.rs::seed_defaults` — UPSERT reconcile pass.
- `crates/cortex-workers/src/retention/scheduler.rs::parse_schedule` — DOW normaliser.
- `crates/cortex-workers/src/retention/scheduler.rs::next_after` — strict-advance loop.
- `crates/cortex-cli/src/ops/sweep_bookkeeping.rs` — single-row writer for every sweep.