## 1. Dependency gate (Nexus phase9 status check)

- [x] 1.1 Track Nexus `phase9_external-node-ids` status in `.rulebook/PLANS.md`; this Cortex task does NOT start §2 onward until Nexus §4 (Cypher executor branches) AND §5 (REST/RPC/SDK) land in a tagged Nexus 2.x release
- [x] 1.2 Smoke IT `crates/cortex-workers/tests/nexus_external_id_smoke_it.rs` (gated on `CORTEX_NEXUS_EXTERNAL_ID_IT=1`) — issues `CREATE (n:Artifact {_id: 'sha256:test', name: 'foo'}) ON CONFLICT MATCH` against the live Nexus, verifies the round-trip via `GET /nodes/by-external-id/sha256:test`, and asserts `RETURN n._id` projects the original prefixed string
- [x] 1.3 Pin minimum SDK in `Cargo.toml` workspace deps (`nexus-graph-sdk = "2.x"`) once §1.2 is green; document the version bump in `CHANGELOG.md` under "Changed"

## 2. NodeOp surface + identity helpers

- [x] 2.1 Extend `crates/cortex-workers/src/graph/patch.rs::NodeOp` with `pub external_id: Option<String>` and `pub conflict_policy: ConflictPolicy` (new enum `Match` | `Replace` | `Error`, `Match` as `Default`)
- [x] 2.2 Update every callsite that constructs a `NodeOp` to populate `external_id` from the `natural_key` field (no value change — the same string fills the new slot); the `props["natural_key"]` stamp stays for one transitional release as a soft fallback
- [x] 2.3 Add `crates/cortex-workers/src/graph/identity.rs::external_id_for_node(label, natural_key)` helper that returns the canonical `_id` string (currently identity, but factoring this through one helper means future per-label format changes land in one place)
- [x] 2.4 Unit tests: every `NodeOp` constructor path produces a populated `external_id`; serde round-trip preserves the new field; `ConflictPolicy::default() == Match`

## 3. Cypher templates

- [ ] 3.1 Rewrite `crates/cortex-workers/cypher/node_*.cypher` (twelve files: `node_{analysis,artifact,decision,law,law_violation,memory,repo,session,symbol,tool_call,turn}.cypher` plus the phase11k-introduced `node_{external_package,unresolved_import,doc_section}.cypher` if present): `MERGE (n:Label { natural_key: row.key }) SET n += row.props` → `CREATE (n:Label {_id: row.key}) ON CONFLICT MATCH SET n += row.props`
- [ ] 3.2 Verify edge templates (`crates/cortex-workers/cypher/edge_*.cypher`) keep their existing `MATCH … MERGE` shape — they already match endpoints on the endpoint's identity property and benefit transparently from the index seek when that property is `_id`
- [ ] 3.3 Per-template unit test in `crates/cortex-workers/src/graph/cypher.rs::tests` asserting the rendered Cypher contains `_id: row.key` and `ON CONFLICT MATCH` for every node template
- [ ] 3.4 Compatibility test against Nexus 2.x sandbox: replay a 100-node fixture patch through both the legacy MERGE path and the new ON CONFLICT MATCH path; assert the resulting graph state is identical (same node count, same property bag, same edge cardinality)

## 4. Schema bootstrap rewrite

- [ ] 4.1 `crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS` drops the seven `natural_key`-keyed CONSTRAINT statements (`artifact_natural_key`, `symbol_natural_key`, `external_package_natural_key`, `unresolved_import_natural_key`, `doc_section_natural_key`, plus any phase11k-introduced equivalents); add a comment block citing this task explaining the supersession
- [ ] 4.2 Keep secondary identity constraints (`session_id`, `turn_id`, `tool_call_id`, `decision_id`, `memory_id`, `analysis_id`, `law_id`, `violation_id`, `repo_name`, `spec_path`) — those properties also serve as the `_id` value, so the constraint is belt-and-braces against an SDK regression
- [ ] 4.3 Doctor extension `crates/cortex-cli/src/ops/doctor.rs::check_node_op_external_id` — sample a small batch of envelopes from the archive partition, parse the embedded graph patch, and assert every NodeOp carries `external_id: Some(_)`; surface a clear PASS/FAIL line in the doctor's report
- [ ] 4.4 Schema-bootstrap IT against Nexus 2.x: drop a fresh DB, apply `SCHEMA_STATEMENTS`, assert the seven dropped constraints are absent and the kept ones are present

## 5. UNKNOWN_CONTENT_HASH sentinel rewrite

- [ ] 5.1 Replace `crates/cortex-workers/src/graph/analyzer/patch_builder.rs::UNKNOWN_CONTENT_HASH = "*"` with a deterministic placeholder format `format!("pending|{repo}|{path}")` (no content_hash slot); export it as `pub const PENDING_ARTIFACT_PREFIX: &str = "pending|"`
- [ ] 5.2 Patch-builder unit test pinning the placeholder shape: a tier-2 IMPORTS_FILE edge whose target hash is unknown emits `to_key = "pending|cortex|src/module.rs"` instead of `"cortex|src/module.rs|*"`; the conflict policy on that node is still `Match` so two unknowns keyed on the same `(repo, path)` collapse correctly
- [ ] 5.3 Stale-sentinel sweeper extension — extend the phase11k §5.3 stale-edge sweeper to also redirect placeholder nodes once the canonical hash arrives: when a real `:Artifact` lands with `_id = "cortex|src/module.rs|sha256:abc"`, the sweeper deletes the placeholder `_id = "pending|cortex|src/module.rs"` after re-pointing every `IMPORTS_FILE` edge from the placeholder to the canonical artifact
- [ ] 5.4 IT `crates/cortex-workers/tests/graph_pending_to_canonical_it.rs` — emit a tier-2 IMPORTS_FILE patch with unknown sibling hash, assert placeholder created; emit the canonical sibling artifact patch; run the sweeper; assert (a) placeholder removed (b) edges re-pointed (c) no orphan edges remain

## 6. Bootstrap envelope shape change

- [ ] 6.1 `crates/cortex-cli/src/bootstrap/graph_static.rs::build_envelope` writes `nodes[*]._id` instead of `nodes[*].natural_key` in the embedded `graph_patch` payload; bump `GRAPH_STATIC_ANALYZER_VERSION` from `"phase11k.1"` → `"phase11l.1"` so the §5.4 coalescer dedupes correctly across the cutover
- [ ] 6.2 `crates/cortex-api/src/archive_loader.rs` parses both shapes during the migration window: prefer `_id` when present, fall back to `natural_key`. Emit a `tracing::warn!` once per partition when the legacy shape is hit so operators can monitor the migration tail
- [ ] 6.3 Doctor extension surfaces "graph patch envelope shape" in the doctor's report — for a sample partition, count envelopes carrying the legacy `natural_key` slot vs the new `_id` slot and exit non-zero when more than 1% of recent partitions still ship the legacy shape after the migration window closes
- [ ] 6.4 Update inline test fixtures in `graph_static.rs::tests` to assert the new envelope shape (every emitted envelope carries `nodes[*]._id`); keep ONE legacy-shape test pinning the dual-read path until §10.x removes it

## 7. Reindex (drop + replay)

- [ ] 7.1 New admin command `cortex-ops graph drop --confirm --dry-run` in `crates/cortex-cli/src/ops/graph_drop.rs` — calls Nexus's `MATCH (n) DETACH DELETE n` (or the SDK's equivalent admin surface) for the Cortex graph DB. Refuses to run without `--confirm`. `--dry-run` prints the count it would delete
- [ ] 7.2 Re-run `cortex-bootstrap --graph-static` against every workspace repo declared in `cortex-bootstrap.toml`. Document the operator runbook in `docs/cortex/external-id-migration.md` (new file): pre-migration checklist, drop, replay, post-migration verification queries, rollback notes
- [ ] 7.3 Boot graph-worker; archive_loader replays the live event stream into the new schema. Verify the live trigger from phase11k §5.2 picks up the new `_id` slot correctly (every Artifact event lands with `_id` on the resulting GraphPatch)
- [ ] 7.4 Verification IT — run the phase11i gold-set against the post-migration graph; assert `MRR@10 ≥ 0.75` (the same gate phase11i shipped); document any drift in `docs/cortex/external-id-migration.md` §Verification

## 8. Dashboard surface

- [ ] 8.1 `crates/cortex-api/src/dashboard.rs` reads `n._id` instead of `props["natural_key"]` for the canonical node identity in the graph view colour-coding; the existing `display_label` prop stays unchanged so the human-facing labels are invariant
- [ ] 8.2 Update inline dashboard tests that pin the JSON shape — assert the `id` field in the graph payload tracks `_id`, not `natural_key`
- [ ] 8.3 IT `crates/cortex-api/tests/dashboard_external_id_it.rs` — seed a small graph via the new mapper, hit `/v1/dashboard/graph`, assert every node carries an `id` field whose value matches the `_id` slot

## 9. ADR + migration documentation

- [ ] 9.1 `rulebook_decision_create` — record the supersession: "Cortex graph nodes carry their identity in Nexus's reserved `_id` slot, replacing the synthetic `natural_key` property convention shipped in phase4-phase11k". Names the quantitative reassessment trigger (Nexus query latency p95 on Artifact MERGE vs. CREATE ON CONFLICT MATCH; if the gap drops below 5% the migration value is no longer self-evident and the legacy fallback can stay)
- [ ] 9.2 `docs/cortex/external-id-migration.md` — operator runbook: pre-migration checklist, drop command, replay command, post-migration verification (gold-set IT + dashboard smoke), rollback procedure (revert the SDK pin, restore the `MERGE`-based templates from git, re-run drop + replay against the legacy schema)
- [ ] 9.3 Update `docs/specs/07-graph-writer.md` §Stable identity to point at Nexus's `_id` as the canonical identity surface; deprecate the synthetic `natural_key` convention with a "removed in phase11l_nexus-external-ids-migration" note pointing at this ADR
- [ ] 9.4 Update `docs/specs/11-query-api.md` §Read path to document `MATCH (n {_id: …})` as an index seek and the `RETURN n._id` projection shape

## 10. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 10.1 Update or create documentation covering the implementation — CHANGELOG entry under "Changed" (`feat(graph): adopt Nexus _id for all node identities`); `docs/architecture.md` §6 graph correlation layer identity story; `docs/cortex/external-id-migration.md` operator runbook (already produced by §9.2); `docs/specs/07-graph-writer.md` §Stable identity rewrite (already produced by §9.3); `docs/specs/11-query-api.md` §Read path update (already produced by §9.4)
- [ ] 10.2 Write tests covering the new behavior — every IT named in §1-§8 lands; coverage ≥ 95 % on the modified `cortex-workers/src/graph/{patch,schema,coalescer,mapper}.rs` slices; the post-migration gold-set IT is the headline acceptance gate
- [ ] 10.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`, full IT suite gated by `CORTEX_*_IT=1` (NEXUS_EXTERNAL_ID, GRAPH_PENDING_TO_CANONICAL, DASHBOARD_EXTERNAL_ID, RELEVANCE plus the phase11i headline); all green
- [ ] 10.4 Capture learnings: `rulebook_learn_capture` for any non-obvious finding from the migration (Nexus SDK 2.x ergonomics, ON CONFLICT REPLACE vs MATCH semantics in production, the `pending|repo|path` placeholder behaviour under high concurrency)
- [ ] 10.5 Capture decision: §9.1 produces the ADR. Sanity-check it lists the trigger for revisiting (when the legacy `MERGE` path can be deleted entirely — driven by the §6.3 doctor's "≤ 1% legacy envelopes" gate)
