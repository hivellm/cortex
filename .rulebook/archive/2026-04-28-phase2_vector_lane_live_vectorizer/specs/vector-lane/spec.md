# Live vector lane spec

## ADDED Requirements

### Requirement: VectorLane returns semantic neighbours
The cortex-api `VectorLane` implementation in production SHALL return KNN matches against the active Vectorizer collection. Returning an empty list (or the keyword lane's hits relabelled) is forbidden when the lane probe succeeded at boot.

#### Scenario: semantically similar prompts surface the same prior turn
Given a prior turn discussing "fix the embedder routing bug" was indexed in the Vectorizer collection
When `/v1/query` arrives with `query: "the embedder is sending docs to the wrong collection"`
Then `results.snippets` MUST contain that prior turn within the top 10
And `debug.lanes.vector_ms` MUST be greater than zero (proving the lane actually executed)

### Requirement: VectorLane fails open
A failure to reach Vectorizer SHALL NOT crash `cortex-api` or break `/v1/query`. The lane MUST return `LaneError::Transport`, the orchestrator continues with the remaining lanes, and `debug.errors["vector"]` is populated.

#### Scenario: Vectorizer down → fail-open
Given Vectorizer is unreachable on the configured URL
When `/v1/query` arrives with a non-empty `query`
Then the response MUST be HTTP 200
And `debug.errors` MUST contain a `"vector"` entry
And the keyword lane's results MUST still flow through

### Requirement: SDK is the only client
The integration SHALL use the official `vectorizer-sdk` crate, not a hand-rolled HTTP client (per anti-pattern `don-t-ship-a-bespoke-http-client-when-an-in-tree-pipeline-crate-already-drives-that-endpoint`).

#### Scenario: sibling client used
Given the workspace already declares `vectorizer-sdk` for `cortex-embedder-worker`
When `cortex-api` integrates the live vector lane
Then the integration MUST take a workspace dependency on `vectorizer-sdk`
And no parallel HTTP client targeting Vectorizer endpoints MAY be introduced
