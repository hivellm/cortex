## 1. Shared build-time version emitter
- [x] 1.1 Create `crates/cortex-build/` as a library crate (path dep, no transitive deps)
- [x] 1.2 Implement `cortex_build::emit_version_env()` to be called from each crate's `build.rs`; uses `std::process::Command` to run `git rev-parse HEAD`, `git status --porcelain`, and a self-contained UTC RFC-3339 formatter (chrono-free)
- [x] 1.3 Emit `cargo:rustc-env=CORTEX_GIT_SHA=...`, `CORTEX_GIT_SHA_SHORT`, `CORTEX_BUILD_TS`, `CORTEX_GIT_DIRTY`, `CORTEX_BUILD_PROFILE`
- [x] 1.4 Provide a `version_info!()` macro + `from_compile_env(crate_version)` helper returning a `VersionInfo` struct
- [x] 1.5 Unit tests for `version_info()` shape, unknown-sentinel fallback, RFC-3339 formatter, serde round-trip (5 tests in cortex-build)

## 2. Wire build.rs into every Cortex crate
- [x] 2.1 cortex-api/build.rs
- [x] 2.2 cortex-adapter-claude-code/build.rs
- [x] 2.3 cortex-ingestion/build.rs
- [x] 2.4 cortex-workers/build.rs (covers classifier + embedder + fulltext + graph + backfill bins after the phase-1 worker consolidation)
- [x] 2.5 cortex-workers/build.rs covers cortex-embedder-worker
- [x] 2.6 cortex-workers/build.rs covers cortex-fulltext-worker
- [x] 2.7 cortex-workers/build.rs covers cortex-graph-worker
- [x] 2.8 cortex-mcp-server/build.rs
- [x] 2.9 cortex-cli/build.rs covers cortex-bootstrap (after the bootstrap consolidation)

## 3. /healthz exposes version
- [x] 3.1 Update each crate's /healthz handler to include `extras.version = version_info()` — adapter, ingestion router, cortex-api self-handler, classifier/embedder/fulltext/graph worker bins
- [x] 3.2 Confirmed via `cargo test -p cortex-api --test health_freshness` (asserts the version block fields are present in /healthz extras as surfaced through the freshness aggregator)

## 4. cortex-api version aggregator
- [x] 4.1 Versions handler lives in `crates/cortex-api/src/health.rs` (alongside freshness/divergence) — no separate file because the handler shares `gather_subsystem_extras` with the other phase8b aggregators
- [x] 4.2 `GET /v1/health/versions` fans out via the same probe target list as `/v1/health` and collects `extras.version`
- [x] 4.3 `head_sha` resolved once at boot via `HeadSha::resolve_now` and cached in `HealthState`; degrades to `"unknown"` when git is unavailable
- [x] 4.4 `behind_by_commits` runs `git rev-list <running_sha>..HEAD --count` per drifted binary; returns `None` + `note` on unreachable SHAs
- [x] 4.5 Response shape: `{ head_sha, head_sha_short, running_binaries[], drift[], all_in_sync }` — `drift` only populated when `head_sha != "unknown"` and per-binary SHA differs
- [x] 4.6 Route mounted in `crates/cortex-api/src/http.rs::build_router_with` alongside `/v1/health/freshness` and `/v1/health/divergence`

## 5. CLI script
- [x] 5.1 NEW `scripts/doctor-versions.bat` — curls `/v1/health/versions`, prints endpoint + `all_in_sync` + raw report
- [x] 5.2 Exit codes: 0 when `all_in_sync == true`, 1 when `false`, 2 when aggregator unreachable
- [x] 5.3 Companion `scripts/doctor-versions.sh` — bash + awk parser, no `jq` dependency

## 6. CI gate
- [x] 6.1 NEW `.github/workflows/version-coherence.yml`
- [x] 6.2 Workflow runs on PR; defensively checks any committed `target/release/<binary>` artifacts against the most-recent source mtime in the owning crate (the project doesn't normally commit `target/`, so the gate skips silently when absent)
- [x] 6.3 Error includes both the stale binary path and the newer source file path

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/architecture.md §13.7 Observability — version coherence (phase8c)` + new `crates/cortex-build/README.md` (wire-it-into-a-new-crate guide + embedded-fields table) + `docs/metrics.md` updated with `/v1/health/versions` row + version-block field table + CHANGELOG entry under `### Added → Observability — version coherence (phase8c)`
- [x] 7.2 Write tests covering the new behavior — cortex-build unit tests (5: macro shape, unknown-sentinel fallback, RFC-3339 formatter, serde round-trip), cortex-api unit tests (6 new in `health.rs`: version_row matches/drift/unknown/non-object inputs, behind_by_commits zero/unknown), integration test in `crates/cortex-api/tests/health_freshness.rs::versions_endpoint_carries_self_row_with_compile_baked_sha` (asserts the self-row + documented top-level fields)
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-build (5 new), cortex-api (lib + integration), and every other touched crate
