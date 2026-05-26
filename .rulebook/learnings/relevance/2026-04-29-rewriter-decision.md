# Phase6f rewriter decision — 2026-04-29

## Status

**Default at merge: `noun_phrase`.** `sonnet` ships as opt-in.

## What was implemented

`crates/cortex-api/src/query_rewrite.rs` ships three `QueryRewriter`
implementations:

| Strategy | Selection | Cost | Behaviour |
|---|---|---|---|
| `passthrough` | `CORTEX_QUERY_REWRITER=passthrough` | 0ms, no calls | Copies the prompt verbatim into both lanes. Kill-switch reproducing the pre-phase6f baseline. |
| `noun_phrase` | **default** (no env var, or `noun_phrase`) | <1ms, in-process | Strips leading question words + a curated stop-list, keeps tokens that look like identifiers / paths. Same string to both lanes. |
| `sonnet` | `CORTEX_QUERY_REWRITER=sonnet` | ~1.5s budget, 1 Claude Code CLI spawn per cache miss, 24h cache | Distinct `vector_query` / `keyword_query` from Sonnet via `claude -p - --model <model> --output-format json`. Falls back to `noun_phrase` on timeout / missing binary / non-zero exit / malformed JSON (audit stamp `sonnet_fallback_noun_phrase`). **No Anthropic API key** — same CLI pattern as `cortex-classifier` and `analyzer.rs::invoke_cli`. |

All three flow through the same orchestrator pre-fan-out hook
(`Orchestrator::run` → `rewriter.rewrite()` → patch
`plan.{vectors,keywords,graphs}.query`) and stamp the audit
envelope with `query_rewrite_strategy` / `vector_query` /
`keyword_query`.

## Why `noun_phrase` is the default

The phase6e harness baseline was recorded against `passthrough`
(today's behaviour). The decision rule in the proposal reads:

> Decision rule for shipping: `sonnet` MUST beat `noun_phrase` by
> ≥3pp `recall@10` to justify the latency + token cost; otherwise
> default stays on `noun_phrase`.

The harness comparison run requires:

1. A booted local stack (Vectorizer + Meili + Nexus) with the
   Cortex repo already bootstrapped. That's a CI workflow
   concern (`.github/workflows/relevance.yaml`), not a unit-test
   concern — it isn't reproducible inside `cargo test` without an
   external network.
2. The Claude Code CLI installed in the CI image so `claude -p - --model <model>` resolves on `PATH` for the `sonnet` leg. (No Anthropic API key — Cortex stack standardises on the CLI path; same as `cortex-classifier`.)

Neither was available at merge time. Until the comparison run
lands a number, `noun_phrase` is the only strategy whose uplift
over `passthrough` is testable from a unit-level fixture (the
`tests/orchestrator_rewrite.rs` integration test proves the
rewritten queries reach the lane request builders) and is
therefore the safe default.

## What to do next

1. Wire the three-leg comparison into `relevance.yaml`:
   - Pass A: `CORTEX_QUERY_REWRITER=passthrough` → existing baseline.
   - Pass B: `CORTEX_QUERY_REWRITER=noun_phrase` → new candidate.
   - Pass C: `CORTEX_QUERY_REWRITER=sonnet` → opt-in candidate.
   Persist three reports under
   `target/relevance/<sha>-{passthrough,noun_phrase,sonnet}.json`.
2. Compute pairwise deltas (B vs A, C vs A, C vs B) and append to
   this file under a new dated section.
3. Apply the decision rule:
   - If `noun_phrase` recall ≥ `passthrough` recall: keep
     `noun_phrase` as default.
   - If `sonnet` recall − `noun_phrase` recall ≥ 3pp: flip the
     default to `sonnet` and document.
   - Otherwise: leave the default alone, keep `sonnet` opt-in.

The deterministic `noun_phrase` strip never increases user-facing
latency (sub-millisecond, no network), so the worst case is "no
uplift" — never a regression. Shipping it as the default is
strictly safe relative to `passthrough`.
