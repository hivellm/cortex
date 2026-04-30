## 1. Adapter stamps tool
- [ ] 1.1 In `crates/cortex-adapter-claude-code/src/events.rs`, every emitted envelope carries `payload.tool = "claude-code"`
- [ ] 1.2 IPC handshake exposes the peer-reported tool string; adapter falls back to `"claude-code"` when absent
- [ ] 1.3 Regression test asserting `payload.tool` is set on a synthetic frame

## 2. ADR + analysis front-matter parser
- [ ] 2.1 In `crates/cortex-cli/src/bootstrap/walker.rs`, parse the YAML front-matter (or the "**Status**: …", "**Date**: …", "**Related Tasks**: …" Markdown header pattern) of `.rulebook/decisions/*.md`
- [ ] 2.2 Stamp `payload.author`, `payload.occurred_at` (RFC-3339 from `Date`), `payload.source_analysis` (when `Related Tasks` points at an analysis dir)
- [ ] 2.3 Same for `docs/analysis/<slug>/README.md` — stamp `payload.author`, `payload.occurred_at`

## 3. Metadata store upsert
- [ ] 3.1 `MetadataStore::upsert_session` keeps the existing `tool` value when the new write passes `NULL`; only overwrites when the new value is `Some(_)`
- [ ] 3.2 Add a `set_session_tool(session_id, tool)` helper for the backfill CLI

## 4. Backfill CLI
- [ ] 4.1 NEW `cortex-ops sessions backfill-tool [--dry-run] [--apply]`
- [ ] 4.2 For every `tool IS NULL` session row, look up the first envelope's `payload.adapter` and stamp it
- [ ] 4.3 Default dry-run; report shows per-tool counts

## 5. Tests
- [ ] 5.1 Adapter unit test: envelope round-trips with `payload.tool`
- [ ] 5.2 Walker unit test: ADR front-matter `Date: 2026-04-22` lands as `payload.occurred_at = "2026-04-22T00:00:00Z"`
- [ ] 5.3 Backfill CLI unit test: dry-run reports the right counts; apply mutates only when run

## 6. Spec / docs
- [ ] 6.1 Update `docs/specs/01-event-schema.md` §payload with the "MUST stamp" clause for `tool` and `author`/`occurred_at` on ADRs
- [ ] 6.2 Update `docs/specs/10-claude-code-adapter.md` §metadata

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
