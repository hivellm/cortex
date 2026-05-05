# Spec: fine-grained backend search

## ADDED Requirements

### Requirement: Per-backend MCP tools expose unfiltered search

The system SHALL expose three new MCP tools — `cortex_vector_search`, `cortex_keyword_search`, `cortex_graph_query` — each backed by a corresponding cortex-api endpoint that proxies to a single backend (Vectorizer / Meili / Nexus respectively) without fusion or cross-lane mixing.

#### Scenario: vector search returns raw cosine scores against the named collection

Given a Vectorizer collection `cortex.consolidation.fp32` with 50 vectors
And a query string "auth rewrite"
When the operator calls `cortex_vector_search` with `{ collection, query_text, k: 10 }`
Then the response MUST contain at most 10 hits ordered by descending score
And each hit MUST carry `event_id`, `score`, `repo`, `kind`, `occurred_at`
And the keyword + graph lanes MUST NOT be invoked

#### Scenario: keyword search forwards Meili filters verbatim

Given a Meili index `cortex_consolidations` with rows for repos "cortex" and "tml"
And a filter `repo = "cortex"`
When the operator calls `cortex_keyword_search` with `{ index, q, filter, limit: 5 }`
Then the response MUST contain only rows where `repo == "cortex"`
And the response MUST echo `processing_time_ms` and `estimated_total_hits` from Meili

#### Scenario: graph neighbors mode walks the Nexus graph from a node id

Given a Nexus node `event:01HSESSA` with 3 direct neighbors via `EMITTED` edges
When the operator calls `cortex_graph_query` with `{ mode: "neighbors", node_id: "event:01HSESSA", depth: 1 }`
Then the response MUST contain 3 neighbor nodes plus the source node (4 nodes total)
And it MUST contain 3 edges with `kind = "EMITTED"`

#### Scenario: cypher mode is disabled by default

Given the env var `CORTEX_GRAPH_CYPHER_ENABLED` is unset OR not "1"
When the operator calls `cortex_graph_query` with `{ mode: "cypher", statement: "MATCH (n) RETURN n LIMIT 1" }`
Then the response MUST be HTTP 403 with reason `cypher_disabled`
And no Cypher MUST be forwarded to Nexus

#### Scenario: depth cap on neighbors mode is enforced

Given a `cortex_graph_query` call with `{ mode: "neighbors", node_id, depth: 10 }`
When the handler validates the input
Then it MUST reject the request with HTTP 400 and reason `depth_exceeds_cap`
And the cap MUST be 5 by default

### Requirement: Per-backend tools share the cortex_query response budget

The system SHALL enforce the same `MCP_RESPONSE_HARD_CAP` payload limit on the three new endpoints as `cortex_query` does (spec 11). Overflow MUST surface a structured `budget_exceeded` soft-error.

#### Scenario: vector search overflow surfaces a budget soft-error

Given a `cortex_vector_search` request with `k = 100` against a collection where each payload averages 1 KB
And the resulting payload exceeds `MCP_RESPONSE_HARD_CAP`
When the handler serialises the response
Then it MUST return a `budget_exceeded` soft-error
And the soft-error MUST carry `payload_bytes`, `transport_cap_bytes`, `suggested_k`

### Requirement: Tools accept either an embedding or a query string for vector mode

`cortex_vector_search` SHALL accept exactly one of `query_text` (server embeds) or `query_vector` (raw f32 array). Neither-or-both MUST be rejected.

#### Scenario: rejects when both query_text and query_vector are present

Given a `cortex_vector_search` call with both `query_text` and `query_vector` populated
When the handler validates the input
Then it MUST return HTTP 400 with reason `bad_input`

#### Scenario: rejects when neither is present

Given a `cortex_vector_search` call with neither `query_text` nor `query_vector`
When the handler validates the input
Then it MUST return HTTP 400 with reason `bad_input`
