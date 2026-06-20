# SCIP precise extraction

## ADDED Requirements

### Requirement: SCIP-derived precise edges
The system SHALL ingest a SCIP symbol index and emit precise
`calls`/`references`/`defines` edges resolved to exact symbol
definitions, tagged with `Extracted` confidence, superseding heuristic
tree-sitter edges for files the index covers.

#### Scenario: Ambiguous call resolved precisely
Given two functions named `foo` in different crates and a SCIP index
When SCIP ingestion emits the `calls` edge for a call to `foo`
Then the edge targets the exact definition the index resolves, tagged `Extracted`

### Requirement: No dangling SCIP edges
The system SHALL stub any unresolved SCIP target as a `scip_external`
node so that every emitted edge has both endpoints present.

#### Scenario: Unresolved external symbol is stubbed
Given a SCIP occurrence referencing a symbol with no in-graph definition
When the resolver processes it
Then a `scip_external` node is created and the edge is anchored to it (not dropped)
