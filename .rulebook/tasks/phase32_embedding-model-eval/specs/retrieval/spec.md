# Retrieval

## ADDED Requirements

### Requirement: Embedding-model choice is benchmarked against real golden data before being changed

Any proposed change to Cortex's embedding model SHALL be benchmarked
against the real (non-placeholder) golden eval set on recall@5, MRR@10,
latency, and cost before being adopted.

#### Scenario: Recommendation is backed by measured deltas, not impression

Given a candidate embedding model and the real golden eval set both exist
When the candidate is benchmarked against the current production model
Then a recommendation (stay or migrate) MUST be backed by measured recall/MRR/latency/cost deltas, not a qualitative impression
