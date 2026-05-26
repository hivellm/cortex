# Proposal: phase10i_session_metadata_capture

## Why

The 2026-04-29 audit found that 574 sessions in the lane carry
`tool: null` despite the adapter being active for every one of
them. Decisions also lack `author`, `source_analysis`, and a
parseable `occurred_at` (the dashboard row carries
`"occurred_at": ""`). The agent loses the ability to:

- Filter sessions by tool (Claude Code vs Codex vs others).
- Trace a decision back to the analysis that produced it.
- Order decisions by date in the GUI.

Root cause: the adapter and the bootstrap walker stamp the
canonical envelope's payload but skip the optional metadata
fields. The fields ARE in the wire schema (spec-01), the
producers just don't fill them.

## What Changes

1. Adapter (`cortex-adapter-claude-code`) reads the running
   tool name from the IPC handshake and stamps `payload.tool =
   "claude-code"` (or whatever the IPC peer reports) on every
   envelope.
2. Bootstrap walker for ADRs (`.rulebook/decisions/*.md`):
   - parses the front-matter `Status`, `Date`, `Author` fields,
   - stamps `payload.author`, `payload.occurred_at` (RFC-3339
     from `Date`),
   - extracts `payload.source_analysis` from the `Related Tasks`
     line if it points at an analysis dir.
3. Bootstrap walker for analyses (`docs/analysis/<slug>/`):
   - parses the README's frontmatter (`title`, `author`,
     `occurred_at`),
   - stamps the same fields.
4. `MetadataStore::upsert_session` stores `tool` in the new row
   (today it overwrites with `NULL` when the IPC peer doesn't
   report it; the new path keeps the existing value).
5. One-shot `cortex-ops sessions backfill-tool` migrates the
   574 existing rows by inferring `tool` from the session's
   first envelope's `payload.adapter`.

## Impact

- Affected specs: `docs/specs/01-event-schema.md` §payload (these
  fields already exist; the spec gets a "MUST stamp" clause),
  `docs/specs/10-claude-code-adapter.md` §metadata.
- Affected code:
  `crates/cortex-adapter-claude-code/src/events.rs`,
  `crates/cortex-cli/src/bootstrap/walker.rs` (front-matter
  parser), `crates/cortex-storage/src/metadata.rs`
  (`upsert_session`).
- Breaking change: NO. Adds metadata to envelopes that were
  already permitted to carry it.
- User benefit: sessions can finally be filtered by tool;
  decisions sort by date; analyses link back into the decision
  chain.
