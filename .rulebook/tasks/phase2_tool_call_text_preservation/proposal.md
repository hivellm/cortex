# Proposal: phase2_tool_call_text_preservation

## Why

The 2026-04-27 audit pulled this top-20 against `query: "completely fake xyz123"`:

```
score=0.0163 src=vector text='canonical envelope smoke'
score=0.0161 src=vector text='canonical envelope V3 ping'
…
score=0.0148 src=vector text='[Bash] {}'
score=0.0145 src=vector text='[TodoWrite] {}'
score=0.0143 src=vector text='[Read] {}'
score=0.0141 src=vector text='[Bash] {}'
score=0.0139 src=vector text='[Edit] {}'
score=0.0138 src=vector text='[Bash] {}'
score=0.0136 src=vector text='[Read] {}'
… (13 of 20 are `[Tool] {}` with empty bodies)
```

1 653 of the 1 714 captured envelopes are tool_calls. Their searchable text is reduced to `[Tool] {}` — bracketed kind label, empty JSON object. The actual command, file path, args, output — all gone. The dashboard `/memory` endpoint reads the original blob and shows the real input ("[Bash] {\"command\":\"(Invoke-WebRequest …\"}"), so the data exists somewhere — but the lane-facing text is stripped before it's stored or before `envelope_to_hit` runs in `archive_loader.rs`.

This means the keyword lane (even when made live in `phase2_keyword_lane_live_meilisearch`) cannot find the call that just ran `git commit -m "fix(...)"` two minutes ago. The vector lane (when made live) embeds `[Bash] {}` — uniform garbage. 96% of captured volume becomes useless for retrieval.

## What Changes

- Inspect the redaction / classifier path: the `cortex-classifier-worker` `NormalisedEvent` derivation (and/or `cortex-core` redaction) is producing the `[Tool] {}` text. The fix is **NOT** to remove redaction — credentials, tokens, secrets must continue to be scrubbed — but to:
  1. Preserve the structural fields (`tool_name`, `command`, `file_path`, `query`, `output_summary`) verbatim where possible.
  2. Run redaction at the value level, not the wholesale-blank-the-input level.
  3. Keep a redaction-trace (which fields were scrubbed) on the envelope per spec-04, so the dashboard / lane can show what was masked.
- The lane-facing `text` field on tool_call hits in `archive_loader::envelope_to_hit` becomes the post-redaction concatenation of structural fields, e.g.:
  - `[Bash] git commit -m "fix(adapter): wire pre-thinking pipeline"` (literal)
  - `[Edit] crates/cortex-adapter-claude-code/src/sync_paths.rs — old="…" → new="…"`
  - `[Read] crates/cortex-api/src/lanes.rs:218-232`
  - `[TodoWrite] 4 todos: 'Wire missing Cortex hooks…' → completed; 'Diagnose…' → in_progress`
- Backfill: existing archive parquet files were captured with the broken redaction. They MUST be re-classified once the fix lands so the historical record becomes searchable too. The task includes a one-shot replay tool (`cortex-ingestion replay --redact-fix`) that reads the raw envelopes, re-runs the new redactor, and re-emits to the lane. Alternatively, the keyword lane's seeder reads from `cortex-ingestion`'s raw frame instead of the classifier's output until backfill finishes.

## Impact

- Affected specs: spec-04 (cortex-core redaction), spec-05 (classifier), spec-08 (fulltext indexer text choice).
- Affected code:
  - `crates/cortex-core/src/redact.rs` (or wherever the wholesale `{}` blank happens)
  - `crates/cortex-classifier-worker/src/worker.rs` (NormalisedEvent — already touched in WIP commit `34a4db0`)
  - `crates/cortex-api/src/archive_loader.rs::envelope_to_hit` (the function that reduces an envelope to a `LaneHit.text`)
  - tests verifying redaction now masks values without blanking the structure
- Breaking change: NO (envelope schema unchanged; only the redaction strategy)
- User benefit: 96% of captured volume becomes useful retrieval material. "What was the last `git commit` I ran?" / "Did I edit `sync_paths.rs` today?" / "What did I read about LAW-007?" all become answerable.

## Source

2026-04-27 audit, top-20 dump showing 13 of 20 hits collapsed to `[Tool] {}`.
