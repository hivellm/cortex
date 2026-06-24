# Extraction Contract

## ADDED Requirements

### Requirement: Deterministic facts gate LLM annotation
The system SHALL produce graph structure (node existence, file paths, line ranges,
imports/exports, call edges) from a deterministic extractor, and the LLM annotation
phase MUST NOT originate structural facts — it may only attach descriptive fields and
semantic edges between nodes that already exist.

#### Scenario: LLM cannot invent a node
Given a deterministic fact set that does not contain node `function:x:y`
When the LLM annotation emits a node `function:x:y`
Then the reconciliation gate rejects that node and logs the rejection to the audit envelope

#### Scenario: Semantic edge endpoints must exist
Given an annotated edge whose target id is in neither the fact set nor the existing graph
When the reconciliation gate runs
Then the edge is rejected

### Requirement: Import-count reconciliation
The system MUST assert that, per file, the number of emitted import edges equals the
deterministic import count, and on mismatch SHALL re-run annotation once before
accepting the deterministic import edges directly.

#### Scenario: Omitted import is backfilled
Given the deterministic extractor finds 10 imports in a file
And the LLM annotation emits only 9 import edges
When the reconciliation gate runs
Then a mismatch is detected and the deterministic import edges are used

### Requirement: Significance filter
The system SHALL emit a function or class node only when the symbol is at least 10
lines long or is exported.

#### Scenario: Small private helper is filtered
Given an 8-line non-exported helper function
When extraction runs
Then no node is emitted for that helper

#### Scenario: Small exported helper is kept
Given an 8-line exported helper function
When extraction runs
Then a node is emitted for that helper
