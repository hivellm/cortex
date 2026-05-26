# Proposal: phase0_event-schema

## Why

Every Cortex component (ingestion, classifier, embedder, graph writer, full-text indexer, adapters, query API) consumes the **same event envelope**. Without a frozen wire format there is no ingestion, no processing, and no retrieval. This task delivers the single source of truth all downstream MVP tasks depend on (see [`docs/dag.md`](../../../docs/dag.md) — spec 01 is the root of the DAG).

## What Changes

- Publish JSON Schemas for the event envelope and every payload `kind` (`turn.*`, `tool_call.*`, `artifact.*`, `decision.*`, `memory.*`, `law.*`, `analysis.*`, `event.notification`).
- Ship canonical-JSON serializer (deterministic key ordering, UTF-8 NFC) used to compute `content_hash`.
- Ship ULID generator + ID conventions (`event_id`, `session_id`, `turn_id`, `tool_call_id`, `analysis_id`, `decision_id`, `violation_id`).
- Ship a validator usable both as a Rust crate and as a CLI (`cortex-core validate <file>`).
- Flip [`docs/specs/01-event-schema.md`](../../../docs/specs/01-event-schema.md) status to 🟢.

## Impact

- **Affected specs:** [`docs/specs/01-event-schema.md`](../../../docs/specs/01-event-schema.md) (primary); unblocks 02, 04, 05, 06, 07, 08, 13.
- **Affected code:** new `cortex-core/schemas/`, `cortex-core/src/events.rs` (generated), `cortex-core/src/canonical_json.rs`, `cortex-core/src/ulid.rs`, `cortex-core/src/validate.rs`.
- **Breaking change:** NO — greenfield.
- **User benefit:** enables every downstream MVP task; guarantees cross-component interop.

## Source

`docs/specs/01-event-schema.md` · PRD FR-1 · DAG root node.
