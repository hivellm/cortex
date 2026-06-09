# Proposal: phase26b_audit-monitoring-quality

## Why

Live audit on 2026-06-09 (docs/analysis/cortex/12-live-audit-2026-06-09.md) found three
monitoring bugs that generate constant false alarms and hide real problems:

- **Bug #2**: The divergence checker reads `ingestion.archived.{kind}` from a KV location
  that the ingestion service never writes to. Result: downstream always reads `0`, making
  every tool_call path appear as a 100% drop (CRITICAL severity). Additionally, the checker
  attempts to self-report by posting synthetic `law_violation` events with invalid ULIDs
  and wrong tool values — ingestion rejects these every few hours and floods the logs.
- **Bug #6**: The fulltext worker discards 85% of live stream events as `skipped_empty`.
  The Meilisearch indexes have >250k docs from bootstrap, but live capture barely lands
  anything. Root cause: the extractor field (`summary` or similar) is empty because the
  classifier is running in Static mode and produces no summaries.
- **Bug #7**: The divergence pair `adapter.frames_parsed → adapter.envelopes_built` is
  flagged CRITICAL because it expects 1:1 conversion — but `PreToolUse` (3,695 frames)
  and `UserPromptSubmit` (188 frames) are never converted to envelopes by design. The
  actual ~52% conversion ratio is correct.

These three bugs make the health/divergence dashboard untrustworthy. Every on-call look at
the dashboard produces false urgency.

## What Changes

### Bug #2 — Divergence checker counter alignment
- `crates/cortex-workers/src/ingestion/router.rs` (or health module): after archiving a
  batch, publish `ingestion.archived.{kind}` to the Synap KV slot that the divergence
  checker reads from, OR expose the counter via the health endpoint under the key the
  checker polls.
- `crates/cortex-api/src/health/divergence.rs` (or equivalent): replace the synthetic
  law_violation alert event (invalid ULID, wrong tool, malformed schema) with a structured
  out-of-band alert — a WARN log line or a proper metric counter. Remove the ingestion
  POST path from the divergence reporter entirely.

### Bug #6 — Fulltext worker: fallback extraction chain
- `crates/cortex-workers/src/fulltext/` extractor: add a fallback chain so that no
  non-empty envelope is ever skipped:
  `summary` → `payload.text` → `payload.output.stdout` → `kind + path + event_id` as
  minimal document.
- `skipped_empty` should only fire when the entire envelope payload is genuinely empty.

### Bug #7 — Frames/envelopes ratio: exclude non-capture hook types
- Health divergence config: exclude `PreToolUse` and `UserPromptSubmit` frame counts from
  the upstream counter of the `frames_parsed → envelopes_built` pair, since these hook
  types never produce envelopes by design.
- Document the expected ~85% ratio (PostToolUse + Stop + SubagentStop → envelopes) as
  the baseline threshold.

## Impact
- Affected specs: spec 10 (adapter), spec 08 (fulltext indexer), health/divergence subsystem
- Affected code: cortex-workers (ingestion, fulltext), cortex-api (health/divergence)
- Breaking change: NO
- User benefit: divergence dashboard shows real data loss instead of false alarms; fulltext captures live events; health checks are trustworthy for on-call use
