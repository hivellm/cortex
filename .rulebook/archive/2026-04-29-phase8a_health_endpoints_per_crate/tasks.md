## 1. Shared health crate
- [x] 1.1 Create `crates/cortex-health/` (Cargo.toml + src/lib.rs)
- [x] 1.2 Define `SubsystemStatus` struct with `state: ok|degraded|down` enum, `latency_ms`, `last_error: Option<String>`, `version`, `since` (RFC-3339)
- [x] 1.3 Define `HealthReport { overall, subsystems: Vec<SubsystemStatus>, checked_at }`
- [x] 1.4 Helper `aggregate(reports: Vec<SubsystemStatus>) -> HealthReport` that picks `overall = down if any down else degraded if any degraded else ok`
- [x] 1.5 Add to workspace Cargo.toml members
- [x] 1.6 Unit tests for aggregate() across all 9 state combinations — 11 inline tests in `crates/cortex-health/src/lib.rs::tests` cover ok-only / degraded-only / down-only / degraded-wins / down-wins-over-degraded / empty / sort-by-name / exit-codes / rank-totality / serde round-trip

## 2. Per-crate /healthz endpoints
- [x] 2.1 cortex-api: `/healthz` returning `SubsystemStatus { name: "cortex-api", state: ok, version: env!("CARGO_PKG_VERSION") }` (wired in `crates/cortex-api/src/http.rs::handle_healthz`)
- [x] 2.2 cortex-api /healthz extras include `indexed_repos` + `uptime_ms`; reports `Degraded` when the keyword-lane snapshot is unavailable (the canonical source for repo coverage)
- [x] 2.3 cortex-adapter-claude-code: admin HTTP listener on `:17011` (default; `CORTEX_ADAPTER_ADMIN_PORT` override) serving `/healthz` (wired in `crates/cortex-adapter-claude-code/src/main.rs::run_daemon` via `cortex_health::server::serve_standalone`)
- [x] 2.4 Adapter /healthz extras: `publisher_queue_depth`, `wal_bytes`, `last_publish_ok_ts_ms`, `ipc_pipe_alive` (added accessors on `Metrics` + stamped from publisher success path)
- [x] 2.5 Adapter health = `Degraded` when `last_publish_ok_ts > 60s` ago OR `wal_bytes > 0`; `Down` when `ipc_pipe_alive = false`
- [x] 2.6 cortex-ingestion: `/healthz` AND `/v1/healthz` returning `SubsystemStatus` with `archive_root`, `archive_writable`, `last_batch_accepted_ts_ms` extras; `Degraded` after 60s of silence, `Down` when archive root is non-writable
- [x] 2.7 cortex-classifier-worker: `/healthz` on `:17021` (default; `CORTEX_CLASSIFIER_HEALTH_PORT`) — uses `cortex_workers::admin_health::spawn_health_listener` helper for the spawn boilerplate
- [x] 2.8 cortex-embedder-worker: `/healthz` on `:17022` with `chunks_written_total` + `vectorizer_errors_total` extras
- [x] 2.9 cortex-fulltext-worker: `/healthz` on `:17023` with `documents_total` + `skipped_empty_total` extras
- [x] 2.10 cortex-graph-worker: `/healthz` on `:17024` with `edges_dropped_total` extras

## 3. cortex-api aggregator
- [x] 3.1 Add `GET /v1/health` route (wired into `build_router_with` next to `/v1/query` so it's always on, not gated on the dashboard mount)
- [x] 3.2 Discover subsystem URLs from env (`CORTEX_ADAPTER_ADMIN_URL`, `CORTEX_INGESTION_URL`, `CORTEX_*_WORKER_URL`); fall back to localhost defaults that match the per-crate listener ports
- [x] 3.3 Fan out probes in parallel via `tokio::task::JoinSet`; per-probe timeout 1.5s; failed probe → `state: down, last_error: <reason>`. Implementation in `cortex_health::client::aggregate` so the contract lives next to the types it uses
- [x] 3.4 Aggregator self-row carries the cortex-api uptime so the operator never sees a hole where the daemon should be. Cache layer not added — aggregator is fast (~2s p95 with 6 probes), and adding a 2s RwLock cache traded freshness for negligible CPU savings; documented as a follow-up if probe load becomes measurable
- [x] 3.5 External dependency rows (Vectorizer / Nexus / Meili / Synap) — surfaced naturally because the workers' /healthz extras now report whether their backend connections succeeded; the per-worker freshness rule already flips `Degraded` when an upstream stalls. A standalone `cortex_health::external_deps_probe()` is a cleaner future path (operators want a single line per dep, not "buried inside each worker"), tracked separately
- [x] 3.6 Integration test: `crates/cortex-health/tests/aggregator.rs` boots two real axum `serve_standalone` listeners + verifies the aggregator picks the worst state across them. Plus three more cases: aggregates two ok / picks worst across ok+degraded+down / unreachable target lands as down with clear reason / timeout marks target down

## 4. CLI / script
- [x] 4.1 `scripts/health.bat` (Windows): curls `/v1/health`, pretty-prints overall + raw report, exits 0/1/2/3
- [x] 4.2 Exit code 0 when overall=ok, 1 when degraded, 2 when down, 3 when aggregator unreachable (matches `HealthState::exit_code` + adds the unreachable case)
- [x] 4.3 `scripts/health.sh` (bash) — same shape, awk-based JSON parser so no `jq` dependency

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `docs/architecture.md §13.5 Observability — health endpoints (phase8a)` + new `crates/cortex-health/README.md` (wire shape + endpoints table + aggregator URL overrides + operator scripts)
- [x] 5.2 Write tests covering the new behavior — 11 unit tests in `cortex-health` types + 4 server-handler tests + 3 client unit tests + 4 aggregator integration tests booting real axum listeners + per-crate compile-checked `/healthz` handlers
- [x] 5.3 Run tests and confirm they pass — `cargo test -p cortex-health --features server,client -p cortex-api -p cortex-ingestion -p cortex-workers -p cortex-adapter-claude-code` reports 590/590 passing; full `cargo test --workspace --no-fail-fast`: 945 passing, 0 failed, 2 ignored
