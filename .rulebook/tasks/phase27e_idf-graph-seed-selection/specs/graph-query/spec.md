# Graph lane seed selection

## ADDED Requirements

### Requirement: IDF-gated graph seed selection
The graph lane SHALL weight candidate seed nodes by per-token inverse
document frequency over node labels, and SHALL only seed a BFS from nodes
scoring above a configured fraction of the top score, so common-token
nodes do not displace specific matches.

#### Scenario: Specific match beats a common hub
Given a query whose terms match both a rare node `FooBarService` and a common node `error`
When the graph lane selects BFS seeds
Then `FooBarService` is seeded and the low-scoring `error` node is excluded by the gate

### Requirement: Graph path and compare primitives
The system SHALL expose MCP tools to return the shortest path between two
nodes and to compare the neighborhoods of two nodes.

#### Scenario: Shortest path between two symbols
Given two nodes that are connected in the graph
When a caller invokes the path tool with both node ids
Then it returns an ordered path with the intermediate hops
