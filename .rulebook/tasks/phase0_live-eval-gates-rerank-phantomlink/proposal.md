# phase0 — live eval gates for reranker + phantom-link (phase17 carry-over)

Source: phase17_cdc-code-doc-correlation §2.7 + §3.10 (2026-06-21).

## Why

phase17 shipped the cross-encoder reranker (P2) and phantom-link verifier
(P3) — code, config, fail-open/flag-first behaviour, specs (27/28), ADRs
(025/026), and unit + integration tests, all green. Two acceptance gates
could not be measured because they require a live eval run with a curated
golden set:

- §2.7 — `cortex-eval --suite retrieval` on the rerank-enabled branch:
  require MRR@10 ≥ +5% over the CDC baseline; p95 latency increase ≤ 250ms.
- §3.10 — phantom-link rate (cited symbols failing verification) ≤ 1% on
  the CDC retrieval suite.

Both are blocked on the same infra gap surfaced by phase28: the golden
CSVs (`crates/cortex-eval/tests/golden/`) carry PLACEHOLDER event IDs and
the baseline (`baselines/cdc-baseline-v1.json`) is 0.0 placeholders, and
the live stack's ingestion pipeline is not flowing current events on the
dev daemon (adapter host hook not installed), so real event IDs can't be
harvested. The reranker + phantom-verify features are also left
unconfigured on the dev stack (`CORTEX_RERANKER_ENABLED` /
`CORTEX_VERIFY_SYMBOLS_ENABLED` unset).

## What Changes

- Curate the golden set: harvest real event IDs from a live Cortex run
  into the 4 golden CSVs; establish a real CDC baseline (replace the 0.0
  placeholders in `cdc-baseline-v1.json`).
- Enable the reranker + verifier on the eval stack
  (`CORTEX_RERANKER_ENABLED=1`, a TEI endpoint; `CORTEX_VERIFY_SYMBOLS_ENABLED=1`).
- Run `cortex-eval --suite retrieval` and record: MRR@10 delta vs baseline
  (gate ≥ +5%), p95 latency delta (gate ≤ 250ms), phantom-link rate
  (gate ≤ 1%). Capture results back into the baseline + knowledge base.

## Impact
- Affected specs: `docs/specs/27-retrieval-rerank.md`,
  `docs/specs/28-phantom-link-verifier.md` (eval-gate sections).
- Affected code: `crates/cortex-eval/` (golden CSVs, baseline JSON) — no
  product-code change expected; this is measurement + curation.
- Breaking change: NO.
- User benefit: confirms the reranker actually lifts retrieval quality and
  the verifier actually suppresses phantom links, with numbers.

Blocked on: a flowing live stack + the adapter host hook (operator setup).
