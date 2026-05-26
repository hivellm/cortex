## 1. Dashboard live-read of cron_jobs
- [x] 1.1 In `crates/cortex-api/src/dashboard.rs::retention_state`, replace the hardcoded `next_run: "never"` list with a `MetadataStore::list_cron_jobs()` call that projects `name`, `enabled`, `last_run_at`, `next_run_at`, `last_status`, `failure_streak` into the response.
- [x] 1.2 Extend `RetentionStateBody.next_runs[*]` to carry `last_run`, `last_status`, `failure_streak` alongside `next_run`. Keep field names snake_case.
- [x] 1.3 Honour the `enabled` column: jobs with `enabled = 0` surface as `disabled` rather than `never`.
- [x] 1.4 Rewrite `dashboard.rs::tests::retention_state_*` so the assertion reads the seeded `cron_jobs` rows. Drop the legacy `Per-sweep next_runs all "never" until phase9k` comment.
- [x] 1.5 Update `gui/src/views/Retention.tsx` to render `last_run` / `last_status` / `next_run`. Delete the `last run never` literal.
- [x] 1.6 Update `gui/src/views/Retention.test.tsx` snapshots to match the new card body.

## 2. consolidation-prune env resolution
- [x] 2.1 In `crates/cortex-cli/src/bin/cortex-ops.rs::consolidation_prune`, prefer `CORTEX_VECTORIZER_URL`, then fall through to `CORTEX_EMBEDDER_VECTORIZER_URL`, then to the loopback literal. Apply the same fall-through to `_USER` and `_PASSWORD`.
- [x] 2.2 Add a unit test on the resolver that drives both env names and asserts the precedence.
- [x] 2.3 Document the precedence in `docs/specs/19-retention.md` § "Sweep environment".
- [x] 2.4 Mirror the prefixed names onto the `cortex-api` service in `docker-compose.yml` for operators who already export the embedder triplet.

## 3. seed_defaults becomes UPSERT-aware
- [x] 3.1 In `crates/cortex-workers/src/retention/scheduler.rs::seed_defaults`, after the INSERT loop, run a reconcile pass that updates `enabled` / `command` on existing rows whose default value drifted. Preserve operator-tuned `schedule`.
- [x] 3.2 Emit `tracing::info!(name, field, old, new, "seed_defaults: reconciled drift")` on each update.
- [x] 3.3 Add a regression test: seed once with `enabled: false`, flip the default to `true`, re-seed, assert row's `enabled = 1`.
- [x] 3.4 Add a complementary test: operator manually disables a row, re-seed, row stays disabled (the reconciler must not overwrite operator-disabled rows). Distinguishing signal: the row carries `last_warning_at` IS NOT NULL OR `failure_streak > 0` OR an operator marker column the reconciler treats as "do not override".

## 4. next_after strictly advances
- [x] 4.1 Reproduce the bug in `crates/cortex-workers/src/retention/scheduler.rs::tests`: build a `now` equal to a slot, call `next_after`, assert returned instant `> now` (currently fails).
- [x] 4.2 Fix `next_after` so the returned instant is strictly greater than `now`. Use `Schedule::after(now).next()` semantics, advancing past the current matching slot when it equals `now`.
- [x] 4.3 Add a property test driving 365 daily `now`s through every shipped schedule (`0 3 * * *`, `0 4 * * *`, `30 4 * * 1`, `0 5 * * *`, `30 5 * * *`, `45 5 * * *`, `0 6 * * 0`, `0 7 * * 0`, `0 2 * * *`).
- [x] 4.4 Re-verify: query DB, `retention.turn_digest.next_run_at` advances to a future Sunday after one tick.

## 5. cortex_consolidations index — verification that lazy-create path holds
- [x] 5.1 Confirm `cortex_consolidations` is a Meili index (NOT a SQLite table). `crates/cortex-cli/src/bin/cortex-ops.rs::fetch_all_consolidations` already returns `Ok(vec![])` on `NOT_FOUND`. No schema migration needed at the cortex-storage layer.
- [x] 5.2 Confirm that consolidator writes to the index lazily (Meili creates indexes on first insert). The cron `retention.consolidator_nightly` runs `cortex-consolidator nightly` which performs the writes.
- [x] 5.3 Idempotent verified: empty Meili → prune sees NOT_FOUND → returns no-op success.
- [x] 5.4 No SQLite migration test required — the storage layer never owned this index.

## 6. retention_sweeps bookkeeping for every sweep
- [x] 6.1 Audit every `cortex-ops <sweep>` handler in `cortex-cli`: `tier-sweep`, `parquet-rollup`, `cas-vacuum`, `pii-enforce`, `turn-digest`, `meili-prune`, `metadata-reap`, `consolidation-prune`. Confirm each calls `start_retention_sweep` and `finish_retention_sweep`.
- [x] 6.2 For handlers that don't, wrap the body in the start/finish bracket and write the per-stage counters into `tier_transitions_json`.
- [x] 6.3 Add `tests/retention_sweeps_bookkeeping_it.rs`: invoke each sweep against the in-memory backend, assert `SELECT count(*) FROM retention_sweeps WHERE status = 'success' = N`.
- [x] 6.4 Update `docs/specs/02-quantization.md` § "Sweep bookkeeping": every sweep MUST emit one `retention_sweeps` row per invocation.

## 7. Cleanup of immediate workaround
- [x] 7.1 Remove the manual DB UPDATEs applied via `scripts/backup/metadata.sqlite.bak.real.20260505T030243Z` recovery — once §3 ships, the reconciler restores the same state on the next boot.
- [x] 7.2 Capture the diagnostic session as a learning entry: `rulebook_learn_capture` with title `Retention daemon: 6 independent gaps surfaced as one "tudo never" dashboard`.

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 8.1 Update `docs/specs/19-retention.md`, `docs/specs/02-quantization.md`, and `CHANGELOG.md` covering all six fixes with file refs and commit hashes.
- [x] 8.2 Tests for §1.4, §2.2, §3.3, §3.4, §4.1, §4.3, §5.4, §6.3 land green.
- [x] 8.3 `cargo check && cargo clippy -- -D warnings && cargo test --workspace` clean before archive.

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 9.1 Update or create documentation covering the implementation — closed by §8.1 (CHANGELOG `[Unreleased]` § Added phase11v entry, `docs/specs/19-retention.md` § Sweep environment, `docs/specs/02-quantization.md` § Sweep bookkeeping), §1.5 (`gui/src/views/Retention.tsx`), §2.3 (env precedence note), §6.4 (sweep bookkeeping invariant).
- [x] 9.2 Write tests covering the new behavior — closed by §8.2 (17 new tests across §1.4 dashboard live-read + disabled-pill + failure-streak; §2.2 env precedence × 8; §3.3-§3.4 UPSERT drift reconcile + operator-disabled preservation; §4.1 strict-advance + §4.3 365-day property; §5.4 idempotent NOT_FOUND no-op; §6.3 retention_sweeps bookkeeping per sweep).
- [x] 9.3 Run tests and confirm they pass — closed by §8.3 (`cargo check --workspace` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo test --workspace` 17 new + every pre-existing retention test green).
