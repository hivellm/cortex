## 1. Edge model + confidence enum
- [ ] 1.1 Add `EdgeConfidence { Extracted, Inferred, Ambiguous }` enum + optional `confidence_score: f32` to the edge / `NodeOp` edge model in `cortex-workers` (serde, default = absent/unknown)
- [ ] 1.2 Thread the field through `graph/projection.rs` and the graph writer so it persists to Nexus as an edge property (additive, back-compatible)

## 2. Stamp confidence in extractors
- [ ] 2.1 Tag deterministic tree-sitter edges (`defines`, `imports`, `calls`, `returns`, `inherits`) as `Extracted` (score 1.0) in `crates/cortex-workers/src/graph/extractors/`
- [ ] 2.2 Tag analyzer/LLM-derived edges (`relates_to`, `about`, `answered_by`, `mentions_file`, `cites`) as `Inferred` with a documented rubric score; reserve `Ambiguous` for below-threshold matches

## 3. Consume confidence
- [ ] 3.1 Weight the graph lane by edge confidence in `crates/cortex-api/src/search/strategies.rs` (down-weight `Inferred`/`Ambiguous`)
- [ ] 3.2 Surface `Ambiguous` edges in the dashboard graph view (`crates/cortex-api/src/dashboard/graph.rs`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (spec 07 edge schema + confidence; CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (per-extractor confidence tagging units; graph-lane weighting test)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
