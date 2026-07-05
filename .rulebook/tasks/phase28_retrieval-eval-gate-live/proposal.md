# Proposal: phase28_retrieval-eval-gate-live

Source: docs/analysis/cortex/11-platform-vision-analysis.md (§2.1 "no
recall@10/MRR benchmark" gap; §5.4 Risk Register "No retrieval quality
measurement → silent degradation"; Phase A5 "Retrieval quality
benchmark"); `.rulebook/knowledge/patterns/separate-pipeline-hygiene-gains-from-embedding-model-ceiling-when-benchmarking-retrieval-quality.md`
(tag `analysis:cortex-platform-2026-07`).

## Why

`.github/workflows/eval.yml` runs on `workflow_dispatch` only. Its own
comment block says the golden fixtures are "10-row starter fixtures"
that "do not satisfy the production acceptance floors" and that the
nightly schedule was removed because it "only ever failed." That
framing is stale in a way that changes the shape of this task:
`crates/cortex-eval/tests/golden/retrieval.csv` already carries 18 real
rows with harvested repo-relative paths, `consolidation.csv` carries 10
real rows with real ULIDs, and `crates/cortex-eval/baselines/cdc-baseline-v1.json`
already holds real measured numbers for both suites (fusion MRR@10
0.4733 / recall@5 1.0; reranked MRR@10 0.6417 / recall@5 0.80;
consolidation entity/fact-recall 1.0 / 1.0) — none of that is a 0.0
placeholder. That work landed under the archived
`phase0_live-eval-gates-rerank-phantomlink` task (2026-06-22).

The actual gap is narrower, but includes one structural bug this task
must fix first: **`.github/workflows/eval.yml` reads its golden
fixtures from a different, still-largely-placeholder directory than
the one described above.** The workflow's `--golden
tests/golden/${{ matrix.suite }}.csv` path has no `working-directory:`
override, so under the default `actions/checkout@v4` layout it resolves
against the **repo-root** `tests/golden/` — not
`crates/cortex-eval/tests/golden/` — while `cargo test -p cortex-eval`'s
`golden_set_acceptance.rs` IT gate loads from
`crates/cortex-eval/tests/golden/` via `CARGO_MANIFEST_DIR`. The two
trees have diverged: root `retrieval.csv`/`consolidation.csv` carry
different rows under the same ids than the crate-level files; root
`mcp_search.csv` has all 16 rows' `expected_ids` genuinely blank; root
has no `access_control.csv` at all; and the root `tests/golden/README.md`
still documents the old `expected_event_ids` column name even though
its own `retrieval.csv` already uses `expected_paths` — a live instance
of the "narrative docs silently drift" anti-pattern already in the
knowledge base. Re-enabling the schedule without reconciling these
trees would gate CI on the wrong (stale/placeholder) fixtures.

Beyond that reconciliation: the `classification` suite has never
actually been executed — its baseline entry is `finished_at:
1970-01-01T00:00:00Z`, `rows_total: 0` — despite a hardcoded `macro_f1
≥ 0.90` floor. `access_control` (40 synthetic Bell-LaPadula rows,
already real/appropriate) and `mcp_search` (4 real rows) exist in
`crates/cortex-eval/src/suite/` and are wired into the `cortex-eval`
binary's `--suite` dispatch, but neither appears in `eval.yml`'s
`matrix.suite: [retrieval, consolidation, classification]` — they are
not measured in CI at all. Golden-set sizes also fall short of `tests/golden/README.md`'s
own curation targets (retrieval ~50-100 rows target, 18 today;
consolidation similarly thin). Finally, the phase17 P2/P3 acceptance
gates (ADR-025 reranker, ADR-026 phantom-link verifier) are each
half-measured: P2's MRR@10 ≥ +5% gate passed live (+36%,
`phase0_live-eval-gates-rerank-phantomlink` §2.1) but its p95 ≤ +250ms
latency arm was explicitly left un-load-tested; P3's verifier is wired
live but `cortex-eval` has no phantom-link-rate metric at all yet, so
the ≤1% gate has never been computed.

Net effect: retrieval-quality and gate regressions are still invisible
to CI, for more precise reasons than "no real data" — a resolvable set
of gaps (fixture-tree mismatch, an unrun suite, two unmeasured suites,
two half-measured gates) rather than a wholesale re-seeding effort.

## What Changes

- Reconcile the two divergent golden-fixture trees (`tests/golden/` at
  repo root vs. `crates/cortex-eval/tests/golden/`) into one source of
  truth that both `cargo test -p cortex-eval` and
  `.github/workflows/eval.yml` read, and finish backfilling the
  placeholder rows that remain in it (all 16 `mcp_search` root rows'
  `expected_ids`; the classification suite's never-executed baseline;
  golden-set size versus the curation targets).
- Grow the reconciled golden set toward a statistically meaningful size
  covering the main query intents (`pre_change_context`,
  `decision_lookup`, `similar_problems`, `law_check`, `free_search`).
- Re-run the retrieval suite against the reconciled/grown set and
  re-lock `cdc-baseline-v1.json` with the real numbers (including a
  first real `classification` run).
- Re-tune `RECALL_AT_5_FLOOR` / `MRR_AT_10_FLOOR` in
  `crates/cortex-eval/src/suite/retrieval.rs` against the new baseline.
- Re-enable the `schedule:` trigger in `.github/workflows/eval.yml`
  once it points at the reconciled fixtures, and add the two suites
  (`access_control`, `mcp_search`) that exist in code but never run in
  CI.
- Close the two remaining phase17 gate gaps (P2 p95-under-load arm; P3
  phantom-link-rate metric) so ADR-025/ADR-026's acceptance criteria
  are fully, not partially, measured.

## Impact

- Affected specs: new `eval` module spec added by this task; touches
  the acceptance language implied by `docs/specs/27-retrieval-rerank.md`
  §2.7 and `docs/specs/28-phantom-link-verifier.md` §3.10.
- Affected code: `tests/golden/*` (repo root),
  `crates/cortex-eval/tests/golden/*`,
  `crates/cortex-eval/baselines/cdc-baseline-v1.json`,
  `crates/cortex-eval/src/suite/retrieval.rs`,
  `crates/cortex-eval/src/suite/mcp_search.rs`,
  `crates/cortex-eval/src/bin/cortex-eval.rs` (new phantom-rate metric
  + p95 latency capture), `.github/workflows/eval.yml`.
- Breaking change: NO (test/CI infra + fixture data only).
- User benefit: retrieval, consolidation, classification,
  access-control, and mcp-search quality regressions become visible in
  CI on a nightly cadence instead of shipping silently; the phase17
  reranker/phantom-link gates become fully measured instead of
  half-measured.
