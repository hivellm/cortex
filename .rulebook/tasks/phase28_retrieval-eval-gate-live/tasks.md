## 1. Reconcile and audit the golden fixture trees
- [ ] 1.1 Diff `tests/golden/*.csv` (repo root, read by
      `.github/workflows/eval.yml`'s relative `--golden` path — no
      `working-directory:` override) against
      `crates/cortex-eval/tests/golden/*.csv` (read by
      `golden_set_acceptance.rs` via `CARGO_MANIFEST_DIR`); pick the
      single source of truth and point both the CI workflow and any
      remaining root-relative references at it.
- [ ] 1.2 Retire or sync whichever tree is not canonical — root
      `tests/golden/README.md` currently documents the old
      `expected_event_ids` column against a `retrieval.csv` that
      already uses `expected_paths`; fix or remove it as part of this.
- [ ] 1.3 Backfill the 16 blank `expected_ids` rows in the root
      `mcp_search.csv` (or drop rows for the tools already found
      broken/unwired — 502/400/empty responses, per the archived
      `phase0_live-eval-gates-rerank-phantomlink` findings) using real
      ids from a live Cortex run.
- [ ] 1.4 Confirm `access_control.csv` (40-row synthetic Bell-LaPadula
      matrix) is intentionally synthetic — not a placeholder needing
      live harvesting — and carry it into the canonical tree (it does
      not exist at the root today).

## 2. Grow the golden set to a statistically meaningful size
- [ ] 2.1 Expand `retrieval.csv` and `consolidation.csv` toward the
      curation targets in `tests/golden/README.md` (currently 18 and
      10 rows respectively), pulling from real historical
      queries/sessions where available.
- [ ] 2.2 Tag each row with the query intent it exercises and ensure
      coverage of all five: `pre_change_context`, `decision_lookup`,
      `similar_problems`, `law_check`, `free_search`.
- [ ] 2.3 Expand `classification.csv` to at least 2 examples per `Kind`
      variant (currently 1×10 rows; missing `Analysis` and `TopicCard`
      of the 12 kinds) so the suite has a meaningful set to run
      against for the first time.

## 3. Re-run the retrieval eval suite and re-lock the baseline
- [ ] 3.1 Run `cortex-eval --suite retrieval|consolidation|classification`
      against the bootstrapped corpus using the reconciled golden set
      from §1-§2.
- [ ] 3.2 Record the first real `classification` result in
      `cdc-baseline-v1.json`, replacing the `finished_at:
      1970-01-01T00:00:00Z` / `rows_total: 0` placeholder entry.
- [ ] 3.3 Re-lock `crates/cortex-eval/baselines/cdc-baseline-v1.json`
      with every suite's real numbers.

## 4. Review and reset the regression-gate floors
- [ ] 4.1 Compare `RECALL_AT_5_FLOOR` / `MRR_AT_10_FLOOR` in
      `crates/cortex-eval/src/suite/retrieval.rs` against the
      re-locked baseline; set each to baseline minus a small tolerance
      so the floor is a real regression gate rather than a value the
      suite already clears by a wide margin.
- [ ] 4.2 Do the same review for the `classification` suite's
      `macro_f1` floor now that it has a real first measurement.

## 5. Re-enable the nightly schedule
- [ ] 5.1 Add back a `schedule:` (cron) trigger in
      `.github/workflows/eval.yml` once the fixtures it reads are the
      reconciled, real ones from §1-§4.
- [ ] 5.2 Add `access_control` and `mcp_search` to `matrix.suite` so
      all five suites run nightly, not just the current three.
- [ ] 5.3 Remove or replace the comment block explaining why the
      schedule was `workflow_dispatch`-only.

## 6. Verify the phase17 P2/P3 gates measure something meaningful
- [ ] 6.1 P2 (reranker, ADR-025 / `docs/specs/37-retrieval-rerank.md`
      §2.7): confirm the already-measured +36% MRR@10 delta still
      holds against the reconciled baseline; add the still-missing
      p95-under-load measurement (≤ +250ms) that was explicitly left
      un-load-tested.
- [ ] 6.2 P3 (phantom-link verifier, ADR-026 /
      `docs/specs/28-phantom-link-verifier.md` §3.10): add a
      phantom-link-rate metric to `cortex-eval` (none exists today —
      the suite currently only measures MRR/recall) and measure it
      against the ≤1% gate.

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
      (`docs/specs/37-retrieval-rerank.md` §2.7 status, `docs/specs/28-phantom-link-verifier.md`
      §3.10 status, new `eval` spec, CHANGELOG)
- [ ] 7.2 Write tests covering the new behavior (phantom-rate metric
      unit tests; reconciled golden CSV structural tests)
- [ ] 7.3 Run tests and confirm they pass
