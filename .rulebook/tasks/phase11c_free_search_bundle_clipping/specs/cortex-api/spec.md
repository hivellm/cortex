# cortex-api — free_search bundle clipping

## ADDED Requirements

### Requirement: free_search obeys section-cap byte budget
The `/v1/query` handler with `intent = free_search` SHALL clip its response so that the serialised JSON length never exceeds the configured `budget_bytes`.

#### Scenario: Default budget applied
Given a `free_search` call with no explicit `budget_bytes`
When `/v1/query` returns
Then the response JSON is ≤ 32768 bytes
And per-snippet `text` is clipped to ≤ ~1024 bytes

#### Scenario: Caller-supplied budget honoured
Given a `free_search` call with `budget_bytes = 8192`
When `/v1/query` returns
Then the response JSON is ≤ 8192 bytes

#### Scenario: Overflow note when results clipped
Given the unclipped result set would exceed the budget
When the formatter clips the bundle
Then the bundle ends with a marker `<!-- N more results clipped -->`
And `N` equals the number of hits removed

### Requirement: budget_bytes is plumbed end-to-end through MCP
The `cortex_query` MCP tool SHALL accept an optional `budget_bytes` parameter (default `32768`) and forward it to the `/v1/query` request body.

#### Scenario: MCP schema exposes budget_bytes
Given a caller introspects the `cortex_query` tool schema
When they read its parameters
Then `budget_bytes` is present as an optional integer with a default of `32768`

#### Scenario: MCP forwards budget_bytes verbatim
Given an MCP `cortex_query` call with `budget_bytes = 8192`
When the adapter builds the HTTP request to `/v1/query`
Then the request body carries `budget_bytes: 8192`

### Requirement: MCP server guards against transport overflow
The `cortex_query` MCP adapter SHALL count the serialised response length and return a structured error when it would exceed the transport's hard limit.

#### Scenario: Response within cap
Given the serialised response is below the transport limit
When the adapter returns it to the caller
Then the result is delivered as-is

#### Scenario: Response exceeds cap
Given the serialised response exceeds the transport limit
When the adapter detects the overflow
Then it returns `BudgetExceeded { hits_returned, total_hits, suggested_budget_bytes }`
And the side-file dump path is NOT triggered
