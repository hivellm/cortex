## 1. Crosswalk & ADR
- [x] 1.1 Verify every current Cortex relation (`IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES`) maps to a UA edge in `03-ontology-mapping.md` §2 (no orphan) — all 18 EdgeType variants verified: 8 UA-mapped, 10 Cortex-only extensions; session-graph RELATIONSHIPS all Cortex-only; no orphans
- [x] 1.2 Finalize the adopted node-kind list (✅ rows of `03-ontology-mapping.md` §1) and edge-kind list (§2) — 17 node kinds, 22 edge kinds adopted; lists in ADR #35
- [x] 1.3 Write ADR "Adopt UA-derived graph ontology" citing UA as prior art — ADR #35 created (adopt-ua-derived-graph-ontology-as-canonical-nexus-relation-vocabulary)

## 2. Node-kind enum
- [x] 2.1 Add adopted node kinds to the `cortex-core` graph node-kind enum (code, non-code, knowledge groups) — `NodeKind` enum (39 variants) added to `cortex-storage/src/graph.rs`
- [x] 2.2 Preserve Cortex-only node kinds (`session`, `decision`, `law`, `consolidation`, `turn`, `tool_call`) — all 22 Cortex-only node kinds preserved
- [x] 2.3 `cargo check` clean — cortex-storage + cortex-workers pass

## 3. Edge-kind enum & shape
- [x] 3.1 Add adopted edge kinds to the edge-kind enum; alias existing `IMPORTS_FILE`→`imports`, `DOCUMENTED_BY`→`documents`, `CITES`→`cites` — 15 new UA variants added to `EdgeType`; `ua_name()` method documents aliases; `from_nexus_label()` handles both canonical and legacy strings
- [x] 3.2 Extend the edge record with `direction`, `weight`, optional `description`, `provenance`, and bitemporal `valid_from`/`valid_to` — `EdgeDirection` enum + 6 optional fields on `EdgeOp` in `patch.rs`; all `..Default::default()` at 25+ construction sites
- [x] 3.3 Keep Cortex-only edges (`SUPERSEDES`, `GOVERNED_BY`, `DERIVED_FROM`/`CONSOLIDATES`) — preserved in `temporal_edges.rs` constants and `EdgeType` variants
- [x] 3.4 `cargo check` clean — full workspace check + clippy -D warnings both clean

## 4. Nexus relation vocabulary
- [x] 4.1 Map the edge-kind enum onto the Nexus relation vocabulary (serialization round-trip) — `EdgeType::label()` (forward) + `EdgeType::from_nexus_label()` (reverse) cover all 33 variants
- [x] 4.2 Backward-compat: existing graph reads still resolve aliased relations — `from_nexus_label` accepts `"IMPORTS_FILE"`, `"IMPORTS"`, `"DOCUMENTED_BY"`, `"CITES"` as legacy strings mapping to the appropriate variants

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — ADR #35 created; task tasks.md updated; NodeKind/EdgeType/EdgeOp all carry doc-comments with UA references
- [x] 5.2 Write tests covering the new behavior (enum round-trip, alias resolution, bitemporal edge serialize) — `cortex-storage/src/graph.rs` NodeKind round-trip + ua_adopted + unknown-label tests; `cortex-workers/src/graph/patch.rs` EdgeOp UA fields serialize/omit/legacy-compat tests; `cortex-workers/src/graph/analyzer/mod.rs` EdgeType label round-trip + alias resolution + ua_name tests
- [x] 5.3 Run tests and confirm they pass — 1030 cortex-workers lib tests + 77 cortex-storage + 705 cortex-api all pass; pre-existing embedder_it_chunk_pipeline failure is unrelated to phase23a
