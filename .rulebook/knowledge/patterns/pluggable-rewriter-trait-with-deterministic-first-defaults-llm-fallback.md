# Pluggable rewriter trait with deterministic-first defaults + LLM fallback

**Category**: architecture
**Tags**: analysis:relevance, phase6f, F-004, architecture, rewriter, fallback

## Description

When adding an optional LLM-backed transformation to a hot path (rewriter, classifier, summariser), structure it as: (1) trait with one async method, (2) a Passthrough impl reproducing the pre-feature behaviour for kill-switch / A/B baseline, (3) a Deterministic impl that ships as the default — no network, no key, no latency surprise, (4) an LLM impl gated on cache + timeout + transparent fallback to the deterministic strategy. Stamp the audit envelope with the strategy that *actually ran* (including a `<llm>_fallback_<deterministic>` discriminant) so operators can tell "the LLM ran" from "the LLM bailed". The deterministic default is strictly safe relative to passthrough — its worst case is "no uplift", never a regression — which lets the feature ship before the harness comparison run produces numbers.

## Example

#[async_trait]
pub trait QueryRewriter: Send + Sync {
    async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError>;
}
// Three impls: PassthroughRewriter (kill-switch), NounPhraseRewriter (deterministic
// default), SonnetRewriter (opt-in, falls back to NounPhrase on timeout/upstream/missing-key
// and stamps strategy="sonnet_fallback_noun_phrase").
// Selection via CORTEX_QUERY_REWRITER env; unknown values warn + fall back to default.

## When to Use

Adding any optional LLM call to a hot path where (a) latency matters, (b) the upstream may be flaky / cost-bounded, (c) operators need a kill-switch, and (d) you want to ship without waiting on a CI A/B comparison.
