## 1. Community summaries (consolidation grain)
- [x] 1.1 New `Community` source selector: `source/community.rs` (`LiveCommunitySource` — two label-less Cypher passes over `community_id`-stamped nodes + cross-community edges, mirroring the phase27b §3.1 dashboard shapes; pure `group_into_inputs()` fully unit-tested — 4 tests incl. the empty-graph live reality). Input type `CommunityInput` (members/god-nodes/cross-edges) in `producer/community.rs`.
- [x] 1.2 Community consolidation envelope reusing existing storage end-to-end: `ConsolidationGrain::Community` + structured `ConsolidationScope::Community { community_id, level }` (cortex-core events + JSON schema oneOf arm + validator allowlist pair, each with tests); `producer/community.rs` (`produce()` → validated payload, stable `cons-com-<hash>` id, tag `community:{id}@{level}`); `templates/community.md` prompt; `Trigger::CommunityDetected` + `Orchestrator::run_community` (Haiku, budget-gated); `grains/community.rs` (`CommunityGrain`, 6 tests); daemon dispatch via optional `with_community()` builder (deployments without a graph client fail-and-ack instead of wedging). Downstream match sites (fulltext builder, archive_loader, producer id derivation, TriggerLabel) all extended.
- [x] 1.3 Multi-resolution: one input per `(community_id, level)` — grouping splits levels (test `group_into_inputs_splits_by_community_and_level`), distinct consolidation ids per level (test `produce_multi_resolution_levels_get_distinct_ids`), grain emits one envelope per community per level (test `community_grain_emits_one_envelope_per_community_per_level`).

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
