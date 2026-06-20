## 1. Reproduce + characterise
- [x] 1.1 Reproduced live (2026-06-20, Nexus 2.3.2, `/cypher` host:17002): `MATCH ()-[r]->() WHERE r.confidence IS NOT NULL RETURN count(r)` = 0 across the whole graph; also 0 for `TOUCHED.operation` and the provenance triple — confirming NO edge prop persists in prod.
- [x] 1.2 Write-form matrix established empirically (fresh node pairs + fresh edge types, read-back in a separate statement):
  - `MATCH (a),(b) MERGE (a)-[r:T { k:v }]->(b)` → edge persists, **prop dropped** (`r.k` = null). ← current writer
  - `SET r.x = ...` / `SET r += {}` / `ON CREATE SET` → **rejected** (`Unknown variable 'r' in SET clause`), even after a plain `MATCH` of an existing edge.
  - `MATCH (a),(b) CREATE (a)-[r:T { k:v }]->(b)` (standalone) → **prop persists** (`r.k` reads back).
  - `... OPTIONAL MATCH old DELETE old CREATE (a)-[r:T {k:v}]->(b)` (combined) → **prop dropped** (Nexus only persists rel props when CREATE is the sole write clause).
  - **Idempotent recipe that works:** two statements — (A) `MATCH (a)-[old:T]->(b) DELETE old`, then (B) `MATCH (a),(b) CREATE (a)-[r:T { k:v }]->(b)`. Replay-safe (always ends with exactly one current-props edge), but **doubles writes per props-bearing edge**.

## 1b. Fix-path decision (ARCHITECTURAL — owner: user, who maintains Nexus)
- [ ] 1b.1 Decide: (A) Cortex writer workaround (two-statement delete+create for props-bearing edges — 2× write volume on already-strained Nexus, nexus#12/phase25) vs (B) upstream Nexus fix (MERGE must persist inline rel props, or support `SET` on a rel var) + gate phase27a/provenance persistence on the fixed release (mirrors nexus#12 pattern). Blocked pending this decision.

## 2. Fix the writer (or gate on Nexus)
- [ ] 2.1 Implement an idempotent edge-prop persistence path in `render_edge_merge`/`nexus_client.rs` that survives replay (no `SET r.*`), or gate phase27a + provenance persistence on a fixed Nexus release with a tracked issue link
- [ ] 2.2 Verify the stale-edge sweeper still reads `analyzer_version`/`source_event_id` off persisted edges after the fix

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (spec 07 edge-prop persistence contract + the write-form matrix)
- [ ] 3.2 Write tests covering the new behavior (writer rel-prop round-trip unit/integration)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`); plus a live read-back of `r.confidence` through the worker
