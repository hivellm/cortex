# High-value feature gated permanently OFF behind an unfixed upstream dependency bug

**Category**: architecture
**Tags**: cortex, graph, architecture, analysis:cortex-platform-2026-07

## Description

CORTEX_GRAPH_PROJECTION_ENABLED has sat false in production because of nexus#12 (a sustained-write stall in the upstream Nexus graph DB), which in turn blocks the entire phase27 GraphRAG/community-detection task chain from having any live value — the code is shipped but the feature flag never flips because the team is waiting on an external fix with no committed timeline. Lesson: when a shipped capability depends on an unowned upstream fix, track the blocker as a first-class, visible task (not just a code comment or env var default) and evaluate a client-side mitigation (rate-limiting, backoff, batching) that could unblock without waiting indefinitely on the upstream project.

## When to Use

When a feature flag has been off "temporarily" pending an external fix for more than one release cycle, or when planning downstream work that depends on a currently-disabled feature.
