# Proposal: phase10b_snippet_body_capture

## Why

`/v1/query` and `cortex_pre_thinking` return snippets whose `text`
field is **the symbol or path itself**, not the actual content. The
2026-04-29 audit logged:

- `free_search "phase9k cron scheduler retention"` → snippet 1
  `text='TodoWrite'` (just the tool name, score 0.016).
- Pre-thinking bundle for "retention sweep idempotence" → 5
  snippets formatted `path:artifact — path` with body `crates/
  cortex-api/src/analyzer.rs` (the path again).

The agent receives `ls`-grade content. Pre-thinking budget bytes
are spent on filenames instead of the prose / code that would
actually inform the next turn.

The ingestion side is fine: `cortex.events.enriched` envelopes
carry a `body_ref` (CAS hash) plus inline body up to 8 KiB. The
keyword-lane projector stamps `text` with `symbol` / `kind` instead
of resolving the body. The vectorizer + nexus lanes have the same
bug because they share the projector helper.

## What Changes

1. Replace the snippet projector in
   `crates/cortex-api/src/lanes.rs` so `text` carries the
   first 1 KiB of the resolved body (or the full body when
   smaller). The current `symbol` value moves to a separate
   `symbol` field that's already on the wire.
2. Resolve the body via the `body_ref` CAS hash when the inline
   body is missing — the keyword lane already holds a CasStore
   handle through `DashboardState`.
3. Cap projection cost: if the CAS round-trip exceeds 50 ms or
   3 hops per query, fall back to the symbol/path so the budget
   isn't blown.
4. Pre-thinking renderer (`crates/cortex-pre-thinking/src/
   bundle.rs`) MUST format snippets as
   `path:line — first 200 chars of body…` instead of the
   current `path:artifact — path`.
5. Add a regression: a unit test that round-trips a known-good
   body through the projector and asserts `snippet.text != path`.

## Impact

- Affected specs: `docs/specs/11-query-api.md` §snippet shape,
  `docs/specs/12-pre-thinking-injection.md` §bundle layout.
- Affected code: `crates/cortex-api/src/lanes.rs`,
  `crates/cortex-api/src/types.rs` (snippet struct),
  `crates/cortex-pre-thinking/src/bundle.rs`.
- Breaking change: NO. Adds body bytes to an existing field;
  consumers that ignored the field continue to work.
- User benefit: the pre-thinking bundle becomes useful again —
  agents see the actual code/prose, not a directory listing.
