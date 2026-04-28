## 1. CI stub crate
- [ ] 1.1 NEW `crates/cortex-ci-stubs/` (workspace member, dev-only feature)
- [ ] 1.2 Tiny HTTP servers stubbing Vectorizer (`/auth/login`, `/v1/health`), Nexus (`/v1/health`), Meili (`/health`, `/indexes`), Synap (`/v1/rooms`, `/v1/publish`)
- [ ] 1.3 Each stub binds a configurable port and returns minimal happy-path responses sufficient for cortex-api boot
- [ ] 1.4 Single binary `cortex-ci-stubs serve --vectorizer-port=N --nexus-port=N ...` boots all stubs in one process

## 2. Stack boot/teardown helpers
- [ ] 2.1 NEW `scripts/ci/boot-stack.sh` (Linux) and `boot-stack.bat` (Windows)
- [ ] 2.2 Helpers spawn cortex-ci-stubs, cortex-ingestion, cortex-api, cortex-adapter-claude-code in background; export PIDs to `$CORTEX_PIDS_FILE`
- [ ] 2.3 Each helper accepts `CORTEX_HOME` env var so concurrent CI runs use isolated archive/wal/state dirs
- [ ] 2.4 Helpers wait for `/v1/health` to report `overall: ok` (60 s timeout); exit non-zero on timeout
- [ ] 2.5 NEW `scripts/ci/teardown-stack.sh` and `.bat` that read `$CORTEX_PIDS_FILE` and kill each pid

## 3. health-smoke GitHub Actions workflow
- [ ] 3.1 NEW `.github/workflows/health-smoke.yml`
- [ ] 3.2 Triggers: `pull_request`, `push: main`
- [ ] 3.3 Matrix: `[windows-latest, ubuntu-latest]`
- [ ] 3.4 Steps: checkout → install Rust toolchain → cache target/ → `cargo build --release --workspace` → boot stack → run `cortex-doctor canary --hook=PostToolUse` → `cortex-doctor health` → `cortex-doctor config` → `cortex-doctor versions` → teardown stack
- [ ] 3.5 Use `actions/upload-artifact@v4` to capture cortex-api + adapter logs on failure for postmortem
- [ ] 3.6 Total wall-clock budget: 6 minutes per matrix entry

## 4. version-coherence GitHub Actions workflow
- [ ] 4.1 NEW `.github/workflows/version-coherence.yml`
- [ ] 4.2 Trigger: `pull_request`
- [ ] 4.3 Compares mtime of every committed `target/release/*.exe` against mtime of touched `crates/<x>/src/*.rs`
- [ ] 4.4 Fails with clear error message naming the stale binary and the newer source file
- [ ] 4.5 Note: this workflow exists for completeness even though Cortex does not commit binaries; if binaries are not under git control the job exits 0 with "no committed binaries to check"

## 5. PR template
- [ ] 5.1 Update `.github/PULL_REQUEST_TEMPLATE.md` adding a "Health checks" section with checkboxes for `scripts/health.bat` and `scripts/canary.bat` outcomes
- [ ] 5.2 Soft signal — not enforced by CI (the automated gate is enough); the checkbox raises author awareness

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update `docs/architecture.md` (CI section) and `docs/runbooks/ci-failures.md` explaining how to diagnose health-smoke failures; CHANGELOG entry on cortex-ci-stubs and root README
- [ ] 6.2 Tests: cortex-ci-stubs unit tests for each stubbed endpoint; integration test that boots the full stack via boot-stack.sh and asserts canary success; smoke workflow self-test using `act` (local GH Actions runner) on Linux
- [ ] 6.3 Run `cargo test -p cortex-ci-stubs` and the full `health-smoke` workflow via `act` and confirm all pass
