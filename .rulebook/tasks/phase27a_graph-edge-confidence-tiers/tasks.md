## 1. Edge model + confidence enum
- [x] 1.1 Add `EdgeConfidence { Extracted, Inferred, Ambiguous }` enum + optional `confidence_score: f32` to the edge / `NodeOp` edge model in `cortex-workers` (serde, default = absent/unknown) <!-- patch.rs: EdgeConfidence enum (snake_case, as_str/default_score) + EdgeOp::with_confidence helper stamping confidence/confidence_score props; re-exported from graph/mod.rs -->
- [x] 1.2 Thread the field through `graph/projection.rs` and the graph writer so it persists to Nexus as an edge property (additive, back-compatible) <!-- confidence rides in EdgeOp.props, which the writer already renders into Cypher generically (like the provenance triple) — persists with no writer change -->

## 2. Stamp confidence in extractors
- [x] 2.1 Tag deterministic edges as `Extracted` <!-- central: patch_builder make_edge maps ResolutionTier (LocalFile/IntraCrate/External → Extracted) for AST analyzer edges; projection EMITTED_BY/MENTIONS_FILE → Extracted -->
- [x] 2.2 Tag analyzer/LLM-derived edges as `Inferred` <!-- central: patch_builder Unresolved tier → Inferred; projection confidence_for_projected_edge maps all classifier relations (relates_to/about/answered_by/cites/calls/...) → Inferred. Central edge_type/tier classifiers chosen over editing 13 extractor files -->

## 3. Consume confidence
- [ ] 3.1 Weight the graph lane by edge confidence in `crates/cortex-api/src/search/strategies.rs` (down-weight `Inferred`/`Ambiguous`)
- [ ] 3.2 Surface `Ambiguous` edges in the dashboard graph view (`crates/cortex-api/src/dashboard/graph.rs`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (spec 07 edge schema + confidence; CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (per-extractor confidence tagging units; graph-lane weighting test)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
