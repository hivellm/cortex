## 1. Live graph lane impl
- [x] 1.1 New `cortex_api::nexus_graph_lane::NexusGraphLane { client }` implementing `cortex_api::GraphLane`
- [x] 1.2 Whitelist of 4 safe Cypher templates: `edge_artifact_touched_neighbours`, `decision_supersedes_chain`, `turn_analysis_decision_chain`, `law_violations_last_30d` (one per orchestrator strategy)
- [x] 1.3 `GraphLane::query` selects template by name, binds `$q` from params, executes via SDK `execute_cypher`, returns `LaneHit`s; unknown templates → `LaneError::Rejected` (closest fit in existing `LaneError` enum) before any Cypher dispatch
- [x] 1.4 Translate Nexus rows to `LaneHit` with `extras["edge_from" / "edge_to" / "edge_type" / "hops"]` matching the contract `derive_graph_neighbors` reads; score by `1.0 / max(hops, 1)`
- [x] 1.5 SDK error path returns `LaneError::Transport` (orchestrator's fail-open turns this into `debug.errors["graph"]`, response stays HTTP 200)

## 2. Boot-time wiring
- [x] 2.1 `cortex-api/src/main.rs` reuses the `Arc<NexusClient>` already built for `DashboardState` — single TCP session, two consumers
- [x] 2.2 When the client is `None` (env unset / probe fail), `MemoryGraphLane` stays as the fallback
- [x] 2.3 Logs `"live graph lane: NexusGraphLane wired"` (live) or `"nexus client unavailable; graph lane stays on MemoryGraphLane"` (fallback) at info on startup

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (spec-07 `## Read path` section: live lane, template whitelist, Cypher shape, hit projection, failure handling, configuration)
- [x] 3.2 Write tests covering the new behavior (5 unit tests for template whitelist + row projection, 3 integration tests for orchestrator overlay / unknown-template rejection / fail-open through orchestrator)
- [x] 3.3 Run tests and confirm they pass — 99 cortex-api tests green
