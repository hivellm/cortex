## 1. Complete vocabulary
- [ ] 1.1 Append `(Kind::Knowledge, 9)`, `(Kind::Learning, 10)`, `(Kind::Consolidation, 11)`, `(Kind::TopicCard, 12)` to `KIND_IDS`.
- [ ] 1.2 Add `pub const COUNT: usize = 12;` on `cortex_core::events::Kind`.
- [ ] 1.3 Add `static_assertions::const_assert_eq!(KIND_IDS.len(), Kind::COUNT)` next to the table.
- [ ] 1.4 Round-trip test: every `Kind` value resolves through `KIND_IDS` and back.

## 2. Miss metric
- [ ] 2.1 Register `cortex_classifier_vocab_lookup_miss_total{kind}` counter.
- [ ] 2.2 Increment on every `KIND_IDS.get(kind).is_none()` path remaining in the codebase (defensive — should be unreachable after §1).
- [ ] 2.3 Add a doctor warning when the counter is non-zero within the last hour.

## 3. Tail (mandatory)
- [ ] 3.1 Update `docs/specs/05-classifier.md` § Vocabulary + `CHANGELOG.md` Fixed.
- [ ] 3.2 Tests: §1.4 round-trip + miss-counter regression.
- [ ] 3.3 `cargo check --workspace && cargo clippy -p cortex-classifier -- -D warnings && cargo test -p cortex-classifier` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
