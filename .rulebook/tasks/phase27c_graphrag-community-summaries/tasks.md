## 1. Community summaries (consolidation grain)
- [ ] 1.1 New `Community` source selector in `crates/cortex-workers/src/consolidator/source/` (input = one community's nodes/edges/god-nodes)
- [ ] 1.2 Emit a community-summary consolidation envelope reusing existing storage; carry community_id + hierarchy level
- [ ] 1.3 Multi-resolution summaries from Leiden levels (coarse subsystem → fine module)

## 2. Global query route
- [ ] 2.1 Detect architecture-level intent in the orchestrator (`crates/cortex-api/src/search/`)
- [ ] 2.2 Map-reduce over community summaries → synthesized answer (instead of per-chunk fusion); budgeted top-N within the pre-thinking byte budget

## 3. Community-aware entity dedup
- [ ] 3.1 Lift the MinHash util out of `crates/cortex-cli/src/ops/memory_consolidate.rs` into a shared crate
- [ ] 3.2 Graph dedup pass: entropy gate → MinHash/LSH blocking → Jaro-Winkler verify → same-community boost → union-find merge (survivor-id preference)
- [ ] 3.3 Optional LLM tiebreaker behind a flag + the existing daily budget tracker; run in the phase27b graph worker

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation (spec 12 `Community` grain; spec 11 global route; CHANGELOG; ADR for community-summaries grain alongside DEC-005)
- [ ] 4.2 Write tests (community-source selection unit; global-route IT; dedup boost/union-find units)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
