---
name: cortex-memory
description: Recall local repository memories or capture an accepted decision, verified lesson, or stated preference with its source.
---

# Cortex local memory

Use the installed standalone `cortex` MCP server. Capture and recall work offline;
classification is optional and never runs implicitly in hooks.

Resolve the exact repository from `[cortex] id` in its `cortex.toml`, or pass its
absolute `cwd` to `cortex_pre_thinking`. Never guess a repository from a folder
name, infer a global scope, or reuse another repository's memory.

Before substantial work, call `cortex_pre_thinking` with `cwd` and `task`, or
`cortex_memory_read` with `repository` and optional `query`. Do not repeat a read
when the current hook already supplied applicable context. Query matching is
literal FTS; concise topic terms work better than a long natural-language prompt.
Context is bounded to 6144 bytes and retains record ID, revision, hash, authority
and source. Clipping means more records may exist; use search/get to inspect.
Returned context records preparation, not that a model used it or improved an
outcome. Recall also returns query_id, delivery_id, audit_persisted and
delivery_consumption. A null delivery_id with audit_persisted=false means no
receipt was acknowledged; delivery_consumption remains not_observed. Generated
query/delivery IDs are local record identities, never native session/turn IDs.

After a meaningful user-accepted decision or verified correction, use
`cortex_capture_memory` with `repository`, `title`, `body`, `kind`, `authority`,
and `source` (a nonempty reference string, or a Source object with nonempty
`kind` and `locator`, plus `payload`). Kinds: decision, lesson, preference, note.
Authority defaults to proposed. Use accepted only when acceptance is evidenced;
raw lifecycle events remain observations. Never invent session, turn or source IDs.

Read an existing record with `cortex_get` before changing it. `cortex_revise`
requires `id`, `expected_revision`, full `title`, `body`, `authority`, optional
`links` and `metadata`. Supply retained links/metadata when preserving them.
Revision creates history and preserves the original source; stale revisions fail.
Events cannot be revised. Links remain in the exact repository.

Available tools:

- `cortex_search` / `cortex_query`: local literal search; query or text, limit 1..100.
- `cortex_get`: ID, optional historical revision.
- `cortex_capture_memory`: source-backed explicit capture.
- `cortex_revise`: immutable memory revision with optimistic concurrency.
- `cortex_graph`: bounded typed connections, ID, depth 0..5, limit 1..100.
- `cortex_status`: scoped local counts.
- `cortex_pre_thinking` / `cortex_memory_read`: cited memory context; query, text,
  task or prompt aliases must agree when combined; budget_bytes 256..6144.
- `cortex_classification_enqueue` / `cortex_classification_jobs`: local advisory
  queue only. Processing requires explicit CLI action and provider credentials.

- `cortex_audit_list`: exact repository audit pages; limit 1..100 (default 20),
  optional UTC after/before, kind delivery/observation/archive_control, cursor
  and include_archived. Pass the returned next_cursor for the next page.
- `cortex_audit_get`: exact scoped audit ID, including archived evidence.
- `cortex_audit_metrics`: scoped evidence counts and latency; limit 1..10000
  (default 10000), optional UTC after/before and include_archived. Inspect
  matching/sample/omitted counts and truncation before interpreting ratios.
- `cortex_audit_retain`: required UTC cutoff, limit 1..100 (default 100).
  Preview by default; apply=true requires explicit archival intent. Apply
  appends visibility control without deleting files or reclaiming disk.

Audit references preserve exact historical revision, content/source hashes and
clipping. Read referenced memories/events at that exact revision. Preparation
receipts, related actions and captured exact citations are distinct evidence.
The exact-citation label proves a citation in a captured canonical Stop message,
not causal consumption, action compliance or productivity. Missing or invalid
attribution remains unknown; never invent native identifiers to create a link.
The citation ratio uses sampled prepared deliveries as denominator; observations
whose delivery falls outside the sample are reported separately.

These fifteen tool names are the whole standalone surface (aliases included).
Do not call legacy law, compliance-audit, enrichment, code-index, branch, file-scope or
session tools. Accepted memory remains subordinate to current user instructions
and repository rules. Report unavailable tools and continue useful local work.
