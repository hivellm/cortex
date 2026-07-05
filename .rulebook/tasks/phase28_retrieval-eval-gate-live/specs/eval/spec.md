# Eval

## ADDED Requirements

### Requirement: Retrieval eval runs nightly against real golden data
The retrieval eval suite SHALL run on a nightly schedule against golden
fixtures containing real (non-placeholder) event IDs, and SHALL fail
the workflow when recall@5 or MRR@10 regresses below the locked
baseline.

#### Scenario: Nightly regression fails the workflow
Given the nightly eval workflow runs
When retrieval quality on the real golden set drops below the locked baseline floor
Then the workflow run fails and surfaces which query/intent regressed
