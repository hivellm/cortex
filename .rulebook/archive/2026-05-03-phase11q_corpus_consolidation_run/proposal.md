# Proposal: phase11q_corpus_consolidation_run

## Why

The Cortex corpus carries **17,487 `tool_call` envelopes** and **2,353 `turn` envelopes** as of 2026-05-03 (per `/v1/dashboard/overview`'s `kind_breakdown`). That's ~80 % of the active event population. Most tool calls are low-signal Bash / Edit / Read traffic the agent would never recall verbatim — but the cumulative weight degrades retrieval quality on `pre_change_context` and `similar_problems` because the lane fan-out has to filter through a wall of noise to surface the few high-signal turns.

Phase11j shipped `cortex-consolidator` (Haiku/Opus producer) and the `Kind::Consolidation` envelope as the compression substrate — every consolidation summarises a session / topic / decision-trace cluster into one compact row that retrieval can prefer. The producer has been smoke-tested but has never been driven over the full active corpus.

This task runs the consolidator over today's corpus (one-shot, not a cron). It does NOT prune the source events — that's `phase11o_vectorizer_demotion_api` and is blocked on Vectorizer SDK 3.2 shipping `move_to_collection` + `delete_vectors`. Without pruning, the consolidation pass ADDS ~50–200 new envelopes; the win comes from retrieval relevance, not from corpus shrinkage.

Companion task `phase11p_corpus_cleanup_sweep` covers the mechanical waste (empty indexes, TML excludes, law dedupe). This one covers the LLM-driven summarisation.

## What Changes

In this repo only:

1. **Pre-flight cost estimate.** `cortex-consolidator` already ships an estimate / dry-run mode; produce a written estimate for: (a) every `Session` cluster in the active corpus, (b) every `Topic` cluster the classifier has identified, (c) every `DecisionTrace` chain. Output Haiku and Opus per-pass cost in USD using the pricing constants already in `summariser.rs`. Operator approves before the actual run.
2. **Session-grain pass.** Run `cortex-consolidator run-session --all --depth Shallow` against the active corpus. Shallow grain is the default — Haiku-priced and tuned for "what was this session about" recall.
3. **Topic-grain pass.** Run `cortex-consolidator run-topic --all --depth Shallow`. Topics cluster across sessions; the resulting envelopes feed the consolidations lane that `pre_change_context` and `similar_problems` already fan out to (phase11j §4.1).
4. **Decision-trace pass.** Run `cortex-consolidator run-decision --all --depth Deep`. Decision traces chain ADRs to their justifying turns; phase11j auto-promotes these to Opus because the trace recall floor matters more than per-token cost. Expected count: ~100 (= the active ADR count).
5. **Spot-check N=20 consolidations.** Sample 5 from each pass plus 5 cross-pass; render via the dashboard's existing `## Consolidated context` block and verify the summaries match the source clusters. Capture the spot-check log in `docs/cortex/2026-05-03-consolidation-run-log.md`.
6. **Dashboard verification.** After the run, the dashboard's "Consolidated context" panel surfaces ≥ 1 consolidation on at least 80 % of `pre_change_context` queries (vs. ~0 % today). Capture the before/after hit-rate using the existing relevance gold-set runner.

## Impact

- **Affected code:** none in cortex-api / cortex-workers (consolidator already ships); new CLI invocation script `scripts/run-corpus-consolidation.ps1` that chains the four passes and pipes their cost ledgers into one report.
- **Affected docs:** `docs/cortex/2026-05-03-consolidation-run-log.md` (new), CHANGELOG entry under `[Unreleased]` Operations (one line — "ran corpus consolidation pass; produced N envelopes; cost USD").
- **Breaking change:** NO. Additive on the corpus side. The consolidator's `derive_consolidation_id` is deterministic so re-runs are idempotent — a second run over the same cluster yields the same envelope id and the worker's content-hash dedupe collapses it to a no-op.
- **Cost estimate (rough):**
  - Session-grain Shallow over ~50 active sessions @ ~6 K input tokens each = **~$0.30–$0.50 USD** at Haiku 4.5 prices.
  - Topic-grain Shallow over ~30 topic clusters = **~$0.20–$0.40 USD**.
  - Decision-trace Deep over ~100 ADRs = **~$5–$15 USD** (Opus 4.7 pricing dominates).
  - **Total: ~$6–$16 USD**, all operator-confirmable via §1's pre-flight estimate before any LLM call fires.
- **Blocked on:** nothing — consolidator is shipped (phase11j), pricing ledger lives in `summariser.rs`. The pruning follow-up is blocked on Vectorizer SDK 3.2 (tracked in `phase11o_vectorizer_demotion_api`); this task is the upstream half that produces the consolidations to be pruned against.

## Source

`cortex-consolidator` lives in `crates/cortex-consolidator/`; its CLI is `crates/cortex-consolidator/src/bin/cortex-consolidator.rs`. Active corpus stats from `/v1/dashboard/overview` on 2026-05-03 (`events_total: 25,097`; `tool_call: 17,487`; `turn: 2,353`). Phase11j archive at `.rulebook/archive/2026-05-03-phase11j_consolidation_tier/`.
