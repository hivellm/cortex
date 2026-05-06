# Proposal: phase14c_golden-set-eval-harness

Source: `docs/analysis/rework/03-relevance.md` Phase 5; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase B.4.

## Why

Today the fidelity IT validates takeaway length, not truth. The user reports "os dados não resultam em nada relevante" and there is no automated way to detect retrieval-quality regression between releases. Without a golden-set harness gating CI, every Phase B subsystem rewrite risks silent quality regression.

## What Changes

- New crate `crates/cortex-eval/` exposing 3 subcommands:
  - `cortex-eval --suite retrieval` — runs MRR@10 + recall@5 against `tests/golden/retrieval.csv`.
  - `cortex-eval --suite consolidation` — fidelity (entity recall, fact recall) against `tests/golden/consolidation.csv`.
  - `cortex-eval --suite classification` — kind-classification F1 against `tests/golden/classification.csv`.
- 3 golden CSVs hand-curated from real Cortex traffic, ~100 rows each.
- CI gate blocks PRs whose suite drops > 5% on any metric vs main.
- Quality bar at acceptance: retrieval `MRR@10 ≥ 0.60`, `recall@5 ≥ 0.50`; consolidation entity-recall ≥ 0.85; classification F1 ≥ 0.90.

## Impact

- Affected specs: new `docs/specs/26-eval.md`.
- Affected code: `crates/cortex-eval/` (new), `tests/golden/{retrieval,consolidation,classification}.csv` (new), `.github/workflows/ci.yml` (new step).
- Breaking change: NO.
- User benefit: retrieval quality becomes measurable; "nada relevante" becomes a measurable + bug-tractable claim.
