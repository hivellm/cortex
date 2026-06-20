## 1. Reproduce + characterise
- [x] 1.1 Reproduced live (2026-06-20, Nexus 2.3.2, `/cypher` host:17002): `MATCH ()-[r]->() WHERE r.confidence IS NOT NULL RETURN count(r)` = 0 across the whole graph; also 0 for `TOUCHED.operation` and the provenance triple — confirming NO edge prop persists in prod.
- [x] 1.2 Write-form matrix established empirically (fresh node pairs + fresh edge types, read-back in a separate statement):
  - `MATCH (a),(b) MERGE (a)-[r:T { k:v }]->(b)` → edge persists, **prop dropped** (`r.k` = null). ← current writer
  - `SET r.x = ...` / `SET r += {}` / `ON CREATE SET` → **rejected** (`Unknown variable 'r' in SET clause`), even after a plain `MATCH` of an existing edge.
  - `MATCH (a),(b) CREATE (a)-[r:T { k:v }]->(b)` (standalone) → **prop persists** (`r.k` reads back).
  - `... OPTIONAL MATCH old DELETE old CREATE (a)-[r:T {k:v}]->(b)` (combined) → **prop dropped** (Nexus only persists rel props when CREATE is the sole write clause).
  - **Idempotent recipe that works:** two statements — (A) `MATCH (a)-[old:T]->(b) DELETE old`, then (B) `MATCH (a),(b) CREATE (a)-[r:T { k:v }]->(b)`. Replay-safe (always ends with exactly one current-props edge), but **doubles writes per props-bearing edge**.

## 1b. Fix-path decision (ARCHITECTURAL — owner: user, who maintains Nexus)
- [x] 1b.1 DECIDED (2026-06-20, user): it is a genuine Nexus engine bug (CREATE persists rel props, MERGE drops them — inconsistent with openCypher MERGE semantics), so fix it **upstream**, not via a Cortex hot-path workaround. Filed **hivellm/nexus#25** ("MERGE silently drops inline relationship properties (2.3.2); CREATE persists them") with the full empirical matrix + repro. No Cortex writer change; phase27a confidence + provenance persistence are gated on a fixed Nexus release.

## 2. Gate on the upstream fix (BLOCKED on hivellm/nexus#25)
- [ ] 2.1 ⏸ blocked: nexus#25 — when a Nexus release persists inline rel props on MERGE (or supports `SET` on a rel var), bump the pinned image and confirm `render_edge_merge`'s existing inline-props output persists `confidence` end-to-end (no writer change expected).
- [ ] 2.2 ⏸ blocked: nexus#25 — re-verify the stale-edge sweeper reads `analyzer_version`/`source_event_id` off persisted edges once props land.

## 2. Fix the writer (or gate on Nexus)
- [ ] 2.1 Implement an idempotent edge-prop persistence path in `render_edge_merge`/`nexus_client.rs` that survives replay (no `SET r.*`), or gate phase27a + provenance persistence on a fixed Nexus release with a tracked issue link
- [ ] 2.2 Verify the stale-edge sweeper still reads `analyzer_version`/`source_event_id` off persisted edges after the fix

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation. DONE: spec 07 §Edge-confidence tiers gained a "⚠ Persistence gated on hivellm/nexus#25" callout documenting the MERGE-drops-rel-props limitation + the write-form matrix; proposal.md carries the full investigation.
- [ ] 3.2 ⏸ blocked: nexus#25 — writer rel-prop round-trip test (unit/integration) can only assert success once Nexus persists the props; writing it now would assert the broken behavior.
- [ ] 3.3 ⏸ blocked: nexus#25 — live read-back of `r.confidence` through the worker requires the fixed Nexus release.
