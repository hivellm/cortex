# Graph

## ADDED Requirements

### Requirement: Semantic graph projection runs continuously without triggering sustained Nexus write stalls
The graph projection pipeline SHALL run continuously in production
(not permanently disabled behind a feature flag) while respecting a
rate limit that prevents nexus#12-class sustained-write stalls.

#### Scenario: 24-hour soak under continuous write load
Given the projection scheduler runs under continuous write load for at least 24 hours
When Nexus write latency is monitored throughout
Then no sustained-stall condition occurs and the graph subgraph used by community detection is non-empty
