## 1. Diagnose the ranking gap (per-lane attribution)
- [ ] 1.1 Run `cortex-eval --suite retrieval` and record the per-row MRR/rank of the expected path; identify which queries rank the correct doc below #1
- [ ] 1.2 For 3–5 worst rows, inspect `/v1/query/explain` (or query_explain) to attribute the bad rank: keyword lane vs vector lane vs fusion vs recency
- [ ] 1.3 Confirm the vector-lane hypothesis: check that embedded text for the expected docs is raw JSON (not NL summary) and that dense similarity is low for the NL query

## 2. Fix the vector lane (natural-language embedding text)
- [ ] 2.1 Make the embedder embed natural-language text (real classifier summary, or a descriptive projection of the payload) instead of raw JSON
- [ ] 2.2 Re-index the affected corpus and re-run the golden harness; confirm MRR@10 rises and recall@5 stays 1.0

## 3. Cross-encoder reranker (phase17 §2.7)
- [ ] 3.1 Stand up a TEI reranker endpoint in compose and wire `CORTEX_RERANKER_ENABLED=1` + `CORTEX_RERANKER_ENDPOINT`
- [ ] 3.2 Run `cortex-eval --suite retrieval` with the reranker on; gate: MRR@10 ≥ +5% over baseline AND p95 latency increase ≤ 250 ms

## 4. Fusion + recency tuning (harness-arbitrated)
- [ ] 4.1 Sweep `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` (`crates/cortex-api/src/search/fusion.rs`) and per-intent recency λ (`config/relevance.toml`); keep only settings that raise MRR@10 on the golden harness
- [ ] 4.2 Revert any lever that fails to raise MRR or regresses recall@5

## 5. Re-measure + record
- [ ] 5.1 Re-run the full retrieval suite; record the final MRR@10 / recall@5 into `crates/cortex-eval/baselines/cdc-baseline-v1.json`
- [ ] 5.2 Capture the per-lever MRR deltas with `rulebook_knowledge_add`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation (specs 05/06/11/27 + CHANGELOG with the measured MRR deltas)
- [ ] 6.2 Write tests covering the new behavior (embedding-text projection unit test; fusion-weight regression pinned to the golden harness)
- [ ] 6.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace` + the live `cortex-eval` retrieval run)
