# Spec: Session metadata capture

## ADDED Requirements

### Requirement: Adapter stamps payload.tool

Every envelope emitted by `cortex-adapter-claude-code` MUST
carry `payload.tool` populated with the running tool name. The
field MUST never be omitted nor `null` for adapter-produced
events.

#### Scenario: synthetic adapter frame
Given a synthetic `PostToolUse` frame from the adapter
When the adapter publishes the resulting envelope
Then `payload.tool` MUST equal `"claude-code"` (or the IPC peer's
  reported tool string).

### Requirement: Walker parses ADR + analysis front-matter

The bootstrap walker MUST parse the front-matter of
`.rulebook/decisions/*.md` and `docs/analysis/*/README.md`.

For ADRs: stamp `payload.author`, `payload.occurred_at`
(RFC-3339), `payload.source_analysis` (analysis-dir slug when
`Related Tasks:` points at one).

For analyses: stamp `payload.author`, `payload.occurred_at`.

#### Scenario: ADR with Date front-matter
Given `.rulebook/decisions/001-bypass-vectorizer-sdk-...md`
  carries `**Date**: 2026-04-22` and
  `**Related Tasks**: phase1_embedder`
When the walker bootstraps the file
Then the emitted envelope MUST carry `payload.occurred_at =
  "2026-04-22T00:00:00Z"`
And `payload.source_analysis` MUST equal `"phase1_embedder"`.

### Requirement: upsert_session preserves the tool field

`MetadataStore::upsert_session` MUST NOT overwrite an existing
`sessions.tool` value with `NULL`. When the new write passes
`NULL` and the existing row already carries a tool, the existing
value MUST stay.

#### Scenario: late ipc frame without tool
Given a session row with `tool='claude-code'` already exists
When `upsert_session` is called with `tool=None`
Then the row's `tool` MUST remain `"claude-code"`.

### Requirement: backfill CLI fills NULL tools

`cortex-ops sessions backfill-tool` MUST inspect every row whose
`tool IS NULL`, look up the session's first envelope, infer
`tool` from `payload.adapter`, and (in `--apply` mode) update
the row.

#### Scenario: 574 NULL tools become claude-code
Given 574 sessions all carry `tool=NULL` and every first
  envelope reports `payload.adapter = "claude-code"`
When the operator runs `cortex-ops sessions backfill-tool --apply`
Then exactly 574 rows MUST be updated to `tool='claude-code'`
And a re-run MUST update zero rows.
