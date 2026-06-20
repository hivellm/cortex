# GraphRAG community summaries

## ADDED Requirements

### Requirement: Community summary consolidation grain
The system SHALL produce a summary for each graph community as a new
consolidation grain, sourced from that community's nodes, edges, and god
nodes, and stored using the existing consolidation envelope. Summaries
SHALL follow the community hierarchy levels.

#### Scenario: A community yields a summary
Given community detection has assigned nodes to a community
When the community consolidation grain runs for that community
Then a summary envelope is produced describing the subsystem and its key nodes

### Requirement: Global (architecture-level) query route
The system SHALL route architecture-level queries to a map-reduce over
community summaries rather than the per-chunk fusion lane, returning a
synthesized answer within the configured byte budget.

#### Scenario: Architecture question uses community summaries
Given community summaries exist
When a caller asks an architecture-level question (e.g. "what are the subsystems")
Then the orchestrator answers from community summaries, not raw per-chunk hits

### Requirement: Community-aware entity dedup
The system SHALL merge near-duplicate graph entities using string
similarity gated by an entropy filter, and SHALL boost merge confidence
when both candidates share a community, preferring a stable survivor id.

#### Scenario: Same-community homonyms handled by boost
Given two nodes with similar labels in the same community
When the dedup pass evaluates them
Then the shared-community boost is applied before deciding the merge
