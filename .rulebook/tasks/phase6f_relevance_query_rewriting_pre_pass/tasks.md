## 1. Trait + types
- [ ] 1.1 Create `crates/cortex-api/src/query_rewrite.rs` with `pub trait QueryRewriter: Send + Sync` + `async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError>`
- [ ] 1.2 Define `pub struct RewrittenQuery { pub vector_query: String, pub keyword_query: String, pub original: String, pub strategy: &'static str }`
- [ ] 1.3 Define `pub enum RewriteError { Timeout, Upstream(String), Disabled }` and a passthrough constructor `RewrittenQuery::passthrough(p: &str)` that copies the prompt into both fields with `strategy = "passthrough"`

## 2. NounPhraseRewriter
- [ ] 2.1 Implement `pub struct NounPhraseRewriter` whose `rewrite` runs deterministically: lowercase → strip leading question word (`why|how|what|when|who|where|is|are|does|do|should|can|could|would`) → drop a curated stop-word list → keep tokens matching `^[A-Za-z][A-Za-z0-9_./-]*$` plus tokens with internal `_` / `-` / `.` (camelCase/snake_case/paths)
- [ ] 2.2 Re-emit the kept tokens space-separated; both `vector_query` and `keyword_query` carry the same string with `strategy = "noun_phrase"`
- [ ] 2.3 Unit tests covering: question-framing strip, technical-token preservation (`PreThinkingTool`, `meili_loader`, `cortex-api`, `crates/cortex-api/src/strategies.rs`), stop-word drop, empty-input no-panic
- [ ] 2.4 Round-trip the original prompt unchanged in `RewrittenQuery.original`

## 3. SonnetRewriter
- [ ] 3.1 Implement `pub struct SonnetRewriter` reusing the Anthropic plumbing in `crates/cortex-api/src/analyzer.rs` — share the HTTP client / auth / model selection
- [ ] 3.2 System prompt: produce a JSON object `{ "vector_query": "...", "keyword_query": "...", "rationale": "..." }`; parse with `serde_json::Value` and validate both fields are non-empty
- [ ] 3.3 Cache by `sha256(prompt + intent.as_str())` with a 24h TTL; reuse `cortex-api::cache::InMemoryCache` (already wired into the service) — bound the entry size to keep memory predictable
- [ ] 3.4 Timeout: 1.5s; on timeout / upstream error, fall back to `NounPhraseRewriter::default().rewrite(...)` rather than fail the user-facing request — log `tracing::warn!`
- [ ] 3.5 Unit test with a mock HTTP server (existing `wiremock` dev-dep) asserting the JSON parse path + the fallback-to-noun-phrase path on timeout

## 4. Orchestrator integration
- [ ] 4.1 In `Orchestrator::query` (`crates/cortex-api/src/orchestrator.rs`), call `self.rewriter.rewrite(&req.query, req.intent).await?` once before per-lane fan-out
- [ ] 4.2 Pass `rewritten.vector_query` to the `VectorRequest` builder and `rewritten.keyword_query` to the `KeywordRequest` builder; the graph lane receives `rewritten.vector_query` (graph queries today are slug-based, so the choice is mostly cosmetic — pick vector for consistency)
- [ ] 4.3 Round-trip `rewritten.original` so the response audit can show what the user actually typed
- [ ] 4.4 Add `Orchestrator::with_rewriter(rewriter: Arc<dyn QueryRewriter>) -> Self` to keep test wiring symmetric with the existing constructors

## 5. Audit envelope
- [ ] 5.1 Extend `AuditEnvelope` with `query_rewrite_strategy: String`, `vector_query: String`, `keyword_query: String`
- [ ] 5.2 Stamp from the `RewrittenQuery` returned by the orchestrator
- [ ] 5.3 Update the audit fixture in `crates/cortex-api/tests/http.rs` to assert all three fields are present

## 6. Configuration
- [ ] 6.1 In `crates/cortex-api/src/main.rs`, read `CORTEX_QUERY_REWRITER` (`noun_phrase` default, `sonnet`, or `passthrough`); fall back to `noun_phrase` on unknown values with `tracing::warn!`
- [ ] 6.2 Construct the matching `Arc<dyn QueryRewriter>` and inject into `Orchestrator::with_rewriter`
- [ ] 6.3 Document in the boot log line which strategy was selected

## 7. Harness validation
- [ ] 7.1 Re-run the `cortex-relevance-eval` harness (from `phase6e`) against `passthrough` (today's baseline) → `noun_phrase` → `sonnet`; capture the three reports under `target/relevance/`
- [ ] 7.2 Decision rule: ship `sonnet` as default ONLY when its global `recall@10` beats `noun_phrase` by ≥3pp; otherwise default stays on `noun_phrase`
- [ ] 7.3 Persist the comparison in `.rulebook/learnings/relevance/<date>-rewriter-decision.md` documenting which strategy won and the numbers

## 8. Spec docs
- [ ] 8.1 In `docs/specs/11-query-api.md`, add a "Query rewriting" subsection documenting the trait, the three strategies, the env knob, and the audit fields
- [ ] 8.2 In `docs/specs/12-pre-thinking.md`, note where in the pipeline the rewriter fires (before fan-out, after intent selection)
- [ ] 8.3 Cross-link from `docs/analysis/relevance/01-findings.md` §F-004 (mark closed-by phase6f on merge)

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 9.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` + `docs/specs/12-pre-thinking.md` per §8
- [ ] 9.2 Write tests covering the new behavior — unit tests in §2.3 (NounPhrase) and §3.5 (Sonnet); orchestrator integration test asserting both lane request bodies receive the rewritten queries; harness validation per §7
- [ ] 9.3 Run tests and confirm they pass — `cargo clippy -p cortex-api --all-targets -- -D warnings`, `cargo test -p cortex-api`, plus the harness comparison runs from §7 produce a documented decision in `.rulebook/learnings/`
