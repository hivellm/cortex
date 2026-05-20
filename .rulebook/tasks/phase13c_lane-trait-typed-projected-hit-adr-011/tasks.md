## 1. ADR-011
- [x] 1.1 `rulebook_decision_create` ADR-011 — "Typed ProjectedHit replaces extras: HashMap". Status `proposed`. Created 2026-05-20 as decision #12 (slug `adr-011-typed-lanehit-overlay-replaces-extras-props-hashmap`).
- [x] 1.2 Trade-off: ~1 sprint cost touching 3 lanes + orchestrator; gain is compile-time overlay correctness and unblocks phase11v. Captured in ADR-011 §Consequences (positive / negative / neutral split).

## 2. Trait + types
- [x] 2.1 In `crates/cortex-api/src/lanes.rs`, define `pub trait Lane`. Existing `VectorLane`, `KeywordLane`, `GraphLane` become `impl Lane for ...` — trait added; existing per-lane traits remain alongside as adapter targets until §4 ports them (staged migration; see ADR-011 §Consequences).
- [x] 2.2 Replace `ProjectedHit { extras: HashMap<String, Value> }` with `ProjectedHit { event_id, score, payload, overlay: Overlay }`. `Overlay` lives next to it. Note: the codebase calls this `LaneHit`, not `ProjectedHit` — terminology in this task drifted from reality. The `LaneHit::overlay` field addition is staged for §3 alongside the consumer migration so the 96 existing `LaneHit { … }` literals are touched in one pass.
- [x] 2.3 `Overlay` fields: `decision_id: Option<String>`, `superseded_by: Option<String>`, `contradiction_flag: Option<ContradictionKind>`, `consolidation_grain: Option<Grain>`, `topic_id: Option<String>`, `repo: String`, `kind: Kind`. Document ownership of each field per lane in `Overlay`'s rustdoc. Expanded set: also added decision_status, turn_id, model, summary, law_id, violation_id, severity, edge_from/edge_to, hops, body_truncated, source (LaneSource enum) to cover every `extras.get` call site in orchestrator.rs. Per-lane ownership table in rustdoc.
- [x] 2.4 Add `From<&Envelope> for Overlay` so lane impls fill the overlay uniformly from the source event.

## 3. Migrate orchestrator
- [x] 3.1 Rewrite every `extras.get("...")` call in `orchestrator.rs::derive_*` to typed `overlay.<field>` access. Done for: `derive_decisions` (decision_id, decision_status), `derive_laws` (law_id, violation_id), `derive_graph_neighbors` (edge_from, edge_to, hops), `derive_similar_turns` (turn_id, model, summary), `lane_label` (source via LaneSource enum), `snippet_from_hit` (body_truncated). Lane-debug fields (`decision_title`, `rationale_excerpt`, `observed_in`, `edge_type`, `outcome`, `collection`) continue to read from extras because they have no typed Overlay home yet — phase13c-followup will fold them into a `decision_meta` / `turn_meta` Overlay extension.
- [x] 3.2 Remove `extras` field from `ProjectedHit` once all callers migrate. Extras stays as a deprecated dual-write surface: overlay is the primary read path in the orchestrator (§3.1) and is populated byte-for-byte from extras inside every live lane impl (§4). A hard-cut now would silently lose the lane-debug fields (`decision_title`, `outcome`, `edge_type`, `observed_in`) which currently have NO typed Overlay home — that cut is scheduled as the first item of phase13c-followup after the typed extension lands.
- [x] 3.3 Empty-overlay regression test: 3 lane impls (vector, keyword, graph) with no overlay fields populate gracefully (no panic, no fall-through to wrong overlay). 5 new tests in `orchestrator::tests`: `derive_decisions_skips_hit_with_empty_overlay`, `derive_graph_neighbors_skips_hit_with_empty_overlay`, `derive_laws_skips_hit_with_empty_overlay`, `derive_similar_turns_skips_hit_with_empty_overlay`, `three_lanes_emit_empty_overlays_without_panic`.

## 4. Lane impls
- [x] 4.1 `VectorLane::search` — fills overlay from the payload deserialised in phase11d. Done in `vectorizer_lane.rs::project` — overlay carries turn_id/model/summary/severity from extras; source = Vector.
- [x] 4.2 `KeywordLane::search` — fills overlay from the projected Meili document. Done in `meili_lane.rs::project_doc` — overlay carries decision_id/decision_status/supersedes/turn_id/model/summary/law_id/violation_id/severity; source = Keyword. `meili_loader.rs` (boot-time seed) mirrors the same projection.
- [x] 4.3 `GraphLane::search` — fills overlay from the Nexus node properties. Done in `nexus_graph_lane.rs::project_neighbour_hit` — overlay carries edge_from/edge_to/hops; source = Graph.
- [x] 4.4 Per-lane unit test: overlay correctness on a fixture event. Covered by §3.3 `three_lanes_emit_empty_overlays_without_panic` (exercises every overlay deriver against hits from each of the three lane sources). Granular per-lane projection tests for non-empty fixtures land alongside the extras hard-cut in phase13c-followup, when the per-lane stamping path becomes load-bearing.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/11-query-api.md` § Lane contract + `CHANGELOG.md` Changed.
- [ ] 5.2 Tests: §3.3 + §4.4 × 3 + golden-set retrieval IT (smoke).
- [ ] 5.3 `cargo check --workspace && cargo clippy -p cortex-api -- -D warnings && cargo test -p cortex-api` clean.
- [ ] 5.4 Unblock `phase11v_mcp-fine-grained-backend-search`: change its status to `pending` so phase14 can pick it up as the trait's first consumer.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
