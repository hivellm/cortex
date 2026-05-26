# Spec: Snippet body capture

## ADDED Requirements

### Requirement: snippet.text carries body, not symbol

`POST /v1/query` snippet rows MUST carry up to 1 KiB of the
resolved body in `text`. The `symbol` field carries the symbol
name; the two MUST NOT be identical for any non-empty body.

#### Scenario: keyword hit returns body bytes
Given an indexed source file `crates/cortex-api/src/orchestrator.rs`
  whose body contains "fan out parallel lanes"
When the operator queries `intent=free_search, query="fan out
  parallel lanes"`
Then the matching snippet's `text` MUST contain the phrase
And `text` MUST NOT equal the file path or symbol name.

### Requirement: pre-thinking bundle renders body excerpt

The Sonnet pre-thinking bundle (`cortex_pre_thinking`) MUST
render each snippet as `path:line — <first 200 chars of body>…`.
A bundle whose snippets render as `path:artifact — path` MUST be
treated as a regression and fail the bundle-renderer tests.

#### Scenario: bundle has substance
Given the pre-thinking pipeline runs against the prompt "retention
  sweep idempotence"
When the bundle is generated
Then at least one snippet line MUST contain "retention" or "sweep"
  in the body excerpt (not in the path)
And no snippet line MUST be of the form `*:artifact — *`.

### Requirement: graceful fall-back when CAS is slow

When the body cannot be resolved within 50 ms or 3 CAS hops, the
projector MUST fall back to `text = symbol || path` and stamp
`extras.body_truncated_reason = "cas_slow"`.

#### Scenario: timeout falls back without erroring
Given a CAS store that consistently takes 80 ms per `get`
When the lane projects 100 hits
Then the response MUST still complete within the query budget
And every snippet MUST carry `extras.body_truncated_reason =
  "cas_slow"`.
