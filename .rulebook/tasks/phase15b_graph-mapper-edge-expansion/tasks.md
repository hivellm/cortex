## 1. Edge extractor scaffold
- [x] 1.1 New module `crates/cortex-workers/src/graph/extractors/` with one file per edge kind. — Created `crates/cortex-workers/src/graph/extractors/mod.rs` carrying the shared scaffold: `Edge` type alias over `super::patch::EdgeOp` so each per-kind file says `use super::Edge` instead of pulling in the whole patch module; `ExtractCtx { analyzer_version, now_ms }` keeps the per-extractor signature uniform so §3.1 projection can dispatch via a single function-pointer table; `ExtractCtx::new()` + `with_now_ms()` constructors so tests pin the timestamp; `stamp_provenance(edge, env, ctx)` helper stamps `source_event_id` + `analyzer_version` + `created_at_ms` on every emitted edge so the stale-edge sweeper at `crates/cortex-workers/src/graph/stale_sweeper.rs` can retire output deterministically. Module-level docstring documents the per-extractor contract (pure → idempotent under (from, to, kind) constraint; empty on miss; stamp provenance). Registered `pub mod extractors;` in `crates/cortex-workers/src/graph/mod.rs`. Tests module exposes a `pub(crate) fixture_envelope(event_id, kind)` helper so the per-kind extractor files (§2.1 onward) reuse the same `EnrichedEvent` fixture builder. Per-edge-kind files (`calls.rs`, `imports.rs`, …) land one-by-one in §2.1-§2.12 commits so each extractor commit is reviewable in isolation.
- [x] 1.2 Each extractor exposes `pub fn extract(env: &Envelope, ctx: &ExtractCtx) -> Vec<Edge>`. — Signature pinned in the scaffold module-level docstring; `EnrichedEvent` (the canonical post-redaction post-classification envelope at `crates/cortex-workers/src/embedder/embedder.rs::EnrichedEvent`) plays the `&Envelope` role since the mapper already routes through it. Each §2.x extractor file lands one `pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge>` plus a `pub(super) fn name() -> &'static str` (constant edge type label) so §3.1 dispatch is table-driven.
- [x] 1.3 `Edge { from, to, kind, properties }` matches the Nexus schema. — Re-used the existing `super::patch::EdgeOp` (already on-wire via the mapper / coalescer / writer path) under the `Edge` type alias. EdgeOp's fields (`edge_type`, `from_label`, `from_key`, `to_label`, `to_key`, `props`) cover the spec's `{from, to, kind, properties}` shape — `from` decomposes into `(from_label, from_key)` and `to` into `(to_label, to_key)` to match Nexus's `MATCH (a:Label {natural_key}) MATCH (b:Label {natural_key}) MERGE (a)-[r:EDGE_TYPE]->(b)` semantics so the writer's `(from, to, kind)` unique constraint maps 1:1. Tests pinning the round-trip live in `patch.rs::tests::{patch_serde_round_trips, node_op_serde_round_trips_with_external_id_and_policy}`; the extractor scaffold inherits the same serde contract via the type alias.
- [ ] 1.4 Per-extractor unit test against the fixture corpus (5 envelopes per extractor).

## 2. 10 extractors
- [ ] 2.1 `CALLS` — function-call extractions from tool-call envelopes.
- [ ] 2.2 `IMPORTS` — import statement parsing from code snippets in envelope payloads.
- [ ] 2.3 `DEFINES` — symbol definitions from code snippets.
- [ ] 2.4 `RETURNS` — return-value annotations from tool-call results.
- [ ] 2.5 `SUPERSEDES` — decision supersession (already partially in DecisionLanded; promote here).
- [ ] 2.6 `CONTRADICTS` — topic-card contradiction detection (sourced from phase14 contradiction work).
- [ ] 2.7 `EMITTED_BY` — every event → producer node.
- [ ] 2.8 `ABOUT` — turn or decision → topic node.
- [ ] 2.9 `ANSWERED_BY` — turn → answering tool-call.
- [ ] 2.10 `CITES` — turn references decision_id / event_id explicitly.
- [ ] 2.11 `MENTIONS_FILE` — turn mentions a file path.
- [ ] 2.12 `RELATES_TO` — fallback semantic-similarity edge between turns (cosine ≥ 0.85).

## 3. Projection wire-up
- [ ] 3.1 `graph::projection::project_envelope(env, ctx)` runs all 12 extractors and batches edge writes to Nexus.
- [ ] 3.2 Idempotent: re-projecting the same envelope is a no-op (unique edge constraint on `(from, to, kind)`).
- [ ] 3.3 Backfill subcommand `cortex-ops graph backfill --since <RFC3339>` re-projects existing envelopes.

## 4. Doctor + coverage
- [ ] 4.1 `cortex-ops doctor graph-coverage` reports edge count per kind across the live graph.
- [ ] 4.2 Threshold: every kind MUST have ≥1% of total edges; warn if any falls below.
- [ ] 4.3 Live smoke: backfill against current Cortex graph; doctor reports all 12 kinds present.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/07-graph.md` § Edge taxonomy + `CHANGELOG.md`.
- [ ] 5.2 Tests: §1.4 × 10 + §3 idempotency IT + §4.3 live coverage.
- [ ] 5.3 `cargo check --workspace && cargo clippy -p cortex-workers -- -D warnings && cargo test -p cortex-workers graph` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
