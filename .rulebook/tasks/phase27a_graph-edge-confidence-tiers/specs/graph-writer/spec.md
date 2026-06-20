# Graph Writer — edge confidence

## ADDED Requirements

### Requirement: Edge confidence tier
The graph writer SHALL stamp every projected edge with a `confidence`
tier of `Extracted`, `Inferred`, or `Ambiguous`, and MAY attach a
`confidence_score` in `[0.0, 1.0]`. Deterministic AST-derived edges MUST
be `Extracted`; analyzer- or LLM-derived edges MUST be `Inferred` or
`Ambiguous`. The property MUST be additive and back-compatible (a missing
value is treated as unknown).

#### Scenario: AST-derived edge is Extracted
Given a Rust source file parsed by tree-sitter
When the projector emits a `defines` edge for a discovered function
Then the edge confidence is `Extracted` with score 1.0

#### Scenario: Inferred edge carries a sub-1.0 score
Given an analyzer/LLM-derived `relates_to` edge between two nodes
When the projector emits the edge
Then the edge confidence is `Inferred` with a score below 1.0

### Requirement: Graph lane weights by confidence
The query graph lane SHALL rank edges by their confidence tier, ranking
`Extracted` above `Inferred` above `Ambiguous`, so low-trust edges do not
dominate retrieval.

#### Scenario: Low-confidence edge ranked below a proven edge
Given two candidate edges to the same target, one `Extracted` and one `Ambiguous`
When the graph lane scores them for a query
Then the `Extracted` edge ranks above the `Ambiguous` edge
