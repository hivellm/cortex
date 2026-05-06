## 1. Eval crate scaffold
- [ ] 1.1 New `crates/cortex-eval/` with `Cargo.toml`, `src/{lib.rs,bin/cortex-eval.rs,suite/{retrieval,consolidation,classification}.rs}`.
- [ ] 1.2 `cortex-eval --suite <name> [--baseline <path>] [--output json|md]` CLI. Reads CSV, calls cortex-api, computes metrics, prints report.

## 2. Golden CSVs
- [ ] 2.1 `tests/golden/retrieval.csv` — 100 rows of `(query, repo, expected_event_ids[])`. Hand-curated from cortex traffic.
- [ ] 2.2 `tests/golden/consolidation.csv` — 50 rows of `(session_id, expected_entities[], expected_facts[])`.
- [ ] 2.3 `tests/golden/classification.csv` — 200 rows of `(envelope_json, expected_kind)`.
- [ ] 2.4 README in `tests/golden/` documenting the curation process and refresh cadence.

## 3. Suite implementations
- [ ] 3.1 `retrieval` suite: for each row, POST `cortex-api /v1/query`, compute MRR@10 + recall@5 against expected_event_ids. Output per-query + aggregate.
- [ ] 3.2 `consolidation` suite: for each row, drive consolidator over the session's events, extract entities + facts from the output, compute recall against expected.
- [ ] 3.3 `classification` suite: for each row, call classifier, compare to expected_kind, compute F1 per kind + macro F1.
- [ ] 3.4 Per-suite acceptance gate: retrieval MRR@10 ≥ 0.60 + recall@5 ≥ 0.50; consolidation entity-recall ≥ 0.85; classification macro-F1 ≥ 0.90.

## 4. CI gate
- [ ] 4.1 New CI step `eval`: runs all 3 suites against the PR and the main baseline. Fails if any metric drops > 5% vs main.
- [ ] 4.2 Cache the main baseline as a CI artifact updated on each main push.
- [ ] 4.3 Document the gate in `docs/specs/26-eval.md` + CONTRIBUTING.md.

## 5. Tail (mandatory)
- [ ] 5.1 New `docs/specs/26-eval.md` + `CHANGELOG.md` Added.
- [ ] 5.2 Tests: per-suite unit tests over a 5-row mini-fixture; CI gate dry-run on a synthetic regression.
- [ ] 5.3 `cargo check --workspace && cargo clippy -p cortex-eval -- -D warnings && cargo test -p cortex-eval` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
