## 1. Shared health crate
- [ ] 1.1 Create `crates/cortex-health/` (Cargo.toml + src/lib.rs)
- [ ] 1.2 Define `SubsystemStatus` struct with `state: ok|degraded|down` enum, `latency_ms`, `last_error: Option<String>`, `version`, `since` (RFC-3339)
- [ ] 1.3 Define `HealthReport { overall, subsystems: Vec<SubsystemStatus>, checked_at }`
- [ ] 1.4 Helper `aggregate(reports: Vec<SubsystemStatus>) -> HealthReport` that picks `overall = down if any down else degraded if any degraded else ok`
- [ ] 1.5 Add to workspace Cargo.toml members
- [ ] 1.6 Unit tests for aggregate() across all 9 state combinations

## 2. Per-crate /healthz endpoints
- [ ] 2.1 cortex-api: add `/healthz` returning `SubsystemStatus { name: "cortex-api", state: ok, version: env!("CARGO_PKG_VERSION") }`
- [ ] 2.2 cortex-api /healthz also reports `extras.last_archive_loader_refresh` and `extras.last_meili_loader_refresh` (timestamps from last successful refresh; `degraded` if older than 60s)
- [ ] 2.3 cortex-adapter-claude-code: add admin HTTP listener (default `:17011`, `CORTEX_ADAPTER_ADMIN_PORT` env override) serving `/healthz`
- [ ] 2.4 Adapter /healthz extras: `publisher_queue_depth`, `wal_bytes`, `last_publish_ok_ts`, `ipc_pipe_alive`
- [ ] 2.5 Adapter health = `degraded` when last_publish_ok_ts > 60s ago OR wal_bytes > 0; `down` when ipc_pipe_alive = false
- [ ] 2.6 cortex-ingestion: add `/v1/healthz` returning archive_root writable, synap room registered, last_batch_accepted_ts
- [ ] 2.7 cortex-classifier-worker: add `/healthz` returning last_claimed_job_ts, queue_lag (jobs)
- [ ] 2.8 cortex-embedder-worker: add `/healthz`
- [ ] 2.9 cortex-fulltext-worker: add `/healthz`
- [ ] 2.10 cortex-graph-worker: add `/healthz`

## 3. cortex-api aggregator
- [ ] 3.1 Add `GET /v1/health` route to `crates/cortex-api/src/dashboard.rs`
- [ ] 3.2 Discover subsystem URLs from env (`CORTEX_ADAPTER_ADMIN_URL`, `CORTEX_INGESTION_URL`, `CORTEX_*_WORKER_URL`); fall back to localhost defaults
- [ ] 3.3 Fan out probes in parallel using `tokio::join!`; per-probe timeout 1.5s; failed probe → `state: down, last_error: <reason>`
- [ ] 3.4 Cache the assembled `HealthReport` for 2s in a `tokio::sync::RwLock` so polling doesn't hammer downstream
- [ ] 3.5 Include external dependencies: Vectorizer, Nexus, Meili, Synap (each gets a `SubsystemStatus`)
- [ ] 3.6 Integration test: spin up two fake `/healthz` HTTP servers + verify aggregator returns correct `overall`

## 4. CLI / script
- [ ] 4.1 NEW `scripts/health.bat` (Windows) that curls `/v1/health` and pretty-prints a table
- [ ] 4.2 Exit code 0 when overall=ok, 1 when degraded, 2 when down
- [ ] 4.3 Companion `scripts/health.sh` for bash users

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update `docs/architecture.md` with the health architecture; add `crates/cortex-health/README.md` and CHANGELOG entries on each touched crate
- [ ] 5.2 Tests: aggregate() unit tests + per-crate /healthz integration tests + aggregator integration test (≥95% line coverage on new code)
- [ ] 5.3 Run `cargo test -p cortex-health -p cortex-api` and confirm all pass
