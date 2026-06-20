## 1. IDF-gated seed selection
- [ ] 1.1 Per-token IDF over node labels when resolving query terms → graph seeds in `crates/cortex-api/src/search/strategies.rs`
- [ ] 1.2 80%-of-top seed gate (only nodes scoring above 80% of the top score become BFS seeds)
- [ ] 1.3 Source-path bonus: boost graph nodes whose `source_file` contains a query term

## 2. Query primitives
- [ ] 2.1 `path(a, b)` MCP tool (shortest path between two symbols, with intermediate hops)
- [ ] 2.2 `compare(a, b)` MCP tool (shared vs. divergent neighborhoods)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation (spec 11 seed selection; CHANGELOG)
- [ ] 3.2 Write tests (IDF + 80%-gate unit tests; path/compare tool ITs; relevance-eval delta on `crates/cortex-eval`)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
