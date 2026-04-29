## 1. Synthetic corpus
- [ ] 1.1 NEW `crates/cortex-retention/tests/support/synth_corpus.rs`
- [ ] 1.2 Generate 1000 events across kinds (turn 600, tool_call 250, decision 50, analysis 50, memory 50)
- [ ] 1.3 Distribute `occurred_at` across boundary buckets: now, now-15d, now-31d, now-91d, now-366d, now-1100d
- [ ] 1.4 Distribute `pii_risk`: 60% null, 25% low, 10% medium, 5% high
- [ ] 1.5 Plant one `.corrupted` Parquet file in the archive root before the canary runs

## 2. Canary harness
- [ ] 2.1 NEW `crates/cortex-retention/tests/canary.rs`
- [ ] 2.2 Reuses `cortex-bootstrap` test runner to boot docker-compose against ephemeral ports
- [ ] 2.3 Ingest the corpus through `cortex-ingestion` so the full pipeline (classifier-worker, embedder, fulltext, graph) populates every store
- [ ] 2.4 Wait for the embedder + classifier queues to drain (assertion against `/healthz` extras)

## 3. Drive retention
- [ ] 3.1 Run `cortex-retention sweep --time-travel now+1s`
- [ ] 3.2 Run `cortex-retention rollup --time-travel now+1s --granularity all`
- [ ] 3.3 Run `cortex-retention pii-enforce --time-travel now+1s`
- [ ] 3.4 Run `cortex-retention turn-digest --time-travel now+1s --max-usd-cents-per-run 5`
- [ ] 3.5 Run `cortex-retention meili-prune --time-travel now+1s`
- [ ] 3.6 Run `cortex-retention metadata-reap --time-travel now+1s`
- [ ] 3.7 Run `cortex-retention cas-vacuum --time-travel now+1s --force`

## 4. Assertions
- [ ] 4.1 FP32 collections contain zero records older than 30 d
- [ ] 4.2 PQ collections contain zero records older than 365 d
- [ ] 4.3 Cold binary contains every >365 d record (count matches expectation)
- [ ] 4.4 Archive: no hourly directories older than 90 d, daily files exist for the 30–365 d band, monthly files for the 365 d–3 y band
- [ ] 4.5 No `*.tmp` orphans, no `*.corrupted*` outside `_quarantine/`
- [ ] 4.6 Meili: zero docs >90 d with non-empty `body`
- [ ] 4.7 SQLite: zero `bootstrap_jobs` success rows >30 d; `bootstrap_jobs_daily` populated
- [ ] 4.8 `cas_blobs` no longer contains orphan rows
- [ ] 4.9 PII-high: `body=null`, `redacted="pii_high_30d"`; PII-medium: summary present, `redacted="pii_medium_90d"`
- [ ] 4.10 Idempotence: re-run every subcommand and assert zero deletions / zero deletes / zero classifier calls

## 5. CI
- [ ] 5.1 NEW `.github/workflows/retention-canary.yml` triggering on PR touching the listed crates plus nightly cron `0 4 * * *`
- [ ] 5.2 Workflow boots docker-compose, runs `cargo test -p cortex-retention --test canary`, fails on assertion failure
- [ ] 5.3 Uploads the final SQLite + Parquet archive as artifacts on failure

## 6. Spec / docs
- [ ] 6.1 Update `docs/specs/19-retention.md` §Tests with the canary contract
- [ ] 6.2 Reference from `docs/specs/03-local-stack.md` §CI

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
