## 1. Apply is_meili_index_missing to remaining handlers
- [ ] 1.1 consolidations_recent.rs — return empty `ConsolidationsResponse` (200) on missing index
- [ ] 1.2 consolidations_search.rs — empty 200
- [ ] 1.3 consolidations_by_entity.rs — empty 200
- [ ] 1.4 consolidations_diff.rs — empty 200
- [ ] 1.5 consolidation_costs.rs — empty 200
- [ ] 1.6 consolidation_lineage.rs — empty 200
- [ ] 1.7 consolidation_get.rs — 404 not-found (single-doc semantics, not empty)
- [ ] 1.8 decision_search.rs — empty 200
- [ ] 1.9 topic_search.rs — empty 200
- [ ] 1.10 law_violations.rs — empty 200
- [ ] 1.11 Audit search/ for any other BAD_GATEWAY-on-non-success handler; align

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update spec 11 error contract + CHANGELOG (missing index → 200 empty across the read surface)
- [ ] 2.2 Per-handler unit test: missing-index body → empty 200 (reuse the is_meili_index_missing pattern)
- [ ] 2.3 Run `cargo check` + `clippy -D warnings` + `cargo test --workspace`; live-verify the consolidations/recent probe returns 200 empty
