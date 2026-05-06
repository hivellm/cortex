## 1. ADR-011
- [ ] 1.1 `rulebook_decision_create` ADR-011 — "Typed ProjectedHit replaces extras: HashMap". Status `proposed`.
- [ ] 1.2 Trade-off: ~1 sprint cost touching 3 lanes + orchestrator; gain is compile-time overlay correctness and unblocks phase11v.

## 2. Trait + types
- [ ] 2.1 In `crates/cortex-api/src/lanes.rs`, define `pub trait Lane`. Existing `VectorLane`, `KeywordLane`, `GraphLane` become `impl Lane for ...`.
- [ ] 2.2 Replace `ProjectedHit { extras: HashMap<String, Value> }` with `ProjectedHit { event_id, score, payload, overlay: Overlay }`. `Overlay` lives next to it.
- [ ] 2.3 `Overlay` fields: `decision_id: Option<String>`, `superseded_by: Option<String>`, `contradiction_flag: Option<ContradictionKind>`, `consolidation_grain: Option<Grain>`, `topic_id: Option<String>`, `repo: String`, `kind: Kind`. Document ownership of each field per lane in `Overlay`'s rustdoc.
- [ ] 2.4 Add `From<&Envelope> for Overlay` so lane impls fill the overlay uniformly from the source event.

## 3. Migrate orchestrator
- [ ] 3.1 Rewrite every `extras.get("...")` call in `orchestrator.rs::derive_*` to typed `overlay.<field>` access.
- [ ] 3.2 Remove `extras` field from `ProjectedHit` once all callers migrate.
- [ ] 3.3 Empty-overlay regression test: 3 lane impls (vector, keyword, graph) with no overlay fields populate gracefully (no panic, no fall-through to wrong overlay).

## 4. Lane impls
- [ ] 4.1 `VectorLane::search` — fills overlay from the payload deserialised in phase11d.
- [ ] 4.2 `KeywordLane::search` — fills overlay from the projected Meili document.
- [ ] 4.3 `GraphLane::search` — fills overlay from the Nexus node properties.
- [ ] 4.4 Per-lane unit test: overlay correctness on a fixture event.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/11-query-api.md` § Lane contract + `CHANGELOG.md` Changed.
- [ ] 5.2 Tests: §3.3 + §4.4 × 3 + golden-set retrieval IT (smoke).
- [ ] 5.3 `cargo check --workspace && cargo clippy -p cortex-api -- -D warnings && cargo test -p cortex-api` clean.
- [ ] 5.4 Unblock `phase11v_mcp-fine-grained-backend-search`: change its status to `pending` so phase14 can pick it up as the trait's first consumer.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
