## 1. Classifier-worker self-healing watchdog

- [ ] 1.1 Audit `crates/cortex-classifier-worker/src/main.rs` (or `crates/cortex-workers/src/classifier/worker.rs` — locate during §1) for the consume loop. Confirm whether `synap consume` errors are caught + retried in-process, exit the process, or get silently swallowed
- [ ] 1.2 Wrap the consume loop in an outer supervisor: count consecutive consume errors; on N (default 5, env `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS`) consecutive failures, `process::exit(1)` so docker `restart: unless-stopped` recovers the container
- [ ] 1.3 Stamp `last_consume_ts` (epoch ms) on every successful `consume` return; expose via the existing `/healthz` extras block (`{"last_consume_ts": 1714200000000, "consume_errors_consecutive": 0}`)
- [ ] 1.4 New `cortex-ops doctor` check: when the classifier `/healthz` `last_consume_ts` is older than `--classifier-staleness-ms` (default 60_000), report `degraded: classifier-worker stuck`. Wire into the existing doctor table output
- [ ] 1.5 Unit test the supervisor logic against a mock consumer that returns `Err` on every call; assert `process::exit(1)` is invoked on the configured threshold (use a test seam — cannot actually exit during unit test)

## 2. Graph-worker durable consumer offset

- [ ] 2.1 Audit `crates/cortex-workers/src/graph/worker.rs` for the offset persistence path. Confirm the comment "Synap 0.11 has no durable consumer-group surface in the SDK" against the current `synap-sdk` version in `Cargo.lock` (live stack runs Synap server 0.12.0 per `/health`)
- [ ] 2.2 If the SDK exposes durable consumer groups, wire `cortex-graph-worker` to use them with key `cortex-graph-{instance_id}`. On boot, resume from the persisted offset; on every successful batch flush, advance the offset
- [ ] 2.3 If the SDK does NOT yet expose durable groups, fall back to: persist `last_event_id` to the `metadata.sqlite` `consumer_offsets` table (new). On boot, replay the stream from `event_id > $last`. The replay path uses the existing `consume(room, Some(offset), Some(max))` API
- [ ] 2.4 New `cortex-ops graph replay --since=<event_id>` subcommand that forces a manual replay window: useful for the operator runbook in §5 when a known event range was lost
- [ ] 2.5 Integration test: bring up Synap + Nexus via testcontainers; publish 1000 envelopes; restart the graph-worker mid-flow; assert post-recovery node count matches the published event count

## 3. Embedder Vectorizer JWT refresh

- [ ] 3.1 Audit `crates/cortex-workers/src/embedder/vectorizer_client.rs` (or wherever the live Vectorizer client lives) for the auth flow. Confirm whether the JWT is fetched once at boot and never refreshed, or whether refresh exists but is broken
- [ ] 3.2 Add a token-cache wrapper that tracks `expires_in` (Vectorizer returns 3600 s); refresh the JWT 60 s before expiry; fall back to a re-login on any 401 response
- [ ] 3.3 Regression test: mock-Vectorizer returns 401 on the first request post-token-expiry; assert the embedder retries with a fresh login + replays the original request; assert the envelope is NOT counted as dropped
- [ ] 3.4 Add `last_login_ts`, `jwt_refresh_total`, `jwt_refresh_errors_total` to the embedder `/healthz` extras block; expose as Prometheus metrics
- [ ] 3.5 Operator verification: post-deploy, run `cortex-ops doctor` and confirm embedder reports a fresh `last_login_ts`; rerun a failed envelope batch and confirm it lands in Vectorizer

## 4. Drain-recovery integration test

- [ ] 4.1 New `crates/cortex-workers/tests/drain_recovery_it.rs` gated behind `CORTEX_DRAIN_RECOVERY_IT=1`
- [ ] 4.2 Spin up Synap + Vectorizer + Nexus via testcontainers (or rely on `docker-compose.test.yml` started externally); seed a fixture repo; bootstrap it; kill+restart each worker mid-flow; assert final per-backend counts match the bootstrap event count

## 5. Backlog drain runbook

- [ ] 5.1 New `docs/cortex/pipeline-drainage-runbook.md` documenting the operator playbook: detect dormant workers via `cortex-ops doctor`, restart, replay missing windows via `cortex-ops graph replay`, re-bootstrap repos where Meili / Nexus / Vectorizer counts diverge by > 10 %
- [ ] 5.2 Add a verification script `scripts/check-pipeline-coverage.sh` that prints per-repo Meili vs Vectorizer vs Nexus doc counts in a single table; flags any repo whose Nexus or Vectorizer count is < 50 % of the Meili count

## 6. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 6.1 Update or create documentation covering the implementation — `docs/cortex/pipeline-drainage-runbook.md` (new); CHANGELOG entry under `[Unreleased]` Operations summarising the watchdog + durable-offset + JWT-refresh trio
- [ ] 6.2 Write tests covering the new behavior — §1.5 supervisor exit-on-threshold, §2.5 graph-worker restart drainage, §3.3 JWT refresh regression, §4.2 drain-recovery IT (gated). Coverage ≥ 95 % on the new wrapper modules
- [ ] 6.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings` (touched files), `cargo fmt --check`, `cargo test -p cortex-workers`. All green before archive
- [ ] 6.4 Capture learning: `rulebook_learn_capture` for the "process-alive `/healthz` is not a liveness probe" pattern (the classifier was technically alive for 26 hours while its consume loop was dead; the watchdog must observe FORWARD progress, not just process state)
- [ ] 6.5 Capture decision: `rulebook_decision_create` for the durable-offset strategy (Synap 0.12 native vs SQLite-backed fallback)
