## 1. P0 — Preconditions + baseline
- [ ] 1.1 Capture a pre-update retrieval baseline: run the MCP query battery (one lexical + one paraphrase/semantic + one similar-problems query via `cortex_query`) and snapshot results into `docs/analysis/phase22-baseline/mcp-pre.json`. Record per-hit `source` (keyword/vector/graph) so the post-update delta is provable.
- [ ] 1.2 Snapshot `cortex-eval` baseline: run `--suite retrieval --suite consolidation --suite classification` against the live stack into `docs/analysis/phase22-baseline/eval-pre.json` (expected: retrieval MRR low / vector lane absent).
- [ ] 1.3 Backend-capability assertions (the gate inputs): probe + record deployed versions and capabilities into `docs/analysis/phase22-baseline/backend-caps.json` — Vectorizer dense-provider check (create a 768 probe collection, assert reported provider is NOT `bm25`; `/embed` honours a dense model), Nexus `$param` bind check (`RETURN $x` returns the value), Nexus property-corruption check (no property-less straggler cohort after a fresh seed). Delete probe artifacts after.

## 2. P1 — Dense lane validation (gated on vectorizer#306)
- [ ] 2.1 Precondition: vectorizer#306 shipped — the deployed Vectorizer serves a dense embedding provider at dim 768. (If not: this whole section stays blocked per LAW-CORTEX-001 exemption 2.)
- [ ] 2.2 Flip `CORTEX_EMBEDDER_DIM` 512 → 768 (`.env` + any worker env in `docker-compose.yml`); apply the dense Vectorizer provider config; re-create the bitemporal/classification reindex aliases if dim-bound.
- [ ] 2.3 Re-index the Cortex repo (`cortex-bootstrap .`); assert every `cortex-cortex-*` collection reports a dense provider at dim 768 (not `bm25`/512) and `vector_count > 0`.
- [ ] 2.4 Re-run the MCP battery from §1.1; assert the paraphrase/semantic query now returns at least one `source: vector` hit AND the top-K is topically relevant (no longer the generic-file failure mode). Record into `docs/analysis/phase22-baseline/mcp-post-dense.json`.
- [ ] 2.5 Assert `cortex-eval --suite retrieval` MRR@10 ≥ 0.60 (the phase14c floor) with the dense lane live; record the delta vs §1.2.

## 3. P2 — Graph lane validation + workaround removal (gated on nexus#3/#4)
- [ ] 3.1 Precondition: nexus#3 (param binding) + nexus#4 (property corruption) shipped on the deployed Nexus. (If not: this section stays blocked.)
- [ ] 3.2 Assert parameterized Cypher binds: `RETURN $name` with `{name:"x"}` returns `"x"`, and `MATCH (d:Decision) WHERE d.id = $id` returns the row. Pin this as a smoke IT (`nexus_param_binding_smoke_it`).
- [ ] 3.3 Remove the inline-literal + `sanitize_literal` workaround in the graph writer (`crates/cortex-workers/src/graph/cypher.rs` + `writer.rs`), switching node/edge MERGE/SET back to `$param` binds. Unit tests + a write→read IT confirming properties persist.
- [ ] 3.4 Remove the same workaround in the operator CLIs (`crates/cortex-cli/src/bin/cortex-ops/{timeline.rs,branch_cmd.rs,query_cmd.rs,backfill_cross_project.rs}`) — replace inline-literal interpolation + `sanitize_literal` with parameter binds.
- [ ] 3.5 Remove the workaround in the HTTP routes (`crates/cortex-api/src/search/timeline_routes.rs`) + verify the graph lane templates (`crates/cortex-api/src/lanes/nexus_graph_lane.rs` — `cross_project_ref` + the 4 strategy templates use `$q` already) bind correctly now. Update the doc-comments that cite the "Nexus 2.2.0 parameter-binding bug".
- [ ] 3.6 Re-seed + assert no property-less straggler cohort remains (`MATCH (n) WHERE size(keys(n)) = 0 RETURN count(n)` is within an acceptable floor); assert the graph lane returns within its budget slice on a default-budget `cortex_query` (no `graph: budget exceeded`).
- [ ] 3.7 Full `cargo test --workspace` + `cargo clippy --workspace -- -D warnings` green after the workaround removal (the inline-literal helpers + their unit tests are deleted, not left dead).

## 4. P3 — Labelled corpus → unblock phase18 eval gates
- [ ] 4.1 Author the labelled time-sensitive query corpus: populate `tests/golden/retrieval.csv` `expected_event_ids` for the temporal subset (queries whose correct answer is a specific bitemporal fact) so the phase18 §3.8 +10% MRR gate is measurable.
- [ ] 4.2 Author the labelled cross-project query subset (queries that should pull a sibling-project fact via `CROSS_PROJECT_REF`) for the phase18 §5.4 gate, with `source_project` provenance expectations.
- [ ] 4.3 Run the phase18 §3.8 gate: temporal-classifier ON vs OFF over the labelled subset; record the MRR delta; assert ≥ +10%. Flip phase18 tasks.md §3.8 from blocked to done with the measured number.
- [ ] 4.4 Run the phase18 §5.4 gate: cross-project ON vs OFF; record the positive delta + per-candidate `source_project` provenance verification. Flip phase18 tasks.md §5.4 from blocked to done.
- [ ] 4.5 Update `docs/specs/31-temporal-classifier.md` + `docs/specs/34-cross-project-axis.md` with the measured gate results (status badges blocked → shipped).

## 5. P4 — Full hybrid acceptance + Synap observability + CI re-enable
- [ ] 5.1 Run the whole `cortex-eval` battery green against the recovered stack: retrieval (MRR@10 ≥ 0.60, recall@5 ≥ 0.50), consolidation (entity-recall ≥ 0.85), classification (macro-F1 ≥ 0.90), and the phase21 access-control suite (false-grant count = 0). Record into `docs/analysis/phase22-baseline/eval-post.json`.
- [ ] 5.2 Fused-result assertion: a single `cortex_query` returns hits from all three lanes (`source` ∈ {keyword, vector, graph}) — prove the hybrid is whole again, not keyword-only.
- [ ] 5.3 Synap observability (gated on synap#196): assert `/metrics` now exposes per-stream length + consumer-group lag; wire a `cortex-ops doctor` probe that fails when any consumer group lag exceeds a threshold. (If synap#196 not shipped: this item stays blocked; the rest of P4 proceeds.)
- [ ] 5.4 Re-enable the CI workflows disabled during the degraded window: `gh workflow enable "Doctor consistency gate" / "eval" / "Relevance harness gate"`; confirm a triggered run of each passes against the recovered stack.
- [ ] 5.5 Author `docs/runbooks/post-backend-update-validation.md` capturing the full validation flow + the gate thresholds so the next backend bump re-runs it deterministically.

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation — the phase22 runbook, the updated specs 31/34, the `docs/analysis/phase22-baseline/` snapshots, and the removed-workaround notes in the graph code doc-comments.
- [ ] 99.2 Write tests covering the new behavior — `nexus_param_binding_smoke_it`, the graph write→read property IT, the labelled-corpus eval gates, the fused-three-lanes IT, and the Synap lag probe.
- [ ] 99.3 Run tests and confirm they pass — `cargo check --workspace` + `cargo clippy --workspace -- -D warnings` clean, all unit + IT green, and the full `cortex-eval` battery at or above every phase14c floor with all three lanes contributing.
