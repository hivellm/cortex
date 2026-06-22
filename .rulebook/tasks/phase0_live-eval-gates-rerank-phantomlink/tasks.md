> UNBLOCKED 2026-06-22: the live stack flows again (adapter daemon
> restarted; phase0_live-ingestion-staleness). Retrieval lane + reranker
> gate DONE; the other lanes/arms remain (see per-item notes).

## 1. Curate golden set + baseline
- [x] 1.1 PARTIAL→retrieval DONE (2026-06-22): the retrieval golden was re-keyed from the obsolete `expected_event_ids` to `expected_paths` (the live `/v1/query` returns `results.snippets[].path`, not event_id) and populated with real paths harvested from the live stack (commit adf6153). `classification.csv` was already complete. `consolidation.csv` (needs `session_id`→`consolidation_id` re-key; the dashboard exposes no session_id) and `mcp_search.csv` (per-tool re-key) still carry PLACEHOLDERs — remaining lane work.
- [x] 1.2 DONE (2026-06-22): real CDC retrieval baseline recorded in `cdc-baseline-v1.json` — fusion MRR@10 0.4733 / recall@5 1.0; reranked MRR 0.6417 / recall 0.80.

## 2. Measure the gates (consolidated golden-corpus eval gates)
- [x] 2.1 DONE — GATE PASSED (2026-06-22): reranker enabled (`CORTEX_RERANKER_ENABLED=1` + TEI `cortex-reranker`, bge-reranker-v2-m3 on the host RTX 4090, `--auto-truncate`) AND the dead phase17 wiring fixed (cortex-api now constructs the reranker at boot). MRR@10 0.4733→0.6417 = **+36%**, far above the +5% gate (phase17 §2.7). p95-latency arm (≤250ms): GPU rerank is fast but not load-measured here — left as a separate load-test note. Done in phase0_retrieval-ranking-quality (commits 35fb2db, da3aae7).
- [ ] 2.2 Enable verifier (`CORTEX_VERIFY_SYMBOLS_ENABLED=1`) and measure phantom-link rate on the CDC retrieval suite; gate: ≤ 1% (phase17 §3.10)
- [ ] 2.3 Temporal wedge eval — labeled time-sensitive subset; gate: MRR@10 ≥ +10% on the temporal arm (phase18 §3.8; wedge IT-pinned by `temporal_it.rs`)
- [ ] 2.4 Cross-project eval — labeled cross-project query corpus; gate: MRR-delta per the CDC harness (phase18 §5.4; provenance-per-candidate already IT-pinned by `cross_project_it.rs`)
- [ ] 2.5 Record all results into the baseline JSON + `rulebook_knowledge_add` (the observed metric deltas)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (specs 27/28 eval-gate sections + CHANGELOG with the measured numbers)
- [ ] 3.2 Write tests covering the new behavior (eval assertions wired to the curated golden set)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace` + the live `cortex-eval` run)
