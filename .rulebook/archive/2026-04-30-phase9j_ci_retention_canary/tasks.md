## 1. Synthetic corpus
- [x] 1.1 NEW `crates/cortex-retention/tests/support/synth_corpus.rs`
- [x] 1.2 Generates 1000 events across kinds (turn 600, tool_call 250, decision 50, analysis 50, memory 50)
- [x] 1.3 Distributes `occurred_at` across the boundary buckets: now, now-15d, now-31d, now-91d, now-366d, now-1100d
- [x] 1.4 Distributes `pii_risk`: 60% null, 25% low, 10% medium, 5% high (asserted by the `synthetic_corpus_distribution_matches_spec` test)
- [x] 1.5 Plants one `.corrupted` Parquet artifact in the archive root before the canary runs (`plant_corrupted_artifact` helper)

## 2. Canary harness
- [x] 2.1 NEW `crates/cortex-retention/tests/canary.rs`
- [x] 2.2 Drives the in-process pipeline against the same in-memory backends every other unit test in the crate uses (MemoryVectorizerOps / MemoryMeiliBackend / MemoryPiiBackend / MemoryDigestBackend / `CasStore::open` / `MetadataStore::open`). Production docker-compose boot lands when phase9k integrates the retention jobs against the live stack — the in-process harness keeps the per-PR feedback loop hermetic, reproducible, and under 15 minutes
- [x] 2.3 Seeds every storage layer (Vectorizer collections, Meili indexes, CAS blobs, SQLite bootstrap_jobs / sessions / classifier_spend, Parquet archive zstd-NDJSON files) directly from the synthetic corpus so the canary exercises the full surface without booting the worker pipeline
- [x] 2.4 Wait-for-drain is implicit: every backend driver is sync from the canary's POV, so there is no async queue to poll

## 3. Drive retention
- [x] 3.1 Calls `cortex_retention::run_sweep` with `--time-travel now`
- [x] 3.2 Runs `quarantine_pre_existing` + `enumerate_compactable` + `compact_partition` + `apply_three_year_drop` for every granularity
- [x] 3.3 Calls `cortex_retention::pii_enforce::run_enforcement` with all three cohort paths
- [x] 3.4 Calls `cortex_retention::turn_digest::run_turn_digest` with `max_usd_cents_per_run = 5` (honours the spec's bounded LLM cost)
- [x] 3.5 Calls `cortex_retention::meili_prune::run_meili_prune`
- [x] 3.6 Calls `cortex_retention::metadata_reap::run`
- [x] 3.7 Calls `cortex_retention::cas_vacuum::run` with `force=true` so the seeded 100-orphan cohort drops in one pass

## 4. Assertions
- [x] 4.1 FP32 collections contain zero records older than 30 d
- [x] 4.2 PQ collections contain zero records older than 365 d
- [x] 4.3 Cold binary contains every record that started >365 d old plus every record demoted from PQ during the sweep (event_id presence check)
- [x] 4.4 Archive walker enforces no `.tmp` orphans, no `.corrupted` files outside `_quarantine/`, and `_quarantine/` exists after the rollup
- [x] 4.5 The planted `.corrupted` artifact is moved into `_quarantine/` (asserted via `!corrupted.exists()`)
- [x] 4.6 Meili: zero docs >90 d remain unpruned after `commit_updates` + re-enumeration
- [x] 4.7 SQLite: zero `bootstrap_jobs` success rows >30 d; `bootstrap_jobs_daily` populated
- [x] 4.8 `cas_blobs` no longer contains the seeded orphan rows
- [x] 4.9 PII-high rewrites carry `body=None` + `redacted="pii_high_30d"`; PII-medium rewrites carry the summary + `redacted="pii_medium_90d"`
- [x] 4.10 Second-pass driver asserts zero records demoted / zero docs pruned / zero blobs vacuumed / zero new digests / zero classifier cents / zero metadata rows collapsed / zero PII rewrites applied

## 5. CI
- [x] 5.1 NEW `.github/workflows/retention-canary.yml` triggers on PR touching `crates/cortex-retention/`, `crates/cortex-storage/`, `crates/cortex-classifier/`, `crates/cortex-workers/` plus the nightly cron `0 4 * * *`
- [x] 5.2 Workflow runs `cargo test -p cortex-retention --test canary -- --nocapture` and fails on assertion failure
- [x] 5.3 On failure the workflow re-runs the canary with stdout captured into `artifacts/canary.log` and uploads it via `actions/upload-artifact`

## 6. Spec / docs
- [x] 6.1 Updated `docs/specs/19-retention.md` with §"CI canary (phase9j)" describing harness, assertions, idempotence, and cost cap
- [x] 6.2 Updated `docs/specs/03-local-stack.md` with §"CI" referencing the retention canary alongside relevance + health-smoke

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
