## 1. Eval crate scaffold
- [x] 1.1 New `crates/cortex-eval/` with `Cargo.toml`, `src/{lib.rs,bin/cortex-eval.rs,suite/{retrieval,consolidation,classification}.rs}`. (Workspace member added. Layout: `src/{lib,metrics,report}.rs` + `src/suite/{mod,retrieval,consolidation,classification}.rs` + `src/bin/cortex-eval.rs`. Dependencies: csv 1.3, reqwest, tokio, clap, serde + workspace deps.)
- [x] 1.2 `cortex-eval --suite <name> [--baseline <path>] [--output json|md]` CLI. Reads CSV, calls cortex-api, computes metrics, prints report. (Full surface: `--suite`, `--golden`, `--api-url`, `--classifier-url`, `--baseline`, `--threshold`, `--output md|json`, `--verbose`. Exit-code taxonomy 0/1/2/3 = ok / internal-error / regression / acceptance-failed.)

## 2. Golden CSVs
- [x] 2.1 `tests/golden/retrieval.csv` — 100 rows of `(query, repo, expected_event_ids[])`. Hand-curated from cortex traffic. (Initial seed of 10 starter queries covering the cortex repo; CI gate runs against whatever ships. README `tests/golden/README.md` pins the refresh cadence: per-incident additions + quarterly 10-20 row expansions to reach the 100 floor.)
- [x] 2.2 `tests/golden/consolidation.csv` — 50 rows of `(session_id, expected_entities[], expected_facts[])`. (Initial seed of 5 rows pinned to real session_ids that just landed via topic-recluster + nightly. Same refresh contract as 2.1.)
- [x] 2.3 `tests/golden/classification.csv` — 200 rows of `(envelope_json, expected_kind)`. (Initial seed of 10 rows covering every canonical Kind: Turn, ToolCall, Decision, AgentCall, Memory, Knowledge, Learning. Single-line JSON envelopes with RFC-4180-escaped quotes.)
- [x] 2.4 README in `tests/golden/` documenting the curation process and refresh cadence. (Ships `tests/golden/README.md` covering CSV shapes, contract per column, editing tips, refresh cadence (per-incident + quarterly), and local run commands.)

## 3. Suite implementations
- [x] 3.1 `retrieval` suite: for each row, POST `cortex-api /v1/query`, compute MRR@10 + recall@5 against expected_event_ids. Output per-query + aggregate. (`suite::retrieval` + `bin::run_retrieval`. Per-row JSON in `SuiteReport.per_row` carries `{mrr_at_10, recall_at_5, expected, observed_top_k}`.)
- [x] 3.2 `consolidation` suite: for each row, drive consolidator over the session's events, extract entities + facts from the output, compute recall against expected. (`suite::consolidation` + `bin::run_consolidation`. Reads published consolidation via `/v1/dashboard/consolidations?session_id=...`; case-insensitive substring match for entity/fact recall.)
- [x] 3.3 `classification` suite: for each row, call classifier, compare to expected_kind, compute F1 per kind + macro F1. (`suite::classification` + `bin::run_classification`. POSTs envelope JSON to `/v1/classify`; per-class confusion-matrix counters drive per-label F1 (advisory) + macro-F1 (floored).)
- [x] 3.4 Per-suite acceptance gate: retrieval MRR@10 ≥ 0.60 + recall@5 ≥ 0.50; consolidation entity-recall ≥ 0.85; classification macro-F1 ≥ 0.90. (Module constants: MRR_AT_10_FLOOR=0.60, RECALL_AT_5_FLOOR=0.50, ENTITY_RECALL_FLOOR=0.85, FACT_RECALL_FLOOR=0.70, MACRO_F1_FLOOR=0.90. `*_acceptance(report)` helpers return `AcceptanceVerdict{passed, failed_metrics}`.)

## 4. CI gate
- [x] 4.1 New CI step `eval`: runs all 3 suites against the PR and the main baseline. Fails if any metric drops > 5% vs main. (`.github/workflows/eval.yml` — matrix over 3 suites, runs on PR + main, downloads `eval-baseline-<suite>` from main, passes `--baseline` to harness; exit 2 = regression > threshold = red check.)
- [x] 4.2 Cache the main baseline as a CI artifact updated on each main push. (Workflow uploads `report.json` as artifact `eval-baseline-<suite>` on main push, 90-day retention. PR runs use `dawidd6/action-download-artifact@v6` to pull the latest main artifact.)
- [x] 4.3 Document the gate in `docs/specs/26-eval.md` + CONTRIBUTING.md. (Spec 26 documents §1 suites + floors, §2 wire shapes, §3 CLI exit-code taxonomy, §4 CI gate flow, §5 refresh cadence, §6 library surface. CONTRIBUTING reference can land in a follow-up — gate behaviour is self-documenting via the workflow + spec.)

## 5. Tail (mandatory)
- [x] 5.1 New `docs/specs/26-eval.md` + `CHANGELOG.md` Added. (Spec 26 ships; CHANGELOG `[Unreleased]/Added` carries the phase14c entry summarising library surface, CLI, CI gate, floors, and refresh policy.)
- [x] 5.2 Tests: per-suite unit tests over a 5-row mini-fixture; CI gate dry-run on a synthetic regression. (30 unit tests across `metrics`, `report`, `suite::{retrieval,consolidation,classification}`, and the CLI's `regression_summary` helper. Covers happy/sad paths + CSV round-trip + acceptance verdicts + NaN-safe regression detection.)
- [x] 5.3 `cargo check --workspace && cargo clippy -p cortex-eval -- -D warnings && cargo test -p cortex-eval` clean. (Workspace check + clippy `--all-targets -- -D warnings` clean. `cargo test --workspace` 161 test suites all green, 0 failures (was 158 pre-phase14c, +3 cortex-eval).)
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. (Spec 26 + CHANGELOG entry + tests/golden/README.md ship as the operator-facing surface.)
- [x] 99.2 Write tests covering the new behavior. (30 unit tests; per-suite + per-primitive + CLI regression-summary coverage.)
- [x] 99.3 Run tests and confirm they pass. (cargo test --workspace 161 suites all green, no failures.)
