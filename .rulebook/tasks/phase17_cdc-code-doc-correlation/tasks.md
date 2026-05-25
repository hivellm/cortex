## 1. P1 — Eval harness dependency gate
- [ ] 1.1 Confirm `phase14c_golden-set-eval-harness` is complete and its acceptance gates (retrieval MRR@10 ≥ 0.60 + recall@5 ≥ 0.50) are green on main. If not, block this task until it lands.
- [ ] 1.2 Lock the current main baseline in `crates/cortex-eval/baselines/cdc-baseline-v1.json`. All P2/P3/P4 deltas measured against this baseline.

## 2. P2 — Cross-encoder reranker
- [ ] 2.1 New module `crates/cortex-workers/src/rerank/mod.rs` with trait `Reranker { fn score(&self, query: &str, candidates: &[Candidate]) -> Result<Vec<f32>> }`.
- [ ] 2.2 New impl `crates/cortex-workers/src/rerank/bge_v2m3.rs` calling a local or remote BGE-reranker-v2-m3 endpoint.
- [ ] 2.3 Wire reranker into the spec-11 fusion lane: after BM25+dense+graph fusion, before final top-K cut. Operates on top-100 fused candidates.
- [ ] 2.4 Extend `crates/cortex-config/src/config.rs` with `RerankerConfig { enabled, model, top_k_input, endpoint, timeout_ms }`. Defaults: `enabled = true`, `top_k_input = 100`, `timeout_ms = 500`.
- [ ] 2.5 Fail-open: on timeout or error, return pre-rerank fusion order; emit `cortex-audit` event `reranker.fallback = true` with reason.
- [ ] 2.6 Integration test in `crates/cortex-api/tests/rerank_it.rs` covering: success path, timeout fallback, disabled-flag passthrough.
- [ ] 2.7 Run `cortex-eval --suite retrieval` against the rerank-enabled branch; require MRR@10 ≥ +5% over CDC baseline; p95 latency increase ≤ 250ms.
- [ ] 2.8 New spec `docs/specs/27-retrieval-rerank.md` documenting wire, config, fallback, eval gate.

## 3. P3 — Phantom-link verifier
- [ ] 3.1 Add Tree-sitter dependencies (`tree-sitter`, `tree-sitter-rust`, `tree-sitter-markdown`) to `crates/cortex-workers/Cargo.toml`.
- [ ] 3.2 New module `crates/cortex-workers/src/verify/symbols.rs` exposing `verify_symbol(path: &Path, symbol: &str) -> SymbolVerdict { Verified, NotFound, FileMissing, Unsupported }`.
- [ ] 3.3 Implement Rust resolver: parse file via Tree-sitter, walk top-level items (fn/struct/enum/trait/impl/mod), match by name.
- [ ] 3.4 Implement Markdown resolver: match heading anchors and code-fence identifiers.
- [ ] 3.5 Post-retrieval pass in `crates/cortex-pre-thinking/src/bundle.rs` and `crates/cortex-api/src/http.rs` query handler: for every cited `(path, symbol)`, attach `verified: bool` and `verdict: SymbolVerdict`.
- [ ] 3.6 Extend `cortex-config` with `VerifyConfig { symbols_enabled, action: "filter"|"flag" }`. Default `enabled = true`, `action = "flag"` for first 2 weeks then switch to `"filter"`.
- [ ] 3.7 File-content cache (LRU, 1k entries) to avoid re-parsing on hot paths.
- [ ] 3.8 Unit tests in `crates/cortex-workers/src/verify/symbols_tests.rs` covering: present symbol, renamed symbol, deleted file, unsupported language.
- [ ] 3.9 Audit event `phantom_link_dropped` with counts per turn emitted via `cortex-audit`.
- [ ] 3.10 Eval: on the CDC retrieval suite, phantom-link rate (cited symbols that fail verification) must drop to ≤ 1%.
- [ ] 3.11 New spec `docs/specs/28-phantom-link-verifier.md`.

## 4. P4 — Supersession + recency weighting on decision lookup
- [ ] 4.1 New scoring function in `crates/cortex-workers/src/scoring/decision_lifecycle.rs`: `score' = base × lifecycle_weight × recency_decay` with `recency_decay = exp(-age_days / 365)`.
- [ ] 4.2 Lifecycle weights configurable in `cortex-config`: `accepted = 1.0`, `proposed = 0.7`, `superseded = 0.2`, `deprecated = 0.1`.
- [ ] 4.3 Apply scoring in the `decision_lookup` intent path and in `cortex-historian`'s initial retrieval (not just the post-walk).
- [ ] 4.4 Emit `cortex-audit` event capturing pre/post-rerank ordering for top-10 ADR candidates per `decision_lookup` query.
- [ ] 4.5 Integration test in `crates/cortex-api/tests/decision_lookup_it.rs` extension: assert accepted ADR outranks superseded predecessor on synthetic fixture.
- [ ] 4.6 Eval: on CDC `decision_lookup` subset, recall@5 ≥ +10% on cases where gold ADR is `accepted`; no regression on historical cases.
- [ ] 4.7 New spec `docs/specs/29-decision-supersession-weighting.md`.
- [ ] 4.8 Mark this section superseded in tasks.md when `phase18_tlb-timeline-branching` Phase 2 (temporal classifier) ships, with a one-line pointer to the replacement.

## 5. Cross-cutting
- [ ] 5.1 Knowledge capture: after each P merges, `rulebook_knowledge_add pattern` recording the observed metric delta and the load-bearing config values.
- [ ] 5.2 Memory: `rulebook_memory_save` the CDC baseline metrics (P1) and the post-P2/P3/P4 deltas, so the next session can compare.
- [ ] 5.3 ADR: `rulebook_decision_create` for reranker model choice (P2) and verifier action policy (P3).

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation. (Specs 27/28/29 + CHANGELOG entries per P.)
- [ ] 99.2 Write tests covering the new behavior. (Integration tests per P, unit tests for verifier.)
- [ ] 99.3 Run tests and confirm they pass. (`cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo test --workspace` clean; `cortex-eval --suite retrieval` meets gates.)
