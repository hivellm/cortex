# Graph Communities

## ADDED Requirements

### Requirement: Leiden community detection over the graph
The system SHALL run a community-detection pass (Leiden) over the
architecture subgraph on a periodic schedule and persist a stable
`community_id` (and hierarchy `level`) on each participating node. The
pass MUST be idempotent for an unchanged graph (deterministic seed) and
MUST NOT block ingestion.

#### Scenario: Nodes receive a community id
Given a graph with code edges (calls/imports/defines) across several modules
When the community-detection worker runs
Then each participating node has a stable `community_id` assigned

#### Scenario: Re-run is stable for an unchanged graph
Given the graph has not changed since the last detection run
When the worker runs again
Then the community assignment is identical to the previous run

### Requirement: Human-scale communities
The detection pass SHALL split any community larger than a configured
share of the graph and SHALL exclude super-hub nodes from partitioning,
re-attaching them by neighbor majority, so no single community dominates.

#### Scenario: Oversized community is split
Given an initial partition where one community exceeds the size threshold
When the worker post-processes the partition
Then that community is recursively re-partitioned into smaller communities

### Requirement: Communities are queryable
The system SHALL expose communities (with their hub nodes and
cross-community edges) through an MCP tool.

#### Scenario: List communities
Given community detection has run
When a caller invokes the communities MCP tool
Then it returns the communities with their god nodes and cross-community edges
