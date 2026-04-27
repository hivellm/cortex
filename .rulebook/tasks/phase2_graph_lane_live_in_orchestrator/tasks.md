## 1. Live graph lane impl
- [ ] 1.1 New `cortex-graph::lane::NexusGraphLane { client, templates }` implementing `cortex_api::GraphLane`
- [ ] 1.2 Whitelist of safe Cypher templates registered at boot (1-hop neighbours, 2-hop neighbours, by-id lookup)
- [ ] 1.3 `GraphLane::query` selects template by name, validates params, executes against the SDK, returns `LaneHit`s
- [ ] 1.4 Translate Nexus rows to `GraphNeighbor { from, to, relation, hops }` and rank by hop distance + path weight
- [ ] 1.5 Connection-error path returns `LaneError::Transport`

## 2. Boot-time wiring
- [ ] 2.1 In `cortex-api/src/main.rs`, build the `NexusClient` once and share between `DashboardState` and the orchestrator
- [ ] 2.2 When the client is `None` (env unset / probe fail), keep `MemoryGraphLane` as the fallback
- [ ] 2.3 Log the active mode at info on startup (live vs memory)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (extend spec-07 with a `## Read path` section)
- [ ] 3.2 Write tests: unit tests against a wiremock Nexus, integration test driving the orchestrator with a live (or mocked) populated graph
- [ ] 3.3 Run tests and confirm they pass
