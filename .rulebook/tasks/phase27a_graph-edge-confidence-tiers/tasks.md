## 1. Edge model + confidence enum
- [x] 1.1 Add `EdgeConfidence { Extracted, Inferred, Ambiguous }` enum + optional `confidence_score: f32` to the edge / `NodeOp` edge model in `cortex-workers` (serde, default = absent/unknown) <!-- patch.rs: EdgeConfidence enum (snake_case, as_str/default_score) + EdgeOp::with_confidence helper stamping confidence/confidence_score props; re-exported from graph/mod.rs -->
- [x] 1.2 Thread the field through `graph/projection.rs` and the graph writer so it persists to Nexus as an edge property (additive, back-compatible) <!-- confidence rides in EdgeOp.props, which the writer already renders into Cypher generically (like the provenance triple) — persists with no writer change -->

## 2. Stamp confidence in extractors
- [x] 2.1 Tag deterministic edges as `Extracted` <!-- central: patch_builder make_edge maps ResolutionTier (LocalFile/IntraCrate/External → Extracted) for AST analyzer edges; projection EMITTED_BY/MENTIONS_FILE → Extracted -->
- [x] 2.2 Tag analyzer/LLM-derived edges as `Inferred` <!-- central: patch_builder Unresolved tier → Inferred; projection confidence_for_projected_edge maps all classifier relations (relates_to/about/answered_by/cites/calls/...) → Inferred. Central edge_type/tier classifiers chosen over editing 13 extractor files -->
- [x] 2.3 DONE: structural mapper now stamps confidence. `mapper.rs` — the classifier `rel_label` edge → `Inferred` at its site; a final pass in `map_event_to_patch` stamps every still-unstamped (structural) edge → `Extracted` (HAS_TURN/HAS_TOOL_CALL/TOUCHED/DEFINES/IN_REPO/REMEMBERS/OWNS/…). Per-site approach avoids the central-edge_type ambiguity. New test `structural_mapper_edges_carry_extracted_confidence` (tests/graph_mapper.rs); 338 graph tests pass. <!-- Original gap: the always-on structural mapper (`crates/cortex-workers/src/graph/mapper.rs::map_event_to_patch`) builds ~13 literal-edge_type EdgeOps (HAS_TURN, HAS_TOOL_CALL, IN_REPO, TOUCHED, DEFINES, OWNS, REMEMBERS, OF, OBSERVED_IN, EVIDENCE_OF, ANALYZES, LINKED_TO, RELATED_TO, SUPERSEDES) WITHOUT confidence, plus one classifier `rel_label` site (line ~258). Because the semantic projection is gated OFF in prod (CORTEX_GRAPH_PROJECTION_ENABLED=false, nexus#12), the mapper is the path that actually writes live edges — so confidence never reaches them. Fix per-site (central edge_type match is ambiguous: the classifier label set DEFINES/TOUCHED/SUPERSEDES/OBSERVED_IN overlaps the structural literals): literal sites → `.with_confidence(Extracted, None)`, the `rel_label` site → `Inferred`. Add a mapper unit test asserting structural edges are Extracted. -->

## 3. Consume confidence
- [ ] 3.1 Weight the graph lane by edge confidence in `crates/cortex-api/src/search/strategies.rs` (down-weight `Inferred`/`Ambiguous`)
- [ ] 3.2 Surface `Ambiguous` edges in the dashboard graph view (`crates/cortex-api/src/dashboard/graph.rs`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (spec 07 edge schema + confidence; CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (per-extractor confidence tagging units; graph-lane weighting test)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
