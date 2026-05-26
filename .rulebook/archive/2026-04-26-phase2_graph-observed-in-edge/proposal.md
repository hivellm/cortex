# Proposal: phase2_graph-observed-in-edge

## Why

`phase1_graph-writer` shipped every relationship edge except
`OBSERVED_IN` (LawViolation → Turn|ToolCall). The blocker, documented
verbatim in that task's §4.5: "`LawViolationPayload.observed_event_id`
carries no kind discriminator, so the writer cannot choose the target
label without phantom-node risk via Cypher `MERGE`." Without the
discriminator, a `MERGE (e:Turn|ToolCall {event_id: $id})` would
either need a runtime label resolution round-trip (adds an extra
query per write) or risk creating phantom nodes when the target
doesn't yet exist under the inferred label.

This task closes that gap so the spec-04 graph contract is complete:
every law violation links back to the event it was observed against.

## What Changes

- Spec 04 (`crates/cortex-core/schemas/kinds/law_violation.schema.json`)
  gains a required `observed_event_kind` enum field next to
  `observed_event_id`. Allowed values match the canonical kind enum:
  `turn` | `tool_call`.
- `cortex_core::events::LawViolation` Rust type gains the matching
  field; `validate_event` enforces the new requirement at the
  ingestion boundary.
- Existing emitters of `LawViolation` payloads (the spec-13 / spec-14
  detectors today, the spec-10 PreToolUse sync path tomorrow) populate
  the new field. The PreToolUse path knows the kind is always
  `tool_call`; spec-13 detectors observing turn-level patterns set
  `turn`.
- `cortex-graph` writer adds the `OBSERVED_IN` edge using the kind to
  pick the right MERGE label — no phantom-node risk because the
  target node is created under the canonical label by upstream
  Turn / ToolCall writes that landed in phase 1.
- Marks `phase1_graph-writer` §4.5 done by reference to this task
  (the parent task was archived; this follow-up is the durable
  pointer).

## Impact

- Affected specs: spec 04 schema bump (additive — required field on a
  payload type that's currently emitted by exactly one path; bumps
  schema_version contract is not needed since the required-field
  addition only fires when a `law_violation` envelope ships, and
  there are no historical envelopes of that kind in the archive
  yet). spec 10 / spec 13 / spec 14 emitter updates noted.
- Affected code: cortex-core schema + Rust type + validator;
  cortex-graph writer; spec-10 / spec-14 emitters.
- Breaking change: YES at the schema layer, NO at the runtime layer
  (no historical `law_violation` envelopes exist yet so no migration
  needed).
- User benefit: the `Decisions` overlay's "what tool call triggered
  this violation?" link works in the spec-16 dashboard; the spec-15
  analyses can join law violations back to their originating
  artifact via the graph.
