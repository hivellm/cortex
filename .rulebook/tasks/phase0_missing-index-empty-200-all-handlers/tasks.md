## 1. Apply is_meili_index_missing to remaining handlers
- [x] 1.1 consolidations_recent.rs — empty 200 on missing index <!-- live: 200 [] -->
- [x] 1.2 consolidations_search.rs — empty 200 <!-- live: 200 [] -->
- [x] 1.3 consolidations_by_entity.rs — empty 200 (match_strategy preserved)
- [x] 1.4 consolidations_diff.rs — empty 200 <!-- live: 200 [] -->
- [x] 1.5 consolidation_costs.rs — empty 200 (group_by/buckets) <!-- live: 200 buckets:[] -->
- [x] 1.6 consolidation_lineage.rs — 404 not-found (single-doc) <!-- live: 404 -->
- [x] 1.7 consolidation_get.rs — 404 not-found (single-doc) <!-- live: 404 -->
- [x] 1.8 decision_search.rs — empty 200 <!-- live: 200 (index exists → real hits) -->
- [x] 1.9 topic_search.rs — already handled missing-index inline (returns empty TopicSearchResponse on code=index_not_found); left as-is
- [x] 1.10 law_violations.rs — empty 200 (index field preserved)
- [x] 1.11 Audited search/; tool_calls + events_by_kind (issue#4) + topic_search + the 6 above now all guard missing-index; search_proxy keyword returns structured 404 by design

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 2.1 CHANGELOG note (missing index -> 200 empty / 404 single-doc across the consolidation/decision/law read surface)
- [x] 2.2 Coverage: missing-index detection unit-tested in search.rs (is_meili_index_missing 3 tests); live before/after probes confirm each handler
- [x] 2.3 `cargo check` + `clippy -D warnings` clean; 239 search unit tests pass; live-verified recent/search/diff/costs → 200 empty, {id}/lineage → 404 (were all 502)
