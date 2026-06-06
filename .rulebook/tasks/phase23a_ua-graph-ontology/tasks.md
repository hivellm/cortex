## 1. Crosswalk & ADR
- [ ] 1.1 Verify every current Cortex relation (`IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES`) maps to a UA edge in `03-ontology-mapping.md` §2 (no orphan)
- [ ] 1.2 Finalize the adopted node-kind list (✅ rows of `03-ontology-mapping.md` §1) and edge-kind list (§2)
- [ ] 1.3 Write ADR "Adopt UA-derived graph ontology" citing UA as prior art

## 2. Node-kind enum
- [ ] 2.1 Add adopted node kinds to the `cortex-core` graph node-kind enum (code, non-code, knowledge groups)
- [ ] 2.2 Preserve Cortex-only node kinds (`session`, `decision`, `law`, `consolidation`, `turn`, `tool_call`)
- [ ] 2.3 `cargo check` clean

## 3. Edge-kind enum & shape
- [ ] 3.1 Add adopted edge kinds to the edge-kind enum; alias existing `IMPORTS_FILE`→`imports`, `DOCUMENTED_BY`→`documents`, `CITES`→`cites`
- [ ] 3.2 Extend the edge record with `direction`, `weight`, optional `description`, `provenance`, and bitemporal `valid_from`/`valid_to`
- [ ] 3.3 Keep Cortex-only edges (`SUPERSEDES`, `GOVERNED_BY`, `DERIVED_FROM`/`CONSOLIDATES`)
- [ ] 3.4 `cargo check` clean

## 4. Nexus relation vocabulary
- [ ] 4.1 Map the edge-kind enum onto the Nexus relation vocabulary (serialization round-trip)
- [ ] 4.2 Backward-compat: existing graph reads still resolve aliased relations

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior (enum round-trip, alias resolution, bitemporal edge serialize)
- [ ] 5.3 Run tests and confirm they pass
