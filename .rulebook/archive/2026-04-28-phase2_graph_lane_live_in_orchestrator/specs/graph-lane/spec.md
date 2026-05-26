# Live graph lane spec

## ADDED Requirements

### Requirement: Orchestrator's graph lane talks to Nexus
The cortex-api orchestrator SHALL drive its `GraphLane` through the same Nexus client instance that the dashboard already uses, when `CORTEX_NEXUS_URL` is set and the boot probe succeeds.

#### Scenario: shared Nexus client
Given `CORTEX_NEXUS_URL` is set and Nexus is reachable
When `cortex-api` starts
Then the boot wiring MUST construct exactly one `NexusClient` instance
And both `DashboardState` and the orchestrator's `GraphLane` MUST share that instance via `Arc`

#### Scenario: graph_neighbors populates for indexed nodes
Given Nexus contains at least one node matching `(:Decision { id: "DEC-0042" })` and one outgoing edge
When `/v1/query` arrives with `query: "DEC-0042"` and `intent: pre_change_context`
Then `results.graph_neighbors` MUST contain at least one entry whose `from` or `to` references `DEC-0042`
And `debug.lanes.graph_ms` MUST be greater than zero

### Requirement: Cypher templates only
The lane SHALL execute only pre-registered Cypher templates and SHALL reject arbitrary client-supplied Cypher. Param substitution MUST go through the SDK's parametrised path, not string interpolation.

#### Scenario: unknown template rejected
Given the lane's whitelist contains `neighbours_1hop` and `neighbours_2hop`
When a `GraphRequest` arrives with `template = "drop_database"`
Then the lane MUST return `LaneError::InvalidRequest`
And no Cypher MUST be sent to Nexus

### Requirement: Fail-open on Nexus failure
A failure to reach Nexus SHALL NOT break `/v1/query`. The lane MUST return `LaneError::Transport` and `debug.errors["graph"]` populates.

#### Scenario: Nexus down → fail-open
Given Nexus is unreachable
When `/v1/query` arrives
Then the response MUST be HTTP 200
And `results.graph_neighbors` MAY be empty
And `debug.errors["graph"]` MUST carry a non-empty message
