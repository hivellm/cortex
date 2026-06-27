## 1. Reconcile with phase23c
- [x] 1.1 Reviewed `phase23c_ua-extraction-contract` (archived 2026-06-24) + `graph/extraction_contract.rs` + the heuristic extractors (`extractors/{calls,defines,imports}.rs`). DECISION: SCIP is a **higher-precision Phase-1 (deterministic-facts) backend**, NOT a new pipeline. It emits the SAME `EdgeConfidence::Extracted` (score 1.0) edges that flow through the existing reconciliation gate (`extraction_contract`), and the same `Symbol` node label the contract's significance filter already gates on. Where SCIP covers a file it SUPERSEDES the heuristic/classifier-relation edges (today `extractors/*` produce classifier-derived edges keyed on `Artifact = repo|path|content_hash`); SCIP adds function-level `Symbol` nodes + precise `CALLS`/`REFERENCES`/`DEFINES` edges with exact target resolution. No scope duplication — SCIP plugs into the FactSet/gate as the authoritative deterministic source. GATING KNOWLEDGE for §2: the parser/resolver needs the EXACT `rust-analyzer scip` JSON schema (SCIP `Index`/`Document`/`Occurrence`/`SymbolInformation`, the `symbol_roles` bitfield, the `local …` / `scheme manager pkg ver descriptors` symbol grammar) captured from REAL rust-analyzer output, plus the `Symbol`-node key scheme + same-document-precedence resolution design — these are real design choices best fixed in a focused session (as phase27b's algorithm was), not coded against a from-memory fixture.

## 2. SCIP ingestion (Rust first)
- [ ] 2.1 New `cortex-scip` module: parse a SCIP index (JSON) into symbols + occurrences
- [ ] 2.2 Two-pass resolver: build symbol→node-id index, then emit `calls`/`references`/`defines` edges resolving exact targets (same-document precedence, global fallback)
- [ ] 2.3 Stub unresolved targets as `scip_external` nodes so edges never dangle
- [ ] 2.4 Run `rust-analyzer scip` in bootstrap/CI; emit edges tagged `Extracted` (1.0), superseding heuristic edges where SCIP covers the file

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation (spec 07 precise edges + `scip_external`; bootstrap/CI indexer step; CHANGELOG; ADR for SCIP adoption)
- [ ] 3.2 Write tests (SCIP parse + two-pass resolver units; `scip_external` stub; dangling-edge guard)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
