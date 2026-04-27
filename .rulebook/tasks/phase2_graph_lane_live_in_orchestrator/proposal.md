# Proposal: phase2_graph_lane_live_in_orchestrator

## Why

`cortex-api/src/main.rs:106-128` already builds a `nexus_sdk::NexusClient` from `CORTEX_NEXUS_URL` / `NEXUS_URL` and stitches it into `DashboardState`. But the orchestrator at line 92 still gets `Arc::new(MemoryGraphLane::new())` — an empty test double. Result: the dashboard's graph view can talk to Nexus, but `/v1/query`'s graph lane never does. Every probed query in 2026-04-27's audit returned `graph_neighbors=0`.

The asymmetry is unjustified: the same `NexusClient` instance can drive both the dashboard graph endpoint and the orchestrator's graph lane. The trait `GraphLane` already exists; only the impl is missing.

The blocker `phase1_graph_writer_nexus_compat` (already in flight, WIP committed at `a5f8ab0`) addresses upstream writes silently dropping under Nexus 1.15. Once that lands, the graph contains the data we want to query — but if the read side still goes through `MemoryGraphLane`, the writer fix is invisible to pre-thinking.

## What Changes

- New `cortex-graph::lane::NexusGraphLane { client: Arc<NexusClient>, default_template }` implementing `cortex_api::GraphLane`:
  - Maps `GraphRequest { template, params, k }` to a Nexus parametrised Cypher read (templates pre-registered server-side, only safe templates allowed).
  - Translates each returned row to a `LaneHit` carrying `from`, `to`, `relation`, `hops`, and a `score` derived from path length / template-supplied weight.
- `cortex-api/src/main.rs` reuses the Nexus client built for `DashboardState` for the orchestrator's graph lane. Single client, two consumers.
- When `CORTEX_NEXUS_URL` is unset or the probe fails, fall back to `MemoryGraphLane` (preserve dev workflow).
- The `/v1/query` `results.graph_neighbors` field starts to populate for queries that resolve to nodes the bootstrap walker has stamped (per `phase1_graph_writer_nexus_compat`'s typed-API path).

## Impact

- Affected specs: spec-07 (graph writer — read path is missing), spec-11 (lane wiring).
- Affected code:
  - `crates/cortex-graph/src/lib.rs` (re-export the new lane impl)
  - new: `crates/cortex-graph/src/lane.rs`
  - `crates/cortex-api/src/main.rs` (share the client, swap the lane)
  - tests in `cortex-graph/tests/lane.rs`
- Breaking change: NO (additive)
- User benefit: pre-thinking starts surfacing semantically related code/decision/turn nodes (1-hop / 2-hop neighbours) — the third leg of the RRF tripod the orchestrator was designed for.

## Dependencies

- Soft-blocked by `phase1_graph_writer_nexus_compat` (WIP at `a5f8ab0`): until the writer actually persists, the read path will return empty. The lane impl itself can land first; integration tests require a populated graph.

## Source

2026-04-27 audit; orchestrator main.rs visibly diverges from the dashboard wiring it shares a struct with.
