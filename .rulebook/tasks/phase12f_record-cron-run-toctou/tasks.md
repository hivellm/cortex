## 1. Atomic upsert
- [ ] 1.1 Rewrite `record_cron_run` as a single UPDATE statement using `CASE` for `failure_streak` (`+ 1` on failure, `0` on success).
- [ ] 1.2 Wrap the update in `BEGIN IMMEDIATE ... COMMIT` so concurrent writers serialise.
- [ ] 1.3 Bench the new path against the prior path on 1k sequential calls; budget +10% latency max.

## 2. Concurrency regression test
- [ ] 2.1 New test `metadata::tests::record_cron_run_concurrent` spawning 4 threads × 250 calls each on the same job.
- [ ] 2.2 Post-condition: `last_status` matches the last write that committed; `failure_streak` matches the count of trailing failures only; total writes == 1000.
- [ ] 2.3 Run the test under `cargo test --release` to expose timing-dependent failures.

## 3. Tail (mandatory)
- [ ] 3.1 Update `docs/specs/19-retention.md` § Cron supervisor + `CHANGELOG.md` Fixed.
- [ ] 3.2 Tests: §2.1 + §1.3 micro-bench guard.
- [ ] 3.3 `cargo check --workspace && cargo clippy -p cortex-storage -- -D warnings && cargo test -p cortex-storage --release` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
