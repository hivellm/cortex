# Spec 26 — cortex-eval golden-set harness

Status: **active** (phase14c)
Authors: phase14c_golden-set-eval-harness.

The `cortex-eval` harness measures end-to-end retrieval / consolidation / classification quality against curated golden CSVs and gates CI on regression vs the `main` baseline. Three suites, three CSVs, one binary.

## §1 Suites

| Suite | Golden CSV | Primary metrics | Acceptance floors |
|---|---|---|---|
| `retrieval` | `tests/golden/retrieval.csv` | `mrr_at_10`, `recall_at_5` | MRR@10 ≥ 0.60, recall@5 ≥ 0.50 |
| `consolidation` | `tests/golden/consolidation.csv` | `entity_recall`, `fact_recall` | entity ≥ 0.85, fact ≥ 0.70 |
| `classification` | `tests/golden/classification.csv` | `macro_f1`, per-kind `f1.<label>` | macro-F1 ≥ 0.90 (per-kind is advisory) |

Floors are pinned at module constants (`MRR_AT_10_FLOOR`, `RECALL_AT_5_FLOOR`, `ENTITY_RECALL_FLOOR`, `FACT_RECALL_FLOOR`, `MACRO_F1_FLOOR`). Changing a floor is a contract change — bump the spec + audit prior baselines.

## §2 Wire shapes

CSV shapes documented in `tests/golden/README.md`. Every CSV has a header row + UTF-8 single-line entries; `csv` crate parses with `trim=All`, strict quotes per RFC 4180.

## §3 CLI

```
cortex-eval --suite <retrieval|consolidation|classification> \
            [--golden <path>] \
            [--api-url <url>] [--classifier-url <url>] \
            [--baseline <path>] [--threshold <fraction>] \
            [--output md|json] [--verbose]
```

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Suite ran, acceptance passed, no regression. |
| 1 | Harness internal error (missing golden, network unreachable beyond row tolerance). |
| 2 | Regression vs baseline > threshold (default 0.05). |
| 3 | Acceptance floor missed on at least one metric. |

## §4 CI gate

`.github/workflows/eval.yml` runs all three suites on every PR + every push to `main`. On main, the report is uploaded as the `eval-baseline-<suite>` artifact (90 day retention). PR runs download the latest main artifact and pass `--baseline` to the harness. Any metric whose absolute drop vs baseline exceeds the `--threshold` (default 0.05) returns exit 2 → red check.

Per-PR diagnostic reports land as `eval-pr-<suite>` artifacts (14 day retention).

## §5 Refresh cadence

- **Per-incident**: any retrieval / consolidation / classification post-mortem MUST extend the relevant CSV with a row that reproduces the regression. That row stays in the set forever.
- **Quarterly**: operator adds 10–20 rows per suite capturing new feature areas + new failure modes.
- **Floor reviews**: every spec point release reviews the floors. Raising a floor is allowed; lowering requires a written rationale in CHANGELOG.

## §6 Library surface

`crates/cortex-eval/src/`:

| Module | Exposes |
|---|---|
| `metrics` | `mrr_at_k`, `recall_at_k`, `f1`, `macro_f1`, `set_recall` |
| `report` | `SuiteReport`, `MetricRow`, `is_regression`, `metric_delta` |
| `suite::retrieval` | `RetrievalRow`, `load_csv`, `build_report`, `retrieval_acceptance` |
| `suite::consolidation` | `ConsolidationRow`, `load_csv`, `build_report`, `consolidation_acceptance` |
| `suite::classification` | `ClassificationRow`, `load_csv`, `build_report`, `classification_acceptance` |

Unit tests cover every metric primitive + happy/sad paths per suite (30 tests, all green).
