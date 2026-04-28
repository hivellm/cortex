# 05 — GUI and API surface

## API routes ([crates/cortex-api/src/](../../../crates/cortex-api/src/))

### Query API ([http.rs](../../../crates/cortex-api/src/http.rs))

| Method | Path           | Purpose                                                              |
|--------|----------------|----------------------------------------------------------------------|
| POST   | `/v1/query`    | Hybrid retrieval (vector + keyword + graph) + RRF fusion (spec 11)   |
| GET    | `/v1/status`   | Health + dependency probes                                           |

### Dashboard backend ([dashboard.rs:56-76](../../../crates/cortex-api/src/dashboard.rs#L56-L76))

| Method | Path                                                  | Source           |
|--------|-------------------------------------------------------|------------------|
| GET    | `/v1/dashboard/overview`                              | Aggregator       |
| GET    | `/v1/dashboard/timeline/recent`                       | Synap + archive  |
| GET    | `/v1/dashboard/timeline/stream` (SSE)                 | Synap pub/sub    |
| GET    | `/v1/dashboard/memory`                                | Meili `cortex-{repo}-misc` + memory loader |
| GET    | `/v1/dashboard/decisions`                             | Meili `cortex-{repo}-decisions` |
| GET    | `/v1/dashboard/decisions/{id}`                        | Meili by id      |
| GET    | `/v1/dashboard/laws`                                  | Derived from `law_violation` envelopes (commit `3f8bbe3`) |
| GET    | `/v1/dashboard/violations`                            | Meili            |
| GET    | `/v1/dashboard/analyses`                              | Meili            |
| GET    | `/v1/dashboard/tools/stats`                           | Aggregator       |
| GET    | `/v1/dashboard/graph`                                 | Nexus via GraphLane |
| GET    | `/v1/dashboard/sessions`                              | Aggregator       |
| GET    | `/v1/dashboard/trust`                                 | Stub (governance not built) |
| GET    | `/v1/dashboard/conversations`                         | Sessions + turns |
| GET    | `/v1/dashboard/conversations/{session_id}`            | Session detail + Sonnet analyzer (commit `a62fcbd`) |
| GET    | `/v1/dashboard/handoffs`                              | Handoff envelopes |

**Reading:** the dashboard backend is **the** read-side aggregator. It reads Meili directly for several views (decisions, laws, memory, analyses) — the [meili_loader](../../../crates/cortex-api/src/) was the unblocker (commit `1dc867f`). This means dashboard UX quality is gated on Meili coverage, which today is single-repo. Phase4a fixes both at once.

### MCP server ([crates/cortex-mcp-server/](../../../crates/cortex-mcp-server/))

Three tools exposed: `cortexQuery`, `cortexPreThinking`, `cortexStatus`. Names and schemas are identifier-safe + camelCase (commit `9f14ef6`, learning [recorded](../../../.rulebook/learnings/2026-04-27T16-35-31-mcp-server-tool-descriptors-must-match-the-spec-contract-names-without-dots-schema-fields-in-camelcase.md)).

## GUI ([gui/src/](../../../gui/src/))

Stack: Electron + React + TypeScript + Vite + React Query + Cytoscape (graph). Single-window app; per-view drawers; design system in `gui/src/atoms/` with sparkline + density controls.

### Views ([gui/src/views/](../../../gui/src/views/))

| View              | File                       | State                                                                            |
|-------------------|----------------------------|----------------------------------------------------------------------------------|
| Timeline          | [Timeline.tsx](../../../gui/src/views/Timeline.tsx)        | ✅ Live SSE; stream controls; sparkline grid (commit `94b6c56`).                  |
| Memory            | [Memory.tsx](../../../gui/src/views/Memory.tsx)            | ✅ Browser; Meili-backed.                                                         |
| Decisions         | [Decisions.tsx](../../../gui/src/views/Decisions.tsx)      | ✅ Register + per-project filter (commit `6908ed2`).                              |
| Laws              | [Laws.tsx](../../../gui/src/views/Laws.tsx)                | 🟡 Reads law catalogue derived from `law_violation` events — there is no live engine producing those yet (see [06](06-governance-gap.md)). |
| Tools             | [Tools.tsx](../../../gui/src/views/Tools.tsx)              | 🟡 Tool-call analytics; backend exists but quality-of-data depends on phase4a coverage. |
| Graph             | [Graph.tsx](../../../gui/src/views/Graph.tsx)              | ✅ Cytoscape renderer with Nexus backend (commit `ce9af59`); shallow because of edge poverty. |
| Analysis          | [Analysis.tsx](../../../gui/src/views/Analysis.tsx)        | 🟡 Lists deep analyses but the workflow that produces them (spec 15) is not built. |
| Conversations     | [Conversations.tsx](../../../gui/src/views/Conversations.tsx) | ✅ Session list + Sonnet summary pane (commit `a62fcbd`).                         |
| Handoffs          | [Handoffs.tsx](../../../gui/src/views/Handoffs.tsx)        | ✅ Handoff envelopes browse.                                                      |

### Cross-cutting GUI features

- **Tweaks drawer** (theme/accent/density/sidebar) — commit `8d7d617`, closes phase2e.
- **Inspector** — richer per-event drawer, commit `5bdaf94`.
- **SSE reconnect ladder** — commit `ac10b5e`.
- **Mobile / narrow viewport** — drawer covers full width, horizontal scroll locked, commit `2dc4832`.

### GUI gaps tracked as tasks

| Gap                                       | Task                                                                                                |
|-------------------------------------------|-----------------------------------------------------------------------------------------------------|
| Multiple Cortex backends (local + remote) | [phase3_gui_multi_connection](../../../.rulebook/tasks/phase3_gui_multi_connection/proposal.md)     |
| Tool-call hash + content preview          | [phase3_tool_call_hash_preview](../../../.rulebook/tasks/phase3_tool_call_hash_preview/)             |
| Auth on dashboard endpoints               | [phase2f_dashboard_auth](../../../.rulebook/tasks/phase2f_dashboard_auth/)                          |
| Enriched tool/tool-call metrics           | [phase2g_dashboard_enriched_metrics](../../../.rulebook/tasks/phase2g_dashboard_enriched_metrics/)   |
| Decision chain + graph richness in GUI    | [phase2h_dashboard_decision_chain_and_graph_richness](../../../.rulebook/tasks/phase2h_dashboard_decision_chain_and_graph_richness/) |

**Reading:** the GUI is *more* complete than spec-16's 🟡 status would suggest — design parity is largely closed, the remaining gaps are operational (multi-backend, auth, deeper graph/decision navigation).

## Observed UX issues / opportunities

1. **Single-backend hard-coded** ([gui/src/lib/api.ts](../../../gui/src/lib/api.ts) → `http://127.0.0.1:15011`). Defeats the cross-team / multi-environment use case. Phase3 task in flight.
2. **No retrieval-quality UI feedback.** When `/v1/query` returns a thin bundle (because Meili has no Rulebook docs, say), the GUI does not surface "this answer is degraded — coverage is at 33%". Users will conclude Cortex is bad rather than that the index is incomplete.
3. **No "doctor" panel.** A single dashboard view that runs `cortex doctor consistency` and renders the result table would catch fan-out drift the moment it appears, instead of weeks later.
4. **Trust score view exists in routes (`/v1/dashboard/trust`) but is a stub.** Either remove it from the GUI until governance lands, or label it explicitly as "coming with spec 14".
5. **No retrieval audit trail in the GUI.** The architecture envisions a `query_id` carried through the bundle and surfaced for retrieval-quality analysis ([spec 12](../../specs/12-pre-thinking-injection.md)). The GUI doesn't expose it yet.

## Plugin / MCP UX

The `cortex-plugin/` Claude Code plugin (spec 18, 🟢) registers `mcp__cortex__*` tools. The user's `MEMORY.md` confirms the daemon is the integration boundary the user actually uses — not the standalone Electron GUI.

Implication: MCP tool quality (descriptors, response shape, error messages) is at least as important as GUI polish, because that's the surface the user himself touches every session.
