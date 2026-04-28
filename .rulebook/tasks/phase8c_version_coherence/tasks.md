## 1. Shared build-time version emitter
- [ ] 1.1 Create `crates/cortex-build/` as a library crate (path dep, no transitive deps)
- [ ] 1.2 Implement `cortex_build::emit_version_env()` to be called from each crate's `build.rs`; uses `std::process::Command` to run `git rev-parse HEAD`, `git status --porcelain`, and `chrono::Utc::now()`
- [ ] 1.3 Emit `cargo:rustc-env=CORTEX_GIT_SHA=...`, `CORTEX_GIT_SHA_SHORT`, `CORTEX_BUILD_TS`, `CORTEX_GIT_DIRTY`, `CORTEX_BUILD_PROFILE`
- [ ] 1.4 Provide a `version_info()` runtime helper returning a `VersionInfo` struct
- [ ] 1.5 Unit test for `version_info()` shape (uses `env!()` macros)

## 2. Wire build.rs into every Cortex crate
- [ ] 2.1 cortex-api/build.rs
- [ ] 2.2 cortex-adapter-claude-code/build.rs
- [ ] 2.3 cortex-ingestion/build.rs
- [ ] 2.4 cortex-classifier-worker/build.rs
- [ ] 2.5 cortex-embedder-worker/build.rs
- [ ] 2.6 cortex-fulltext-worker/build.rs
- [ ] 2.7 cortex-graph-worker/build.rs
- [ ] 2.8 cortex-mcp-server/build.rs
- [ ] 2.9 cortex-bootstrap/build.rs

## 3. /healthz exposes version
- [ ] 3.1 Update each crate's /healthz handler to include `extras.version = version_info()`
- [ ] 3.2 Confirm via `curl /healthz | jq .extras.version` for each binary

## 4. cortex-api version aggregator
- [ ] 4.1 NEW `crates/cortex-api/src/health/versions.rs`
- [ ] 4.2 Handler `GET /v1/health/versions` fans out to each subsystem and collects `extras.version`
- [ ] 4.3 Compute `head_sha` by running `git rev-parse HEAD` once at boot (cached); refresh on file watcher trigger of `.git/HEAD`
- [ ] 4.4 Compute `drift` rows: for each binary with `running_sha != head_sha`, run `git rev-list running_sha..HEAD --count` to compute `behind_by_commits`
- [ ] 4.5 Return shape: `{ head_sha, running_binaries: [...], drift: [...] }`
- [ ] 4.6 Wire route in `dashboard.rs`

## 5. CLI script
- [ ] 5.1 NEW `scripts/doctor-versions.bat` that curls `/v1/health/versions` and prints a table
- [ ] 5.2 Exit non-zero (1) if any binary is behind workspace HEAD
- [ ] 5.3 Companion `scripts/doctor-versions.sh`

## 6. CI gate
- [ ] 6.1 NEW `.github/workflows/version-coherence.yml`
- [ ] 6.2 Workflow runs on PR; checks out HEAD, builds all binaries, then asserts `target/release/<binary>.exe` mtime ≥ mtime of every modified `crates/<crate-of-binary>/src/*.rs` file
- [ ] 6.3 Helpful error message including the binary that's stale and the source file that's newer

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update `docs/architecture.md` with the version-coherence section + CHANGELOG entries on every touched crate + `crates/cortex-build/README.md`
- [ ] 7.2 Tests: build.rs works without git (fallback to "unknown"); `/healthz extras.version` populated; `/v1/health/versions` correctly computes drift
- [ ] 7.3 Run `cargo test --workspace` and confirm all pass
