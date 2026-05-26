# Spec: Meili archival pruner

## ADDED Requirements

### Requirement: Body pruning at 90 days

For every document in `cortex_turns` and `cortex_tool_calls` whose
`occurred_at < now - prune_after_days` (default 90 days), the runner MUST:
- blank the body fields (`user_message`, `assistant_message`,
  `command_or_input`, `output_excerpt`),
- truncate `summary` to ≤ `summary_cap_bytes` (default 4096) preserving a
  trailing ellipsis marker when truncation occurred,
- set `pruned = true` and `pruned_at = now` (RFC-3339).

The document MUST NOT be deleted.

#### Scenario: 91-day-old turn loses its body but keeps its summary
Given a `cortex_turns` document with `occurred_at = now - 91d`, a 12 KB
  `assistant_message`, and a 200-byte `summary`
When `cortex-retention meili-prune` runs
Then the document MUST still exist in Meili
And `assistant_message` MUST be empty
And `summary` MUST equal the original 200-byte summary
And `pruned` MUST equal `true`.

### Requirement: Idempotence

Pruned documents MUST be excluded from the next pruner run. Re-running
without `--rebuild` MUST report `documents_pruned = 0` if no new
documents have crossed the boundary.

#### Scenario: re-run on the same data is a no-op
Given a previous prune touched 500 documents
When the runner re-executes one minute later
Then it MUST report 0 newly pruned documents
And no Meili task MUST be issued.

### Requirement: Keyword lane compatibility

After pruning, the keyword retrieval lane MUST still surface the
document on queries that match its `summary` or its metadata. A
post-prune query that previously matched only the body MUST instead
match the summary or fail to match at all — never crash the lane.

#### Scenario: keyword lane handles missing body
Given a turn whose body has been blanked but whose summary contains "auth bug"
When the keyword lane runs the query "auth bug"
Then the document MUST be returned in the result set
And the lane response MUST NOT contain a "missing field" error.

### Requirement: Hard cap on summary size

`summary` length after pruning MUST be ≤ `summary_cap_bytes`. Documents
whose summary exceeds the cap MUST be truncated with an ellipsis marker
indicating the truncation.

#### Scenario: oversize summary is truncated
Given a document whose original summary is 8 KB
And `summary_cap_bytes = 4096`
When the pruner runs
Then `summary.len()` MUST be ≤ 4096
And the last 3 bytes MUST be the ellipsis marker.
