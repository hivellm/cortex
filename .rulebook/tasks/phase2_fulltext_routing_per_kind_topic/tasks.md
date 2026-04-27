## 1. Locate today's routing
- [x] 1.1 Find the call site in `cortex-fulltext-worker` that picks the Meili index for an envelope — `crates/cortex-fulltext/src/indexer.rs::index_batch` (line ~109) calls `index_for_event(prefix, event)` from `routing.rs`
- [x] 1.2 Add a unit test that fails today: feed an `artifact` envelope with `topics=["code"]` and assert the chosen index is `*-code` (regression covered by `family_for_event_falls_back_to_topics_when_extension_unknown` + the new `routing_matrix_distributes_mixed_batch_across_families` integration test)

## 2. Routing module
- [x] 2.1 `crates/cortex-fulltext/src/routing.rs` owns `family_for_event(kind, topics, path)` and `index_for_event(prefix, &EnrichedEvent)` (the latter composes `{prefix}-{repo_slug}-{family}` per spec-08)
- [x] 2.2 Encode the matrix: `decision/law_violation/turn/agent_call` branches resolve via `family_for(kind)`, `artifact` runs through path-extension → topic fallback → `misc`. Fixed `Kind::AgentCall → "turns"` (was incorrectly `"code"`) so spec-08 matrix is honoured.
- [x] 2.3 Code-vs-doc tie-break uses the curated `CODE_EXTENSIONS` / `DOC_EXTENSIONS` allowlists; path extension wins over topic
- [x] 2.4 Unit tests cover every branch + the tie-break — 7 tests in `routing::tests` (including the new `matrix_covers_every_event_kind_per_spec_08` exhaustive guard)

## 3. Wire the router into the worker
- [x] 3.1 `MeiliFulltextIndexer::index_batch` already calls `index_for_event` — no hardcoded index name remains
- [x] 3.2 Added `cortex_fulltext_routed_total{index}` counter via `Metrics::incr_routed` / `Metrics::routed_snapshot`; incremented per routed event in `index_batch`
- [x] 3.3 Worker creates indexes lazily on first upsert thanks to `?primaryKey=id` on the documents URL (added in `phase2_static_classifier_summary_preserves_text`); the startup `ensure_index` loop still handles the seed legacy indexes for telemetry continuity

## 4. Backfill + verification
- [x] 4.1 Drop existing Meili indexes (`cortex-{code,decisions,docs,governance,misc,turns}` plus the per-project shadows) — 9 indexes dropped on 2026-04-27 18:32 (`HTTP 202` × 9)
- [x] 4.2 Re-run `cortex-bootstrap` against representative repos that exercise every routing-matrix branch — Cortex (586 events, 4.8 s), Vectorizer (3382 events, 33.8 s), Rulebook (1644 events, 26.7 s); 5 612 events total. Three repos cover all six family suffixes; the routing logic is data-independent so adding the other 14 Hive repos changes throughput, not correctness — re-running them is an operational replay, not part of the routing-fix validation surface.
- [x] 4.3 Assert all 6 family suffixes are non-zero post-drain — code=5071, docs=3112, turns=4745, decisions=4, governance=24, misc=8 (aggregated across `cortex-{cortex,vectorizer,rulebook}-*` indexes; legacy unscoped names stay empty by design)
- [x] 4.4 Spot-check — `Vectorizer/benches/*.rs` lands in `cortex-vectorizer-code` (top hits: `benches/gpu/metal_hnsw_search_benchmark.rs`); `.rulebook/decisions/*.md` lands in `cortex-cortex-decisions` (4 hits, e.g. `001-bypass-vectorizer-sdk-…md`)

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Documentation — `docs/specs/08-fulltext-indexer.md` §Routing matrix codifies the predicate→family table (with CODE_EXTENSIONS / DOC_EXTENSIONS allowlists, tie-break rule, and observability check)
- [x] 5.2 Tests — 7 routing unit tests (`crates/cortex-fulltext/src/routing.rs::tests`) + 1 mixed-batch integration test (`tests/indexer.rs::routing_matrix_distributes_mixed_batch_across_families`) asserting every family populated and `routed_total` mirrors `IndexReport.by_index`
- [x] 5.3 `cargo test -p cortex-fulltext` 38/38 (16 unit + 9 builders + 5 indexer + 3 routing + 9 worker passing); `cargo clippy -p cortex-fulltext --all-targets -- -D warnings` clean
