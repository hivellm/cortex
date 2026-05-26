# phase10i — preserve sessions.tool + ADR front-matter parser
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10i_session_metadata_capture
**Tags**: session, metadata, decision, front-matter, phase10i
The 2026-04-29 audit caught 574 sessions in the metadata DB with `tool IS NULL` despite the adapter being active for every one of them. Decisions also lacked `author` / `occurred_at`. Two distinct fixes:

1. **MetadataStore::upsert_session preservation** — pre-phase10i SQL wrote `tool=excluded.tool` on conflict, so any lifecycle hook that didn't capture the tool name (`Stop` / `Notification`) overwrote the session-start value. New SQL: `tool=COALESCE(NULLIF(excluded.tool, ''), sessions.tool)`. NULLIF treats empty string as null; COALESCE then preserves the existing value. First write keeps the literal value (schema is `tool TEXT NOT NULL`, so writing `NULLIF('', '')` would violate the constraint — ON CONFLICT only does the COALESCE dance).

2. **Decision/Analysis front-matter parser** — added `author`, `occurred_at` (RFC-3339 from `Date: YYYY-MM-DD`), and `source_analysis` (first `docs/analysis/<slug>` from `Related Tasks:`) to `emit_decision_imported` + `author` / `occurred_at` to `emit_analysis_imported`. Both parsers handle the **bolded** markdown style (`**Status**: accepted`) via a new `unbold_label()` helper that collapses `**LABEL**: value` into `LABEL: value` so the case-insensitive prefix scanner matches both shapes uniformly. `Author: Unknown` and malformed dates are dropped silently — better to leave the field absent than stamp garbage.

3. **Backfill CLI** — `cortex-ops sessions backfill-tool [--dry-run|--apply] [--tool <name>]` walks `sessions_missing_tool` and stamps via `set_session_tool`. Default tool is `claude-code` (the only adapter pre-phase10i daemons ran).

The audit's 574 NULL rows must have come from a mix of `NULL` and `''` — the new `sessions_missing_tool` query catches both via `WHERE tool IS NULL OR tool = ''`. Schema-level NULL would require a migration; we sidestepped it because `''` is functionally equivalent for the backfill path.