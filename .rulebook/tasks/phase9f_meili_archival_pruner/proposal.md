# Proposal: phase9f_meili_archival_pruner

## Why

Meilisearch holds the full body of every turn and tool call indefinitely:
`cortex_turns.user_message`, `cortex_turns.assistant_message`,
`cortex_tool_calls.command_or_input`, `cortex_tool_calls.output_excerpt`.
On a busy repo this index outgrows Vectorizer within months, and the
relevance lane (phase 6c) loses speed under multi-MB documents.

After 90 days the user-facing value of those raw bodies is small — the
`summary` field plus topic/repo metadata is enough for keyword recall,
and the original bytes still live in the Parquet archive for audit. The
spec already declares that the FP32 collection lives ≤30 d and PQ
30–365 d; Meili's parallel surface needs a matching contract.

This task ships the pruner that brings Meili in line with the rest of
the retention pipeline.

## What Changes

1. NEW subcommand `cortex-retention meili-prune`.
2. For documents in `cortex_turns` and `cortex_tool_calls` whose
   `occurred_at < now - prune_after_days` (default 90):
   - keep `event_id`, `summary`, `topics`, `repo`, `occurred_at`,
   - blank `user_message`, `assistant_message`, `command_or_input`,
     `output_excerpt`,
   - set a `pruned: true` boolean and `pruned_at` timestamp.
3. Cap pruned-document size at 4 KB; documents whose `summary` exceeds
   the cap are truncated with an ellipsis marker.
4. Uses the Meili SDK `update_documents` task in batches of 1000,
   awaits the task to terminal state per batch, fails the run on any
   `failed` task.
5. Per-index pruning idempotence via `pruned == true` filter.
6. Emits `retention.meili_prune` events; updates `retention_sweeps`.
7. The query path (`cortex-api` keyword lane) MUST already tolerate
   missing body fields — the relevance lane was rewritten in phase6g
   to do this; this task adds an explicit compatibility test.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` §Meilisearch,
  `docs/specs/19-retention.md`, `docs/specs/08-fulltext-indexer.md`.
- Affected code: NEW `crates/cortex-retention/src/meili_prune.rs`,
  small additions in `crates/cortex-fulltext/src/meili_client.rs`
  (`prune_document_body` helper).
- Breaking change: NO. The keyword lane already handles missing bodies.
- User benefit: Meili index size stays bounded; faster keyword recall
  on long-lived repos; one pipeline-wide retention story across the
  three retrieval lanes.
