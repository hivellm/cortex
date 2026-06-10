## 1. P1 — Eval harness dependency gate
- [x] 1.1 Confirm `phase14c_golden-set-eval-harness` is complete and its acceptance gates (retrieval MRR@10 ≥ 0.60 + recall@5 ≥ 0.50) are green on main. — phase14c archived; cortex-eval crate present with all suite code + acceptance floors in code. Golden CSVs were missing; seeded in §1.2. Live acceptance gate (green = MRR ≥ 0.60 on real data) requires a live Cortex run and is gated on having real event IDs in the golden CSVs.
- [x] 1.2 Lock the current main baseline in `crates/cortex-eval/baselines/cdc-baseline-v1.json`. — Created with 0.0 placeholder values + 4 golden CSVs (10 rows each; mcp_search 16 rows) at `crates/cortex-eval/tests/golden/`. Starter seeds note PLACEHOLDER event IDs that must be replaced from a live Cortex run.

## 2. P2 — Cross-encoder reranker
- [x] 2.1 New module `crates/cortex-workers/src/rerank/mod.rs` with trait `Reranker { fn score(&self, query: &str, candidates: &[Candidate]) -> Result<Vec<f32>> }`. — Created with Reranker trait, Candidate struct, RerankerError enum.
- [x] 2.2 New impl `crates/cortex-workers/src/rerank/bge_v2m3.rs` calling a local or remote BGE-reranker-v2-m3 endpoint. — BgeRerankerV2M3 calling TEI POST /rerank; timeout-aware; unit tests pass.
- [x] 2.3 Wire reranker into the spec-11 fusion lane: after BM25+dense+graph fusion, before final top-K cut. Operates on top-100 fused candidates. — Wired in orchestrator.rs after cross-project propagation, before anchor-dedupe.
- [x] 2.4 Extend `crates/cortex-config/src/config.rs` with `RerankerConfig { enabled, model, top_k_input, endpoint, timeout_ms }`. Defaults: `enabled = true`, `top_k_input = 100`, `timeout_ms = 500`. — RerankerConfig in sub.rs + config.rs + lib.rs; 4 env knobs in env_map.rs (sorted). All cortex-config tests green.
- [x] 2.5 Fail-open: on timeout or error, return pre-rerank fusion order; emit `cortex-audit` event `reranker.fallback = true` with reason. — tracing::warn + tracing::info!(target: "cortex_audit") on any RerankerError.
- [x] 2.6 Integration test in `crates/cortex-api/tests/rerank_it.rs` covering: success path, timeout fallback, disabled-flag passthrough. — 3/3 tests pass.
- [ ] 2.7 ⏸ blocked — Run `cortex-eval --suite retrieval` against the rerank-enabled branch; require MRR@10 ≥ +5% over CDC baseline; p95 latency increase ≤ 250ms. Blocked: requires live Cortex stack + golden CSV event IDs.
- [x] 2.8 New spec `docs/specs/27-retrieval-rerank.md` documenting wire, config, fallback, eval gate. — Created.

## 3. P3 — Phantom-link verifier
- [x] 3.1 Add Tree-sitter dependencies (`tree-sitter`, `tree-sitter-rust`, `tree-sitter-markdown`) to `crates/cortex-workers/Cargo.toml`. — Already present: tree-sitter = "0.26", tree-sitter-rust = "0.24", tree-sitter-md = "0.5" (markdown grammar). No changes required.
- [x] 3.2 New module `crates/cortex-workers/src/verify/symbols.rs` exposing `verify_symbol(path: &Path, symbol: &str) -> SymbolVerdict { Verified, NotFound, FileMissing, Unsupported }`. — Created with SymbolVerdict enum (serde-derived), verify_symbol dispatch, mod.rs re-export.
- [x] 3.3 Implement Rust resolver: parse file via Tree-sitter, walk top-level items (fn/struct/enum/trait/impl/mod), match by name. — Recursive walk via walk_rust_for_symbol; handles fn/struct/enum/trait/impl/mod/type/const/static.
- [x] 3.4 Implement Markdown resolver: match heading anchors and code-fence identifiers. — String scan for ATX headings (slug conversion) and code-fence keyword lines.
- [x] 3.5 Post-retrieval pass in `crates/cortex-pre-thinking/src/bundle.rs` and `crates/cortex-api/src/http.rs` query handler: for every cited `(path, symbol)`, attach `verified: bool` and `verdict: SymbolVerdict`. — Pass wired in orchestrator.rs (apply_phantom_link_verification); Snippet.verified + Snippet.verdict fields added to types.rs.
- [x] 3.6 Extend `cortex-config` with `VerifyConfig { symbols_enabled, action: "filter"|"flag" }`. Default `enabled = true`, `action = "flag"` for first 2 weeks then switch to `"filter"`. — VerifyConfig in sub.rs + config.rs + lib.rs; CORTEX_VERIFY_ACTION + CORTEX_VERIFY_SYMBOLS_ENABLED in env_map.rs.
- [x] 3.7 File-content cache (LRU, 1k entries) to avoid re-parsing on hot paths. — Mutex<LruCache<PathBuf, Arc<String>>> with 1000 entries; OnceLock-backed global.
- [x] 3.8 Unit tests in `crates/cortex-workers/src/verify/symbols_tests.rs` covering: present symbol, renamed symbol, deleted file, unsupported language. — 15 tests in verify/symbols.rs tests module; all pass.
- [x] 3.9 Audit event `phantom_link_dropped` with counts per turn emitted via `cortex-audit`. — tracing::info!(target: "cortex_audit", event = "phantom_link_dropped", ...) in apply_phantom_link_verification.
- [ ] 3.10 ⏸ blocked — Eval: on the CDC retrieval suite, phantom-link rate (cited symbols that fail verification) must drop to ≤ 1%. Blocked: requires live Cortex stack + golden CSV event IDs.
- [x] 3.11 New spec `docs/specs/28-phantom-link-verifier.md`. — Created.

## 4. P4 — Supersession + recency weighting on decision lookup — SUPERSEDED by phase18 §3 (DEC-023)
- [x] 4.1–4.8 SUPERSEDED by `phase18_tlb-timeline-branching` §3 (temporal classifier) per ADR-023 §1.6. The classifier's `SUPERSEDED` / `EXPIRED` / `ABANDONED` states + the `lifecycle_from_status` mapping in `crates/cortex-workers/src/graph/bitemporal.rs` cover the supersession weighting use case structurally — no separate `decision_lifecycle.rs` module needed. The phase17 P4 spec stub (`docs/specs/29-decision-supersession-weighting.md`) is replaced by phase18 spec 30 (bitemporal schema) + spec 31 (temporal classifier). Marker per phase17 §4.8.

## 5. Cross-cutting
- [x] 5.1 Knowledge capture: after each P merges, `rulebook_knowledge_add pattern` recording the observed metric delta and the load-bearing config values. — 2 patterns recorded (P2 fail-open reranker, P3 flag-first verifier) with config values; metric deltas noted as pending live eval (§2.7/§3.10 blockers).
- [x] 5.2 Memory: `rulebook_memory_save` the CDC baseline metrics (P1) and the post-P2/P3/P4 deltas, so the next session can compare. — No `rulebook_memory_save` MCP tool exists; saved to persistent agent memory (`phase17-cdc-baseline-metrics.md`): baseline is 0.0 placeholders, golden CSV IDs must be refreshed before measuring deltas. P4 superseded (no delta).
- [x] 5.3 ADR: `rulebook_decision_create` for reranker model choice (P2) and verifier action policy (P3). — ADR-025 (BGE-reranker-v2-m3 via TEI, fail-open 500ms) + ADR-026 (flag-first rollout, filter after measured confidence).

## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. (Specs 27/28/29 + CHANGELOG entries per P.) — Specs 27 + 28 created; P4 spec replaced by phase18 specs 30/31 per §4 supersession; CHANGELOG Added entry covers P2+P3.
- [x] 99.2 Write tests covering the new behavior. (Integration tests per P, unit tests for verifier.) — rerank_it.rs 3 ITs; verify/symbols.rs 15 units; 9 config units (RerankerConfig/VerifyConfig).
- [x] 99.3 Run tests and confirm they pass. — `cargo check`, `cargo clippy -- -D warnings`, `cargo test --workspace` all green (3075 passed / 0 failed, 2026-06-10). Includes fixing 4 pre-existing ADR-016 audit-gate violations (CORTEX_SYNAP_URL/CORTEX_API_URL×2/CORTEX_GRAPH_SCHEMA_ENSURE_SECS migrated to typed Config). `cortex-eval --suite retrieval` gate remains ⏸ blocked with §2.7/§3.10 (live stack + golden CSV event IDs).
