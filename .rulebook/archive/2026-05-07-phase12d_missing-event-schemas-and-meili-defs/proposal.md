# Proposal: phase12d_missing-event-schemas-and-meili-defs

Source: `docs/analysis/rework/glm5.1/findings.md` F-001 + F-002 (both CRITICAL).

## Why

Two schema gaps cause silent data loss:

1. JSON schemas for event kinds `knowledge` and `learning` are absent from `crates/cortex-core/schemas/`. The classifier emits both kinds, but the validation pass at ingest rejects them silently because the schema map has no entry, then logs an INFO-level "unknown kind" that nobody monitors.
2. Meili index definitions for `cortex_consolidations` and `cortex_topic_cards` are absent from the bootstrap config. Bootstrap creates the indexes lazily on first write, but with no `searchable_attributes` / `filterable_attributes` configured, every query against them returns ranked-by-document-id (useless for retrieval).

Both gaps are blockers for the relevance closure work in Phase C.

## What Changes

- Add `crates/cortex-core/schemas/knowledge.schema.json` and `learning.schema.json` mirroring the structure of `decision.schema.json` and the kinds defined in `crates/cortex-core/src/events.rs`.
- Wire both new schemas into `EventValidator::load_schemas()` and add validation tests for both kinds (happy path + missing required field).
- Add `cortex_consolidations` + `cortex_topic_cards` to the bootstrap-time index config in `crates/cortex-workers/src/fulltext/bootstrap.rs` with appropriate `searchable_attributes`, `filterable_attributes`, and `sortable_attributes`.
- Add a doctor check `cortex-ops doctor meili-indexes` that surfaces any index missing its expected attribute config.

## Impact

- Affected specs: `docs/specs/04-event-schema.md` (knowledge + learning kinds), `docs/specs/06-fulltext.md` (consolidations + topic_cards indexes).
- Affected code: `crates/cortex-core/schemas/{knowledge,learning}.schema.json`, `crates/cortex-core/src/validate.rs`, `crates/cortex-workers/src/fulltext/bootstrap.rs`, `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO. Existing events keep validating.
- User benefit: knowledge + learning envelopes stop being silently rejected; consolidation queries become rankable.
