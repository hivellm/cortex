# Proposal: phase6e_relevance_recall_mrr_harness

## Why

Relevance is unmeasured today. There is no labeled query set, no recall@k / MRR harness, no canary queries that catch regressions. The `query_id` carried through the audit envelope (`crates/cortex-api/src/audit.rs:53-76`) is published to `cortex.events.query_audit` but no consumer reads it for quality scoring.

Every "the bundle feels weak" conversation stays qualitative. Fixes for F-001..F-007 cannot be ranked or proven; regressions land silently. Coverage gaps (F-001) and ranking gaps (F-005) look identical from the operator's seat. The execution plan in `docs/analysis/relevance/02-execution-plan.md` explicitly blocks further "make Cortex smarter" investment until this lands — because without a number, every other relevance task ships on faith.

R3 step 7 in the relevance plan, closes F-008. **Sequencing**: this MUST land before `phase6c` (score-aware RRF) and `phase6f` (query rewriting), otherwise their alpha-tuning / prompt-tuning is guesswork.

Source: `docs/analysis/relevance/01-findings.md` §F-008; `docs/analysis/cortex/09-risks-and-debt.md` (R9).

## What Changes

### Labeled query set
A fixture file `tests/relevance/queries.yaml` (or `.toml` to match the workspace style) carrying ~50 query/expected-doc pairs across the five intents:

```yaml
- id: rel-001
  intent: pre_change_context
  scope: { repo: "Cortex" }
  query: "the meili fan-out worker offset"
  expected_doc_ids: ["evt:cortex-Cortex-misc:01ABC...", "evt:cortex-Cortex-decisions:01DEF..."]
  notes: "Operator audit prompt — the worker leaves stale offsets in Synap."

- id: rel-014
  intent: explain
  scope: { repo: "Vectorizer" }
  query: "how does the upsert path work"
  expected_doc_ids: ["evt:cortex-Vectorizer-code:01GHI..."]
```

The set covers each of the 5 intents (`pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `explain`) with ≥10 queries each. IDs are stable across runs so regressions can be diffed.

### Harness binary `cortex-relevance-eval`
A new binary under `crates/cortex-relevance-eval/` (or `bin/relevance-eval/`) that:
1. Loads the query set.
2. Boots a local `cortex-api` (or hits a running one via `CORTEX_API_URL`).
3. Issues each query, captures the response.
4. Computes per-query `recall@10` (was the expected doc in the top-10 results?) and `MRR` (`1/rank` of the first expected doc, `0` if absent).
5. Computes per-intent + global aggregates.
6. Emits a JSON report `target/relevance/<git-sha>.json` with the full breakdown.

### CI gate
A workflow step that runs the harness on every PR touching a retrieval-path file (`crates/cortex-api/src/{strategies,orchestrator,fusion,meili_lane,vectorizer_lane,nexus_graph_lane}.rs` or any `intent_select.rs`). The step:
1. Runs the harness against the PR's HEAD.
2. Loads the previous report from `main` (cached as a CI artifact).
3. Asserts global `recall@10` and `MRR` are within `2%` of `main`. Per-intent thresholds: same `2%` band, but warning-only (the global gate is the hard stop).

### `.rulebook/learnings/` persistence
Each merged PR's report lands under `.rulebook/learnings/relevance/<date>-<sha>.json` so the trend is queryable from the dashboard later.

### Stability requirements
- Harness MUST produce deterministic results for a given index state. Use a frozen seed for any sampling; capture the upstream backend versions in the report.
- Backend availability checks: harness `tracing::warn!`s and skips per-intent buckets when the corresponding backend (Meili / Vectorizer / Nexus) is unhealthy at boot — surfaces in the report as `"skipped_intents": [...]` rather than exiting with an error.

## Impact

- Affected specs: new section in [`docs/specs/16-dashboard.md`](../../../docs/specs/16-dashboard.md) (the GUI later renders harness trends), plus a top-level callout in [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) referencing the harness as the canonical relevance gate.
- Affected code: new `crates/cortex-relevance-eval/` crate with `[[bin]]`; new `tests/relevance/queries.yaml` fixture; CI workflow YAML (`.github/workflows/ci-relevance.yaml` or `.gitlab-ci.yml` extension — pick the project's actual CI tool); harness output directory `target/relevance/` (gitignored).
- Breaking change: NO — the harness is observe-only and runs out-of-process.
- Depends on: `phase6a` + `phase6b` should land first so the harness measures the *post-fix* baseline rather than the broken-overlay status quo. If they haven't shipped, the harness still works but every `*_lookup` query records empty overlays.
- User benefit: every retrieval change ships with a measurable delta. "We made Cortex smarter" stops being a vibes-based claim; it becomes a number on a dashboard. R3 step 8 (query rewriting) and R2 step 5 (score-aware RRF) become tunable instead of guessed.
