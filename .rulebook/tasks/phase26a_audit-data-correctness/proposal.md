# Proposal: phase26a_audit-data-correctness

## Why

Live audit on 2026-06-09 (docs/analysis/cortex/12-live-audit-2026-06-09.md) identified four
bugs causing silent data loss and incorrect governance reporting, all with root causes that
are independent of each other and each fixable in under 2 hours:

- **Bug #1**: 100% of `agent_call` events are rejected by ingestion because the adapter
  emits `description: ""` and the JSON Schema requires `minLength: 1`. Every sub-agent
  invocation (researcher, implementer, tester) is invisible in Cortex.
- **Bug #3**: The embedder counts HTTP 409 Conflict (collection already exists) as a
  vectorizer error, inflating `vectorizer_errors_total` by ~50% with false positives and
  masking real failures.
- **Bug #4**: `.env` declares `CORTEX_CLASSIFIER_MODE=disabled` but the running container
  logs `mode: Static`. The config is lying about the actual runtime behavior.
- **Bug #5**: The bootstrap emits rule files (`.claude/rules/*.md`) with `kind: "law_violation"`
  instead of `kind: "law"`. Result: 3,332 "law violations" in the dashboard are mostly law
  definitions, not real breach events.

All four bugs were confirmed on the live stack; all four have low-risk targeted fixes.

## What Changes

### Bug #1 — agent_call schema: allow empty description
- `crates/cortex-adapter-claude-code/src/envelope_builder.rs` (or equivalent): use a
  non-empty default when the SubagentStop description field is empty.
- OR relax the JSON Schema for the `agent_call` payload: change `"minLength": 1` to
  `"minLength": 0` in `crates/cortex-core/schemas/agent_call.json`.

### Bug #3 — Embedder: 409 Conflict is not an error
- `crates/cortex-workers/src/embedder/vectorizer_client.rs`: handle HTTP 409 on
  `create_collection` as `Ok(())` — the collection exists, which is the desired end state.
- Separate the error counter: distinguish `conflict` from `auth` from real transport failures.

### Bug #4 — Config documentation
- Update `.env` comment to reflect the actual runtime mode, or fix the docker-compose
  override that shadows `CORTEX_CLASSIFIER_MODE`.

### Bug #5 — Bootstrap: law files → kind "law"
- `crates/cortex-cli/src/bootstrap/promoter.rs` (or equivalent): when promoting
  `.claude/rules/*.md` and `AGENTS.override.md`, emit `kind: EventKind::Law` not
  `EventKind::LawViolation`.
- Existing `law_violation` documents that were actually law definitions should be
  cleaned from Meilisearch on the next bootstrap run (or via a one-shot cleanup).

## Impact
- Affected specs: spec 01 (event schema), spec 09 (bootstrap CLI), spec 10 (adapter)
- Affected code: cortex-core (schema), cortex-adapter-claude-code, cortex-workers (embedder), cortex-cli (bootstrap)
- Breaking change: NO — law_violation events with detector=null and ts=0 already look wrong; kind correction is a data quality fix
- User benefit: agent calls captured, embedder error counter trustworthy, governance dashboard shows real violations
