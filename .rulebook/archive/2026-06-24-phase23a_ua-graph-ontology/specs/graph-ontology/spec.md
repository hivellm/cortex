# Graph Ontology

## ADDED Requirements

### Requirement: Adopted node and edge taxonomy
The system SHALL extend its graph node-kind and edge-kind vocabulary with the adopted
subset of the Understand-Anything taxonomy (code, non-code, and knowledge groups) as
catalogued in `docs/analysis/understand-anything/03-ontology-mapping.md`, while
retaining all Cortex-only node and edge kinds.

#### Scenario: New node kind is representable
Given the node-kind vocabulary has been extended
When a graph node of kind `table` is constructed
Then it serializes and deserializes without loss and is rejected by no schema check

#### Scenario: Cortex-only kinds preserved
Given the edge-kind vocabulary has been extended with UA edges
When an edge of kind `SUPERSEDES` is constructed
Then it remains valid and is not removed by the extension

### Requirement: Backward-compatible relation aliasing
The system MUST map the existing relations `IMPORTS_FILE`, `DOCUMENTED_BY`, and `CITES`
onto the adopted edge kinds `imports`, `documents`, and `cites` without breaking reads
of previously persisted graph data.

#### Scenario: Legacy relation resolves
Given a persisted edge stored under the legacy relation `IMPORTS_FILE`
When the graph is read after the vocabulary extension
Then the edge resolves to the adopted `imports` kind and no read error occurs

### Requirement: Bitemporal edge shape
The system SHALL represent every graph edge with `source`, `target`, `type`,
`direction`, `weight`, optional `description`, `provenance`, and the bitemporal fields
`valid_from` and optional `valid_to`.

#### Scenario: Edge carries bitemporal envelope
Given an edge is created at time T
When it is persisted
Then `valid_from` equals T and `valid_to` is absent until the edge is closed
