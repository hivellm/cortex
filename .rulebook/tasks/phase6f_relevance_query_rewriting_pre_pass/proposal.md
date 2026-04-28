# Proposal: phase6f_relevance_query_rewriting_pre_pass

## Why

The query string is the user prompt verbatim, in every lane. `crates/cortex-api/src/strategies.rs:101-117` does `VectorRequest.query = req.query.clone()` and `KeywordRequest.query = req.query.clone()`; `crates/cortex-pre-thinking/src/pipeline.rs:117` forwards `input.user_prompt` to `req.query` as-is. No enrichment, no rewriting, no expansion, no decoupling of code-search from question-answering.

A prompt like *"why is the meili fan-out broken — should we just rewrite the worker?"* hits the keyword lane with the entire English sentence, not the load-bearing tokens (`meili`, `fan-out`, `worker`). Meili's typo-tolerant tokenizer copes, but the vector lane sends the full question to the embedder; semantic match is dominated by framing words ("why is ... broken") rather than the technical content. The intent table at `intent_select.rs` *routes* by keyword but does not *rewrite* the query.

R3 step 8 in the relevance plan, closes F-004. **Sequencing**: this MUST land AFTER `phase6e` so the uplift is measurable; without the harness, picking between deterministic NP-extraction and Sonnet rewriting is guesswork.

Source: `docs/analysis/relevance/01-findings.md` §F-004; `docs/analysis/relevance/02-execution-plan.md` §R3 step 8.

## What Changes

### Rewriter trait
A small abstraction in a new module `crates/cortex-api/src/query_rewrite.rs`:

```rust
#[async_trait::async_trait]
pub trait QueryRewriter: Send + Sync {
    async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError>;
}

pub struct RewrittenQuery {
    pub vector_query: String,    // optimised for embedding (load-bearing terms)
    pub keyword_query: String,   // optimised for Meili (BM25-friendly tokens)
    pub original: String,        // round-tripped for audit
    pub strategy: &'static str,  // "noun_phrase" | "sonnet" | "passthrough"
}
```

### Two impls
1. **`NounPhraseRewriter`** (default, `strategy = "noun_phrase"`): deterministic, no LLM call. Strips question framing (`why|how|what|when|who|where|is|are|does|do|should|can|could|would` at the start), keeps proper nouns + technical tokens (camelCase / snake_case / kebab-case / paths / file extensions), drops common stop words. Same string is sent to both lanes — its job is removing noise, not specialising per lane. ~50 lines of Rust + a regex set, zero dependencies.

2. **`SonnetRewriter`** (opt-in, `strategy = "sonnet"`): one-shot Anthropic Sonnet call producing distinct `vector_query` and `keyword_query`. Reuses the existing `crates/cortex-api/src/analyzer.rs` Anthropic plumbing — same SDK, same auth, different system prompt. Cached by `sha256(prompt + intent)` with a 24-hour TTL so the same operator phrasing doesn't re-burn tokens.

### Selection
- `CORTEX_QUERY_REWRITER` env: `noun_phrase` (default) | `sonnet` | `passthrough`. `passthrough` reproduces today's behaviour for kill-switch.
- The orchestrator runs the rewriter once at the top of `Orchestrator::query` and threads the resulting `RewrittenQuery` into the per-lane `VectorRequest` / `KeywordRequest` builders.

### Audit
Stamp `query_rewrite_strategy`, `vector_query`, `keyword_query` on the audit envelope so the harness can attribute uplift to the rewriter and operators can debug why a specific query routed where it did.

### Validation against the harness
The phase6e harness MUST be re-run against:
1. Baseline (`passthrough` — today's behaviour, recorded in `phase6e` baseline).
2. `noun_phrase` rewriter.
3. `sonnet` rewriter.

Decision rule for shipping: `sonnet` MUST beat `noun_phrase` by ≥3pp `recall@10` to justify the latency + token cost; otherwise default stays on `noun_phrase`.

## Impact

- Affected specs: [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) (rewriter contract + env knob); [`docs/specs/12-pre-thinking.md`](../../../docs/specs/12-pre-thinking.md) (when the rewriter fires in the pipeline).
- Affected code: new `crates/cortex-api/src/query_rewrite.rs`; `crates/cortex-api/src/orchestrator.rs` (call rewriter before per-lane fan-out); `crates/cortex-api/src/audit.rs` (stamp rewrite metadata); `crates/cortex-api/src/main.rs` (read env knob, build the impl); reuse `crates/cortex-api/src/analyzer.rs` for the Sonnet path.
- Breaking change: NO — `passthrough` strategy reproduces today's behaviour as a kill-switch; `noun_phrase` default is empirically validated through the phase6e harness before merge.
- Depends on: `phase6e` (the harness — without it, the `sonnet` vs `noun_phrase` decision rule has no data to test against).
- User benefit: queries stop being whipsawed by question-framing words; technical terms in the prompt get the weight they deserve in both lanes. Measurable through the harness; the proposal explicitly blocks merge unless the metrics agree.
