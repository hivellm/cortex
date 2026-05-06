## 1. Edge extractor scaffold
- [ ] 1.1 New module `crates/cortex-workers/src/graph/extractors/` with one file per edge kind.
- [ ] 1.2 Each extractor exposes `pub fn extract(env: &Envelope, ctx: &ExtractCtx) -> Vec<Edge>`.
- [ ] 1.3 `Edge { from, to, kind, properties }` matches the Nexus schema.
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
