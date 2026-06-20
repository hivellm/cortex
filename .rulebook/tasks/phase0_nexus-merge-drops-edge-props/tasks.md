## 1. Reproduce + characterise
- [ ] 1.1 Add a live/integration probe asserting Nexus persists relationship props written by the worker (write edge via `LiveNexusClient`, read `r.confidence` back) — currently fails on 2.3.2
- [ ] 1.2 Confirm which write forms persist rel props on the pinned Nexus (MERGE inline = drops; CREATE inline = persists; UNWIND+MERGE = TBD) and record the matrix in the spec

## 2. Fix the writer (or gate on Nexus)
- [ ] 2.1 Implement an idempotent edge-prop persistence path in `render_edge_merge`/`nexus_client.rs` that survives replay (no `SET r.*`), or gate phase27a + provenance persistence on a fixed Nexus release with a tracked issue link
- [ ] 2.2 Verify the stale-edge sweeper still reads `analyzer_version`/`source_event_id` off persisted edges after the fix

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (spec 07 edge-prop persistence contract + the write-form matrix)
- [ ] 3.2 Write tests covering the new behavior (writer rel-prop round-trip unit/integration)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`); plus a live read-back of `r.confidence` through the worker
