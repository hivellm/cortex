# Proposal: phase12e_vocab-kind-ids-complete

Source: `docs/analysis/rework/glm5.1/findings.md` F-007 (HIGH).

## Why

`crates/cortex-classifier/src/vocab.rs::KIND_IDS` declares 8 of the 12 event kinds defined in `cortex-core::events::Kind`. The 4 missing kinds (`knowledge`, `learning`, `consolidation`, `topic_card`) cause vocab lookups to return `None` and the classifier silently routes those events to the `Unknown` bucket. The miss is invisible because there is no metric on `vocab_lookup_miss`.

## What Changes

- Complete the `KIND_IDS` table to cover all 12 kinds with stable u16 ids (preserve existing ids; append the missing 4 at the end of the enum range).
- Add a compile-time assertion (`const_assert_eq!`) that `KIND_IDS.len() == Kind::COUNT`.
- Add `cortex_classifier_vocab_lookup_miss_total{kind}` counter and a doctor warning when it is non-zero on a 1-hour window.

## Impact

- Affected specs: `docs/specs/05-classifier.md` § Vocabulary.
- Affected code: `crates/cortex-classifier/src/vocab.rs`, `crates/cortex-classifier/src/metrics.rs`, `crates/cortex-core/src/events.rs` (Kind::COUNT const).
- Breaking change: NO. Existing ids preserved.
- User benefit: knowledge + learning + consolidation + topic_card kinds participate in classification correctly.
