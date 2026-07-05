## 1. Fixes (2026-07-05 live-verification findings, in order)
- [ ] 1.1 Fix the chunker fallback regression: `nl_projection()`'s `Kind::ToolCall` branch (`crates/cortex-workers/src/embedder/chunker_fallback.rs`) must return empty (not the placeholder `"()"`) when no tool-call fields are present, so `event_text()` falls through to the raw source text; make `unknown_language_falls_back` (`crates/cortex-workers/tests/embedder_it_chunk_pipeline.rs:182`) pass
- [ ] 1.2 Fix `bin/cortex-doctor` and `bin/cortex-doctor.ps1`: change `-p cortex-ops` to `-p cortex-cli --bin cortex-ops` (no `cortex-ops` package exists; it is a `[[bin]]` target inside `cortex-cli`)
- [ ] 1.3 Fix `crates/cortex-cli/src/bin/cortex-ops/doctor.rs` (~lines 33-37, ~68-72): replace the `curl -o /dev/null` shell-out with a portable null sink or an in-process `reqwest` HTTP check so it works on native Windows
- [ ] 1.4 Investigate and fix the `cortex-graph-worker` Nexus-consumer stall (silent since 2026-06-27T12:38 after repeated "transient nexus error; engaging backpressure" in `crates/cortex-workers/src/graph/worker.rs`); root-cause why backpressure never recovered (do not just blindly restart the process), then add a supervisor-level "restart on sustained stall" safeguard as defense in depth
- [ ] 1.5 Upgrade `quinn-proto` to ≥0.11.15 (RUSTSEC-2026-0185) and `rmcp` to ≥1.4.0 (RUSTSEC-2026-0189) in the workspace lockfile; re-run `cargo audit` and confirm both are clear
- [ ] 1.6 Reconcile `docs/specs/03-local-stack.md` with the real 12-service `docker-compose.yml` topology (currently documents ~5 of 12 services)

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
