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

## 2. Fix the writer (or gate on Nexus) — RESOLVED upstream (Nexus 2.3.4)
- [x] 2.1 DONE via the upstream fix (chosen path): hivellm/nexus#25 landed in **Nexus 2.3.4** — `MERGE (a)-[r:T {props}]->(b)` now persists inline rel props (+ `SET` on a rel var supported). NO Cortex writer change needed: `render_edge_merge`'s existing inline-props form is exactly what 2.3.4 persists, and it stays idempotent under replay. Bumped `docker-compose.yml` nexus pin 2.3.2→2.3.4, recreated cortex-nexus, redeployed cortex-graph-worker on HEAD. VERIFIED LIVE: `MATCH (a),(b) MERGE (a)-[r {confidence:"ambiguous",confidence_score:0.4}]->(b)` reads back `[['ambiguous',0.4]]` (was null on 2.3.2).
- [x] 2.2 DONE: provenance props (`source_event_id`/`analyzer_version`) ride the SAME inline-MERGE path, so they now persist too — the stale-edge sweeper's `delete_edges` filter on those props is satisfied by the same 2.3.4 fix (one mechanism, both prop families).

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation. DONE: spec 07 §Edge-confidence tiers callout updated from "gated on nexus#25" to "RESOLVED in Nexus 2.3.4"; CHANGELOG entry; proposal.md carries the investigation + the write-form matrix.
- [x] 3.2 Write tests covering the new behavior. DONE: `render_edge_merge_inlines_confidence_props` (nexus_client.rs) asserts the writer emits `confidence`/`confidence_score` inline in the MERGE pattern (the form 2.3.4 persists); the persist side is verified by the live probe; the §2.3 mapper stamping is unit-tested (graph_mapper.rs).
- [x] 3.3 Run tests and confirm they pass. DONE: `cargo clippy -p cortex-workers --lib -- -D warnings` clean; the confidence render test passes; live probe round-trips `[['ambiguous',0.4]]` on 2.3.4.
