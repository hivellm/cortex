## 1. Curate golden set + baseline (BLOCKED on a flowing live stack)
- [ ] 1.1 Harvest real event IDs from a live Cortex run into the 4 golden CSVs (`crates/cortex-eval/tests/golden/`), replacing the PLACEHOLDER IDs
- [ ] 1.2 Establish a real CDC baseline — run `cortex-eval --suite retrieval` against main and record MRR@10 / recall@5 into `crates/cortex-eval/baselines/cdc-baseline-v1.json` (replace the 0.0 placeholders)

## 2. Measure the gates
- [ ] 2.1 Enable reranker (`CORTEX_RERANKER_ENABLED=1` + a TEI endpoint) and run `cortex-eval --suite retrieval`; gate: MRR@10 ≥ +5% over baseline AND p95 latency increase ≤ 250ms (phase17 §2.7)
- [ ] 2.2 Enable verifier (`CORTEX_VERIFY_SYMBOLS_ENABLED=1`) and measure phantom-link rate on the CDC retrieval suite; gate: ≤ 1% (phase17 §3.10)
- [ ] 2.3 Record results into the baseline JSON + `rulebook_knowledge_add` (the observed metric deltas)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (specs 27/28 eval-gate sections + CHANGELOG with the measured numbers)
- [ ] 3.2 Write tests covering the new behavior (eval assertions wired to the curated golden set)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace` + the live `cortex-eval` run)
