# Spec: Memory kind filter

## ADDED Requirements

### Requirement: ?kind= filters the memory lane

`GET /v1/dashboard/memory?kind=<canonical>` MUST filter the lane
projection so the response carries ONLY rows whose `kind` matches
one of the requested values. Repeating the parameter ORs the
kinds. The pagination cap (`?limit=`) MUST apply after the filter
so the operator gets `limit` rows of the requested kind.

#### Scenario: requesting decisions returns only decisions
Given the lane carries 26 decisions and 7000 tool_calls
When the GUI calls `/v1/dashboard/memory?kind=decision&limit=10`
Then the body MUST contain 10 rows
And every row MUST carry `kind=decision`.

#### Scenario: multi-kind ORs
Given the lane carries decisions + analyses + tool_calls
When the GUI calls
  `/v1/dashboard/memory?kind=decision&kind=analysis&limit=20`
Then every returned row MUST have kind in {decision, analysis}
And no `tool_call` rows MUST appear.

### Requirement: unknown kinds reject with 400

When a `?kind=` value is not in the canonical set
(`turn|tool_call|agent_call|memory|decision|analysis|law_violation|
knowledge|learning`), the handler MUST return HTTP 400 with body
`{"error": "unknown_kind", "received": "<value>"}`.

#### Scenario: typo rejected
Given the request `/v1/dashboard/memory?kind=desicion` (typo)
When it lands on cortex-api
Then the response MUST be HTTP 400
And the body MUST be `{"error":"unknown_kind","received":"desicion"}`.
