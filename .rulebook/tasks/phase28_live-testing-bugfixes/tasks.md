## 1. Fixes (2026-07-05 live-verification findings, in order)
- [x] 1.1 Fix the chunker fallback regression: `nl_projection()`'s `Kind::ToolCall` branch now returns empty when no tool-call fields (tool_name/input/output.text) are present, so `event_text()` falls through to content/text/body; `unknown_language_falls_back` passes (embedder_it_chunk_pipeline 5/5, chunker_fallback units 9/9)
- [x] 1.2 Fixed `bin/cortex-doctor` + `bin/cortex-doctor.ps1`: `-p cortex-ops` → `-p cortex-cli --bin cortex-ops`
- [x] 1.3 Replaced all three `curl` shell-outs in `doctor()` (service probes, api /v1/health/* probes, classifier-worker /healthz body fetch) with in-process reqwest on a current-thread runtime (the binary already links reqwest for the other doctor subcommands). Live-verified on native Windows: vectorizer/nexus/synap/meili all `ok` against the freshly-bumped 3.5.0/2.5.0/1.0.0 services
- [ ] 1.4 Investigate and fix the `cortex-graph-worker` Nexus-consumer stall (silent since 2026-06-27T12:38 after repeated "transient nexus error; engaging backpressure" in `crates/cortex-workers/src/graph/worker.rs`); root-cause why backpressure never recovered (do not just blindly restart the process), then add a supervisor-level "restart on sustained stall" safeguard as defense in depth
- [ ] 1.5 Upgrade `quinn-proto` to ≥0.11.15 (RUSTSEC-2026-0185) and `rmcp` to ≥1.4.0 (RUSTSEC-2026-0189) in the workspace lockfile; re-run `cargo audit` and confirm both are clear
- [ ] 1.6 Reconcile `docs/specs/03-local-stack.md` with the real 12-service `docker-compose.yml` topology (currently documents ~5 of 12 services)

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
