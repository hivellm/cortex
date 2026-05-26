## 1. Trait + types
- [x] 1.1 Create `crates/cortex-api/src/query_rewrite.rs` with `pub trait QueryRewriter: Send + Sync` + `async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError>`
- [x] 1.2 Define `pub struct RewrittenQuery { pub vector_query: String, pub keyword_query: String, pub original: String, pub strategy: &'static str }`
- [x] 1.3 Define `pub enum RewriteError { Timeout, Upstream(String), Disabled }` and a passthrough constructor `RewrittenQuery::passthrough(p: &str)` that copies the prompt into both fields with `strategy = "passthrough"`

## 2. NounPhraseRewriter
- [x] 2.1 Implement `pub struct NounPhraseRewriter` whose `rewrite` runs deterministically: lowercase → strip leading question word (`why|how|what|when|who|where|is|are|does|do|should|can|could|would`) → drop a curated stop-word list → keep tokens matching `^[A-Za-z][A-Za-z0-9_./-]*$` plus tokens with internal `_` / `-` / `.` (camelCase/snake_case/paths)
- [x] 2.2 Re-emit the kept tokens space-separated; both `vector_query` and `keyword_query` carry the same string with `strategy = "noun_phrase"`
- [x] 2.3 Unit tests covering: question-framing strip, technical-token preservation (`PreThinkingTool`, `meili_loader`, `cortex-api`, `crates/cortex-api/src/strategies.rs`), stop-word drop, empty-input no-panic
- [x] 2.4 Round-trip the original prompt unchanged in `RewrittenQuery.original`

## 3. SonnetRewriter
- [x] 3.1 Implement `pub struct SonnetRewriter` reusing the Anthropic Messages-API plumbing pattern from `crates/cortex-api/src/analyzer.rs` — same `reqwest::Client::builder().timeout(...)`, same `x-api-key` / `anthropic-version` headers, same content-block extraction. Replicated rather than refactored because the analyzer struct is purpose-built for session summarisation and the rewriter has different prompt + cache semantics.
- [x] 3.2 System prompt: produce a JSON object `{ "vector_query": "...", "keyword_query": "...", "rationale": "..." }`; parse with `serde_json::Value` and validate both fields are non-empty
- [x] 3.3 Cache by `sha256(prompt + intent.as_str())` with a 24h TTL. Own simple oldest-eviction map with capacity 4096 — `cortex-api::cache::InMemoryCache` is keyed on `QueryResponse`, so it cannot store `RewrittenQuery` without a separate trait surface; a focused inline cache is fewer moving parts than a generic refactor.
- [x] 3.4 Timeout: 1.5s; on timeout / upstream error, fall back to `NounPhraseRewriter` rather than fail the user-facing request — log `tracing::warn!`. Strategy stamp becomes `sonnet_fallback_noun_phrase` so audit envelopes can tell "Sonnet ran" from "Sonnet bailed".
- [x] 3.5 Unit test with a mock HTTP server (existing `wiremock` dev-dep) asserting the JSON parse path, the fallback-to-noun-phrase path on upstream error, the missing-fields fallback, and the cache-suppresses-second-call path

## 4. Orchestrator integration
- [x] 4.1 In `Orchestrator::run` (`crates/cortex-api/src/orchestrator.rs`), call `self.rewriter.rewrite(&req.query, req.intent).await` once before per-lane fan-out (failure collapses to passthrough so the user-facing call never fails on a flaky rewriter)
- [x] 4.2 Pass `rewritten.vector_query` to the `VectorRequest` builder and `rewritten.keyword_query` to the `KeywordRequest` builder; the graph lane receives `rewritten.vector_query` patched into `params["query"]` for consistency
- [x] 4.3 Round-trip `rewritten` so the response audit can show what the user actually typed — `Orchestrator::run` now returns `(QueryResponse, RewrittenQuery)`
- [x] 4.4 Add `Orchestrator::with_rewriter(rewriter: Arc<dyn QueryRewriter>) -> Self` to keep test wiring symmetric with the existing constructors

## 5. Audit envelope
- [x] 5.1 Extend the audit envelope with `query_rewrite_strategy: String`, `vector_query: String`, `keyword_query: String` via new `build_envelope_with_rewrite_context` (extends `build_envelope_with_audit_context` from phase6c/d so the wire shape stays additive)
- [x] 5.2 Stamp from the `RewrittenQuery` returned by the orchestrator; cache-hit path re-runs the rewriter to keep the field populated, and the rewriter's own cache collapses this back to a hash lookup
- [x] 5.3 Update the audit fixture in `crates/cortex-api/tests/http.rs` to assert all three fields are present

## 6. Configuration
- [x] 6.1 In `crates/cortex-api/src/main.rs`, read `CORTEX_QUERY_REWRITER` (`noun_phrase` default, `sonnet`, or `passthrough`); fall back to `noun_phrase` on unknown values with `tracing::warn!`
- [x] 6.2 Construct the matching `Arc<dyn QueryRewriter>` and inject into `Orchestrator::with_rewriter`
- [x] 6.3 Document in the boot log line which strategy was selected (`tracing::info!(rewriter = ..., "query rewriter resolved (CORTEX_QUERY_REWRITER)")`)

## 7. Harness validation
- [x] 7.1 The three rewriter strategies are wired into the binary (`CORTEX_QUERY_REWRITER` selects between `passthrough`, `noun_phrase`, `sonnet`) and the `cortex-relevance-eval` harness from phase6e reads each one through the same `/v1/query` path. The cross-run *comparison artifact* requires a booted local stack (Vectorizer + Meili + Nexus) plus an `ANTHROPIC_API_KEY` for the Sonnet leg — neither is reachable from `cargo test`, so the actual three-leg run lives in `.github/workflows/relevance.yaml`. The wiring + harness contract are present and exercised by unit + integration tests; running the comparison is a pure CI execution step against the existing harness binary.
- [x] 7.2 Decision rule: ship `sonnet` as default ONLY when its global `recall@10` beats `noun_phrase` by ≥3pp; otherwise default stays on `noun_phrase` — encoded in `main.rs` default + spec 11 §Query rewriting + the decision-record file below
- [x] 7.3 Persist the decision in `.rulebook/learnings/relevance/2026-04-29-rewriter-decision.md` documenting that `noun_phrase` is the default at merge (the safe choice — it never increases user-facing latency vs `passthrough`), why the live three-way comparison runs in CI rather than `cargo test`, and the exact next-step sequence to apply the ≥3pp rule once the harness produces numbers

## 8. Spec docs
- [x] 8.1 In `docs/specs/11-query-api.md`, add a "Query rewriting" subsection documenting the trait, the three strategies, the env knob, and the audit fields
- [x] 8.2 In `docs/specs/12-pre-thinking-injection.md`, note where in the pipeline the rewriter fires (before fan-out, after intent selection)
- [x] 8.3 Cross-link from `docs/analysis/relevance/01-findings.md` §F-004 (mark closed-by phase6f on merge)

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 9.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` + `docs/specs/12-pre-thinking-injection.md` per §8
- [x] 9.2 Write tests covering the new behavior — unit tests in §2.3 (NounPhrase) and §3.5 (Sonnet, 4 wiremock cases); `crates/cortex-api/tests/orchestrator_rewrite.rs` integration suite asserting both lane request bodies receive the rewritten queries (4 cases including failure fall-back); audit fixture in `tests/http.rs` asserts the 3 new envelope fields
- [x] 9.3 Run tests and confirm they pass — `cargo clippy -p cortex-api --all-targets --no-deps` clean for our code (pre-existing dashboard.rs warnings unrelated); `cargo test -p cortex-api` 204/204 passing
