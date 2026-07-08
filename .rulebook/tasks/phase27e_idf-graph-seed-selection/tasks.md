## 1. IDF-gated seed selection
- [x] 1.1 Per-token IDF seed resolution: new pure `search/graph_seeds.rs` (tokenizer that keeps snake_case identifiers whole AND emits their parts, ~25-entry stopword list, smoothed `idf = ln(1 + N/(1+df))`, deterministic `select_seeds`, cap 5) + lane integration in `lanes/nexus_graph_lane.rs`: for the strategy templates that match on node text (`SEED_FAN_OUT_TEMPLATES`), the lane tokenizes the query, probes per-token document frequency via template-mirrored COUNT Cypher (LRU-cached, 1024 entries, keyed (template, token)), and runs the template once per surviving seed (dedup by (edge_from, edge_to, edge_type)) instead of one whole-string CONTAINS pass. Fallback preserved: zero surviving seeds → the original raw-query single pass, so nothing regresses. (Implemented in the lane rather than strategies.rs — the DF probes need the Nexus client, which only the lane holds; strategies stays pure.)
- [x] 1.2 80%-of-top gate: `DEFAULT_TOP_GATE = 0.8` — only tokens with `idf >= 0.8 × max_idf` survive as seeds (tested: a dominant rare token excludes common ones; near-equal scores all survive).
- [x] 1.3 Source-path bonus: `SOURCE_PATH_BONUS = 1.2` multiplier on the hit's native score when its label/path contains any seed token (applied in row projection; tested).

## 2. Query primitives
- [ ] 2.1 `path(a, b)` MCP tool (shortest path between two symbols, with intermediate hops)
- [ ] 2.2 `compare(a, b)` MCP tool (shared vs. divergent neighborhoods)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation (spec 11 seed selection; CHANGELOG)
- [ ] 3.2 Write tests (IDF + 80%-gate unit tests; path/compare tool ITs; relevance-eval delta on `crates/cortex-eval`)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
