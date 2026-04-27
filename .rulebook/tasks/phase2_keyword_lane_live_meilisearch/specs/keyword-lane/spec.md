# Live keyword lane spec

## ADDED Requirements

### Requirement: KeywordLane filters by query
The cortex-api `KeywordLane` implementation in production SHALL filter hits by the request's `query` string. Returning the same hit set regardless of `query` is forbidden — `MemoryKeywordLane` is a test double and MUST NOT ship as the default lane when a Meili endpoint is reachable.

#### Scenario: distinct queries return distinct hit sets
Given the live Meili index is seeded with envelopes covering the term "embedder" and the term "vectorizer"
When `/v1/query` arrives with `query: "embedder"` and again with `query: "vectorizer"`
Then the two responses MUST differ in `results.snippets` (set or order)
And neither response MUST be empty when the term has indexed coverage

#### Scenario: nonsense query returns at most-empty results
Given the live Meili index is seeded but no envelope contains the term "asdfqwerty"
When `/v1/query` arrives with `query: "asdfqwerty"`
Then `results.snippets` MUST be empty (or a small set of fuzzy-but-relevant matches if `typoTolerance` is enabled)
And the response MUST NOT include any snippet that does not lexically or fuzzily match the query

### Requirement: Lane source label is accurate
A `LaneHit` produced by the keyword lane SHALL carry `source = "keyword"`. The orchestrator's `source` field MUST reflect the lane that produced the hit, not the lane that ran first.

#### Scenario: keyword-only hit has keyword source
Given the vector lane returns zero hits and the keyword lane returns one hit
When the orchestrator fuses the lanes
Then the surviving hit's `source` field MUST equal `"keyword"`

### Requirement: Live lane fails open
A failure to reach Meili (timeout, 5xx, malformed response) SHALL NOT crash `cortex-api` or break `/v1/query`. The lane MUST return `LaneError::Transport`, the orchestrator continues with whatever the other lanes produced, and the query response carries `debug.errors["keyword"]` populated.

#### Scenario: Meili down → fail-open
Given Meili is unreachable on the configured URL
When `/v1/query` arrives
Then the response MUST be HTTP 200
And `debug.errors` MUST contain a `"keyword"` entry with a non-empty message
And `results` MAY be empty without violating the contract
