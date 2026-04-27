# Proposal: phase2h_dashboard_decision_chain_and_graph_richness

## Why

The Decision register's lineage view (`views-mid.jsx` lines 119–131 — `supersede-chain` with current/old nodes connected by arrows) cannot render because `/v1/dashboard/decisions` does not return the `chain` field today. The Decisions stats grid in the design also shows a "Cited in last 7d" tile that needs a `cites_7d` aggregate the backend does not derive.

The Graph explorer is currently limited to Session → Turn → ToolCall nodes (12 hits, hardcoded). The design's `MOCK.graph` (`data.js` lines 182–210) shows the richer schema: Decision, Law, Violation, Analysis, Artifact nodes with REFERENCES / OBSERVED_IN / OF / PRODUCED edges. The renderer is ready (legend already lists all kinds, color map is in place); what is missing is the backend producing those nodes from real envelopes.

Source: `phase2_dashboard/tasks.md` items 1.5 (decision detail + chain) + 1.9 (graph payload); `gui/assets/views-mid.jsx` lines 119–131; `gui/assets/data.js` lines 182–210.

## What Changes

### `/v1/dashboard/decisions` — chain + cites
- Each `DecisionRow` gains:
  - `chain: Vec<ChainNode>` where `ChainNode = { id, title, state }` and `state ∈ {"current","old"}`. Built by walking the `supersedes` field backwards through the captured decisions.
  - `cites_7d: u64` — count of distinct turns/decisions in the last 7 days whose body references the decision id (regex match in the lane's text).
- When no chain is found, the field is an empty array (not null).
- A new `/v1/dashboard/decisions/{id}` endpoint returns the same shape as a list row plus the full Markdown body (the existing `rationale` field is the clipped 600-char preview).

### `/v1/dashboard/graph` — richer node set
- Extend the synthetic graph to include:
  - Decision nodes — one per `kind=decision` envelope captured in the session window.
  - Law / Violation nodes — one per `kind=law_violation` envelope, plus a Law node it references when the violation carries a `law_id`.
  - Analysis nodes — one per `kind=analysis` envelope; edge `PRODUCED` to any Decision id it cites.
  - Artifact nodes — one per distinct file path seen in `tool_call:Edit` / `tool_call:Write` payloads.
- Edge types: `CONTAINS`, `INVOKED`, `WROTE`, `READ`, `REFERENCES`, `OBSERVED_IN`, `OF`, `PRODUCED`.
- Layout: keep the existing column-based placement (Session at x=60, Turns at x=220, ToolCalls at x=420, Artifacts at x=540, Decisions/Analyses at x=720, Laws/Violations at x=540 lower band), match `MOCK.graph` coordinates as the seed.
- Cap node count at 60 to keep the SVG readable; surface a `truncated: true` flag when exceeded.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (decision + graph shapes); `phase2_dashboard` §1.5 + §1.9 close.
- Affected code: `crates/cortex-api/src/dashboard.rs` (extend decisions handler, add detail handler, expand graph handler), `gui/src/lib/api.ts` (extend types), `gui/src/views/Decisions.tsx` (consume `chain` — already wired in phase2d to render gracefully), `gui/src/views/Graph.tsx` (no code change expected — already iterates whatever the backend returns).
- Breaking change: NO — fields are additive.
- Depends on: nothing (phase2d renders chain when present).
- User benefit: Decision lineage becomes visible, Graph explorer shows the full provenance of an event instead of a flat 3-layer hierarchy.
