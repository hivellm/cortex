## 1. Reconcile with phase23c
- [ ] 1.1 Review `phase23c_ua-extraction-contract`; fold SCIP in as the precise-extraction backend rather than duplicating scope

## 2. SCIP ingestion (Rust first)
- [ ] 2.1 New `cortex-scip` module: parse a SCIP index (JSON) into symbols + occurrences
- [ ] 2.2 Two-pass resolver: build symbol→node-id index, then emit `calls`/`references`/`defines` edges resolving exact targets (same-document precedence, global fallback)
- [ ] 2.3 Stub unresolved targets as `scip_external` nodes so edges never dangle
- [ ] 2.4 Run `rust-analyzer scip` in bootstrap/CI; emit edges tagged `Extracted` (1.0), superseding heuristic edges where SCIP covers the file

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation (spec 07 precise edges + `scip_external`; bootstrap/CI indexer step; CHANGELOG; ADR for SCIP adoption)
- [ ] 3.2 Write tests (SCIP parse + two-pass resolver units; `scip_external` stub; dangling-edge guard)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
